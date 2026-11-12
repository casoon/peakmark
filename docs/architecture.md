---
title: Architecture
description: The crate's modules, how a check flows through them, and who owns what.
order: 5
---

peakmark is one crate with a library and a thin binary. `src/main.rs` calls `peakmark::cli::main`
and nothing else; everything the command does is reachable through the library's public API, which
is documented with `cargo doc`. The crate forbids `unsafe` code.

## Modules

```text
src/
├── main.rs        the binary: passes the arguments to cli::main
├── lib.rs         crate documentation and the module list
├── cli/           arguments (args.rs), commands (commands.rs), exit codes (mod.rs)
├── config.rs      peakmark.toml: modules, budgets, scenarios, validation
├── baseline.rs    peakmark.baseline.json and the history log
├── measure.rs     one module in, one Measured out
├── wasm.rs        validation, sections and declared memory, read with wasmparser
├── compress.rs    gzip and brotli length
├── runner.rs      runs a scenario in Node (runner.mjs) under a deadline, reads the memory it held
├── check.rs       budget and baseline against the measured values: Issue, Level, verdict
├── report/        text report (text.rs) and JSON (json.rs)
├── metric.rs      Metric and Values: the five gated values
├── units.rs       "258 kB" in, "257.1 kB" out
├── date.rs        today's date without a date library
└── error.rs       Error and UsageError
tests/
├── cli.rs         the binary against a generated module and Node scenarios
└── library.rs     the public API without Node
```

## Data flow

`peakmark check`:

1. `cli::args` parses the arguments into a `Cli`.
2. `config::load` reads `peakmark.toml`; `baseline::load` reads the baseline next to it.
3. For each configured module, `measure::measure` reads the artifact, asks `wasm::inspect` for its
   sections and declared memory, `compress` for the gzip and brotli length, and `runner` for the
   memory after each scenario. The result is a `Measured`.
4. `check::evaluate` judges every value: over the budget is an error, past the baseline by more than
   the tolerance is a warning.
5. `report::report` or `report::json` turns the results into the output; `check::verdict` decides
   the exit code.

`peakmark accept` runs steps 1 to 3 for the chosen modules, refuses a module that is over its
budget, then `baseline::save` and `baseline::append_history` store the values and the reason.

## Responsibilities

- **Only `runner` starts Node**, and only `runner.mjs` knows how a module is instantiated. Nothing
  else depends on the scenario protocol, a single `PEAKMARK_RESULT` line on stdout.
- **Only `check` decides** what is an error and what is a warning. `report` formats those decisions
  and does not make its own.
- **Only `error` owns message text.** Modules return `Error` variants that carry the path or name
  involved; `Display` renders the sentence the command prints. Exit code 2 always means an `Error`.
- **Formats stay put.** The shapes of `peakmark.toml`, the baseline file, the history log and
  `--json` are public; changing them is a user-facing change.

## Dependencies

| Crate | Used for |
|---|---|
| `wasmparser` | sections and memory of a module |
| `flate2`, `brotli` | gzip and brotli length |
| `serde`, `serde_json`, `toml` | configuration, baseline, runner protocol, `--json` |
| `runemark` | report layout, colors and the error block |

Dev-only: `wat` builds the modules the tests use.
