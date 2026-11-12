//! Running a scenario in Node and reading the memory the instance holds.
//!
//! The scenario script is run by `runner.mjs`, which instantiates the module,
//! calls the script `repeat` times and prints one `PEAKMARK_RESULT` line.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::config::{Config, Module, Scenario};
use crate::error::Error;

const RUNNER: &str = include_str!("runner.mjs");
const RESULT_PREFIX: &str = "PEAKMARK_RESULT ";

/// What one scenario measured.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    /// The scenario script as configured.
    pub file: PathBuf,
    /// Memory held right after instantiation.
    pub after_instantiate: u64,
    /// Memory held after the last run.
    pub peak_memory: u64,
    /// What each run after the first adds; `None` if the scenario runs once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub growth_per_run: Option<u64>,
    /// Milliseconds instantiation took.
    pub instantiate_ms: f64,
}

#[derive(Serialize)]
struct Job {
    artifact: PathBuf,
    glue: Option<PathBuf>,
    scenario: PathBuf,
    repeat: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerResult {
    after_instantiate: u64,
    after_run: Vec<u64>,
    instantiate_ms: f64,
}

/// Runs `scenario` against `module` in Node.
///
/// # Errors
///
/// Fails if Node cannot be started, the scenario throws, or its result is
/// missing or unreadable.
pub fn run_scenario(
    config: &Config,
    module: &Module,
    scenario: &Scenario,
) -> Result<ScenarioResult, Error> {
    let absolute = |path: &Path| {
        std::path::absolute(config.dir.join(path)).map_err(|source| Error::Resolve {
            path: path.to_path_buf(),
            source,
        })
    };
    let job = Job {
        artifact: absolute(&module.artifact)?,
        glue: module.glue.as_deref().map(absolute).transpose()?,
        scenario: absolute(&scenario.file)?,
        repeat: scenario.repeat,
    };

    let output = Command::new("node")
        .args(["--input-type=module", "-e", RUNNER])
        .env(
            "PEAKMARK_JOB",
            serde_json::to_string(&job).expect("a job is plain data"),
        )
        .current_dir(&config.dir)
        .output()
        .map_err(Error::NodeStart)?;

    if !output.status.success() {
        return Err(Error::ScenarioFailed {
            scenario: scenario.file.clone(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(RESULT_PREFIX))
        .ok_or_else(|| Error::ScenarioNoResult {
            scenario: scenario.file.clone(),
        })?;
    let result: RunnerResult =
        serde_json::from_str(line).map_err(|source| Error::ScenarioUnreadable {
            scenario: scenario.file.clone(),
            source,
        })?;
    Ok(summarise(&scenario.file, &result))
}

fn summarise(file: &Path, result: &RunnerResult) -> ScenarioResult {
    // Linear memory never shrinks, so what is held after the last run is the peak.
    let first = result.after_run[0];
    let last = result.after_run[result.after_run.len() - 1];
    let later_runs = result.after_run.len() as u64 - 1;
    ScenarioResult {
        file: file.to_path_buf(),
        after_instantiate: result.after_instantiate,
        peak_memory: last,
        // The first run may fill caches and pools; growth is what the runs after it add.
        growth_per_run: (later_runs > 0).then(|| (last - first) / later_runs),
        instantiate_ms: result.instantiate_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(after_run: &[u64]) -> RunnerResult {
        RunnerResult {
            after_instantiate: 65_536,
            after_run: after_run.to_vec(),
            instantiate_ms: 1.0,
        }
    }

    #[test]
    fn the_peak_is_what_is_held_after_the_last_run() {
        let summary = summarise(Path::new("s.mjs"), &result(&[100, 200, 300]));
        assert_eq!(summary.peak_memory, 300);
    }

    #[test]
    fn growth_ignores_the_first_run() {
        let summary = summarise(Path::new("s.mjs"), &result(&[400, 400, 400]));
        assert_eq!(summary.growth_per_run, Some(0));
        let summary = summarise(Path::new("s.mjs"), &result(&[100, 200, 300, 400]));
        assert_eq!(summary.growth_per_run, Some(100));
    }

    #[test]
    fn a_scenario_that_runs_once_has_no_growth() {
        let summary = summarise(Path::new("s.mjs"), &result(&[100]));
        assert_eq!(summary.growth_per_run, None);
    }
}
