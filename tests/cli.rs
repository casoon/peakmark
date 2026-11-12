//! Runs the binary against a hand-written module and small scenarios.
//! The scenarios run in Node, so these tests need `node` on the PATH.

use std::path::PathBuf;
use std::process::Command;

/// One page of memory, a function that grows it, and a data segment whose
/// length the test chooses to make the module larger.
fn module(padding: usize) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
            (memory (export "memory") 1)
            (func (export "grow") (param i32) (result i32)
                local.get 0
                memory.grow)
            (data (i32.const 0) "{}"))"#,
        "peakmark-".repeat(padding)
    ))
    .unwrap()
}

/// Grows by three pages on the first run and holds that: 4 pages = 256 KiB.
const STEADY: &str = "let grown = false;
export default (module) => {
  if (!grown) module.grow(3);
  grown = true;
};
";

/// Grows by one page on every run.
const LEAK: &str = "export default (module) => {
  module.grow(1);
};
";

const FAILING: &str = "export default () => {
  throw new Error('the scenario broke');
};
";

/// A configuration for the one fixture module: its budget lines and the
/// scenario it runs.
fn config(budget: &str, scenario: &str, repeat: u32) -> String {
    format!(
        r#"[[module]]
name = "fixture"
artifact = "fixture.wasm"

[module.budget]
{budget}

[[scenario]]
module = "fixture"
file = "{scenario}"
repeat = {repeat}
"#
    )
}

struct Project {
    dir: PathBuf,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Project {
    fn new(name: &str, config: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let project = Project { dir };
        project.write("fixture.wasm", module(1));
        project.write("steady.mjs", STEADY);
        project.write("leak.mjs", LEAK);
        project.write("failing.mjs", FAILING);
        project.write("peakmark.toml", config);
        project
    }

    fn write(&self, name: &str, content: impl AsRef<[u8]>) {
        std::fs::write(self.dir.join(name), content).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap()
    }

    fn exists(&self, name: &str) -> bool {
        self.dir.join(name).exists()
    }

    fn run(&self, args: &[&str]) -> Run {
        let output = Command::new(env!("CARGO_BIN_EXE_peakmark"))
            .args(args)
            .current_dir(&self.dir)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        Run {
            code: output.status.code().unwrap(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }
}

#[track_caller]
fn assert_contains(text: &str, expected: &str) {
    assert!(
        text.contains(expected),
        "`{expected}` is missing in:\n{text}"
    );
}

#[test]
fn a_module_without_a_baseline_warns_and_names_the_next_step() {
    let project = Project::new("no-baseline", &config("", "steady.mjs", 3));
    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "peakmark check: no baseline for fixture");
    assert_contains(&run.stdout, "[baseline/missing]");
    assert_contains(&run.stdout, "peakmark accept fixture --reason");
}

#[test]
fn an_accepted_baseline_passes_and_is_logged_with_its_reason() {
    let project = Project::new("accept", &config("", "steady.mjs", 3));

    let accept = project.run(&["accept", "--all", "--reason", "first measurement"]);
    assert_eq!(accept.code, 0, "{}", accept.stderr);
    assert_contains(&accept.stdout, "Baseline accepted for fixture");
    assert_contains(&accept.stdout, "peak memory 256.0 KiB");

    let baseline: serde_json::Value =
        serde_json::from_str(&project.read("peakmark.baseline.json")).unwrap();
    let values = &baseline["modules"]["fixture"]["values"];
    assert_eq!(values["peak_memory"], 4 * 65_536);
    assert_eq!(values["growth_per_run"], 0);
    assert_eq!(values["raw"], module(1).len());

    let history = project.read("peakmark.history.jsonl");
    assert_eq!(history.lines().count(), 1);
    assert_contains(&history, r#""reason":"first measurement""#);

    let check = project.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.stderr);
    assert_contains(
        &check.stdout,
        "peakmark check: 1 module within budget and baseline",
    );
}

#[test]
fn peak_memory_over_the_budget_fails_and_points_at_the_scenario() {
    let project = Project::new(
        "over-peak",
        &config("peak_memory = \"128 KiB\"", "steady.mjs", 3),
    );
    let run = project.run(&["check"]);

    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_contains(&run.stdout, "peakmark check: over budget: fixture");
    assert_contains(
        &run.stdout,
        "Peak memory 256.0 KiB, budget 128.0 KiB [mem/peak]",
    );
    assert_contains(&run.stdout, "steady.mjs");
}

#[test]
fn memory_that_grows_with_every_run_fails_a_growth_budget_of_zero() {
    let project = Project::new("leak", &config("growth_per_run = \"0\"", "leak.mjs", 5));
    let run = project.run(&["check"]);

    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "Memory growth per run 64.0 KiB, budget 0 B [mem/growth]",
    );
}

#[test]
fn a_module_that_grew_past_the_baseline_warns_without_failing() {
    let project = Project::new("grown", &config("", "steady.mjs", 3));
    project.run(&["accept", "fixture", "--reason", "before"]);
    project.write("fixture.wasm", module(400));

    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "peakmark check: grown past the baseline: fixture",
    );
    assert_contains(&run.stdout, "over the baseline [size/raw]");
}

