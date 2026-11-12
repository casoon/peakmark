# Changelog

All notable changes to peakmark are listed here. peakmark follows
[Semantic Versioning](https://semver.org/): before 1.0, a minor version may
change the configuration, the baseline format or the output.

## [Unreleased]

### Added

- **Config:** `scenario_timeout` and a `timeout` per `[[scenario]]` set how long a scenario may
  run; the default is 60 seconds. Details:
  [Limits](https://casoon.github.io/peakmark/docs/reference/limits/#scenario-deadline).
- **Baseline:** `check` reports removed and new scenarios, metrics no longer measured and
  measurements that cannot be compared, as warnings with exit code 0 until the baseline is
  accepted again. Details:
  [Changed coverage](https://casoon.github.io/peakmark/docs/concepts/budget-and-baseline/#changed-coverage).
- **Memory:** `--json` and the baseline keep the capacity after instantiation and after every run;
  `check --details` shows the first run apart from the later ones. Details:
  [What the memory values are](https://casoon.github.io/peakmark/docs/concepts/measurements/#what-the-memory-values-are).
- **Memory:** what could not be observed (a declared memory that is neither exported nor imported,
  a module run in a worker) is reported as a `memory/coverage` note and under `coverage` in
  `--json`, never as zero. Details:
  [What is not observed](https://casoon.github.io/peakmark/docs/concepts/measurements/#what-is-not-observed).
- **Inventory:** `check --details` and `--json` list what each module imports, exports and
  requires, read from the file without running it: typed interface, memory and table limits,
  derived features with what cannot be detected, sections and custom sections. Details:
  [Inventory](https://casoon.github.io/peakmark/docs/concepts/inventory/).
- **Library:** `peakmark::inventory::Inventory` is the typed, versioned model behind it.
- **Baseline:** `accept` keeps a compact, sorted snapshot of the inventory (imports, exports,
  internal memories and tables, needed features) in the entry of the module; the format stays
  version 2 and entries without it still read. Details:
  [What the baseline stores](https://casoon.github.io/peakmark/docs/concepts/budget-and-baseline/#what-the-baseline-stores).
- **Inventory:** `check` reports added, removed and changed imports, exports, memory and table
  limits and needed features against the accepted inventory as warnings with exit code 0, with
  before and after in `--json`; a module is identified by import module, name and kind. Details:
  [Against the baseline](https://casoon.github.io/peakmark/docs/concepts/inventory/#against-the-baseline).
- **Config:** an optional `[module.policy]` with `require_exports`, `allow_imports` and
  `allow_features` fails the check when broken; accepting the baseline does not waive it, and what
  peakmark cannot classify is refused. Details:
  [Policy](https://casoon.github.io/peakmark/docs/concepts/inventory/#policy).
- **Library:** `peakmark::snapshot` and `peakmark::policy` hold the stored inventory, its diff and
  the policy evaluation.
- **Size:** a change of size is attributed to the parts of the file and, where names or exports
  allow a trustworthy match, to function bodies; `--details` and `--json` show the composition
  from the first run on. Details:
  [Attributing a size change](https://casoon.github.io/peakmark/docs/concepts/inventory/#attributing-a-size-change).
- **Baseline:** the inventory snapshot also keeps the bytes of the file by purpose and the 20
  largest function bodies; the format stays version 2 and entries without them still read.
- **Library:** `Inventory::functions` and `Inventory::data` hold every function body and data
  segment with raw sizes for later parts.
- **Size:** `check --details` and `--json` show why the largest functions are in the module: a
  path from an export or the start function and the direct callers. Indirect calls are marked, and
  a function without a path is never called dead. Details:
  [Why a function is in the module](https://casoon.github.io/peakmark/docs/concepts/inventory/#why-a-function-is-in-the-module).
- **Library:** `peakmark::reach` (`Reach::from_bytes`, `Reach::explain`, `Reach::report`) is the
  reusable reference graph behind it.

### Changed

- **Memory:** values cover the instances of the configured artifact only; other modules a
  scenario loads are left out and a shared memory is counted once. Details:
  [What is covered](https://casoon.github.io/peakmark/docs/concepts/measurements/#what-is-covered).
- **Docs:** memory is described as exported linear-memory capacity in pages, not live allocation,
  JS heap or GC objects; a growth budget of `"0"` is a check for page growth and does not prove the
  absence of leaks. Details:
  [Release what you allocate](https://casoon.github.io/peakmark/docs/guides/scenarios/#release-what-you-allocate).
- **Baseline:** the format is version 2: memory per scenario plus the measurement contract.
  Version 1 files are still read, any other version exits with 2, and `accept` writes the file
  atomically. Details:
  [Baseline format versions](https://casoon.github.io/peakmark/docs/concepts/budget-and-baseline/#baseline-format-versions).
- **Config:** a module may list a scenario file only once.
- **Size:** a component, or a module that uses a feature outside WebAssembly 3.0, is refused
  with exit code 2 instead of being measured. Details:
  [What is measured](https://casoon.github.io/peakmark/docs/concepts/measurements/#validation-comes-first).

### Fixed

- **Baseline:** growth of one scenario is no longer hidden by a larger scenario of the same
  module, and removing the scenarios and memory budgets no longer drops the memory coverage of
  the baseline without a finding.
- **Size:** the artifact is validated completely, function bodies included, before anything is
  measured; an invalid module exits with 2 instead of passing a size-only check.
- **Scenarios:** a scenario that never returns, awaits a promise that never settles or prints
  without end no longer blocks `check` and `accept` or exhausts memory. It is stopped, with the
  processes it started on Unix, and the check exits with 2 naming the module, the scenario and the
  deadline.
- **Scenarios:** a timer or socket left open by a scenario no longer keeps Node, and the check,
  waiting after the result is written.

## [0.1.1] - 2026-11-12

### Changed

- **Release:** the binaries are built and published by the release workflow, with SHA-256
  checksums, signed build provenance and an SBOM; the crate is published by trusted publishing.
  0.1.0 was the first upload of the crate and has no binaries.

## [0.1.0] - 2026-11-12

### Added

- **CLI:** `peakmark check` measures every configured module and compares it with its budget
  and baseline; it exits with 1 when a module is over its budget.
- **CLI:** `peakmark accept` stores the measured values as the new baseline and logs the
  reason in `peakmark.history.jsonl`.
- **CLI:** `--details` lists the measured values, `--json` prints the result as JSON.
- **Config:** `peakmark.toml` with modules, a budget per module, scenarios and a `tolerance`
  for growth over the baseline.
- **Memory:** scenarios run in Node; peak memory and growth per run are read from the
  module's linear memory.
- **Size:** raw, gzip and brotli size of a module.
