//! Running a scenario in Node and reading the memory the instance holds.
//!
//! The scenario script is run by `runner.mjs`, which instantiates the module,
//! calls the script `repeat` times and prints one `PEAKMARK_RESULT` line.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::{Config, Module, Scenario};
use crate::digest::sha256;
use crate::error::Error;

const RUNNER: &str = include_str!("runner.mjs");
const RESULT_PREFIX: &[u8] = b"PEAKMARK_RESULT ";

/// How much of the end of stderr is kept for a failure message.
const STDERR_TAIL: usize = 16 * 1024;
/// A result line longer than this is cut, and then fails to parse.
const MAX_RESULT_LINE: usize = 1024 * 1024;
/// How often a running scenario is checked against its deadline.
const POLL: Duration = Duration::from_millis(10);
/// How long the output of a finished or stopped runner may take to end before
/// whatever still holds it open is stopped.
const OUTPUT_GRACE: Duration = Duration::from_millis(250);
/// How long to wait for the output after the descendants were stopped.
const OUTPUT_GIVE_UP: Duration = Duration::from_secs(2);

/// How the runner came to see a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The instance exports it.
    Export,
    /// The instance imports it and it was found in the imports it was given.
    Import,
}

/// The capacity of one observed linear memory over the scenario. A memory is
/// listed once, however many instances of the module share it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryTrace {
    /// Whether it was seen as an export or as an import.
    pub origin: Origin,
    /// Whether it is a shared memory.
    pub shared: bool,
    /// How many instances of the module use it.
    pub instances: u32,
    /// Capacity after instantiation; `None` if the memory did not exist yet.
    #[serde(alias = "afterInstantiate")]
    pub after_instantiate: Option<u64>,
    /// Capacity after each run; `None` before it existed or after it was
    /// garbage collected.
    #[serde(alias = "afterRun")]
    pub after_run: Vec<Option<u64>>,
}

/// Something a measurement could not observe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Unobserved {
    /// The module declares memory that is neither exported nor imported, so
    /// no instance gives it away.
    InternalMemory {
        /// How many such memories the observed instances hold together.
        count: u32,
    },
    /// The scenario started worker threads; a module instantiated there is not watched.
    Workers {
        /// How many workers were started.
        count: u32,
    },
}

/// What the runner watched, and what it did not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    /// Instances of the configured module that were observed.
    pub instances: u32,
    /// Instances of other modules; their memory is not part of the values.
    pub unrelated_instances: u32,
    /// What could not be observed.
    pub unobserved: Vec<Unobserved>,
}

/// What the allocator probe (`peakmark-probe`) reported, summed over the
/// instances that carry it. These are observed values of what the module
/// requested from its allocator, not of the memory it reserved; no budget or
/// baseline applies to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeValues {
    /// How many observed instances carry the probe; see `coverage.instances`
    /// for how many were observed.
    pub instances: u32,
    /// Bytes requested and not yet freed at the end of the scenario.
    pub live: u64,
    /// The highest requested-live value reached. With several instances, the
    /// sum of their peaks.
    pub peak: u64,
    /// Allocations, each `realloc` included.
    pub allocations: u64,
    /// Frees, each `realloc` included.
    pub frees: u64,
}

/// What one scenario measured. Memory values are the summed capacity of the
/// observed memories of the configured module: the pages the instance has
/// reserved, not what is allocated in them.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    /// The scenario script as configured.
    pub file: PathBuf,
    /// Capacity right after instantiation.
    pub after_instantiate: u64,
    /// Capacity after each run; the last entry is the capacity at the end.
    pub after_run: Vec<u64>,
    /// The highest capacity observed.
    pub peak_memory: u64,
    /// What the warm-up runs added over the capacity after instantiation.
    pub warmup_growth: u64,
    /// What each run after the warm-up adds, averaged; `None` if there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub growth_per_run: Option<u64>,
    /// Every observed memory with its own trace.
    pub memories: Vec<MemoryTrace>,
    /// What was and was not observed.
    pub coverage: Coverage,
    /// What the allocator probe reported; `None` (`null` in JSON) if no
    /// observed instance carries it. That is "not observed", never zero.
    pub probe: Option<ProbeValues>,
    /// Milliseconds instantiation took.
    pub instantiate_ms: f64,
    /// How often the script ran on the instance.
    pub repeat: u32,
    /// SHA-256 of the scenario script, to tell an edited scenario from a changed module.
    pub sha256: String,
    /// The Node version that ran it, such as `v22.4.0`.
    pub node: String,
}

