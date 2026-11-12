//! The values peakmark gates on, and how they are named and printed.

use serde::{Deserialize, Serialize};

use crate::units::{format_binary, format_decimal};

/// One gated value of a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// Bytes of the file.
    Raw,
    /// Bytes after gzip.
    Gzip,
    /// Bytes after brotli.
    Brotli,
    /// Linear memory held after the last run of a scenario.
    PeakMemory,
    /// What each run after the first adds to linear memory.
    GrowthPerRun,
}

impl Metric {
    /// Every metric, in report order.
    pub const ALL: [Metric; 5] = [
        Metric::Raw,
        Metric::Gzip,
        Metric::Brotli,
        Metric::PeakMemory,
        Metric::GrowthPerRun,
    ];

    /// The rule id a finding about this metric carries.
    #[must_use]
    pub fn rule(self) -> &'static str {
        match self {
            Metric::Raw => "size/raw",
            Metric::Gzip => "size/gzip",
            Metric::Brotli => "size/brotli",
            Metric::PeakMemory => "mem/peak",
            Metric::GrowthPerRun => "mem/growth",
        }
    }

    /// The name used in sentences, such as `peak memory`.
    #[must_use]
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
    #[must_use]
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
    /// Bytes of the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Bytes after gzip.
    pub gzip: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Bytes after brotli.
    pub brotli: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Linear memory held after the last run.
    pub peak_memory: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Memory added per run after the first.
    pub growth_per_run: Option<u64>,
}

impl Values {
    /// The value for `metric`, if there is one.
    #[must_use]
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
    #[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn summarises_only_the_values_that_are_present() {
        let values = Values {
            raw: Some(512),
            peak_memory: Some(65_536),
            ..Values::default()
        };
        assert_eq!(values.summary(), "raw size 512 B, peak memory 64.0 KiB");
    }
}
