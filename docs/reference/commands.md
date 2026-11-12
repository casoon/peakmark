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
It refuses a missing reason and a module that is over its budget.

```text
✔ Baseline accepted for engine
  engine: raw size 463.8 kB, gzip size 195.8 kB, brotli size 159.2 kB, peak memory 23.6 MiB, memory growth per run 0 B
```

## Options

| Option | Meaning |
|---|---|
| `--config <file>` | configuration file, `peakmark.toml` by default |
| `--details` | `check`: also list the measured values |
| `--json` | `check`: print JSON instead of the report |
| `--all` | `accept`: every configured module |
| `--reason <text>` | `accept`: why the baseline changes |
| `--color <mode>` | `auto`, `always` or `never` |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | within budget |
| `1` | over budget |
| `2` | could not run |

Findings carry a rule: `size/raw`, `size/gzip`, `size/brotli`, `mem/peak`, `mem/growth` and
`baseline/missing`.
