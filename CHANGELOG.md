# Changelog

All notable changes to peakmark are listed here. peakmark follows
[Semantic Versioning](https://semver.org/): before 1.0, a minor version may
change the configuration, the baseline format or the output.

## [Unreleased]

### Added

- **Config:** `scenario_timeout` and a `timeout` per `[[scenario]]` set how long a scenario may
  run; the default is 60 seconds. Details:
  [Limits](https://casoon.github.io/peakmark/docs/reference/limits/#scenario-deadline).

### Fixed

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
