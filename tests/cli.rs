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

/// `drop` on an empty stack: every section parses, the function body does not type.
const DROP_ON_EMPTY_STACK: &str = "0061736d01000000010401600000030201000a050103001a0b";

/// A module with a raw-size budget and no scenario, so no Node is involved.
const SIZE_ONLY: &str = r#"[[module]]
name = "fixture"
artifact = "fixture.wasm"

[module.budget]
raw = "1 MB"
"#;

fn from_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn a_function_body_that_does_not_type_exits_with_2_without_node_or_scenarios() {
    let project = Project::new("invalid-body", SIZE_ONLY);
    project.write("fixture.wasm", from_hex(DROP_ON_EMPTY_STACK));
    let run = project.run(&["check", "--json"]);

    assert_eq!(run.code, 2, "{}", run.stdout);
    assert_contains(
        &run.stderr,
        "fixture.wasm is not a valid WebAssembly module",
    );
    assert!(run.stdout.is_empty(), "{}", run.stdout);
}

#[test]
fn a_component_exits_with_2_instead_of_reporting_sizes() {
    let project = Project::new("component", SIZE_ONLY);
    project.write("fixture.wasm", wat::parse_str("(component)").unwrap());
    let run = project.run(&["check"]);

    assert_eq!(run.code, 2, "{}", run.stdout);
    assert_contains(&run.stderr, "is a WebAssembly component");
    assert!(run.stdout.is_empty(), "{}", run.stdout);
}

#[test]
fn a_valid_module_without_a_scenario_still_passes() {
    let project = Project::new("size-only", SIZE_ONLY);
    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
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

/// Grows to eight pages once and holds that.
const BIG: &str = "let grown = false;
export default (module) => {
  if (!grown) module.grow(7);
  grown = true;
};
";

/// Grows by the pages `pages.txt` names, once. Changing the file changes the
/// behaviour and leaves the script, and so its fingerprint, as it is.
const FOLLOWS_FILE: &str = "import { readFileSync } from 'node:fs';
let grown = false;
export default (module) => {
  const pages = Number(readFileSync(new URL('./pages.txt', import.meta.url), 'utf8'));
  if (!grown && pages > 0) module.grow(pages);
  grown = true;
};
";

fn two_scenarios(budget: &str, repeat: u32) -> String {
    format!(
        "{}\n[[scenario]]\nmodule = \"fixture\"\nfile = \"follows.mjs\"\nrepeat = {repeat}\n",
        config(budget, "big.mjs", 3)
    )
}

fn baseline_json(project: &Project) -> serde_json::Value {
    serde_json::from_str(&project.read("peakmark.baseline.json")).unwrap()
}

#[test]
fn growth_of_one_scenario_is_found_below_the_maximum_of_another() {
    let project = Project::new("per-scenario", &two_scenarios("", 3));
    project.write("big.mjs", BIG);
    project.write("follows.mjs", FOLLOWS_FILE);
    project.write("pages.txt", "0");
    project.run(&["accept", "--all", "--reason", "first"]);
    let accepted = baseline_json(&project);
    assert_eq!(accepted["version"], 2);
    let fixture = &accepted["modules"]["fixture"];
    assert_eq!(fixture["scenarios"]["big.mjs"]["peak_memory"], 8 * 65_536);
    assert_eq!(fixture["scenarios"]["follows.mjs"]["peak_memory"], 65_536);
    assert_eq!(fixture["values"]["peak_memory"], 8 * 65_536);

    project.write("pages.txt", "3");
    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "peakmark check: grown past the baseline: fixture",
    );
    assert_contains(
        &run.stdout,
        "Peak memory 256.0 KiB, +192.0 KiB over the baseline [mem/peak]",
    );
    assert_contains(&run.stdout, "follows.mjs");
    let json = project.run(&["check", "--json"]);
    let result: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(result["verdict"], "warning");
    // The module maximum, still eight pages, is what a budget would judge.
    assert_eq!(result["modules"][0]["values"]["peak_memory"], 8 * 65_536);
    assert_eq!(result["modules"][0]["issues"][0]["scenario"], "follows.mjs");
}

#[test]
fn removing_scenarios_and_budgets_is_reported_until_it_is_accepted() {
    let project = Project::new(
        "removed-coverage",
        &config("peak_memory = \"512 KiB\"", "steady.mjs", 3),
    );
    project.run(&["accept", "--all", "--reason", "first"]);

    project.write(
        "peakmark.toml",
        "[[module]]\nname = \"fixture\"\nartifact = \"fixture.wasm\"\n",
    );
    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "peakmark check: baseline no longer matches: fixture",
    );
    assert_contains(&run.stdout, "[baseline/scenario-removed]");
    assert_contains(&run.stdout, "Scenario steady.mjs is in the baseline");
    assert_contains(
        &run.stdout,
        "Peak memory was accepted and is not measured now [baseline/metric-missing]",
    );
    assert_contains(&run.stdout, "peakmark accept fixture --reason");
    let json = project.run(&["check", "--json"]);
    let result: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(result["verdict"], "warning");
    let rules: Vec<_> = result["modules"][0]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["rule"].as_str().unwrap())
        .collect();
    assert_eq!(
        rules,
        [
            "baseline/scenario-removed",
            "baseline/metric-missing",
            "baseline/metric-missing"
        ]
    );

    let accept = project.run(&["accept", "--all", "--reason", "dropped the memory scenario"]);
    assert_eq!(accept.code, 0, "{}", accept.stderr);
    assert!(
        baseline_json(&project)["modules"]["fixture"]
            .get("scenarios")
            .is_none()
    );
    let after = project.run(&["check"]);
    assert_contains(
        &after.stdout,
        "peakmark check: 1 module within budget and baseline",
    );
}

