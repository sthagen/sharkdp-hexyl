#[macro_use]
extern crate clap;

use std::convert::TryFrom;
use std::fs::File;
use std::io::{self, prelude::*, BufWriter, SeekFrom};
use std::num::{NonZeroI64, NonZeroU64, NonZeroU8};

use clap::builder::ArgPredicate;
use clap::{crate_name, crate_version, Arg, ArgAction, ColorChoice, Command};

use anyhow::{anyhow, Context, Result};

use const_format::formatcp;

use thiserror::Error as ThisError;

use terminal_size::terminal_size;

use hexyl::{Base, BorderStyle, Endianness, Input, PrinterBuilder};

#[cfg(test)]
mod tests;

const DEFAULT_BLOCK_SIZE: i64 = 512;

fn run() -> Result<()> {
    let command = Command::new(crate_name!())
        .color(ColorChoice::Auto)
        .max_term_width(90)
        .version(crate_version!())
        .about(crate_description!())
        .arg(
            Arg::new("FILE")
                .help("The file to display. If no FILE argument is given, read from STDIN."),
        )
        .arg(
            Arg::new("length")
                .short('n')
                .long("length")
                .num_args(1)
                .value_name("N")
                .help(
                    "Only read N bytes from the input. The N argument can also include a \
                     unit with a decimal prefix (kB, MB, ..) or binary prefix (kiB, MiB, ..), \
                     or can be specified using a hex number. \
                     The short option '-l' can be used as an alias.\n\
                     Examples: --length=64, --length=4KiB, --length=0xff",
                ),
        )
        .arg(
            Arg::new("bytes")
                .short('c')
                .long("bytes")
                .num_args(1)
                .value_name("N")
                .conflicts_with("length")
                .help("An alias for -n/--length"),
        )
        .arg(
            Arg::new("count")
                .short('l')
                .num_args(1)
                .value_name("N")
                .conflicts_with_all(["length", "bytes"])
                .hide(true)
                .help("Yet another alias for -n/--length"),
        )
        .arg(
            Arg::new("skip")
                .short('s')
                .long("skip")
                .num_args(1)
                .value_name("N")
                .help(
                    "Skip the first N bytes of the input. The N argument can also include \
                     a unit (see `--length` for details)\n\
                     A negative value is valid and will seek from the end of the file.",
                ),
        )
        .arg(
            Arg::new("block_size")
                .long("block-size")
                .num_args(1)
                .value_name("SIZE")
                .help(formatcp!(
                    "Sets the size of the `block` unit to SIZE (default is {}).\n\
                     Examples: --block-size=1024, --block-size=4kB",
                    DEFAULT_BLOCK_SIZE
                )),
        )
        .arg(
            Arg::new("nosqueezing")
                .short('v')
                .long("no-squeezing")
                .action(ArgAction::SetFalse)
                .help(
                    "Displays all input data. Otherwise any number of groups of output \
                     lines which would be identical to the preceding group of lines, are \
                     replaced with a line comprised of a single asterisk.",
                ),
        )
        .arg(
            Arg::new("color")
                .long("color")
                .num_args(1)
                .value_name("WHEN")
                .value_parser(["always", "auto", "never"])
                .default_value_if("plain", ArgPredicate::IsPresent, Some("never"))
                .default_value("always")
                .help(
                    "When to use colors. The auto-mode only displays colors if the output \
                     goes to an interactive terminal",
                ),
        )
        .arg(
            Arg::new("border")
                .long("border")
                .num_args(1)
                .value_name("STYLE")
                .value_parser(["unicode", "ascii", "none"])
                .default_value_if("plain", ArgPredicate::IsPresent, Some("none"))
                .default_value("unicode")
                .help(
                    "Whether to draw a border with Unicode characters, ASCII characters, \
                    or none at all",
                ),
        )
        .arg(Arg::new("plain").short('p').long("plain").action(ArgAction::SetTrue).help(
            "Display output with --no-characters, --no-position, --border=none, and --color=never.",
        ))
        .arg(
            Arg::new("no_chars")
                .long("no-characters")
                .action(ArgAction::SetFalse)
                .help("Do not show the character panel on the right."),
        )
        .arg(
            Arg::new("chars")
                .short('C')
                .long("characters")
                .overrides_with("no_chars")
                .action(ArgAction::SetTrue)
                .help("Show the character panel on the right. This is the default, unless --no-characters has been specified."),
        )
        .arg(
            Arg::new("no_position")
                .short('P')
                .long("no-position")
                .action(ArgAction::SetFalse)
                .help("Whether to display the position panel on the left."),
        )
        .arg(
            Arg::new("display_offset")
                .short('o')
                .long("display-offset")
                .num_args(1)
                .value_name("N")
                .help(
                    "Add N bytes to the displayed file position. The N argument can also \
                    include a unit (see `--length` for details)\n\
                    A negative value is valid and calculates an offset relative to the \
                    end of the file.",
                ),
        )
        .arg(
            Arg::new("panels")
                .long("panels")
                .num_args(1)
                .value_name("N")
                .help(
                    "Sets the number of hex data panels to be displayed. \
                    `--panels=auto` will display the maximum number of hex data panels \
                    based on the current terminal width. By default, hexyl will show \
                    two panels, unless the terminal is not wide enough for that.",
                ),
        )
        .arg(
            Arg::new("group_size")
                .short('g')
                .long("group-size")
                .alias("groupsize")
                .num_args(1)
                .value_name("N")
                .help(
                    "Number of bytes/octets that should be grouped together. \
                    Possible group sizes are 1, 2, 4, 8. The default is 1. You \
                    can use the '--endianness' option to control the ordering of \
                    the bytes within a group. '--groupsize' can be used as an \
                    alias (xxd-compatibility).",
                ),
        )
        .arg(
            Arg::new("endianness")
                .long("endianness")
                .num_args(1)
                .value_name("FORMAT")
                .value_parser(["big", "little"])
                .default_value("big")
                .help(
                    "Whether to print out groups in little-endian or big-endian \
                     format. This option only has an effect if the '--group-size' \
                     is larger than 1. '-e' can be used as an alias for \
                     '--endianness=little'.",
                ),
        )
        .arg(
            Arg::new("little_endian_format")
                .short('e')
                .action(ArgAction::SetTrue)
                .overrides_with("endianness")
                .hide(true)
                .help("An alias for '--endianness=little'."),
        )
        .arg(
            Arg::new("base")
                .short('b')
                .long("base")
                .num_args(1)
                .value_name("B")
                .help(
                    "Sets the base used for the bytes. The possible options are \
                    binary, octal, decimal, and hexadecimal. The default base \
                    is hexadecimal."
                )
        )
        .arg(
            Arg::new("terminal_width")
                .long("terminal-width")
                .num_args(1)
                .value_name("N")
                .conflicts_with("panels")
                .help(
                    "Sets the number of terminal columns to be displayed.\nSince the terminal \
                    width may not be an evenly divisible by the width per hex data column, this \
                    will use the greatest number of hex data panels that can fit in the requested \
                    width but still leave some space to the right.\nCannot be used with other \
                    width-setting options.",
                ),
        );

    let matches = command.get_matches();

    let stdin = io::stdin();

    let mut reader = match matches.get_one::<String>("FILE") {
        Some(filename) => Input::File(File::open(filename)?),
        None => Input::Stdin(stdin.lock()),
    };

    let block_size = matches
        .get_one::<String>("block_size")
        .map(|bs| {
            if let Some(hex_number) = try_parse_as_hex_number(bs) {
                return hex_number.map_err(|e| anyhow!(e)).and_then(|x| {
                    PositiveI64::new(x)
                        .ok_or_else(|| anyhow!("block size argument must be positive"))
                });
            }
            let (num, unit) = extract_num_and_unit_from(bs)?;
            if let Unit::Block { custom_size: _ } = unit {
                return Err(anyhow!(
                    "can not use 'block(s)' as a unit to specify block size"
                ));
            };
            num.checked_mul(unit.get_multiplier())
                .ok_or_else(|| anyhow!(ByteOffsetParseError::UnitMultiplicationOverflow))
                .and_then(|x| {
                    PositiveI64::new(x)
                        .ok_or_else(|| anyhow!("block size argument must be positive"))
                })
        })
        .transpose()?
        .unwrap_or_else(|| PositiveI64::new(DEFAULT_BLOCK_SIZE).unwrap());

    let skip_arg = matches
        .get_one::<String>("skip")
        .map(|s| {
            parse_byte_offset(s, block_size).context(anyhow!(
                "failed to parse `--skip` arg {:?} as byte count",
                s
            ))
        })
        .transpose()?;

    let skip_offset = if let Some(ByteOffset { kind, value }) = skip_arg {
        let value = value.into_inner();
        reader
            .seek(match kind {
                ByteOffsetKind::ForwardFromBeginning | ByteOffsetKind::ForwardFromLastOffset => {
                    SeekFrom::Current(value)
                }
                ByteOffsetKind::BackwardFromEnd => SeekFrom::End(value.checked_neg().unwrap()),
            })
            .map_err(|_| {
                anyhow!(
                    "Failed to jump to the desired input position. \
                     This could be caused by a negative offset that is too large or by \
                     an input that is not seek-able (e.g. if the input comes from a pipe)."
                )
            })?
    } else {
        0
    };

    let parse_byte_count = |s| -> Result<u64> {
        Ok(parse_byte_offset(s, block_size)?
            .assume_forward_offset_from_start()?
            .into())
    };

    let mut reader = if let Some(length) = matches
        .get_one::<String>("length")
        .or_else(|| matches.get_one::<String>("bytes"))
        .or_else(|| matches.get_one::<String>("count"))
        .map(|s| {
            parse_byte_count(s).context(anyhow!(
                "failed to parse `--length` arg {:?} as byte count",
                s
            ))
        })
        .transpose()?
    {
        Box::new(reader.take(length))
    } else {
        reader.into_inner()
    };

    let show_color = match matches.get_one::<String>("color").map(String::as_ref) {
        Some("never") => false,
        Some("always") => true,
        _ => supports_color::on(supports_color::Stream::Stdout)
            .map(|level| level.has_basic)
            .unwrap_or(false),
    };

    let border_style = match matches.get_one::<String>("border").map(String::as_ref) {
        Some("unicode") => BorderStyle::Unicode,
        Some("ascii") => BorderStyle::Ascii,
        _ => BorderStyle::None,
    };

    let &squeeze = matches.get_one::<bool>("nosqueezing").unwrap_or(&true);

    let show_char_panel = *matches.get_one::<bool>("no_chars").unwrap_or(&true)
        && !matches.get_one::<bool>("plain").unwrap_or(&false);

    let show_position_panel = *matches.get_one::<bool>("no_position").unwrap_or(&true)
        && !matches.get_one::<bool>("plain").unwrap_or(&false);

    let display_offset: u64 = matches
        .get_one::<String>("display_offset")
        .map(|s| {
            parse_byte_count(s).context(anyhow!(
                "failed to parse `--display-offset` arg {:?} as byte count",
                s
            ))
        })
        .transpose()?
        .unwrap_or(0);

    let max_panels_fn = |terminal_width: u64, base_digits: u64, group_size: u64| {
        let offset = if show_position_panel { 10 } else { 1 };
        let col_width = if show_char_panel {
            ((8 / group_size) * (base_digits * group_size + 1)) + 2 + 8
        } else {
            ((8 / group_size) * (base_digits * group_size + 1)) + 2
        };
        if (terminal_width - offset) / col_width < 1 {
            1
        } else {
            (terminal_width - offset) / col_width
        }
    };

    let base = if let Some(base) = matches.get_one::<String>("base")
    .map(|s| {
        if let Ok(base_num) = s.parse::<u8>() {
            match base_num {
                2 => Ok(Base::Binary),
                8 => Ok(Base::Octal),
                10 => Ok(Base::Decimal),
                16 => Ok(Base::Hexadecimal),
                _ => Err(anyhow!("The number provided is not a valid base. Valid bases are 2, 8, 10, and 16.")),
            }
        } else {
            match s.as_str() {
                "b" | "bin" | "binary" => Ok(Base::Binary),
                "o" | "oct" | "octal" => Ok(Base::Octal),
                "d" | "dec" | "decimal" => Ok(Base::Decimal),
                "x" | "hex" | "hexadecimal" => Ok(Base::Hexadecimal),
                _ => Err(anyhow!("The base provided is not valid. Valid bases are \"b\", \"o\", \"d\", and \"x\"."))
            }
        }
    }).transpose()? {
        base
    } else {
        Base::Hexadecimal
    };

    let base_digits = match base {
        Base::Binary => 8,
        Base::Octal => 3,
        Base::Decimal => 3,
        Base::Hexadecimal => 2,
    };

    let group_size = if let Some(group_size) = matches
        .get_one::<String>("group_size")
        .map(|s| {
            s.parse::<NonZeroU8>().map(u8::from).context(anyhow!(
                "Failed to parse `--group-size`/`-g` argument {:?} as unsigned nonzero integer",
                s
            ))
        })
        .transpose()?
    {
        if (group_size <= 8) && ((group_size & (group_size - 1)) == 0) {
            group_size
        } else {
            return Err(anyhow!(
                "Possible sizes for the `--group-size`/`-g` option are 1, 2, 4 or 8. "
            ));
        }
    } else {
        1
    };

    let terminal_width = terminal_size().map(|s| s.0 .0 as u64).unwrap_or(80);

    let panels = if matches.get_one::<String>("panels").map(String::as_ref) == Some("auto") {
        max_panels_fn(terminal_width, base_digits, group_size.into())
    } else if let Some(panels) = matches
        .get_one::<String>("panels")
        .map(|s| {
            s.parse::<NonZeroU64>().map(u64::from).context(anyhow!(
                "failed to parse `--panels` arg {:?} as unsigned nonzero integer",
                s
            ))
        })
        .transpose()?
    {
        panels
    } else if let Some(terminal_width) = matches
        .get_one::<String>("terminal_width")
        .map(|s| {
            s.parse::<NonZeroU64>().map(u64::from).context(anyhow!(
                "failed to parse `--terminal-width` arg {:?} as unsigned nonzero integer",
                s
            ))
        })
        .transpose()?
    {
        max_panels_fn(terminal_width, base_digits, group_size.into())
    } else {
        std::cmp::min(
            2,
            max_panels_fn(terminal_width, base_digits, group_size.into()),
        )
    };

    let little_endian_format = *matches.get_one::<bool>("little_endian_format").unwrap();
    let endianness = matches.get_one::<String>("endianness");
    let endianness = match (
        endianness.map(|s| s.as_ref()).unwrap(),
        little_endian_format,
    ) {
        (_, true) | ("little", _) => Endianness::Little,
        ("big", _) => Endianness::Big,
        _ => unreachable!(),
    };
    let stdout = io::stdout();
    let mut stdout_lock = BufWriter::new(stdout.lock());

    let mut printer = PrinterBuilder::new(&mut stdout_lock)
        .show_color(show_color)
        .show_char_panel(show_char_panel)
        .show_position_panel(show_position_panel)
        .with_border_style(border_style)
        .enable_squeezing(squeeze)
        .num_panels(panels)
        .group_size(group_size)
        .with_base(base)
        .endianness(endianness)
        .build();
    printer.display_offset(skip_offset + display_offset);
    printer.print_all(&mut reader).map_err(|e| anyhow!(e))?;

    Ok(())
}

