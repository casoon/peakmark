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
├── baseline.rs    peakmark.baseline.json (format 2: per-scenario values and trace, contract, inventory snapshot) and the history log
├── measure.rs     one module in, one Measured out
├── wasm.rs        validation, sections and declared memory, read with wasmparser
├── inventory.rs   the static inventory: interface, memories, derived features, sections, composition, function bodies, data segments, metadata
├── snapshot.rs    the inventory as the baseline keeps it, and the diff of two: Snapshot, Drift, section and function drift
├── policy.rs      [module.policy]: required exports, allowed imports and features, Violation
├── reach.rs       the conservative reference graph of a module: Reach, Explanation (path from a root, callers)
├── compress.rs    gzip and brotli length
├── runner.rs      runs a scenario in Node (runner.mjs) under a deadline, reads the memory capacity and coverage
├── check.rs       budget and baseline against the measured values: Issue, Change, Level, verdict
├── report/        text report (text.rs) and JSON (json.rs)
├── metric.rs      Metric and Values: the five gated values
├── digest.rs      SHA-256 of artifacts and scenario scripts
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
   sections, declared memory and `Inventory`, `reach` for the explanation of its largest functions,
   `compress` for the gzip and brotli length, and `runner` for the memory after each scenario. The
   result is a `Measured`.
4. `check::evaluate` judges every value: over the budget is an error, past the baseline by more than
   the tolerance is a warning. Budgets judge the module-wide values; the baseline judges size per
   module and memory per scenario. Where the baseline and the measurement no longer line up
   (a scenario removed or new, a metric no longer measured, a measurement that cannot be compared)
   it records a `Change` instead of a number. The inventory is compared as well: `snapshot::diff`
   of the accepted `Snapshot` and the measured one becomes `Change::Interface`, and the module's
   `Policy` is evaluated into `Violation`s, which fail the check like a budget. A change of size is
   attributed the same way: `snapshot::section_drift` and `snapshot::function_drift` become
   `Change::Sections` and `Change::Functions`, notes that never decide the verdict.
5. `report::report` or `report::json` turns the results into the output; `check::verdict` decides
   the exit code.

`peakmark accept` runs steps 1 to 3 for the chosen modules, refuses a module that is over its
budget or breaks its policy, then `baseline::save` and `baseline::append_history` store the entry (`Entry::from_measured`:
module values, per-scenario values, contract, inventory snapshot) and the reason.

## Responsibilities

- **Only `runner` starts Node**, and only `runner.mjs` knows how a module is instantiated and which
  of its memories are watched. Nothing else depends on the scenario protocol, a single
  `PEAKMARK_RESULT` line on stdout.
- **Memory means capacity of the configured module's observed memories.** `runner.mjs` attributes
  an instance to the configured module by the bytes it was compiled from, holds memories weakly,
  counts a shared memory once, and reports what it could not watch. `runner.rs` adds what only the
  artifact knows (declared memories, via `wasm`) and turns that into `Coverage`; `check` shows it
  as a note and never as a number.
- **The inventory is a model, not text.** `inventory` reads a validated module into typed data
  (`Inventory`) that `report` formats and `--json` serialises. Later parts (drift, size
  attribution, explanations) use the model, not the output. It states what the bytes contain;
  `Coverage` states what it could not decide, and a gap is never "not used". The baseline keeps a
  `snapshot::Snapshot` of it, not the model: compact, sorted, with an identity per declaration
  (module, name, kind), and extensible by optional fields; section sizes and the largest function bodies are such fields.
  `Inventory::functions` holds every body with its raw size, name and exports for later parts
  (the snapshot keeps only the largest); functions are matched across builds by name or export,
  never by index.
  Differences are warnings; only a `policy::Policy` that the project configured fails the check, and
  it is strict about what it cannot classify.
- **`reach` explains, it does not judge.** It reads the module itself (no `Inventory` needed) and
  answers by function index: a shortest path from an export or the start function, and the direct
  callers. The table is a pseudo node, so an indirect call is marked and never resolved, a
  function without a path is `no_static_root` (or `unknown`), and nothing in it is a size, a
  saving or a verdict. `report` adds the names from `Inventory::functions`. Nothing of it is
  stored in the baseline.
- **Only `check` decides** what is an error and what is a warning. A policy `Violation` is an error
  and no baseline waives it; an inventory difference is a warning. `report` formats those decisions
  and does not make its own.
- **Only `error` owns message text.** Modules return `Error` variants that carry the path or name
  involved; `Display` renders the sentence the command prints. Exit code 2 always means an `Error`.
- **Only `baseline` knows the file format.** It reads versions 1 and 2, writes 2, refuses others
  with `Error::BaselineVersion`, and replaces the file atomically. An entry from version 1 has no
  `contract` and no `scenarios`; `check` treats that as "compare the module values", never as zero.
- **Formats stay put.** The shapes of `peakmark.toml`, the baseline file, the history log and
  `--json` are public; changing them is a user-facing change.

## Dependencies

| Crate | Used for |
|---|---|
| `wasmparser` | validation, sections, memory and inventory of a module |
| `flate2`, `brotli` | gzip and brotli length |
| `serde`, `serde_json`, `toml` | configuration (including the policy), baseline, runner protocol, `--json` |
| `runemark` | report layout, colors and the error block |

Dev-only: `wat` builds the modules the tests use.