#[test]
fn a_changed_repeat_is_not_compared_and_a_new_scenario_has_no_values() {
    let project = Project::new("repeat", &config("", "leak.mjs", 3));
    project.run(&["accept", "--all", "--reason", "first"]);

    project.write("peakmark.toml", config("", "leak.mjs", 6));
    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "Scenario leak.mjs is not compared: it runs 6 times, accepted with 3 [baseline/incomparable]",
    );
    assert!(!run.stdout.contains("[mem/"), "{}", run.stdout);

    project.write("peakmark.toml", two_scenarios("", 3));
    project.write("big.mjs", BIG);
    project.write("follows.mjs", FOLLOWS_FILE);
    project.write("pages.txt", "0");
    let added = project.run(&["check"]);
    assert_contains(&added.stdout, "[baseline/scenario-new]");
}

#[test]
fn a_format_1_baseline_is_read_and_migrated_by_accept_without_touching_other_modules() {
    let project = Project::new("format-1", &config("", "steady.mjs", 3));
    let entry = |raw: u64| {
        format!(
            r#"{{"accepted":"2026-01-01","reason":"old","values":{{"raw":{raw},"peak_memory":{}}}}}"#,
            4 * 65_536
        )
    };
    let raw = std::fs::metadata(project.dir.join("fixture.wasm"))
        .unwrap()
        .len();
    project.write(
        "peakmark.baseline.json",
        format!(
            r#"{{"version":1,"modules":{{"fixture":{},"other":{}}}}}"#,
            entry(raw),
            entry(7)
        ),
    );

    let run = project.run(&["check"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "Scenario steady.mjs has no accepted values yet",
    );

    project.run(&["accept", "fixture", "--reason", "migrate"]);
    let saved = baseline_json(&project);
    assert_eq!(saved["version"], 2);
    assert!(saved["modules"]["fixture"]["scenarios"]["steady.mjs"].is_object());
    assert_eq!(saved["modules"]["other"]["values"]["raw"], 7);
    assert!(saved["modules"]["other"].get("contract").is_none());
    let after = project.run(&["check"]);
    assert_contains(
        &after.stdout,
        "peakmark check: 1 module within budget and baseline",
    );
}

#[test]
fn a_baseline_format_this_peakmark_does_not_read_stops_the_check() {
    let project = Project::new("format-9", &config("", "steady.mjs", 3));
    let text = r#"{"version":9,"modules":{}}"#;
    project.write("peakmark.baseline.json", text);

    let run = project.run(&["check"]);

    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "has format version 9");
    assert_contains(&run.stderr, "update peakmark");
    assert_eq!(project.read("peakmark.baseline.json"), text);
}

// Memory observation: what is counted, what is left out, what is reported as not seen.

/// A bump allocator that never frees: allocation inside the one page it has.
fn leaky_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (global $top (mut i32) (i32.const 0))
            (func (export "alloc") (param i32) (result i32)
                global.get $top
                global.get $top
                local.get 0
                i32.add
                global.set $top))"#,
    )
    .unwrap()
}

/// A project whose one scenario is `source`.
fn memory_project(name: &str, scenario: &str, repeat: u32, source: &str) -> Project {
    let project = Project::new(name, &config("", scenario, repeat));
    project.write(scenario, source);
    project
}