#[test]
fn json_carries_the_verdict_the_values_and_the_issue() {
    let project = Project::new(
        "json",
        &config("peak_memory = \"128 KiB\"", "steady.mjs", 3),
    );
    let run = project.run(&["check", "--json"]);

    assert_eq!(run.code, 1, "{}", run.stderr);
    let result: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(result["verdict"], "failed");
    let fixture = &result["modules"][0];
    assert_eq!(fixture["initial_memory"], 65_536);
    assert_eq!(fixture["values"]["peak_memory"], 4 * 65_536);
    assert_eq!(fixture["issues"][0]["rule"], "mem/peak");
    assert_eq!(fixture["issues"][0]["limit"], 128 * 1024);
    assert_eq!(fixture["scenarios"][0]["after_instantiate"], 65_536);
}

#[test]
fn details_list_the_measured_values_of_a_passing_module() {
    let project = Project::new("details", &config("", "steady.mjs", 3));
    project.run(&["accept", "--all", "--reason", "first measurement"]);

    let run = project.run(&["check", "--details"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "fixture: raw size");
    assert_contains(&run.stdout, "peak memory 256.0 KiB");
}

#[test]
fn accept_refuses_a_module_over_its_budget_and_a_missing_reason() {
    let project = Project::new(
        "accept-refused",
        &config("peak_memory = \"128 KiB\"", "steady.mjs", 3),
    );

    let over = project.run(&["accept", "--all", "--reason", "too large"]);
    assert_eq!(over.code, 2);
    assert_contains(&over.stderr, "`fixture` is over its budget");
    assert!(!project.exists("peakmark.baseline.json"));

    let silent = project.run(&["accept", "--all"]);
    assert_eq!(silent.code, 2);
    assert_contains(&silent.stderr, "accept needs --reason");
}

#[test]
fn a_scenario_that_throws_stops_the_check_with_its_message() {
    let project = Project::new("failing", &config("", "failing.mjs", 1));
    let run = project.run(&["check"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "scenario failing.mjs failed");
    assert_contains(&run.stderr, "the scenario broke");
}

#[test]
fn a_command_line_that_does_not_parse_exits_with_2_and_points_at_the_usage() {
    let project = Project::new("usage", &config("", "steady.mjs", 1));
    let run = project.run(&["bogus"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "unknown command `bogus`");
    assert_contains(&run.stderr, "peakmark --help");
}

#[test]
fn a_missing_configuration_exits_with_2() {
    let project = Project::new("no-config", &config("", "steady.mjs", 1));
    let run = project.run(&["check", "--config", "elsewhere.toml"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "cannot read elsewhere.toml");
}

#[test]
fn help_and_version_print_to_stdout() {
    let project = Project::new("help", &config("", "steady.mjs", 1));

    let help = project.run(&["--help"]);
    assert_eq!(help.code, 0);
    assert_contains(&help.stdout, "Exit codes: 0 within budget");

    let version = project.run(&["--version"]);
    assert_eq!(version.code, 0);
    assert_contains(
        &version.stdout,
        concat!("peakmark ", env!("CARGO_PKG_VERSION")),
    );
}

/// A configuration whose one scenario has a deadline of its own.
fn timed_config(scenario: &str, timeout: &str) -> String {
    format!(
        r#"[[module]]
name = "fixture"
artifact = "fixture.wasm"

[[scenario]]
module = "fixture"
file = "{scenario}"
timeout = "{timeout}"
"#
    )
}

const SPIN: &str = "export default () => {
  for (;;) {}
};
";

/// The promise never settles and a timer keeps Node alive, as an open socket would.
const PENDING: &str = "export default () => {
  setInterval(() => {}, 1000);
  return new Promise(() => {});
};
";

/// The scenario is done, a timer it left behind is not.
const LEFTOVER_HANDLE: &str = "export default (module) => {
  module.grow(1);
  setInterval(() => {}, 1000);
};
";

/// Ordinary output, a lot of it, from an asynchronous scenario that finishes.
const CHATTY: &str = "export default async (module) => {
  for (let i = 0; i < 2000; i++) {
    process.stdout.write('x'.repeat(65536) + '\\n');
    process.stderr.write('y'.repeat(65536) + '\\n');
  }
  await new Promise((resolve) => setTimeout(resolve, 50));
  module.grow(1);
};
";

/// Floods stderr with something recognisable at the end, then never returns.
const FLOOD_AND_SPIN: &str = "import { writeSync } from 'node:fs';
export default () => {
  for (let i = 0; i < 300; i++) writeSync(2, 'y'.repeat(65536) + '\\n');
  writeSync(2, 'the last words\\n');
  for (;;) {}
};
";

