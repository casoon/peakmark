# Changelog

All notable changes to peakmark are listed here. peakmark follows
[Semantic Versioning](https://semver.org/): before 1.0, a minor version may
change the configuration, the baseline format or the output.

## [Unreleased]

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
