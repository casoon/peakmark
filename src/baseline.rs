//! The accepted values per module, kept in the repository next to the
//! configuration, and the log of why each baseline was set.
//!
//! The file has a format version. Version 2 stores the memory values per
//! scenario and the measurement contract they were taken under; version 1
//! (peakmark 0.1) stored only the module values and is still read. Any other
//! version is refused with [`Error::BaselineVersion`].

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::compress::SETTINGS;
use crate::date::today;
use crate::error::Error;
use crate::measure::Measured;
use crate::metric::Values;
use crate::runner::{RUNNER_SCHEMA, Unobserved, WARMUP_RUNS};
use crate::snapshot::Snapshot;

/// The baseline file, next to the configuration.
pub const BASELINE_FILE: &str = "peakmark.baseline.json";
/// The append-only log of every accepted baseline, next to the configuration.
pub const HISTORY_FILE: &str = "peakmark.history.jsonl";
/// The format version `save` writes.
pub const VERSION: u32 = 2;
/// The oldest format version `load` still reads.
pub const OLDEST_VERSION: u32 = 1;

/// The contents of [`BASELINE_FILE`].
#[derive(Clone, Serialize, Deserialize)]
pub struct Baseline {
    version: u32,
    /// The accepted entry per module name.
    pub modules: BTreeMap<String, Entry>,
}

/// One accepted measurement of a module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// The day it was accepted, `YYYY-MM-DD`.
    pub accepted: String,
    /// Why the baseline changed.
    pub reason: String,
    /// The accepted values: the module-wide summary, and the basis for budgets.
    pub values: Values,
    /// What the values were measured under. Absent in entries from format 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<Contract>,
    /// The memory values per scenario, keyed by the scenario file as configured.
    /// Empty in entries from format 1 and for modules without scenarios.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scenarios: BTreeMap<String, ScenarioEntry>,
    /// The inventory the module was accepted with. Absent in entries accepted
    /// before it was kept; never read as an empty inventory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory: Option<Snapshot>,
}

/// The measurement contract an entry was accepted under. A difference in
/// `runner_schema`, `warmup_runs` or `compression` makes the affected values
/// not comparable; `node` and `artifact_sha256` are for traceability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    /// The measurement protocol, see [`RUNNER_SCHEMA`].
    pub runner_schema: u32,
    /// Runs not counted in `growth_per_run`, see [`WARMUP_RUNS`].
    pub warmup_runs: u32,
    /// The gzip and brotli settings.
    pub compression: String,
    /// The Node version the scenarios ran in; `None` for a module without scenarios.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// SHA-256 of the artifact.
    pub artifact_sha256: String,
}

/// The accepted memory values of one scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioEntry {
    /// SHA-256 of the scenario script.
    pub sha256: String,
    /// How often the script ran on the instance.
    pub repeat: u32,
    /// The highest capacity observed.
    pub peak_memory: u64,
    /// What each run after the first added; `None` if it ran once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub growth_per_run: Option<u64>,
    /// Capacity right after instantiation. Absent in entries accepted before
    /// the trace was kept; never read as zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_instantiate: Option<u64>,
    /// Capacity after each run, in order. Empty in entries accepted before the
    /// trace was kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after_run: Vec<u64>,
    /// What the measurement could not observe when it was accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unobserved: Vec<Unobserved>,
}

impl Entry {
    /// The entry that accepts `measured` today, for `reason`.
    #[must_use]
    pub fn from_measured(measured: &Measured, reason: &str) -> Entry {
        Entry {
            accepted: today(),
            reason: reason.to_string(),
            values: measured.values,
            contract: Some(Contract {
                runner_schema: RUNNER_SCHEMA,
                warmup_runs: WARMUP_RUNS,
                compression: SETTINGS.to_string(),
                node: measured.scenarios.first().map(|s| s.node.clone()),
                artifact_sha256: measured.artifact_sha256.clone(),
            }),
            scenarios: measured
                .scenarios
                .iter()
                .map(|s| {
                    (
                        s.id(),
                        ScenarioEntry {
                            sha256: s.sha256.clone(),
                            repeat: s.repeat,
                            peak_memory: s.peak_memory,
                            growth_per_run: s.growth_per_run,
                            after_instantiate: Some(s.after_instantiate),
                            after_run: s.after_run.clone(),
                            unobserved: s.coverage.unobserved.clone(),
                        },
                    )
                })
                .collect(),
            inventory: Some(Snapshot::from_inventory(&measured.inventory)),
        }
    }
}

