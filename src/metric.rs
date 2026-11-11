//! The values peakmark gates on, and how they are read and written.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One gated value of a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Raw,
    Gzip,
    Brotli,
    PeakMemory,
    GrowthPerRun,
}

impl Metric {
    pub const ALL: [Metric; 5] = [
        Metric::Raw,
        Metric::Gzip,
        Metric::Brotli,
        Metric::PeakMemory,
        Metric::GrowthPerRun,
    ];

    /// The rule id a finding about this metric carries.
    pub fn rule(self) -> &'static str {
        match self {
            Metric::Raw => "size/raw",
            Metric::Gzip => "size/gzip",
            Metric::Brotli => "size/brotli",
            Metric::PeakMemory => "mem/peak",
            Metric::GrowthPerRun => "mem/growth",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Metric::Raw => "raw size",
            Metric::Gzip => "gzip size",
            Metric::Brotli => "brotli size",
            Metric::PeakMemory => "peak memory",
            Metric::GrowthPerRun => "memory growth per run",
        }
    }

    /// Sizes on the wire read in decimal units, memory in binary ones.
    pub fn format(self, bytes: u64) -> String {
        match self {
            Metric::Raw | Metric::Gzip | Metric::Brotli => format_decimal(bytes),
            Metric::PeakMemory | Metric::GrowthPerRun => format_binary(bytes),
        }
    }
}

/// A value per metric: what was measured, what the budget allows, or what the
/// baseline holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Values {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gzip: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brotli: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub growth_per_run: Option<u64>,
}

impl Values {
    pub fn get(&self, metric: Metric) -> Option<u64> {
        match metric {
            Metric::Raw => self.raw,
            Metric::Gzip => self.gzip,
            Metric::Brotli => self.brotli,
            Metric::PeakMemory => self.peak_memory,
            Metric::GrowthPerRun => self.growth_per_run,
        }
    }

    /// `raw 812.4 kB, gzip 301.2 kB, …` for every value that is present.
    pub fn summary(&self) -> String {
        Metric::ALL
            .iter()
            .filter_map(|&metric| {
                self.get(metric)
                    .map(|value| format!("{} {}", metric.label(), metric.format(value)))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Reads `258 kB`, `24 MiB` or a plain number of bytes.
pub fn parse_bytes(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| format!("`{text}` is not a size"))?;
    let factor = match unit.trim() {
        "" | "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        other => {
            return Err(format!(
                "unknown unit `{other}` in `{text}`; use B, kB, MB, KiB or MiB"
            ));
        }
    };
    Ok((number * factor).round() as u64)
}

/// Reads `1%` as `0.01`.
pub fn parse_percent(text: &str) -> Result<f64, String> {
    text.trim()
        .strip_suffix('%')
        .and_then(|number| number.trim().parse::<f64>().ok())
        .map(|percent| percent / 100.0)
        .ok_or_else(|| format!("`{text}` is not a percentage such as `1%`"))
}

pub fn format_decimal(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.1} kB", bytes as f64 / 1e3),
        _ => format!("{:.2} MB", bytes as f64 / 1e6),
    }
}

pub fn format_binary(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    match bytes {
        0..KIB => format!("{bytes} B"),
        KIB..MIB => format!("{:.1} KiB", bytes as f64 / KIB as f64),
        _ => format!("{:.1} MiB", bytes as f64 / MIB as f64),
    }
}

/// Today's date in UTC as `YYYY-MM-DD`.
pub fn today() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970")
        .as_secs();
    date_from_unix(seconds as i64)
}

/// Days since 1970 to a civil date (Howard Hinnant's `civil_from_days`).
fn date_from_unix(seconds: i64) -> String {
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
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
    fn parses_a_percentage() {
        assert_eq!(parse_percent("1%"), Ok(0.01));
        assert!(parse_percent("1").is_err());
    }

    #[test]
    fn formats_sizes_and_memory_in_their_own_units() {
        assert_eq!(Metric::Brotli.format(257_100), "257.1 kB");
        assert_eq!(Metric::Raw.format(1_240_000), "1.24 MB");
        assert_eq!(Metric::Raw.format(512), "512 B");
        assert_eq!(Metric::PeakMemory.format(24 * 1024 * 1024), "24.0 MiB");
        assert_eq!(Metric::GrowthPerRun.format(196_608), "192.0 KiB");
        assert_eq!(Metric::GrowthPerRun.format(0), "0 B");
    }

    #[test]
    fn converts_unix_seconds_to_a_date() {
        assert_eq!(date_from_unix(0), "1970-01-01");
        assert_eq!(date_from_unix(951_782_400), "2000-02-29");
        assert_eq!(date_from_unix(1_700_000_000), "2023-11-14");
    }
}
