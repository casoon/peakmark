//! The accepted values per module, kept in the repository next to the
//! configuration, and the log of why each baseline was set.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::metric::Values;

pub const BASELINE_FILE: &str = "peakmark.baseline.json";
pub const HISTORY_FILE: &str = "peakmark.history.jsonl";

#[derive(Serialize, Deserialize)]
pub struct Baseline {
    version: u32,
    pub modules: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub accepted: String,
    pub reason: String,
    pub values: Values,
}

#[derive(Serialize)]
struct HistoryLine<'a> {
    module: &'a str,
    #[serde(flatten)]
    entry: &'a Entry,
}

/// A missing file is an empty baseline: nothing has been accepted yet.
pub fn load(dir: &Path) -> Result<Baseline, String> {
    let path = dir.join(BASELINE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Baseline {
            version: 1,
            modules: BTreeMap::new(),
        }),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

pub fn save(dir: &Path, baseline: &Baseline) -> Result<(), String> {
    let path = dir.join(BASELINE_FILE);
    let mut text = serde_json::to_string_pretty(baseline).expect("a baseline is plain data");
    text.push('\n');
    std::fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

pub fn append_history(dir: &Path, module: &str, entry: &Entry) -> Result<(), String> {
    let path = dir.join(HISTORY_FILE);
    let line = serde_json::to_string(&HistoryLine { module, entry }).expect("plain data");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}