#[derive(Serialize)]
struct HistoryLine<'a> {
    module: &'a str,
    #[serde(flatten)]
    entry: &'a Entry,
}

#[derive(Deserialize)]
struct Header {
    version: u32,
}

/// Reads the baseline in `dir`. A missing file is an empty baseline: nothing
/// has been accepted yet. A file in format 1 is read as it is; entries without
/// a contract or scenarios are compared on their module values until they are
/// accepted again.
///
/// # Errors
///
/// Fails if the file exists but cannot be read or parsed, or declares a format
/// version outside [`OLDEST_VERSION`] to [`VERSION`].
pub fn load(dir: &Path) -> Result<Baseline, Error> {
    let path = dir.join(BASELINE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let syntax = |source| Error::BaselineSyntax {
                path: path.clone(),
                source,
            };
            let header: Header = serde_json::from_str(&text).map_err(syntax)?;
            if !(OLDEST_VERSION..=VERSION).contains(&header.version) {
                return Err(Error::BaselineVersion {
                    path,
                    found: header.version,
                });
            }
            serde_json::from_str(&text).map_err(syntax)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Baseline {
            version: VERSION,
            modules: BTreeMap::new(),
        }),
        Err(source) => Err(Error::Read { path, source }),
    }
}

/// Writes the baseline to `dir` in the current format. The text goes to a
/// temporary file first and replaces the old baseline in one step, so a failed
/// write leaves the previous baseline as it was.
///
/// # Errors
///
/// Fails if the file cannot be written.
pub fn save(dir: &Path, baseline: &Baseline) -> Result<(), Error> {
    let path = dir.join(BASELINE_FILE);
    let current = Baseline {
        version: VERSION,
        modules: baseline.modules.clone(),
    };
    let mut text = serde_json::to_string_pretty(&current).expect("a baseline is plain data");
    text.push('\n');
    let temporary = dir.join(format!("{BASELINE_FILE}.tmp"));
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|source| {
            let _ = std::fs::remove_file(&temporary);
            Error::Write { path, source }
        })
}