fn module_json(project: &Project) -> serde_json::Value {
    let run = project.run(&["check", "--json"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    result["modules"][0].clone()
}

fn trace(scenario: &serde_json::Value) -> Vec<u64> {
    scenario["after_run"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap())
        .collect()
}

#[test]
fn the_trace_tells_steady_warm_up_per_run_and_delayed_growth_apart() {
    const PAGE: u64 = 65_536;
    const DELAYED: &str = "let run = 0;
export default (module) => {
  run += 1;
  if (run === 4) module.grow(2);
};
";
    // (scenario source, capacity after each of five runs, warm-up growth, growth per run)
    let cases = [
        ("trace-steady", STEADY, vec![4 * PAGE; 5], 3 * PAGE, 0),
        (
            "trace-per-run",
            LEAK,
            vec![2 * PAGE, 3 * PAGE, 4 * PAGE, 5 * PAGE, 6 * PAGE],
            PAGE,
            PAGE,
        ),
        (
            "trace-delayed",
            DELAYED,
            vec![PAGE, PAGE, PAGE, 3 * PAGE, 3 * PAGE],
            0,
            PAGE / 2,
        ),
    ];
    for (name, source, expected, warmup, growth) in cases {
        let project = memory_project(name, "case.mjs", 5, source);
        let module = module_json(&project);
        let scenario = &module["scenarios"][0];
        assert_eq!(trace(scenario), expected, "{name}");
        assert_eq!(scenario["warmup_growth"], warmup, "{name}");
        assert_eq!(scenario["growth_per_run"], growth, "{name}");
    }
}

#[test]
fn a_leak_inside_reserved_pages_does_not_move_the_capacity() {
    let project = memory_project(
        "leak-in-capacity",
        "bump.mjs",
        5,
        "export default (module) => { module.alloc(1000); };\n",
    );
    project.write("fixture.wasm", leaky_module());
    let module = module_json(&project);
    let scenario = &module["scenarios"][0];
    assert_eq!(trace(scenario), vec![65_536; 5]);
    assert_eq!(scenario["growth_per_run"], 0);
    assert_eq!(scenario["coverage"]["unobserved"], serde_json::json!([]));
}

#[test]
fn the_details_split_the_first_run_from_the_later_ones() {
    let project = memory_project("details-trace", "steady.mjs", 5, STEADY);
    let run = project.run(&["check", "--details"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "capacity 64.0 KiB after instantiate, +192.0 KiB in the first run",
    );
    assert_contains(&run.stdout, "then 0 B per run (5 runs");
}

#[test]
fn accept_keeps_the_trace_per_scenario() {
    let project = memory_project("accept-trace", "steady.mjs", 3, STEADY);
    project.run(&["accept", "--all", "--reason", "first"]);
    let accepted = baseline_json(&project);
    let entry = &accepted["modules"]["fixture"]["scenarios"]["steady.mjs"];
    assert_eq!(entry["after_instantiate"], 65_536);
    assert_eq!(
        entry["after_run"],
        serde_json::json!([262_144, 262_144, 262_144])
    );
}

#[test]
fn an_unrelated_instance_is_left_out_and_a_second_instance_of_the_module_is_counted() {
    let other = wat::parse_str(r#"(module (memory (export "memory") 10))"#).unwrap();
    let scenario = "import { readFileSync } from 'node:fs';
const other = readFileSync(new URL('./other.wasm', import.meta.url));
const own = readFileSync(new URL('./fixture.wasm', import.meta.url));
let done = false;
export default async () => {
  if (done) return;
  done = true;
  await WebAssembly.instantiate(other, {});
  await WebAssembly.instantiate(own, {});
};
";
    let project = memory_project("other-instance", "second.mjs", 2, scenario);
    project.write("other.wasm", other);
    let module = module_json(&project);
    let scenario = &module["scenarios"][0];

    // The unrelated module's ten pages are not in the values; the second own instance is.
    assert_eq!(scenario["coverage"]["instances"], 2);
    assert_eq!(scenario["coverage"]["unrelated_instances"], 1);
    assert_eq!(scenario["memories"].as_array().unwrap().len(), 2);
    assert_eq!(
        scenario["memories"][1]["after_instantiate"],
        serde_json::Value::Null
    );
    assert_eq!(trace(scenario), vec![2 * 65_536; 2]);
    assert_eq!(scenario["peak_memory"], 2 * 65_536);
}

#[test]
fn a_memory_shared_by_two_instances_is_counted_once() {
    let imported = wat::parse_str(
        r#"(module
            (import "env" "memory" (memory 1 4 shared))
            (func (export "grow") (param i32) (result i32)
                local.get 0
                memory.grow))"#,
    )
    .unwrap();
    let glue = "import { readFileSync } from 'node:fs';
const memory = new WebAssembly.Memory({ initial: 1, maximum: 4, shared: true });
const bytes = readFileSync(new URL('./fixture.wasm', import.meta.url));
const first = await WebAssembly.instantiate(bytes, { env: { memory } });
await WebAssembly.instantiate(bytes, { env: { memory } });
export default first.instance.exports;
";
    let project = Project::new(
        "shared-memory",
        &config("", "steady.mjs", 3).replace(
            "artifact = \"fixture.wasm\"",
            "artifact = \"fixture.wasm\"\nglue = \"glue.mjs\"",
        ),
    );
    project.write("fixture.wasm", imported);
    project.write("glue.mjs", glue);
    let module = module_json(&project);
    let scenario = &module["scenarios"][0];

    assert_eq!(scenario["coverage"]["instances"], 2);
    let memories = scenario["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["origin"], "import");
    assert_eq!(memories[0]["shared"], true);
    assert_eq!(memories[0]["instances"], 2);
    assert_eq!(scenario["peak_memory"], 4 * 65_536);
}

#[test]
fn memory_nobody_exports_or_imports_is_reported_as_not_observed_and_not_as_zero() {
    let two = wat::parse_str(r#"(module (memory (export "memory") 1) (memory 3))"#).unwrap();
    let project = memory_project(
        "internal-memory",
        "noop.mjs",
        2,
        "export default () => {};\n",
    );
    project.write("fixture.wasm", two);
    let module = module_json(&project);
    let scenario = &module["scenarios"][0];
    assert_eq!(scenario["peak_memory"], 65_536);
    assert_eq!(
        scenario["coverage"]["unobserved"],
        serde_json::json!([{ "kind": "internal_memory", "count": 1 }])
    );
    let change = &module["changes"][0];
    assert_eq!(change["rule"], "memory/coverage");
    assert_eq!(change["level"], "note");

    let text = project.run(&["check"]);
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert_contains(&text.stdout, "[memory/coverage]");
    assert_contains(&text.stdout, "neither exported nor imported");
}

#[test]
fn a_module_with_no_observable_memory_stops_the_check_instead_of_reporting_zero() {
    let hidden = wat::parse_str("(module (memory 1))").unwrap();
    let project = memory_project("no-memory", "noop.mjs", 2, "export default () => {};\n");
    project.write("fixture.wasm", hidden);
    let run = project.run(&["check"]);
    assert_eq!(run.code, 2);
    assert_contains(&run.stderr, "no memory of the module can be observed");
}

#[test]
fn a_worker_is_reported_as_not_observed() {
    let scenario = "import { Worker } from 'node:worker_threads';
export default () => new Promise((resolve) => {
  new Worker('0', { eval: true }).on('exit', resolve);
});
";
    let project = memory_project("worker", "worker.mjs", 2, scenario);
    let module = module_json(&project);
    assert_eq!(
        module["scenarios"][0]["coverage"]["unobserved"],
        serde_json::json!([{ "kind": "workers", "count": 2 }])
    );
}

#[test]
fn details_list_the_inventory_without_running_a_scenario() {
    let project = Project::new("inventory-details", SIZE_ONLY);

    let run = project.run(&["check", "--details"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "Inventory: fixture");
    assert_contains(&run.stdout, "export grow (index 0): func (i32) -> i32");
    assert_contains(
        &run.stdout,
        "memory 0 (exported as memory): min 1 page (64.0 KiB), no maximum",
    );
    assert_contains(&run.stdout, "none found");
    assert_contains(&run.stdout, "no name section");
    assert_contains(&run.stdout, "not detectable:");
}

#[test]
fn json_carries_a_versioned_inventory() {
    let project = Project::new("inventory-json", SIZE_ONLY);

    let run = project.run(&["check", "--json"]);

    let result: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    let inventory = &result["modules"][0]["inventory"];
    assert_eq!(inventory["schema_version"], 1);
    assert_eq!(inventory["artifact"]["kind"], "core_module");
    assert_eq!(inventory["exports"][0]["name"], "memory");
    assert_eq!(inventory["exports"][1]["type"]["params"][0], "i32");
    assert_eq!(inventory["memories"][0]["type"]["page_size"], 65_536);
    assert_eq!(inventory["names"]["status"], "absent");
    assert_eq!(inventory["coverage"]["complete"], true);
}

// Interface and feature drift against the accepted inventory, and the module policy.

const ACCEPTED_INTERFACE: &str = r#"(module
    (import "env" "log" (func (param i32)))
    (memory (export "memory") 1 4)
    (func (export "add") (param i32 i32) (result i32) local.get 0))"#;

/// A project without scenarios whose accepted module is `ACCEPTED_INTERFACE`. The
/// tolerance is wide so that the size of the variants does not add findings.
fn accepted_interface(name: &str, policy: &str) -> Project {
    let project = Project::new(name, &format!("tolerance = \"1000%\"\n{SIZE_ONLY}{policy}"));
    project.write("fixture.wasm", wat::parse_str(ACCEPTED_INTERFACE).unwrap());
    let accept = project.run(&["accept", "--all", "--reason", "first"]);
    assert_eq!(accept.code, 0, "{}", accept.stderr);
    project
}

fn change_to(project: &Project, wat: &str) {
    project.write("fixture.wasm", wat::parse_str(wat).unwrap());
}

fn changes(run: &Run) -> Vec<serde_json::Value> {
    let document: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    document["modules"][0]["changes"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn the_accepted_inventory_is_stored_and_an_unchanged_module_stays_quiet() {
    let project = accepted_interface("drift-stored", "");

    let saved = baseline_json(&project);
    let inventory = &saved["modules"]["fixture"]["inventory"];
    assert_eq!(inventory["imports"][0]["module"], "env");
    assert_eq!(inventory["exports"][0]["name"], "add");
    assert_eq!(inventory["exports"][0]["type"], "(i32, i32) -> i32");
    assert_eq!(inventory["exports"][1]["type"], "1..4");

    let run = project.run(&["check"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "peakmark check: 1 module within budget and baseline",
    );
}

#[test]
fn a_reordered_module_is_not_a_difference() {
    let project = accepted_interface("drift-reordered", "");
    change_to(
        &project,
        r#"(module
            (import "env" "log" (func (param i32)))
            (func (export "add") (param i32 i32) (result i32) local.get 0)
            (memory (export "memory") 1 4))"#,
    );

    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "within budget and baseline");
}

#[test]
fn a_new_host_import_is_a_warning_with_the_exact_declaration() {
    let project = accepted_interface("drift-import", "");
    change_to(
        &project,
        r#"(module
            (import "env" "log" (func (param i32)))
            (import "env" "fetch" (func (param i32) (result i32)))
            (memory (export "memory") 1 4)
            (func (export "add") (param i32 i32) (result i32) local.get 0))"#,
    );

    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "baseline no longer matches: fixture");
    assert_contains(
        &run.stdout,
        "Import env.fetch func (i32) -> i32 was added; the host must provide one more binding",
    );
    assert_contains(&run.stdout, "[baseline/interface-added]");
    assert_contains(&run.stdout, "peakmark accept fixture --reason");
}

#[test]
fn a_changed_export_signature_shows_before_and_after_in_json() {
    let project = accepted_interface("drift-export", "");
    change_to(
        &project,
        r#"(module
            (import "env" "log" (func (param i32)))
            (memory (export "memory") 1 4)
            (func (export "add") (param i64 i64) (result i64) local.get 0))"#,
    );

    let text = project.run(&["check"]);
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert_contains(
        &text.stdout,
        "Export add changed: func (i32, i32) -> i32 was, (i64, i64) -> i64 now",
    );

    let changes = changes(&project.run(&["check", "--json"]));
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["rule"], "baseline/interface-changed");
    assert_eq!(changes[0]["level"], "warning");
    assert_eq!(changes[0]["detail"]["part"], "export");
    assert_eq!(changes[0]["detail"]["name"], "add");
    assert_eq!(changes[0]["detail"]["was"], "(i32, i32) -> i32");
    assert_eq!(changes[0]["detail"]["now"], "(i64, i64) -> i64");
}

#[test]
fn a_new_threads_requirement_is_a_changed_limit_and_a_new_feature() {
    let project = accepted_interface("drift-threads", "");
    change_to(
        &project,
        r#"(module
            (import "env" "log" (func (param i32)))
            (memory (export "memory") 1 4 shared)
            (func (export "add") (param i32 i32) (result i32) local.get 0))"#,
    );

    let changes = changes(&project.run(&["check", "--json"]));

    let rules: Vec<_> = changes
        .iter()
        .map(|c| c["rule"].as_str().unwrap())
        .collect();
    assert_eq!(
        rules,
        ["baseline/interface-changed", "baseline/feature-added"]
    );
    assert_eq!(changes[0]["detail"]["was"], "1..4");
    assert_eq!(changes[0]["detail"]["now"], "1..4 shared");
    assert_eq!(changes[1]["detail"]["feature"], "shared_memory");
}

#[test]
fn an_entry_accepted_before_the_inventory_was_kept_only_notes_it() {
    let project = accepted_interface("drift-old-entry", "");
    let mut saved = baseline_json(&project);
    saved["modules"]["fixture"]
        .as_object_mut()
        .unwrap()
        .remove("inventory");
    project.write("peakmark.baseline.json", saved.to_string());

    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "[baseline/inventory-missing]");
    assert_contains(&run.stdout, "within budget and baseline");
}