impl ScenarioResult {
    /// The identity of the scenario within its module: the script as configured,
    /// with `/` as separator on every platform.
    #[must_use]
    pub fn id(&self) -> String {
        self.file.to_string_lossy().replace('\\', "/")
    }
}

/// The version of the measurement protocol: what `runner.mjs` reads and what
/// `growth_per_run` and `peak_memory` mean. A baseline taken under another
/// version is not comparable for memory.
pub const RUNNER_SCHEMA: u32 = 1;
/// Runs at the start of a scenario that are not counted in `growth_per_run`.
pub const WARMUP_RUNS: u32 = 1;

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
    node: String,
    memories: Vec<MemoryTrace>,
    coverage: RunnerCoverage,
    #[serde(default)]
    probe: Option<ProbeValues>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerCoverage {
    instances: u32,
    unrelated: u32,
    workers: u32,
    /// For each observed instance, how many memories it was seen with.
    per_instance: Vec<u32>,
}

/// Runs `scenario` against `module` in Node. `declared_memories` is how many
/// memories the module declares, imported ones included.
///
/// # Errors
///
/// Fails if Node cannot be started, the scenario throws, does not finish within
/// its timeout, or its result is missing or unreadable.
pub fn run_scenario(
    config: &Config,
    module: &Module,
    scenario: &Scenario,
    declared_memories: u32,
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

    let script = std::fs::read(&job.scenario).map_err(|source| Error::Read {
        path: scenario.file.clone(),
        source,
    })?;

    let mut command = Command::new("node");
    command
        .args(["--input-type=module", "-e", RUNNER])
        .env(
            "PEAKMARK_JOB",
            serde_json::to_string(&job).expect("a job is plain data"),
        )
        .current_dir(&config.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Its own process group, so that the runner and everything it started can be stopped together.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(Error::NodeStart)?;

    let (sender, receiver) = mpsc::channel();
    for (stream, tail) in [
        (Stream::Stdout(child.stdout.take()), 0),
        (Stream::Stderr(child.stderr.take()), STDERR_TAIL),
    ] {
        let sender = sender.clone();
        thread::spawn(move || {
            let _ = sender.send(stream.drain(tail));
        });
    }
    drop(sender);

    let deadline = Instant::now() + scenario.timeout;
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if Instant::now() >= deadline => {
                kill_tree(&mut child);
                break (child.wait().ok(), true);
            }
            Ok(None) => thread::sleep(POLL),
            Err(source) => {
                kill_tree(&mut child);
                let _ = child.wait();
                return Err(Error::ScenarioWait {
                    scenario: scenario.file.clone(),
                    source,
                });
            }
        }
    };
    let (stdout, stderr) = collect(&receiver, &mut child, timed_out);

    if timed_out {
        return Err(Error::ScenarioTimeout {
            module: module.name.clone(),
            scenario: scenario.file.clone(),
            timeout: scenario.timeout,
            stderr: stderr.text(),
        });
    }
    if !status.is_some_and(|status| status.success()) {
        return Err(Error::ScenarioFailed {
            scenario: scenario.file.clone(),
            stderr: stderr.text(),
        });
    }
    let line = stdout.result.ok_or_else(|| Error::ScenarioNoResult {
        scenario: scenario.file.clone(),
    })?;
    let line = String::from_utf8_lossy(&line);
    let result: RunnerResult =
        serde_json::from_str(&line).map_err(|source| Error::ScenarioUnreadable {
            scenario: scenario.file.clone(),
            source,
        })?;
    Ok(ScenarioResult {
        sha256: sha256(&script),
        ..summarise(&scenario.file, result, declared_memories)
    })
}

enum Stream {
    Stdout(Option<ChildStdout>),
    Stderr(Option<ChildStderr>),
}

/// What was read from one stream of the runner.
#[derive(Default)]
struct Captured {
    /// The end of the stream, at most the requested number of bytes.
    tail: Vec<u8>,
    /// Whether bytes before `tail` were dropped.
    truncated: bool,
    /// The line that starts with the result prefix, without the prefix.
    result: Option<Vec<u8>>,
}

