---
title: Commands
description: check, accept and history, their options and exit codes.
order: 2
---

```text
peakmark check [--details] [--json]
peakmark accept (<module>... | --all) --reason <text>
peakmark history [<module>...] [--since <date>] [--json]
```

## `check`

Measures every module and compares it with its budget and baseline. Under the findings of a
module it lists up to three [explanations](../../concepts/explanations/) of what changed, with
before and after, status, coverage and a next step; they never change the verdict.

```text
✔ peakmark check: 1 module within budget and baseline

  Modules: 1   Over budget: 0   Grown: 0

● Measured (1)
  - engine: raw size 463.8 kB, gzip size 195.8 kB, brotli size 159.2 kB, peak memory 23.6 MiB, memory growth per run 0 B
```

## `accept`

Stores the measured values as the new baseline and appends the reason to `peakmark.history.jsonl`.
It refuses a missing reason, a module that is over its budget and a module that breaks its
[policy](../../concepts/inventory/#policy); accepting waives neither.

```text
✔ Baseline accepted for engine
  engine: raw size 463.8 kB, gzip size 195.8 kB, brotli size 159.2 kB, peak memory 23.6 MiB, memory growth per run 0 B
```

## `history`

Lists the baselines accepted so far, read from `peakmark.history.jsonl`: per module, oldest first,
the day, the reason and either every value (the first entry) or what changed since the entry
before. It measures nothing, runs no scenario and changes no file, so it needs no Node and always
exits with `0` unless it cannot read the log.

```text
[INFO] peakmark history: engine

  Modules: 1   Accepted: 3

* engine (3)
  - 2026-11-02 "first measurement": raw size 451.2 kB, gzip size 190.4 kB, brotli size 155.0 kB, peak memory 23.6 MiB, memory growth per run 0 B
  - 2026-11-11 "pivot totals": raw size 463.8 kB (+12.6 kB), gzip size 195.8 kB (+5.4 kB), brotli size 159.2 kB (+4.2 kB)
  - 2026-11-20 "cache released after export": peak memory 21.1 MiB (-2.5 MiB)
```

Without module names it lists every module in the log, including one no longer in the
configuration. `--since 2026-11-01` lists only entries accepted on or after that day; the first of
them is still compared with the entry before it. A value measured under other settings than the
entry before it (other compression settings, another measurement protocol, other scenarios for the
memory values) is shown with both values and marked as not comparable, never as growth; see
[Changed coverage](../../concepts/budget-and-baseline/#changed-coverage). The values are the
module-wide ones; the memory values per scenario are in `--json`.

## Options

| Option | Meaning |
|---|---|
| `--config <file>` | configuration file, `peakmark.toml` by default |
| `--details` | `check`: also list the measured values and the [inventory](../../concepts/inventory/) of every module, including why its largest functions are in it |
| `--json` | `check`, `history`: print JSON instead of the report |
| `--all` | `accept`: every configured module |
| `--reason <text>` | `accept`: why the baseline changes |
| `--since <date>` | `history`: only entries accepted on or after this day, `YYYY-MM-DD` |
| `--color <mode>` | `auto`, `always` or `never` |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | within budget and policy |
| `1` | over budget or policy broken |
| `2` | could not run |

Findings carry a rule: `size/raw`, `size/gzip`, `size/brotli`, `mem/peak`, `mem/growth`,
`baseline/missing` and, for coverage that changed, `baseline/scenario-removed`,
`baseline/scenario-new`, `baseline/metric-missing`, `baseline/incomparable` and `baseline/runtime`;
see [Budget and baseline](../../concepts/budget-and-baseline/#changed-coverage). Differences from
the accepted inventory carry `baseline/interface-added`, `baseline/interface-removed`,
`baseline/interface-changed`, `baseline/feature-added`, `baseline/feature-removed` and the note
`baseline/inventory-missing`. A change of size is attributed with the notes
`baseline/size-sections` and `baseline/size-functions`, and `baseline/sizes-missing` if the baseline
predates them; see [Attributing a size change](../../concepts/inventory/#attributing-a-size-change).
Explanations carry the rules `explain/composition`, `explain/not-comparable`,
`explain/insufficient`, `explain/import-added`, `explain/feature-added`,
`explain/function-growth`, `explain/code-growth`, `explain/data-growth`, `explain/debug-growth`,
`explain/names-growth`, `explain/custom-growth` and `explain/memory-growth`; see
[Explanations](../../concepts/explanations/#the-rules). A [policy](../../concepts/inventory/#policy) adds
`policy/export-missing`, `policy/export-signature`, `policy/import-not-allowed`,
`policy/import-signature`, `policy/feature-not-allowed` and `policy/feature-unverifiable`. Only a
metric over its budget or a broken policy exits with `1`. Growth, a missing baseline, changed
coverage and inventory differences are warnings: exit code `0`, verdict `warning`. An unsupported
baseline format exits with `2`.

## JSON

`--json` prints one document: `verdict` (`passed`, `warning` or `failed`) and `modules`. Each module
has its `values`, `budget`, `baseline` (the accepted module values), `baseline_scenarios` and
`baseline_contract` (`null` for a baseline from format 1 or none), the `scenarios` measured
(with `repeat`, `sha256` and `node`), the `issues` that are over budget or past the baseline, and
`changes`: one object per finding about coverage, with its `rule`, a `level` of `warning` or `note`
and the `detail` of what changed; for an inventory difference that is the `part`, the declaration
(`module`, `name`, `kind`) and its accepted (`was`) and current (`now`) type. `violations` lists the
broken policy rules, each with `rule`, `level` (`error`), `message`, the `action` to take and the
`detail`; `baseline_inventory` is the accepted snapshot. For a size change the `detail` of
`baseline/size-sections` lists the parts that differ and an `action`; the one of
`baseline/size-functions` carries `confidence`, `status`, `changes`, the unmatched functions and
`unaccounted`. `inventory` is the static inventory of the artifact, with its own
`schema_version`; see [Inventory](../../concepts/inventory/). `reach` explains the largest functions:
its `assumptions`, its `coverage` and, per function, the `reachability`, a `path` from an export or
the start function and the direct `callers`; see
[Why a function is in the module](../../concepts/inventory/#why-a-function-is-in-the-module).

`explanations` lists the best three explanations of the module, in rank order (`rule`, `status`,
`subject`, `summary`, `evidence` with `was` and `now`, `reached`, `coverage`, `action`), and
`explanations_omitted` counts the ones the cap left out; see
[Explanations](../../concepts/explanations/#what-an-explanation-is-made-of). The findings they
rest on stay complete in `issues` and `changes`.

`history --json` prints `modules`, each with its `name` and `entries`, oldest first. An entry has
`accepted`, `reason`, `values`, `scenarios` (the memory values per scenario), `contract`, `first`
(`true` if no entry came before it) and `changes`: one object per module value that differs from
the entry before, with its `rule` (`size/raw` and so on), `was`, `now` (`null` where the value was
not measured) and `comparable`. The inventory snapshot of each entry stays in the log and is not
printed.