const POLICY: &str = r#"
[module.policy]
require_exports = [{ name = "add", signature = "(i32, i32) -> i32" }]
allow_imports = [{ module = "env", name = "log", signature = "(i32) -> ()" }]
allow_features = []
"#;

#[test]
fn a_module_within_its_policy_passes() {
    let project = accepted_interface("policy-ok", POLICY);

    let run = project.run(&["check"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "within budget and baseline");
}

#[test]
fn a_policy_violation_fails_names_the_declaration_and_survives_accept() {
    let project = accepted_interface("policy-violated", POLICY);
    change_to(
        &project,
        r#"(module
            (import "env" "log" (func (param i32)))
            (import "env" "fetch" (func (param i32) (result i32)))
            (memory (export "memory") 1 4 shared)
            (func (export "add") (param i64 i64) (result i64) local.get 0))"#,
    );

    let run = project.run(&["check"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_contains(&run.stdout, "peakmark check: policy broken: fixture");
    assert_contains(
        &run.stdout,
        "Export add is func (i64, i64) -> i64, the policy requires func (i32, i32) -> i32",
    );
    assert_contains(&run.stdout, "[policy/export-signature]");
    assert_contains(
        &run.stdout,
        "Import env.fetch (func (i32) -> i32) is not in allow_imports",
    );
    assert_contains(&run.stdout, "[policy/import-not-allowed]");
    assert_contains(&run.stdout, "shared_memory");
    assert_contains(&run.stdout, "does not waive it");

    let json: serde_json::Value =
        serde_json::from_str(&project.run(&["check", "--json"]).stdout).unwrap();
    assert_eq!(json["verdict"], "failed");
    let violations = json["modules"][0]["violations"].as_array().unwrap();
    assert_eq!(violations[0]["rule"], "policy/export-signature");
    assert_eq!(violations[0]["level"], "error");
    assert_eq!(violations[0]["detail"]["name"], "add");
    assert_eq!(violations[0]["detail"]["found"], "func (i64, i64) -> i64");
    assert!(
        violations[0]["action"]
            .as_str()
            .unwrap()
            .contains("require_exports")
    );

    let accept = project.run(&["accept", "fixture", "--reason", "try to waive"]);
    assert_eq!(accept.code, 2);
    assert_contains(&accept.stderr, "breaks its policy");
    assert_eq!(project.run(&["check"]).code, 1);
}

#[test]
fn a_missing_required_export_fails() {
    let project = accepted_interface("policy-missing-export", POLICY);
    change_to(
        &project,
        r#"(module (import "env" "log" (func (param i32))) (memory (export "memory") 1 4))"#,
    );

    let run = project.run(&["check"]);

    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_contains(
        &run.stdout,
        "Required export add (i32, i32) -> i32 is missing",
    );
    assert_contains(&run.stdout, "[policy/export-missing]");
}

// Attribution of a size change to sections and functions.

fn sized_module(parse: usize, data: &str, extra: &str) -> String {
    format!(
        r#"(module
    (memory (export "memory") 1)
    (func $parse (export "parse") {})
    (func $render (export "render") {})
    (data (i32.const 0) "{data}")
    {extra})"#,
        "nop ".repeat(parse),
        "nop ".repeat(20),
    )
}

fn accepted_sizes(name: &str, wat: &str) -> Project {
    let project = Project::new(name, &format!("tolerance = \"1000%\"\n{SIZE_ONLY}"));
    project.write("fixture.wasm", wat::parse_str(wat).unwrap());
    let accept = project.run(&["accept", "--all", "--reason", "first"]);
    assert_eq!(accept.code, 0, "{}", accept.stderr);
    project
}

fn with_rule<'a>(changes: &'a [serde_json::Value], rule: &str) -> Option<&'a serde_json::Value> {
    changes.iter().find(|change| change["rule"] == rule)
}

