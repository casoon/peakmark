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
/// Fails if Node cannot be started, the scenario throws, does not finish within
/// its timeout, or its result is missing or unreadable.
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
    Ok(summarise(&scenario.file, &result))
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
        if self.result.is_none() {
            if let Some(result) = line.strip_prefix(RESULT_PREFIX) {
                self.result = Some(result.to_vec());
            }
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
