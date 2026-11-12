//! The accepted values per module, kept in the repository next to the
//! configuration, and the log of why each baseline was set.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::metric::Values;

/// The baseline file, next to the configuration.
pub const BASELINE_FILE: &str = "peakmark.baseline.json";
/// The append-only log of every accepted baseline, next to the configuration.
pub const HISTORY_FILE: &str = "peakmark.history.jsonl";

/// The contents of [`BASELINE_FILE`].
#[derive(Serialize, Deserialize)]
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
    /// The accepted values.
    pub values: Values,
}

#[derive(Serialize)]
struct HistoryLine<'a> {
    module: &'a str,
    #[serde(flatten)]
    entry: &'a Entry,
}

/// Reads the baseline in `dir`. A missing file is an empty baseline: nothing
/// has been accepted yet.
///
/// # Errors
///
/// Fails if the file exists but cannot be read or parsed.
pub fn load(dir: &Path) -> Result<Baseline, Error> {
    let path = dir.join(BASELINE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|source| Error::BaselineSyntax { path, source })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Baseline {
            version: 1,
            modules: BTreeMap::new(),
        }),
        Err(source) => Err(Error::Read { path, source }),
    }
}

/// Writes the baseline to `dir`.
///
/// # Errors
///
/// Fails if the file cannot be written.
pub fn save(dir: &Path, baseline: &Baseline) -> Result<(), Error> {
    let path = dir.join(BASELINE_FILE);
    let mut text = serde_json::to_string_pretty(baseline).expect("a baseline is plain data");
    text.push('\n');
    std::fs::write(&path, text).map_err(|source| Error::Write { path, source })
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
        }
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
}
