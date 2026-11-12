---
title: Commands
description: check and accept, their options and exit codes.
order: 2
---

```text
peakmark check [--details] [--json]
peakmark accept (<module>... | --all) --reason <text>
```

## `check`

Measures every module and compares it with its budget and baseline.

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

## Options

| Option | Meaning |
|---|---|
| `--config <file>` | configuration file, `peakmark.toml` by default |
| `--details` | `check`: also list the measured values and the [inventory](../../concepts/inventory/) of every module, including why its largest functions are in it |
| `--json` | `check`: print JSON instead of the report |
| `--all` | `accept`: every configured module |
| `--reason <text>` | `accept`: why the baseline changes |
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
A [policy](../../concepts/inventory/#policy) adds
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
