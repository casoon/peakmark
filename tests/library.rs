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
        baseline::Entry {
            accepted: "2026-01-01".to_string(),
            reason: "test".to_string(),
            values: first.values,
        },
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