impl Captured {
    fn text(&self) -> String {
        let tail = String::from_utf8_lossy(&self.tail);
        if self.truncated {
            format!("[earlier output dropped]\n{tail}")
        } else {
            tail.into_owned()
        }
    }

    fn line(&mut self, line: &[u8]) {
        if self.result.is_none()
            && let Some(result) = line.strip_prefix(RESULT_PREFIX)
        {
            self.result = Some(result.to_vec());
        }
    }
}

impl Stream {
    /// Reads to the end, keeping the result line and the last `tail` bytes.
    /// Everything else is read and dropped, so a scenario that prints without
    /// end uses neither memory nor a full pipe.
    fn drain(self, tail: usize) -> (bool, Captured) {
        let (is_stdout, mut reader): (bool, Box<dyn Read>) = match self {
            Self::Stdout(Some(pipe)) => (true, Box::new(pipe)),
            Self::Stderr(Some(pipe)) => (false, Box::new(pipe)),
            Self::Stdout(None) => return (true, Captured::default()),
            Self::Stderr(None) => return (false, Captured::default()),
        };
        let mut captured = Captured::default();
        let mut line = Vec::new();
        let mut buffer = [0_u8; 8192];
        while let Ok(read @ 1..) = reader.read(&mut buffer) {
            let chunk = &buffer[..read];
            for part in chunk.split_inclusive(|&byte| byte == b'\n') {
                let room = MAX_RESULT_LINE.saturating_sub(line.len());
                line.extend_from_slice(&part[..part.len().min(room)]);
                if part.ends_with(b"\n") {
                    line.pop();
                    captured.line(&line);
                    line.clear();
                }
            }
            if tail > 0 {
                captured.tail.extend_from_slice(chunk);
                if captured.tail.len() > tail {
                    captured.tail.drain(..captured.tail.len() - tail);
                    captured.truncated = true;
                }
            }
        }
        captured.line(&line);
        (is_stdout, captured)
    }
}

/// Takes the output of the two reader threads. It ends when every holder of the
/// pipes is gone; a descendant that outlived the runner is stopped if it holds
/// them longer than the grace period.
fn collect(
    receiver: &Receiver<(bool, Captured)>,
    child: &mut Child,
    already_stopped: bool,
) -> (Captured, Captured) {
    let (mut stdout, mut stderr) = (None, None);
    let mut give_up = Instant::now() + OUTPUT_GRACE;
    let mut stopped = already_stopped;
    while stdout.is_none() || stderr.is_none() {
        match receiver.recv_timeout(give_up.saturating_duration_since(Instant::now())) {
            Ok((true, captured)) => stdout = Some(captured),
            Ok((false, captured)) => stderr = Some(captured),
            Err(_) if !stopped => {
                kill_tree(child);
                stopped = true;
                give_up = Instant::now() + OUTPUT_GIVE_UP;
            }
            Err(_) => break,
        }
    }
    (stdout.unwrap_or_default(), stderr.unwrap_or_default())
}

/// Stops the runner and, where the platform allows, everything it started.
///
/// On Unix that is the runner's process group, which it leads; a descendant
/// that left the group (`setsid`, a daemon) is not reached. On Windows
/// `taskkill /T` follows the process tree.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

