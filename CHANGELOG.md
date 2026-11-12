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

### Changed

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
