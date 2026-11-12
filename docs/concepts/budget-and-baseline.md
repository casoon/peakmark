---
title: Budget and baseline
description: The budget is the limit, the baseline is what you last accepted.
order: 2
---

- The **budget** in `peakmark.toml` is the limit. A value over it fails the check with exit code 1.
- The **baseline** in `peakmark.baseline.json` is what you last accepted. A value past it by more
  than the tolerance (`tolerance = "1%"` by default) is a warning, and the report names the command
  that accepts the growth on purpose.
- `peakmark accept <module>... --reason "<text>"` stores the measured values as the new baseline and
  appends the reason to `peakmark.history.jsonl`. It refuses a module that is over its budget:
  raise the budget first. A module that breaks its [policy](../inventory/#policy) is refused the
  same way: neither a budget nor a policy is waived by accepting.

Commit all three files. Growth then shows up in review as a decision with a reason, not as drift.

## What the baseline stores

Per module, the baseline keeps:

- **`values`**: the module-wide numbers, `raw`, `gzip`, `brotli` and the largest `peak_memory` and
  `growth_per_run` of any scenario. They are a summary, and the basis for budgets, which always
  judge the module-wide maximum.
- **`scenarios`**: the memory values of each scenario, under its identity. Growth is compared here,
  so a scenario that grows is found even while another scenario holds more memory. Since the
  memory trace was added, an entry also keeps `after_instantiate`, `after_run` and what was not
  observed; they are for reading and are not compared. An entry without them stays valid and is
  never read as zero.
- **`contract`**: what the values were measured under (see below).
- **`inventory`**: the accepted interface of the module: imports and exports with their types, the
  memories and tables it keeps to itself, and the features it needs. It is compared by identity
  (module, name, kind), not by position; see [Against the baseline](../inventory/#against-the-baseline).
  Parts that later versions add (sections, function sizes) are stored next to these. An entry
  without it is valid and never read as an empty inventory.

A scenario's identity is its `file` as written in `peakmark.toml`, once per module. Renaming the
file is a removed and a new scenario.

```json
{
  "version": 2,
  "modules": {
    "engine": {
      "accepted": "2026-11-12",
      "reason": "first measurement",
      "values": { "raw": 463800, "gzip": 195800, "brotli": 159200, "peak_memory": 8388608 },
      "contract": {
        "runner_schema": 1,
        "warmup_runs": 1,
        "compression": "gzip-9 brotli-11-w22",
        "node": "v22.4.0",
        "artifact_sha256": "e3b0c442..."
      },
      "scenarios": {
        "bench/pivot.mjs": { "sha256": "ba7816bf...", "repeat": 5, "peak_memory": 8388608, "growth_per_run": 0 }
      },
      "inventory": {
        "imports": [{ "module": "env", "name": "log", "kind": "func", "type": "(i32) -> ()" }],
        "exports": [
          { "name": "add", "kind": "func", "type": "(i32, i32) -> i32" },
          { "name": "memory", "kind": "memory", "type": "1..256" }
        ],
        "features": ["bulk_memory", "mutable_globals"]
      }
    }
  }
}
```

The file is written deterministically (modules and scenarios sorted, every inventory list sorted) and holds one entry per
module and one record per scenario: it does not grow with every check. `accept` writes it to a
temporary file and replaces the old one in one step, so a failed write leaves the previous baseline
in place, and it changes only the modules it accepts. The history log gets one line per accepted
module and nothing on `check`.

## Changed coverage

A baseline is only worth something while it covers what is measured. `check` compares both and
reports every difference as a warning until the baseline is accepted again; nothing is accepted
automatically, and a value that was not measured is never counted as zero.

| Rule | Meaning |
|---|---|
| `baseline/scenario-removed` | the baseline has a scenario that is no longer configured |
| `baseline/scenario-new` | a scenario is measured that has no accepted values |
| `baseline/metric-missing` | a value was accepted and is not measured now, for example `peak_memory` after the scenarios were removed |
| `baseline/incomparable` | a measurement exists but cannot be compared, and says why |
| `baseline/runtime` | a note: the Node version differs from the accepted one |
| `baseline/interface-added`, `-removed`, `-changed` | an import, export, internal memory or table differs from the accepted inventory, see [Against the baseline](../inventory/#against-the-baseline) |
| `baseline/feature-added`, `-removed` | the features the binary needs differ from the accepted ones |
| `baseline/inventory-missing` | a note: the entry was accepted before the inventory was kept, so none of it is compared |
| `memory/coverage` | a note: part of what the scenario should measure was not observed, see [What is not observed](../measurements/#what-is-not-observed) |

All of these leave the exit code at 0, like growth, and turn the verdict to `warning`. Removing a
scenario or a budget on purpose is a decision, and accepting is how it is recorded:

```sh
peakmark accept engine --reason "dropped the pivot scenario"
```

## What can still be compared

Not comparable is narrower than not valid. A budget is judged on every check; only the comparison
with the baseline is skipped for what cannot be compared, and the rest is still judged.

| What changed | Effect |
|---|---|
| The scenario script (its SHA-256) | that scenario's memory is not compared (`baseline/incomparable`) |
| The scenario's `repeat` | the same: peak memory and growth per run both depend on it |
| gzip or brotli settings (a peakmark release that changes them) | `gzip` and `brotli` are not compared; `raw` and memory are |
| The measurement protocol or warm-up (`runner_schema`, `warmup_runs`) | memory is not compared; sizes are |
| The inventory (imports, exports, limits, features) | a warning per difference; nothing else is skipped, because it is compared by declaration, not by position |
| The artifact (its SHA-256) | nothing: it changes with every build and only identifies the file |
| The Node version | nothing; reported as `baseline/runtime`, because memory is read from the instance, not from the process around it |

A changed scenario or repeat is often legitimate. Review it, then accept to compare against the new
values from then on.

## Baseline format versions

`peakmark.baseline.json` has a `version`. peakmark reads versions 1 and 2 and writes 2.

- **Inventory** is an optional field of an entry in version 2 and does not raise the version: older
  peakmark versions ignore it, and this one reads an entry without it.
- **Version 1** (peakmark 0.1) has module values only. It is read as it is: sizes and the
  module-wide memory values are compared as before, and every scenario shows as
  `baseline/scenario-new`. `accept` replaces the entry of the module it accepts with a version 2
  entry and leaves other modules' entries untouched, so a project migrates module by module. The
  file is saved as version 2 from then on.
- **Any other version** stops the command with exit code 2 and a message naming the file and the
  versions it reads. A newer file asks for a newer peakmark; version 0 asks to delete the file and
  accept again.

## Without a baseline

A module without a baseline warns with `baseline/missing` and names the next step:

```text
⚠ peakmark check: no baseline for engine

  Modules: 1   Over budget: 0   Grown: 0

● engine (1)
  - No baseline accepted yet [baseline/missing]

Next steps:
  • Accept the measured values as the new baseline
    $ peakmark accept engine --reason "..."
```