fn summarise(file: &Path, result: RunnerResult, declared_memories: u32) -> ScenarioResult {
    let first = result.after_run[0];
    let last = result.after_run[result.after_run.len() - 1];
    let later_runs = result.after_run.len() as u64 - 1;
    // Capacity only grows, unless a memory was collected; the highest sample is the peak.
    let peak = result
        .after_run
        .iter()
        .copied()
        .fold(result.after_instantiate, u64::max);

    let hidden: u32 = result
        .coverage
        .per_instance
        .iter()
        .map(|&seen| declared_memories.saturating_sub(seen))
        .sum();
    let mut unobserved = Vec::new();
    if hidden > 0 {
        unobserved.push(Unobserved::InternalMemory { count: hidden });
    }
    if result.coverage.workers > 0 {
        unobserved.push(Unobserved::Workers {
            count: result.coverage.workers,
        });
    }
    ScenarioResult {
        file: file.to_path_buf(),
        after_instantiate: result.after_instantiate,
        peak_memory: peak,
        // The first run may fill caches and pools; growth is what the runs after it add.
        warmup_growth: first.saturating_sub(result.after_instantiate),
        growth_per_run: (later_runs > 0).then(|| last.saturating_sub(first) / later_runs),
        instantiate_ms: result.instantiate_ms,
        repeat: result.after_run.len() as u32,
        sha256: String::new(),
        node: result.node,
        after_run: result.after_run,
        memories: result.memories,
        probe: result.probe,
        coverage: Coverage {
            instances: result.coverage.instances,
            unrelated_instances: result.coverage.unrelated,
            unobserved,
        },
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
            node: "v22.0.0".to_string(),
            memories: Vec::new(),
            probe: None,
            coverage: RunnerCoverage {
                instances: 1,
                unrelated: 0,
                workers: 0,
                per_instance: vec![1],
            },
        }
    }

    fn summary(after_run: &[u64]) -> ScenarioResult {
        summarise(Path::new("s.mjs"), result(after_run), 1)
    }

    #[test]
    fn the_peak_is_the_highest_capacity_seen() {
        assert_eq!(summary(&[100_000, 200_000, 300_000]).peak_memory, 300_000);
        // A memory collected after the second run takes its capacity out of the sum.
        assert_eq!(summary(&[100_000, 300_000, 100_000]).peak_memory, 300_000);
    }

    #[test]
    fn steady_capacity_has_no_growth_at_all() {
        let summary = summary(&[65_536, 65_536, 65_536]);
        assert_eq!(summary.warmup_growth, 0);
        assert_eq!(summary.growth_per_run, Some(0));
    }

    #[test]
    fn growth_in_the_warm_up_run_is_not_growth_per_run() {
        let summary = summary(&[262_144, 262_144, 262_144]);
        assert_eq!(summary.warmup_growth, 196_608);
        assert_eq!(summary.growth_per_run, Some(0));
    }

    #[test]
    fn growth_in_every_run_is_averaged_over_the_later_runs() {
        assert_eq!(summary(&[100, 200, 300, 400]).growth_per_run, Some(100));
    }

    #[test]
    fn growth_that_comes_late_is_averaged_not_lost() {
        let summary = summary(&[100, 100, 100, 400]);
        assert_eq!(summary.growth_per_run, Some(100));
    }

    #[test]
    fn a_scenario_that_runs_once_has_no_growth() {
        assert_eq!(summary(&[100]).growth_per_run, None);
    }

    #[test]
    fn a_declared_memory_nobody_exports_is_reported_as_not_observed() {
        let mut observed = result(&[100, 100]);
        observed.coverage.per_instance = vec![1, 1];
        let summary = summarise(Path::new("s.mjs"), observed, 2);
        assert_eq!(
            summary.coverage.unobserved,
            vec![Unobserved::InternalMemory { count: 2 }]
        );
    }

    #[test]
    fn started_workers_are_reported_as_not_observed() {
        let mut observed = result(&[100, 100]);
        observed.coverage.workers = 2;
        let summary = summarise(Path::new("s.mjs"), observed, 1);
        assert_eq!(
            summary.coverage.unobserved,
            vec![Unobserved::Workers { count: 2 }]
        );
    }

    #[test]
    fn an_absent_probe_is_not_observed_rather_than_zero() {
        assert_eq!(summary(&[100, 100]).probe, None);
        let line = r#"{"afterInstantiate":65536,"afterRun":[65536],"instantiateMs":1.0,"node":"v22","memories":[],
            "coverage":{"instances":1,"unrelated":0,"workers":0,"perInstance":[1]},
            "probe":{"instances":1,"live":10,"peak":30,"allocations":4,"frees":3}}"#;
        let parsed: RunnerResult = serde_json::from_str(line).unwrap();
        let probe = summarise(Path::new("s.mjs"), parsed, 1).probe.unwrap();
        assert_eq!(
            (probe.live, probe.peak, probe.allocations, probe.frees),
            (10, 30, 4, 3)
        );
        // A runner result without the field at all, such as an older one, is also not observed.
        let without = line.replace("\"probe\"", "\"other\"");
        let parsed: RunnerResult = serde_json::from_str(&without).unwrap();
        assert_eq!(summarise(Path::new("s.mjs"), parsed, 1).probe, None);
    }

    #[test]
    fn full_coverage_reports_nothing_unobserved() {
        assert_eq!(summary(&[100, 100]).coverage.unobserved, Vec::new());
    }
}