#[test]
fn the_first_run_explains_the_composition_without_a_delta() {
    let project = Project::new("size-first", SIZE_ONLY);
    project.write(
        "fixture.wasm",
        wat::parse_str(sized_module(10, "ab", "")).unwrap(),
    );

    let json = project.run(&["check", "--json"]);
    let document: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    let inventory = &document["modules"][0]["inventory"];
    let file = std::fs::metadata(project.dir.join("fixture.wasm"))
        .unwrap()
        .len();
    let composition = inventory["composition"].as_object().unwrap();
    let total: u64 = composition.values().map(|v| v.as_u64().unwrap()).sum();
    assert_eq!(total, file);
    let largest = inventory["functions"]["largest"].as_array().unwrap();
    assert_eq!(largest[0]["name"], "render");
    assert_eq!(inventory["data"]["largest"][0]["size"], 2);
    assert!(
        changes(&json)
            .iter()
            .all(|c| !c["rule"].as_str().unwrap().starts_with("baseline/size"))
    );

    let text = project.run(&["check", "--details"]);
    assert_contains(&text.stdout, "composition: code");
    assert_contains(&text.stdout, "largest function bodies (raw bytes");
    assert_contains(&text.stdout, "largest data segments (contents not shown)");
}

#[test]
fn a_larger_function_body_is_found_by_name_and_accounted_exactly() {
    let project = accepted_sizes("size-function", &sized_module(10, "ab", ""));
    change_to(&project, &sized_module(110, "ab", ""));

    let run = project.run(&["check", "--json"]);
    let all = changes(&run);
    let functions = with_rule(&all, "baseline/size-functions").unwrap();
    assert_eq!(functions["level"], "note");
    let detail = &functions["detail"];
    assert_eq!(detail["confidence"], "high");
    assert_eq!(detail["status"], "exact");
    assert_eq!(detail["body_delta"], 100);
    assert_eq!(detail["unaccounted"], 0);
    assert_eq!(detail["changes"][0]["key"]["value"], "parse");
    assert_eq!(detail["changes"][0]["kind"], "changed");
    let sections = with_rule(&all, "baseline/size-sections").unwrap();
    assert_eq!(sections["detail"]["changed"][0]["part"], "code");
    assert!(
        sections["detail"]["action"]
            .as_str()
            .unwrap()
            .contains("Review the function parse")
    );

    let text = project.run(&["check"]);
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert_contains(&text.stdout, "[baseline/size-functions]");
    assert_contains(&text.stdout, "Function parse: ");
    assert_contains(&text.stdout, "raw bytes, not a compressed saving");
}

#[test]
fn added_data_is_the_contributor_and_its_contents_are_not_printed() {
    let project = accepted_sizes("size-data", &sized_module(10, "ab", ""));
    let secret = "s3cret-token-".repeat(40);
    change_to(&project, &sized_module(10, &secret, ""));

    let run = project.run(&["check", "--json"]);
    let all = changes(&run);
    let sections = with_rule(&all, "baseline/size-sections").unwrap();
    assert_eq!(sections["detail"]["changed"][0]["part"], "data");
    assert!(
        sections["detail"]["action"]
            .as_str()
            .unwrap()
            .contains("Examine the embedded data")
    );
    assert!(with_rule(&all, "baseline/size-functions").is_none());
    assert!(!run.stdout.contains("s3cret"));
    let text = project.run(&["check", "--details"]);
    assert!(!text.stdout.contains("s3cret"));
    assert_contains(&text.stdout, "[baseline/size-sections]");
}

