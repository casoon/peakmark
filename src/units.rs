//! Sizes and percentages as written in the configuration and as printed in
//! reports.

use std::fmt;

/// A size or percentage that does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The text is not a number with an optional unit.
    NotASize(String),
    /// The unit after the number is not one peakmark knows.
    UnknownUnit {
        /// The unit as written.
        unit: String,
        /// The whole value as written.
        text: String,
    },
    /// The text is not a number followed by `%`.
    NotAPercentage(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotASize(text) => write!(f, "`{text}` is not a size"),
            Self::UnknownUnit { unit, text } => write!(
                f,
                "unknown unit `{unit}` in `{text}`; use B, kB, MB, KiB or MiB"
            ),
            Self::NotAPercentage(text) => {
                write!(f, "`{text}` is not a percentage such as `1%`")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Reads `258 kB`, `24 MiB` or a plain number of bytes.
///
/// # Errors
///
/// Fails if the number or the unit is not understood.
pub fn parse_bytes(text: &str) -> Result<u64, ParseError> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| ParseError::NotASize(text.to_string()))?;
    let factor = match unit.trim() {
        "" | "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        other => {
            return Err(ParseError::UnknownUnit {
                unit: other.to_string(),
                text: text.to_string(),
            });
        }
    };
    Ok((number * factor).round() as u64)
}

/// Reads `1%` as `0.01`.
///
/// # Errors
///
/// Fails if the text is not a number followed by `%`.
pub fn parse_percent(text: &str) -> Result<f64, ParseError> {
    text.trim()
        .strip_suffix('%')
        .and_then(|number| number.trim().parse::<f64>().ok())
        .map(|percent| percent / 100.0)
        .ok_or_else(|| ParseError::NotAPercentage(text.to_string()))
}

/// Bytes in decimal units: `812 B`, `257.1 kB`, `1.24 MB`.
#[must_use]
pub fn format_decimal(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.1} kB", bytes as f64 / 1e3),
        _ => format!("{:.2} MB", bytes as f64 / 1e6),
    }
}

/// Bytes in binary units: `812 B`, `192.0 KiB`, `24.0 MiB`.
#[must_use]
pub fn format_binary(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    match bytes {
        0..KIB => format!("{bytes} B"),
        KIB..MIB => format!("{:.1} KiB", bytes as f64 / KIB as f64),
        _ => format!("{:.1} MiB", bytes as f64 / MIB as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes_in_decimal_and_binary_units() {
        assert_eq!(parse_bytes("258 kB"), Ok(258_000));
        assert_eq!(parse_bytes("24 MiB"), Ok(24 * 1024 * 1024));
        assert_eq!(parse_bytes("1.5KiB"), Ok(1536));
        assert_eq!(parse_bytes("0"), Ok(0));
        assert!(parse_bytes("12 parsecs").is_err());
        assert!(parse_bytes("large").is_err());
    }

    #[test]
    fn names_the_unit_it_does_not_know() {
        assert_eq!(
            parse_bytes("12 parsecs").unwrap_err().to_string(),
            "unknown unit `parsecs` in `12 parsecs`; use B, kB, MB, KiB or MiB"
        );
    }

    #[test]
    fn parses_a_percentage() {
        assert_eq!(parse_percent("1%"), Ok(0.01));
        assert!(parse_percent("1").is_err());
    }

    #[test]
    fn formats_decimal_and_binary_units() {
        assert_eq!(format_decimal(512), "512 B");
        assert_eq!(format_decimal(257_100), "257.1 kB");
        assert_eq!(format_decimal(1_240_000), "1.24 MB");
        assert_eq!(format_binary(0), "0 B");
        assert_eq!(format_binary(196_608), "192.0 KiB");
        assert_eq!(format_binary(24 * 1024 * 1024), "24.0 MiB");
    }
}