/// Appends one accepted entry to the history log in `dir`.
///
/// # Errors
///
/// Fails if the log cannot be opened or written.
pub fn append_history(dir: &Path, module: &str, entry: &Entry) -> Result<(), Error> {
    let path = dir.join(HISTORY_FILE);
    let line = serde_json::to_string(&HistoryLine { module, entry }).expect("plain data");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|source| Error::Write { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("peakmark-baseline-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry() -> Entry {
        Entry {
            accepted: "2026-01-02".to_string(),
            reason: "first".to_string(),
            values: Values {
                raw: Some(10),
                ..Values::default()
            },
            contract: None,
            scenarios: BTreeMap::new(),
            inventory: None,
        }
    }

    #[test]
    fn an_entry_without_an_inventory_reads_as_none_and_is_saved_without_one() {
        let text = r#"{"accepted":"2026-01-02","reason":"first","values":{"raw":10}}"#;
        let entry: Entry = serde_json::from_str(text).unwrap();
        assert!(entry.inventory.is_none());
        assert!(!serde_json::to_string(&entry).unwrap().contains("inventory"));
    }

    #[test]
    fn the_inventory_is_stored_sorted_and_compact() {
        let bytes = wat::parse_str(
            r#"(module (import "env" "log" (func (param i32)))
                (func (export "b")) (func (export "a") (result i32) i32.const 0))"#,
        )
        .unwrap();
        let inventory = crate::inventory::Inventory::from_bytes(&bytes).unwrap();
        let text = serde_json::to_string(&Snapshot::from_inventory(&inventory)).unwrap();
        assert_eq!(
            text,
            r#"{"imports":[{"module":"env","name":"log","kind":"func","type":"(i32) -> ()"}],"exports":[{"name":"a","kind":"func","type":"() -> i32"},{"name":"b","kind":"func","type":"() -> ()"}],"sections":{"code":9,"data":0,"names":0,"debug":0,"custom":0,"structure":35,"overhead":18},"functions":{"count":2,"body_bytes":6,"largest":[{"size":4,"export":"a"},{"size":2,"export":"b"}]}}"#
        );
    }

    #[test]
    fn a_scenario_entry_without_a_trace_still_reads_and_leaves_the_trace_empty() {
        let text = r#"{"sha256":"a","repeat":3,"peak_memory":65536,"growth_per_run":0}"#;
        let entry: ScenarioEntry = serde_json::from_str(text).unwrap();
        assert_eq!(entry.after_instantiate, None);
        assert!(entry.after_run.is_empty() && entry.unobserved.is_empty());
        let written = serde_json::to_string(&entry).unwrap();
        assert!(!written.contains("after_run"), "{written}");
    }

    #[test]
    fn a_missing_file_is_an_empty_baseline() {
        let dir = scratch("missing");
        assert!(load(&dir).unwrap().modules.is_empty());
    }

    #[test]
    fn saved_entries_load_again_and_history_appends_a_line_each() {
        let dir = scratch("roundtrip");
        let mut baseline = load(&dir).unwrap();
        baseline.modules.insert("engine".to_string(), entry());
        save(&dir, &baseline).unwrap();
        append_history(&dir, "engine", &entry()).unwrap();
        append_history(&dir, "engine", &entry()).unwrap();

        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.modules["engine"].values.raw, Some(10));
        let history = std::fs::read_to_string(dir.join(HISTORY_FILE)).unwrap();
        assert_eq!(history.lines().count(), 2);
        assert!(history.contains(r#""module":"engine""#), "{history}");
    }

    #[test]
    fn a_broken_file_names_itself() {
        let dir = scratch("broken");
        std::fs::write(dir.join(BASELINE_FILE), "{").unwrap();
        let error = load(&dir).err().unwrap();
        assert!(error.to_string().contains(BASELINE_FILE), "{error}");
    }

    #[test]
    fn a_format_1_file_is_read_and_saved_as_format_2_with_its_entries_untouched() {
        let dir = scratch("v1");
        let old = r#"{"version":1,"modules":{"engine":{"accepted":"2026-01-02","reason":"first","values":{"raw":10,"peak_memory":65536}}}}"#;
        std::fs::write(dir.join(BASELINE_FILE), old).unwrap();

        let baseline = load(&dir).unwrap();
        assert!(baseline.modules["engine"].contract.is_none());
        assert!(baseline.modules["engine"].scenarios.is_empty());
        save(&dir, &baseline).unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(BASELINE_FILE)).unwrap())
                .unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["modules"]["engine"]["values"]["peak_memory"], 65536);
        assert!(saved["modules"]["engine"].get("scenarios").is_none());
    }

    #[test]
    fn an_unsupported_format_version_is_refused_with_the_version() {
        for version in [0, 3, 99] {
            let dir = scratch(&format!("version-{version}"));
            std::fs::write(
                dir.join(BASELINE_FILE),
                format!(r#"{{"version":{version},"modules":{{}}}}"#),
            )
            .unwrap();
            let error = load(&dir).err().unwrap();
            assert!(
                matches!(error, Error::BaselineVersion { found, .. } if found == version),
                "{error}"
            );
        }
    }

    #[test]
    fn a_failed_save_keeps_the_previous_baseline() {
        let dir = scratch("failed-save");
        let mut baseline = load(&dir).unwrap();
        baseline.modules.insert("engine".to_string(), entry());
        save(&dir, &baseline).unwrap();
        let before = std::fs::read_to_string(dir.join(BASELINE_FILE)).unwrap();

        // A directory where the temporary file goes makes the write fail.
        std::fs::create_dir(dir.join(format!("{BASELINE_FILE}.tmp"))).unwrap();
        baseline.modules.get_mut("engine").unwrap().reason = "second".to_string();
        assert!(matches!(save(&dir, &baseline), Err(Error::Write { .. })));

        assert_eq!(
            std::fs::read_to_string(dir.join(BASELINE_FILE)).unwrap(),
            before
        );
    }
}