#[test]
fn an_added_debug_section_is_named_and_the_module_is_not_modified() {
    let project = accepted_sizes("size-debug", &sized_module(10, "ab", ""));
    change_to(
        &project,
        &sized_module(
            10,
            "ab",
            &format!(r#"(@custom ".debug_info" "{}")"#, "0".repeat(500)),
        ),
    );
    let before = std::fs::read(project.dir.join("fixture.wasm")).unwrap();

    let run = project.run(&["check", "--json"]);
    let all = changes(&run);
    let sections = with_rule(&all, "baseline/size-sections").unwrap();
    assert_eq!(sections["detail"]["changed"][0]["part"], "debug");
    assert_contains(
        sections["detail"]["action"].as_str().unwrap(),
        "deployment artifact",
    );
    assert_eq!(
        before,
        std::fs::read(project.dir.join("fixture.wasm")).unwrap()
    );
}

#[test]
fn a_stripped_reordered_module_is_not_matched_by_index() {
    let stripped = |first: usize, second: usize| {
        format!(
            "(module (func {}) (func {}))",
            "nop ".repeat(first),
            "nop ".repeat(second)
        )
    };
    let project = accepted_sizes("size-stripped", &stripped(10, 50));
    change_to(&project, &stripped(50, 90));

    let run = project.run(&["check", "--json"]);
    let all = changes(&run);
    let detail = &with_rule(&all, "baseline/size-functions").unwrap()["detail"];
    assert_eq!(detail["confidence"], "none");
    assert_eq!(detail["status"], "estimated");
    assert_eq!(detail["changes"].as_array().unwrap().len(), 0);
    assert_eq!(detail["unmatched_now"].as_array().unwrap().len(), 2);
    assert_eq!(detail["unaccounted"], detail["body_delta"]);

    let text = project.run(&["check", "--details"]);
    assert_contains(&text.stdout, "Unmatched now");
    assert_contains(&text.stdout, "func[");
}

#[test]
fn an_entry_without_sizes_notes_it_and_attributes_nothing() {
    let project = accepted_sizes("size-old-entry", &sized_module(10, "ab", ""));
    let mut saved = baseline_json(&project);
    let inventory = saved["modules"]["fixture"]["inventory"]
        .as_object_mut()
        .unwrap();
    inventory.remove("sections");
    inventory.remove("functions");
    project.write("peakmark.baseline.json", saved.to_string());
    change_to(&project, &sized_module(110, "ab", ""));

    let run = project.run(&["check", "--json"]);
    let all = changes(&run);
    assert!(with_rule(&all, "baseline/sizes-missing").is_some());
    assert!(with_rule(&all, "baseline/size-functions").is_none());
    assert_eq!(run.code, 0, "{}", run.stderr);
}

// Why the largest functions are in the module: the reference graph, as a detail.

const REACH_MODULE: &str = r#"(module
    (type $t (func))
    (table 1 funcref)
    (elem (i32.const 0) $dispatched)
    (func $entry (export "run") (call_indirect (type $t) (i32.const 0)) call $helper)
    (func $helper nop nop nop nop nop nop nop nop nop nop nop nop)
    (func $dispatched nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop)
    (func $unused nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop))"#;

fn reach_project(name: &str, wat: &str) -> Project {
    let project = Project::new(name, SIZE_ONLY);
    project.write("fixture.wasm", wat::parse_str(wat).unwrap());
    project
}

