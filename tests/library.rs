//! Drives the public API of the library without Node: modules without a
//! scenario are measured from the file alone.

use std::path::{Path, PathBuf};

use peakmark::check::{Level, evaluate};
use peakmark::measure::measure;
use peakmark::metric::Metric;
use peakmark::{Error, baseline, config};

/// A project directory with one module of a size the test chooses.
fn project(name: &str, config_text: &str, padding: usize) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wasm = wat::parse_str(format!(
        r#"(module (memory 2) (data (i32.const 0) "{}"))"#,
        "peakmark-".repeat(padding)
    ))
    .unwrap();
    std::fs::write(dir.join("fixture.wasm"), wasm).unwrap();
    std::fs::write(dir.join("peakmark.toml"), config_text).unwrap();
    dir
}

const MODULE: &str = "[[module]]\nname = \"fixture\"\nartifact = \"fixture.wasm\"\n";

#[test]
fn measures_size_and_declared_memory_without_a_scenario() {
    let dir = project("lib-measure", MODULE, 4);
    let config = config::load(&dir.join("peakmark.toml")).unwrap();

    let measured = measure(&config, &config.modules[0]).unwrap();

    let raw = std::fs::metadata(dir.join("fixture.wasm")).unwrap().len();
    assert_eq!(measured.values.raw, Some(raw));
    assert!(measured.values.gzip.is_some());
    assert!(measured.values.brotli.is_some());
    assert_eq!(measured.values.peak_memory, None);
    assert_eq!(measured.initial_memory, Some(2 * 65_536));
    assert!(measured.scenarios.is_empty());
}

#[test]
fn a_budget_below_the_size_is_an_error_issue_and_a_baseline_is_not_needed() {
    let text = format!("{MODULE}[module.budget]\nraw = \"1 B\"\n");
    let dir = project("lib-budget", &text, 4);
    let config = config::load(&dir.join("peakmark.toml")).unwrap();
    let measured = measure(&config, &config.modules[0]).unwrap();

    let result = evaluate(&config.modules[0], measured, None, config.tolerance);

    assert!(result.over_budget());
    assert_eq!(result.issues.len(), 1);
    assert_eq!(result.issues[0].metric, Metric::Raw);
    assert_eq!(result.issues[0].level, Level::Error);
    assert_eq!(result.issues[0].limit, 1);
}

#[test]
fn growth_past_an_accepted_baseline_is_a_warning() {
    let dir = project("lib-growth", MODULE, 4);
    let path = dir.join("peakmark.toml");
    let config = config::load(&path).unwrap();
    let first = measure(&config, &config.modules[0]).unwrap();
    let mut stored = baseline::load(&config.dir).unwrap();
    stored.modules.insert(
        "fixture".to_string(),
        baseline::Entry::from_measured(&first, "test"),
    );
    baseline::save(&config.dir, &stored).unwrap();

    project("lib-growth", MODULE, 400);
    baseline::save(&config.dir, &stored).unwrap();
    let config = config::load(&path).unwrap();
    let grown = measure(&config, &config.modules[0]).unwrap();
    let stored = baseline::load(&config.dir).unwrap();
    let result = evaluate(
        &config.modules[0],
        grown,
        stored.modules.get("fixture"),
        config.tolerance,
    );

    assert!(!result.over_budget());
    assert!(result.grown());
}

#[test]
fn a_missing_configuration_is_a_read_error_that_names_the_file() {
    let error = config::load(Path::new("does/not/exist.toml"))
        .err()
        .unwrap();
    assert!(matches!(error, Error::Read { .. }));
    assert!(
        error
            .to_string()
            .starts_with("cannot read does/not/exist.toml: "),
        "{error}"
    );
}

#[test]
fn an_artifact_that_is_not_webassembly_is_an_invalid_module() {
    let dir = project("lib-invalid", MODULE, 1);
    std::fs::write(dir.join("fixture.wasm"), b"not wasm").unwrap();
    let config = config::load(&dir.join("peakmark.toml")).unwrap();

    let error = measure(&config, &config.modules[0]).err().unwrap();

    assert!(matches!(error, Error::InvalidModule { .. }));
    assert!(
        error
            .to_string()
            .contains("is not a valid WebAssembly module"),
        "{error}"
    );
}

#[test]
fn a_function_body_that_does_not_type_is_an_invalid_module() {
    let dir = project("lib-body", MODULE, 1);
    // `drop` on an empty stack: every section parses, the function body does not type.
    let hex = "0061736d01000000010401600000030201000a050103001a0b";
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    std::fs::write(dir.join("fixture.wasm"), bytes).unwrap();
    let config = config::load(&dir.join("peakmark.toml")).unwrap();

    let error = measure(&config, &config.modules[0]).err().unwrap();

    assert!(matches!(error, Error::InvalidModule { .. }), "{error}");
}