const FLOOD_AND_FAIL: &str = "export default () => {
  for (let i = 0; i < 2000; i++) process.stderr.write('y'.repeat(65536) + '\\n');
  throw new Error('the real reason');
};
";

/// Starts a process that holds the same pipes and outlives its parent, then hangs.
#[cfg(unix)]
const SPAWN_AND_SPIN: &str = "import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
export default () => {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
    stdio: 'inherit',
  });
  writeFileSync('descendant.pid', String(child.pid));
  for (;;) {}
};
";

fn timed_project(name: &str, scenario: &str, source: &str, timeout: &str) -> Project {
    let project = Project::new(name, &timed_config(scenario, timeout));
    project.write(scenario, source);
    project
}

#[test]
fn a_scenario_that_never_returns_is_stopped_and_named() {
    let project = timed_project("timeout-spin", "spin.mjs", SPIN, "1s");
    let started = std::time::Instant::now();
    let run = project.run(&["check"]);

    assert!(started.elapsed().as_secs() < 20, "{:?}", started.elapsed());
    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "scenario spin.mjs of module `fixture`");
    assert_contains(&run.stderr, "did not finish within 1s");
}

#[test]
fn a_promise_that_never_settles_is_stopped() {
    let project = timed_project("timeout-pending", "pending.mjs", PENDING, "1s");
    let run = project.run(&["check"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "scenario pending.mjs of module `fixture`");
    assert_contains(&run.stderr, "did not finish within 1s");
}

#[test]
fn a_timer_left_behind_does_not_hold_the_check() {
    let project = timed_project("leftover", "leftover.mjs", LEFTOVER_HANDLE, "30s");
    let started = std::time::Instant::now();
    let run = project.run(&["check", "--details"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(started.elapsed().as_secs() < 20, "{:?}", started.elapsed());
    assert_contains(&run.stdout, "peak memory 128.0 KiB");
}

#[test]
fn a_scenario_that_prints_a_lot_still_finishes() {
    let project = timed_project("chatty", "chatty.mjs", CHATTY, "60s");
    let run = project.run(&["check", "--details"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "peak memory 128.0 KiB");
}

#[test]
fn the_stderr_of_a_stopped_scenario_is_cut_to_its_end() {
    let project = timed_project("flood-timeout", "flood.mjs", FLOOD_AND_SPIN, "4s");
    let run = project.run(&["check"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "did not finish within 4s");
    assert_contains(&run.stderr, "[earlier output dropped]");
    assert_contains(&run.stderr, "the last words");
    assert!(run.stderr.len() < 40_000, "{} bytes", run.stderr.len());
}

#[test]
fn the_stderr_of_a_failed_scenario_is_cut_to_its_end() {
    let project = timed_project("flood-fail", "flood.mjs", FLOOD_AND_FAIL, "60s");
    let run = project.run(&["check"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "scenario flood.mjs failed");
    assert_contains(&run.stderr, "the real reason");
    assert!(run.stderr.len() < 40_000, "{} bytes", run.stderr.len());
}

#[test]
fn the_deadline_comes_from_the_top_level_unless_the_scenario_sets_its_own() {
    let config = "scenario_timeout = \"1s\"\n\n[[module]]\nname = \"fixture\"\n\
                  artifact = \"fixture.wasm\"\n\n[[scenario]]\nmodule = \"fixture\"\n\
                  file = \"spin.mjs\"\n";
    let project = Project::new("timeout-default", config);
    project.write("spin.mjs", SPIN);
    let run = project.run(&["check"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "did not finish within 1s");
}

#[cfg(unix)]
#[test]
fn processes_the_scenario_started_are_stopped_with_it() {
    let project = timed_project("descendant", "spawn.mjs", SPAWN_AND_SPIN, "2s");
    let run = project.run(&["check"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "did not finish within 2s");
    let pid = project.read("descendant.pid");
    let alive = |pid: &str| {
        Command::new("kill")
            .args(["-0", pid])
            .output()
            .unwrap()
            .status
            .success()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while alive(&pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!alive(&pid), "process {pid} outlived the check");
}

#[test]
fn a_timed_out_accept_leaves_the_baseline_and_its_history_alone() {
    let project = Project::new("accept-timeout", &config("", "steady.mjs", 3));
    project.run(&["accept", "--all", "--reason", "first measurement"]);
    let baseline = project.read("peakmark.baseline.json");
    let history = project.read("peakmark.history.jsonl");

    project.write("peakmark.toml", timed_config("spin.mjs", "1s"));
    project.write("spin.mjs", SPIN);
    let run = project.run(&["accept", "--all", "--reason", "should not be stored"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "did not finish within 1s");
    assert_eq!(project.read("peakmark.baseline.json"), baseline);
    assert_eq!(project.read("peakmark.history.jsonl"), history);
}