#[test]
fn details_explain_the_largest_functions_without_calling_anything_unused() {
    let project = reach_project("reach-details", REACH_MODULE);

    let run = project.run(&["check", "--details"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "why unused (");
    assert_contains(
        &run.stdout,
        "no static root in the reference graph (not a statement that it is unused)",
    );
    assert_contains(
        &run.stdout,
        "reached only by indirect dispatch, which assumes every table entry is callable: export \"run\": entry -indirect-> table -element-> dispatched",
    );
    assert_contains(&run.stdout, "reached from export \"run\": entry -> helper");
    assert!(!run.stdout.contains("dead code"), "{}", run.stdout);
}

#[test]
fn json_names_the_path_the_callers_and_the_assumptions() {
    let project = reach_project("reach-json", REACH_MODULE);

    let run = project.run(&["check", "--json"]);

    let result: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    let reach = &result["modules"][0]["reach"];
    assert_eq!(reach["coverage"]["complete"], true);
    assert!(reach["assumptions"].as_array().unwrap().len() >= 4);
    let function = |label: &str| {
        reach["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["function"]["label"] == label)
            .unwrap_or_else(|| panic!("{label} not in {reach}"))
    };
    let helper = function("helper");
    assert_eq!(helper["reachability"], "direct");
    assert_eq!(helper["path"][0]["edge"]["export"], "run");
    assert_eq!(helper["path"][0]["node"]["function"]["label"], "entry");
    assert_eq!(helper["path"][1]["edge"], "call");
    assert_eq!(helper["callers"]["count"], 1);
    let dispatched = function("dispatched");
    assert_eq!(dispatched["reachability"], "via_table");
    assert_eq!(dispatched["path"][1]["node"], "table");
    assert_eq!(function("unused")["reachability"], "no_static_root");
    assert_eq!(function("unused")["path"].as_array().unwrap().len(), 0);
}

#[test]
fn a_stripped_module_is_explained_by_export_and_local_id_never_by_a_guessed_name() {
    let project = reach_project(
        "reach-stripped",
        r#"(module
            (func (export "run") call 1)
            (func call 2 nop nop nop nop nop nop nop nop)
            (func nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop nop))"#,
    );

    let run = project.run(&["check", "--details"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_contains(&run.stdout, "why func[2] (");
    assert_contains(
        &run.stdout,
        "reached from export \"run\" -> func[1] -> func[2]",
    );
}

// Automatic explanations: ranked, evidence-backed, never a guess and never a verdict.

/// A project like `accepted_sizes`, with a tolerance narrow enough that the
/// variants below are growth.
fn accepted_tight(name: &str, wat: &str) -> Project {
    let project = Project::new(name, &format!("tolerance = \"1%\"\n{SIZE_ONLY}"));
    project.write("fixture.wasm", wat::parse_str(wat).unwrap());
    let accept = project.run(&["accept", "--all", "--reason", "first"]);
    assert_eq!(accept.code, 0, "{}", accept.stderr);
    project
}

fn explanations(run: &Run) -> Vec<serde_json::Value> {
    let document: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    document["modules"][0]["explanations"]
        .as_array()
        .unwrap()
        .clone()
}

fn rules(explained: &[serde_json::Value]) -> Vec<&str> {
    explained
        .iter()
        .map(|e| e["rule"].as_str().unwrap())
        .collect()
}

fn evidence_of<'a>(explanation: &'a serde_json::Value, what: &str) -> &'a serde_json::Value {
    explanation["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["what"] == what)
        .unwrap_or_else(|| panic!("no evidence `{what}` in {explanation}"))
}

#[test]
fn the_first_run_explains_the_composition_and_claims_no_growth() {
    let project = Project::new("explain-first", SIZE_ONLY);
    project.write(
        "fixture.wasm",
        wat::parse_str(sized_module(60, "ab", "")).unwrap(),
    );

    let json = project.run(&["check", "--json"]);
    let listed = explanations(&json);
    assert_eq!(rules(&listed), ["explain/composition"]);
    assert_eq!(listed[0]["status"], "exact");
    assert_eq!(listed[0]["evidence"][0]["was"], serde_json::Value::Null);
    assert_eq!(listed[0]["evidence"][0]["what"], "code bytes");
    assert_contains(listed[0]["reached"]["root"].as_str().unwrap(), "export");

    let text = project.run(&["check"]);
    assert_contains(&text.stdout, "Explanations: fixture");
    assert_contains(&text.stdout, "No baseline yet, so no growth is claimed");
    assert_contains(&text.stdout, "[explain/composition]");
    assert_contains(&text.stdout, "Next: ");
    assert_contains(&text.stdout, "Coverage: ");
    assert!(!text.stdout.contains("grew"));
}

#[test]
fn a_grown_debug_section_is_named_with_before_and_after_and_a_distinct_action() {
    let project = accepted_tight("explain-debug", &sized_module(10, "ab", ""));
    change_to(
        &project,
        &sized_module(
            10,
            "ab",
            &format!(r#"(@custom ".debug_info" "{}")"#, "0".repeat(500)),
        ),
    );
    let before = std::fs::read(project.dir.join("fixture.wasm")).unwrap();

    let json = project.run(&["check", "--json"]);
    let listed = explanations(&json);
    assert_eq!(rules(&listed), ["explain/debug-growth"]);
    assert_eq!(listed[0]["status"], "exact");
    let part = evidence_of(&listed[0], "debug bytes");
    assert_eq!(part["was"], 0);
    assert!(part["now"].as_u64().unwrap() >= 500);
    assert!(
        evidence_of(&listed[0], "custom section .debug_info")["now"]
            .as_u64()
            .is_some()
    );
    assert_contains(
        listed[0]["action"].as_str().unwrap(),
        "without debug information",
    );
    assert_contains(
        listed[0]["action"].as_str().unwrap(),
        "does not modify files",
    );
    assert_eq!(
        before,
        std::fs::read(project.dir.join("fixture.wasm")).unwrap()
    );
}

#[test]
fn grown_data_is_distinct_from_debug_and_never_prints_its_content() {
    let project = accepted_tight("explain-data", &sized_module(10, "ab", ""));
    change_to(&project, &sized_module(10, &"s3cret-".repeat(60), ""));

    let run = project.run(&["check", "--json"]);
    let listed = explanations(&run);
    assert_eq!(rules(&listed), ["explain/data-growth"]);
    assert_contains(
        listed[0]["action"].as_str().unwrap(),
        "Examine the embedded data",
    );
    assert_contains(
        listed[0]["coverage"].as_str().unwrap(),
        "not tied to one segment",
    );
    assert!(!run.stdout.contains("s3cret"));
    assert!(!project.run(&["check"]).stdout.contains("s3cret"));
}

#[test]
fn the_function_that_grew_is_named_with_its_reach_from_an_export() {
    let project = accepted_tight("explain-function", &sized_module(10, "ab", ""));
    change_to(&project, &sized_module(110, "ab", ""));

    let json = project.run(&["check", "--json"]);
    let listed = explanations(&json);
    assert_eq!(rules(&listed), ["explain/function-growth"]);
    let function = &listed[0];
    assert_eq!(function["status"], "exact");
    assert_eq!(function["subject"], "parse");
    let body = evidence_of(function, "function parse");
    assert_eq!(
        body["now"].as_u64().unwrap() - body["was"].as_u64().unwrap(),
        100
    );
    assert_eq!(function["reached"]["reachability"], "direct");
    assert_eq!(function["reached"]["root"], "export \"parse\"");
    assert_contains(
        function["action"].as_str().unwrap(),
        "Review the function parse",
    );

    let text = project.run(&["check"]);
    assert_contains(&text.stdout, "Function parse grew by 100 B (exact");
    assert_contains(
        &text.stdout,
        "Why parse is in the module: reached from export \"parse\"",
    );
}

#[test]
fn a_new_import_is_explained_as_a_host_requirement_without_a_size_claim() {
    let project = accepted_interface("explain-import", "");
    change_to(
        &project,
        r#"(module
    (import "env" "log" (func (param i32)))
    (import "env" "now" (func (result f64)))
    (memory (export "memory") 1 4)
    (func (export "add") (param i32 i32) (result i32) local.get 0))"#,
    );

    let json = project.run(&["check", "--json"]);
    let listed = explanations(&json);
    assert_eq!(rules(&listed), ["explain/import-added"]);
    assert_eq!(listed[0]["status"], "exact");
    assert_contains(
        listed[0]["evidence"][0]["what"].as_str().unwrap(),
        "env.now",
    );
    assert_contains(
        listed[0]["action"].as_str().unwrap(),
        "host provides env.now",
    );
    assert_contains(
        listed[0]["coverage"].as_str().unwrap(),
        "what it costs is not known",
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
}

#[test]
fn memory_growth_in_one_scenario_is_located_in_the_trace() {
    let project = Project::new("explain-memory", &two_scenarios("", 3));
    project.write("big.mjs", BIG);
    project.write("follows.mjs", FOLLOWS_FILE);
    project.write("pages.txt", "0");
    project.run(&["accept", "--all", "--reason", "first"]);
    project.write("pages.txt", "3");

    let json = project.run(&["check", "--json"]);
    let listed = explanations(&json);
    assert_eq!(rules(&listed), ["explain/memory-growth"]);
    let memory = &listed[0];
    assert_eq!(memory["subject"], "follows.mjs");
    assert_eq!(memory["status"], "exact");
    assert_contains(
        memory["summary"].as_str().unwrap(),
        "The first run of follows.mjs",
    );
    let peak = evidence_of(memory, "peak capacity");
    assert_eq!(peak["was"], 65_536);
    assert_eq!(peak["now"], 4 * 65_536);
    let first = evidence_of(memory, "capacity after the first run");
    assert_eq!(first["now"], 4 * 65_536);
    assert_contains(
        memory["action"].as_str().unwrap(),
        "first call of the scenario",
    );
    let text = project.run(&["check"]);
    assert_contains(&text.stdout, "[explain/memory-growth]");
    assert_contains(&text.stdout, "peak capacity 64.0 KiB -> 256.0 KiB");
}

#[test]
fn without_evidence_there_is_no_explanation_instead_of_a_guess() {
    // Only declarations grow: more exports of one function. Nothing the
    // explanations read (code, data, names, debug, imports) moved.
    let few = r#"(module (func $f (export "a")) (func $g (export "b") (nop) (nop)))"#;
    let many = format!(
        "(module (func $f (export \"a\")) (func $g (export \"b\") (nop) (nop)) {})",
        (0..40)
            .map(|i| format!("(export \"alias_with_a_long_name_{i}\" (func $f))"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let project = accepted_tight("explain-nothing", few);
    change_to(&project, &many);

    let json = project.run(&["check", "--json"]);
    let document: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert!(
        document["modules"][0]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["rule"] == "size/raw")
    );
    assert_eq!(explanations(&json).len(), 0);
    assert_eq!(document["modules"][0]["explanations_omitted"], 0);
    assert!(!project.run(&["check"]).stdout.contains("Explanations:"));
}

#[test]
fn a_stripped_module_gets_no_guessed_function_only_the_part_that_grew() {
    let stripped = |first: usize, second: usize| {
        format!(
            "(module (func {}) (func {}))",
            "nop ".repeat(first),
            "nop ".repeat(second)
        )
    };
    let project = accepted_tight("explain-stripped", &stripped(10, 50));
    change_to(&project, &stripped(50, 90));

    let listed = explanations(&project.run(&["check", "--json"]));
    assert_eq!(rules(&listed), ["explain/code-growth"]);
    assert_contains(
        listed[0]["summary"].as_str().unwrap(),
        "peakmark never matches by index",
    );
    assert_eq!(listed[0]["reached"], serde_json::Value::Null);
}

#[test]
fn a_baseline_without_sizes_is_named_before_any_optimization() {
    let project = accepted_tight("explain-old-entry", &sized_module(10, "ab", ""));
    let mut saved = baseline_json(&project);
    let inventory = saved["modules"]["fixture"]["inventory"]
        .as_object_mut()
        .unwrap();
    inventory.remove("sections");
    inventory.remove("functions");
    project.write("peakmark.baseline.json", saved.to_string());
    change_to(&project, &sized_module(110, &"x".repeat(300), ""));

    let listed = explanations(&project.run(&["check", "--json"]));
    assert_eq!(rules(&listed), ["explain/insufficient"]);
    assert_contains(
        listed[0]["action"].as_str().unwrap(),
        "Accept the baseline again on purpose",
    );
}

#[test]
fn the_list_is_capped_ranked_and_the_same_on_every_run() {
    let project = accepted_tight("explain-cap", &sized_module(10, "ab", ""));
    change_to(
        &project,
        &format!(
            r#"(module
    (import "env" "log" (func))
    (memory (export "memory") 1)
    (func $parse (export "parse") {})
    (func $render (export "render") {})
    (data (i32.const 0) "{}")
    (@custom ".debug_info" "{}"))"#,
            "nop ".repeat(110),
            "nop ".repeat(20),
            "d".repeat(400),
            "0".repeat(200),
        ),
    );

    let first = project.run(&["check", "--json"]);
    let listed = explanations(&first);
    assert_eq!(listed.len(), 3, "{}", rules(&listed).join(", "));
    // The interface comes first, then the exact size explanations by bytes.
    assert_eq!(listed[0]["rule"], "explain/import-added");
    assert_eq!(listed[1]["rule"], "explain/data-growth");
    let document: serde_json::Value = serde_json::from_str(&first.stdout).unwrap();
    assert_eq!(document["modules"][0]["explanations_omitted"], 1);
    // The cap hides no finding: every change stays in the document.
    let all = changes(&first);
    assert!(with_rule(&all, "baseline/size-sections").is_some());
    assert!(with_rule(&all, "baseline/size-functions").is_some());
    assert!(with_rule(&all, "baseline/interface-added").is_some());

    let second = project.run(&["check", "--json"]);
    assert_eq!(explanations(&second), listed);
    assert_contains(
        &project.run(&["check"]).stdout,
        "1 further explanation was left out",
    );
}

#[test]
fn explanations_come_after_the_hard_failure_and_change_neither_exit_code_nor_files() {
    let project = Project::new(
        "explain-failure",
        "[[module]]\nname = \"fixture\"\nartifact = \"fixture.wasm\"\n[module.budget]\nraw = \"100 B\"\n",
    );
    project.write(
        "fixture.wasm",
        wat::parse_str(sized_module(60, "ab", "")).unwrap(),
    );
    let before = project.read("peakmark.toml");

    let run = project.run(&["check"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let over = run.stdout.find("[size/raw]").unwrap();
    let explained = run.stdout.find("[explain/composition]").unwrap();
    assert!(over < explained, "{}", run.stdout);
    assert_contains(&run.stdout, "peakmark check: over budget: fixture");
    assert_eq!(before, project.read("peakmark.toml"));
    assert!(!project.exists("peakmark.baseline.json"));
    assert!(!project.exists("peakmark.history.jsonl"));
}