fn main() {
    let result = run();

    if let Err(err) = result {
        if let Some(io_error) = err.downcast_ref::<io::Error>() {
            if io_error.kind() == ::std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
        }
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct NonNegativeI64(i64);

impl NonNegativeI64 {
    pub fn new(x: i64) -> Option<Self> {
        if x.is_negative() {
            None
        } else {
            Some(Self(x))
        }
    }

    pub fn into_inner(self) -> i64 {
        self.0
    }
}

impl From<NonNegativeI64> for u64 {
    fn from(x: NonNegativeI64) -> u64 {
        u64::try_from(x.0)
            .expect("invariant broken: NonNegativeI64 should contain a non-negative i64 value")
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct PositiveI64(i64);

impl PositiveI64 {
    pub fn new(x: i64) -> Option<Self> {
        if x < 1 {
            None
        } else {
            Some(Self(x))
        }
    }

    pub fn into_inner(self) -> i64 {
        self.0
    }
}

impl From<PositiveI64> for u64 {
    fn from(x: PositiveI64) -> u64 {
        u64::try_from(x.0)
            .expect("invariant broken: PositiveI64 should contain a positive i64 value")
    }
}

#[derive(Debug, PartialEq)]
enum Unit {
    Byte,
    Kilobyte,
    Megabyte,
    Gigabyte,
    Terabyte,
    Kibibyte,
    Mebibyte,
    Gibibyte,
    Tebibyte,
    /// a customizable amount of bytes
    Block {
        custom_size: Option<NonZeroI64>,
    },
}

impl Unit {
    const fn get_multiplier(self) -> i64 {
        match self {
            Self::Byte => 1,
            Self::Kilobyte => 1000,
            Self::Megabyte => 1_000_000,
            Self::Gigabyte => 1_000_000_000,
            Self::Terabyte => 1_000_000_000_000,
            Self::Kibibyte => 1 << 10,
            Self::Mebibyte => 1 << 20,
            Self::Gibibyte => 1 << 30,
            Self::Tebibyte => 1 << 40,
            Self::Block {
                custom_size: Some(size),
            } => size.get(),
            Self::Block { custom_size: None } => DEFAULT_BLOCK_SIZE,
        }
    }
}

const HEX_PREFIX: &str = "0x";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ByteOffsetKind {
    ForwardFromBeginning,
    ForwardFromLastOffset,
    BackwardFromEnd,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ByteOffset {
    value: NonNegativeI64,
    kind: ByteOffsetKind,
}

#[derive(Clone, Debug, ThisError)]
#[error(
    "negative offset specified, but only positive offsets (counts) are accepted in this context"
)]
struct NegativeOffsetSpecifiedError;

impl ByteOffset {
    fn assume_forward_offset_from_start(
        &self,
    ) -> Result<NonNegativeI64, NegativeOffsetSpecifiedError> {
        let &Self { value, kind } = self;
        match kind {
            ByteOffsetKind::ForwardFromBeginning | ByteOffsetKind::ForwardFromLastOffset => {
                Ok(value)
            }
            ByteOffsetKind::BackwardFromEnd => Err(NegativeOffsetSpecifiedError),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, ThisError)]
enum ByteOffsetParseError {
    #[error("no character data found, did you forget to write it?")]
    Empty,
    #[error("no digits found after sign, did you forget to write them?")]
    EmptyAfterSign,
    #[error(
        "found {0:?} sign after hex prefix ({:?}); signs should go before it",
        HEX_PREFIX
    )]
    SignFoundAfterHexPrefix(char),
    #[error("{0:?} is not of the expected form <pos-integer>[<unit>]")]
    InvalidNumAndUnit(String),
    #[error("{0:?} is a valid unit, but an integer should come before it")]
    EmptyWithUnit(String),
    #[error("invalid unit {0:?}")]
    InvalidUnit(String),
    #[error("failed to parse integer part")]
    ParseNum(#[source] std::num::ParseIntError),
    #[error("count multiplied by the unit overflowed a signed 64-bit integer; are you sure it should be that big?")]
    UnitMultiplicationOverflow,
}

fn parse_byte_offset(n: &str, block_size: PositiveI64) -> Result<ByteOffset, ByteOffsetParseError> {
    use ByteOffsetParseError::*;

    let (n, kind) = process_sign_of(n)?;

    let into_byte_offset = |value| {
        Ok(ByteOffset {
            value: NonNegativeI64::new(value).unwrap(),
            kind,
        })
    };

    if let Some(hex_number) = try_parse_as_hex_number(n) {
        return hex_number.map(into_byte_offset)?;
    }

    let (num, mut unit) = extract_num_and_unit_from(n)?;
    if let Unit::Block { custom_size: None } = unit {
        unit = Unit::Block {
            custom_size: Some(
                NonZeroI64::new(block_size.into_inner()).expect("PositiveI64 was zero"),
            ),
        };
    }

    num.checked_mul(unit.get_multiplier())
        .ok_or(UnitMultiplicationOverflow)
        .and_then(into_byte_offset)
}

/// Takes a string containing a base-10 number and an optional unit, and returns them with their proper types.
/// The unit must directly follow the number (e.g. no whitespace is allowed between them).
/// When no unit is given, [Unit::Byte] is assumed.
/// When the unit is [Unit::Block], it is returned without custom size.
/// No normalization is performed, that is "1024" is extracted to (1024, Byte), not (1, Kibibyte).
fn extract_num_and_unit_from(n: &str) -> Result<(i64, Unit), ByteOffsetParseError> {
    use ByteOffsetParseError::*;
    if n.is_empty() {
        return Err(Empty);
    }
    match n.chars().position(|c| !c.is_ascii_digit()) {
        Some(unit_begin_idx) => {
            let (n, raw_unit) = n.split_at(unit_begin_idx);
            let unit = match raw_unit.to_lowercase().as_str() {
                "" => Unit::Byte, // no "b" => Byte to allow hex nums with units
                "kb" => Unit::Kilobyte,
                "mb" => Unit::Megabyte,
                "gb" => Unit::Gigabyte,
                "tb" => Unit::Terabyte,
                "kib" => Unit::Kibibyte,
                "mib" => Unit::Mebibyte,
                "gib" => Unit::Gibibyte,
                "tib" => Unit::Tebibyte,
                "block" | "blocks" => Unit::Block { custom_size: None },
                _ => {
                    return if n.is_empty() {
                        Err(InvalidNumAndUnit(raw_unit.to_string()))
                    } else {
                        Err(InvalidUnit(raw_unit.to_string()))
                    }
                }
            };
            let num = n.parse::<i64>().map_err(|e| {
                if n.is_empty() {
                    EmptyWithUnit(raw_unit.to_owned())
                } else {
                    ParseNum(e)
                }
            })?;
            Ok((num, unit))
        }
        None => {
            // no unit part
            let num = n.parse::<i64>().map_err(ParseNum)?;
            Ok((num, Unit::Byte))
        }
    }
}

/// Extracts a [ByteOffsetKind] based on the sign at the beginning of the given string.
/// Returns the input string without the sign (or an equal string if there wasn't any sign).
fn process_sign_of(n: &str) -> Result<(&str, ByteOffsetKind), ByteOffsetParseError> {
    use ByteOffsetParseError::*;
    let mut chars = n.chars();
    let next_char = chars.next();
    let check_empty_after_sign = || {
        if chars.clone().next().is_none() {
            Err(EmptyAfterSign)
        } else {
            Ok(chars.as_str())
        }
    };
    match next_char {
        Some('+') => Ok((
            check_empty_after_sign()?,
            ByteOffsetKind::ForwardFromLastOffset,
        )),
        Some('-') => Ok((check_empty_after_sign()?, ByteOffsetKind::BackwardFromEnd)),
        None => Err(Empty),
        _ => Ok((n, ByteOffsetKind::ForwardFromBeginning)),
    }
}

/// If `n` starts with a hex prefix, its remaining part is returned as some number (if possible),
/// otherwise None is returned.
fn try_parse_as_hex_number(n: &str) -> Option<Result<i64, ByteOffsetParseError>> {
    use ByteOffsetParseError::*;
    n.strip_prefix(HEX_PREFIX).map(|num| {
        let mut chars = num.chars();
        match chars.next() {
            Some(c @ '+') | Some(c @ '-') => {
                return if chars.next().is_none() {
                    Err(EmptyAfterSign)
                } else {
                    Err(SignFoundAfterHexPrefix(c))
                }
            }
            _ => (),
        }
        i64::from_str_radix(num, 16).map_err(ParseNum)
    })
}