#[test]
fn a_component_and_an_unsupported_feature_are_told_apart_from_an_invalid_module() {
    let dir = project("lib-kinds", MODULE, 1);
    let config = config::load(&dir.join("peakmark.toml")).unwrap();

    let component = wat::parse_str("(component)").unwrap();
    std::fs::write(dir.join("fixture.wasm"), component).unwrap();
    let error = measure(&config, &config.modules[0]).err().unwrap();
    assert!(
        matches!(error, Error::UnsupportedComponent { .. }),
        "{error}"
    );

    let paged = wat::parse_str("(module (memory 1 (pagesize 1)))").unwrap();
    std::fs::write(dir.join("fixture.wasm"), paged).unwrap();
    let error = measure(&config, &config.modules[0]).err().unwrap();
    assert!(matches!(error, Error::UnsupportedFeature { .. }), "{error}");
}

#[test]
fn the_inventory_is_read_from_the_bytes_alone() {
    use peakmark::inventory::{Feature, Inventory, Names};

    let bytes = wat::parse_str(
        r#"(module $engine
            (import "env" "memory" (memory 1))
            (import "env" "log" (func (param i32)))
            (memory (export "bounded") 1 4)
            (memory 2 2 shared)
            (table (export "table") 2 funcref)
            (func (export "run") (param i32) (result i32) local.get 0)
            (func v128.const i64x2 0 0 drop))"#,
    )
    .unwrap();

    let inventory = Inventory::from_bytes(&bytes).unwrap();

    assert_eq!(inventory.memories.len(), 3);
    assert!(inventory.memories[0].imported_from.is_some());
    assert_eq!(inventory.memories[1].ty.maximum, Some(4));
    assert!(inventory.memories[2].ty.shared);
    assert_eq!(inventory.tables[0].exported_as, ["table"]);
    assert_eq!(inventory.exports.len(), 3);
    assert!(inventory.feature(Feature::Simd).is_some());
    assert!(inventory.feature(Feature::SharedMemory).is_some());
    assert!(inventory.feature(Feature::MultiMemory).is_some());
    assert!(matches!(inventory.names, Names::Present { .. }));
    assert_eq!(inventory.accounted_bytes(), bytes.len() as u64);
}

#[test]
fn measuring_a_module_carries_its_inventory() {
    let dir = project("lib-inventory", MODULE, 1);
    let config = config::load(&dir.join("peakmark.toml")).unwrap();

    let measured = measure(&config, &config.modules[0]).unwrap();

    assert_eq!(measured.inventory.memories[0].ty.minimum, 2);
    assert_eq!(measured.inventory.counts.data_segments, 1);
}

#[test]
fn a_feature_outside_the_supported_set_fails_instead_of_being_left_out() {
    use peakmark::inventory::Inventory;
    use peakmark::wasm::InspectError;

    let bytes = wat::parse_str("(module (memory 1 (pagesize 1)))").unwrap();

    assert!(matches!(
        Inventory::from_bytes(&bytes),
        Err(InspectError::UnsupportedFeature(_))
    ));
}

#[test]
fn drift_and_policy_come_from_the_inventory_of_the_measured_file() {
    use peakmark::check::Change;

    let text = format!(
        "{MODULE}[module.policy]\nallow_imports = []\nallow_features = [\"bulk_memory\"]\n"
    );
    let dir = project("lib-policy", &text, 1);
    let config = config::load(&dir.join("peakmark.toml")).unwrap();
    let accepted = measure(&config, &config.modules[0]).unwrap();
    let entry = baseline::Entry::from_measured(&accepted, "test");
    assert!(entry.inventory.is_some());

    let changed = wat::parse_str(
        r#"(module (import "env" "f" (func)) (memory 2)
            (func i32.const 0 i32.const 0 i32.const 0 memory.fill))"#,
    )
    .unwrap();
    std::fs::write(dir.join("fixture.wasm"), changed).unwrap();
    let now = measure(&config, &config.modules[0]).unwrap();

    let result = evaluate(&config.modules[0], now, Some(&entry), config.tolerance);

    assert!(result.violated());
    assert_eq!(result.violations[0].rule(), "policy/import-not-allowed");
    assert!(
        result
            .changes
            .iter()
            .any(|change| matches!(change, Change::Interface(_)))
    );
}

// The reference graph: the same program in two builds, and a module of realistic size.

