---
title: Quickstart
description: Gate a module on size and memory in four steps.
order: 2
---

The numbers on this page come from a real run: the engine module of
[opengrid](https://github.com/casoon/opengrid), loaded and queried with 100 000 rows.

## 1. Describe the module

Put `peakmark.toml` next to the baseline it will keep. Paths in it are relative to the file.

```toml
[[module]]
name     = "engine"
artifact = "pkg/engine_bg.wasm"
glue     = "pkg/engine.js"

[module.budget]
brotli         = "170 kB"
peak_memory    = "26 MiB"
growth_per_run = "0"

[[scenario]]
module = "engine"
file   = "bench/pivot.mjs"
repeat = 5
```

## 2. Write a scenario

A scenario is an ES module whose default export gets the loaded module and drives it the way a page
would. See [Writing a scenario](../../guides/scenarios/).

## 3. Accept the first measurement

Until something is accepted, `peakmark check` warns that there is no baseline and names the command:

```bash
peakmark accept --all --reason "first measurement"
```

## 4. Check on every change

```bash
peakmark check
```

Within budget and baseline it passes. Over the budget it exits with 1 and names the scenario that
reached the value; here with `peak_memory` lowered to 20 MiB:

```text
✖ peakmark check: over budget: engine

  Modules: 1   Over budget: 1   Grown: 0

● engine (1)
  - Peak memory 23.6 MiB, budget 20.0 MiB [mem/peak] at bench/pivot.mjs

Next steps:
  • Shrink the module, or raise its budget in the configuration on purpose
```

Raise the budget in `peakmark.toml` on purpose, or shrink the module. Growth that stays under the
budget but passes the baseline is only a warning; see
[Budget and baseline](../../concepts/budget-and-baseline/).

## In a pipeline

`peakmark check` exits with `0` within budget, `1` over budget and `2` if it could not run, so it
works as a step in any CI. `--json` prints the result for other tools.