/// The path to `target`, as names: stable across builds, unlike indexes.
fn named_path(wat: &str, target: &str) -> Vec<String> {
    use peakmark::inventory::Inventory;
    use peakmark::reach::{Node, Reach};

    let bytes = wat::parse_str(wat).unwrap();
    let inventory = Inventory::from_bytes(&bytes).unwrap();
    let index = inventory
        .functions
        .bodies
        .iter()
        .find(|body| body.name.as_deref() == Some(target))
        .unwrap()
        .index;
    let explanation = Reach::from_bytes(&bytes).unwrap().explain(index).unwrap();
    explanation
        .path
        .iter()
        .map(|hop| match hop.node {
            Node::Function(index) => inventory.functions.body(index).unwrap().label(),
            Node::Table => "table".to_string(),
        })
        .collect()
}

#[test]
fn a_path_is_the_same_when_the_indexes_moved_between_two_builds() {
    let before = named_path(
        r#"(module
            (func $main (export "main") call $mid)
            (func $mid call $big)
            (func $big))"#,
        "big",
    );
    // Same program, functions in another order and one function added in front.
    let after = named_path(
        r#"(module
            (func $big)
            (func $added nop)
            (func $mid call $big)
            (func $main (export "main") call $mid))"#,
        "big",
    );

    assert_eq!(before, ["main", "mid", "big"]);
    assert_eq!(before, after);
}

#[test]
fn a_large_module_is_analysed_in_one_pass() {
    use peakmark::reach::{Reach, Reachability};
    use std::fmt::Write;

    // About 1.5 MB of code: a call chain through every tenth function, a
    // table of every seventh, and functions nothing refers to.
    const FUNCTIONS: usize = 20_000;
    let mut wat = String::from("(module (type $t (func)) (table 1 funcref)\n");
    wat.push_str("(elem (i32.const 0)");
    for i in (0..FUNCTIONS).step_by(7) {
        write!(wat, " $f{i}").unwrap();
    }
    wat.push_str(
        ")\n(func $entry (export \"run\") call $f0 (call_indirect (type $t) (i32.const 0)))\n",
    );
    for i in 0..FUNCTIONS {
        write!(wat, "(func $f{i}").unwrap();
        for _ in 0..12 {
            wat.push_str(" i32.const 1 i32.const 2 i32.add drop");
        }
        if i % 10 == 0 && i + 10 < FUNCTIONS {
            write!(wat, " call $f{}", i + 10).unwrap();
        }
        wat.push_str(")\n");
    }
    wat.push(')');
    let bytes = wat::parse_str(&wat).unwrap();
    assert!(bytes.len() > 1_000_000, "{} bytes", bytes.len());

    let started = std::time::Instant::now();
    let reach = Reach::from_bytes(&bytes).unwrap();
    let report = reach.report(1..=20);
    let elapsed = started.elapsed();
    println!(
        "reach on {} bytes, {} functions: {elapsed:?}",
        bytes.len(),
        reach.function_count()
    );

    assert_eq!(report.explanations.len(), 20);
    let far = reach.explain(1 + 10 * 1_000).unwrap();
    assert_eq!(far.reachability, Reachability::Direct);
    assert_eq!(far.path.len(), 1_002);
    // Generous: the point is that nothing is quadratic.
    assert!(elapsed.as_secs() < 5, "{elapsed:?}");
}

#[test]
fn explanations_come_with_the_result_and_never_change_the_verdict() {
    use peakmark::explain::{MAX_EXPLANATIONS, Status};

    let dir = project("lib-explain", MODULE, 4);
    let config = config::load(&dir.join("peakmark.toml")).unwrap();
    let first = measure(&config, &config.modules[0]).unwrap();
    let entry = baseline::Entry::from_measured(&first, "test");

    // First run: the composition, with no growth claimed.
    let measured = measure(&config, &config.modules[0]).unwrap();
    let result = evaluate(&config.modules[0], measured, None, config.tolerance);
    let rules: Vec<_> = result.explanations.listed.iter().map(|e| e.rule).collect();
    assert_eq!(rules, ["explain/composition"]);
    assert_eq!(result.explanations.listed[0].status, Status::Exact);

    // The same file against its own baseline: nothing changed, nothing to explain.
    let measured = measure(&config, &config.modules[0]).unwrap();
    let same = evaluate(&config.modules[0], measured, Some(&entry), config.tolerance);
    assert_eq!(same.explanations.listed.len(), 0);

    // More data: one explanation, and the verdict inputs are the ones without it.
    project("lib-explain", MODULE, 400);
    let measured = measure(&config, &config.modules[0]).unwrap();
    let grown = evaluate(&config.modules[0], measured, Some(&entry), config.tolerance);
    let rules: Vec<_> = grown.explanations.listed.iter().map(|e| e.rule).collect();
    assert_eq!(rules, ["explain/data-growth"]);
    assert!(grown.explanations.listed.len() <= MAX_EXPLANATIONS);
    assert!(grown.grown());
    assert!(!grown.over_budget());
    assert!(grown.issues.iter().any(|issue| issue.metric == Metric::Raw));
}
