---
title: Configuration
description: Every key of peakmark.toml.
order: 1
---

`peakmark.toml` lists the modules, their budgets and their scenarios. Paths in it are relative to
the file. Unknown keys are an error.

```toml
tolerance = "1%"

[[module]]
name     = "engine"
artifact = "pkg/engine_bg.wasm"
glue     = "pkg/engine.js"

[module.budget]
raw            = "500 kB"
gzip           = "200 kB"
brotli         = "170 kB"
peak_memory    = "26 MiB"
growth_per_run = "0"

[[scenario]]
module = "engine"
file   = "bench/pivot.mjs"
repeat = 5
```

## Top level

| Key | Meaning |
|---|---|
| `tolerance` | growth over the baseline below this fraction is not reported, `"1%"` by default |
| `scenario_timeout` | how long a scenario may run before peakmark stops it, `"60s"` by default; applies to every scenario that sets no `timeout` |

Durations take `ms`, `s` or `m` (`"500ms"`, `"30s"`, `"2m"`) and must be longer than zero.

## `[[module]]`

| Key | Meaning |
|---|---|
| `name` | the name used in reports and by `accept`; each name once |
| `artifact` | the `.wasm` file |
| `glue` | optional: the `wasm-bindgen` JavaScript glue, loaded as an ES module |
| `[module.budget]` | optional limits: `raw`, `gzip`, `brotli`, `peak_memory`, `growth_per_run` |

Sizes take `B`, `kB`, `MB`, `KiB` or `MiB`; a plain number is bytes. A `peak_memory` or
`growth_per_run` budget needs at least one scenario for the module.

## `[[scenario]]`

| Key | Meaning |
|---|---|
| `module` | the module it drives |
| `file` | the ES module, see [Writing a scenario](../../guides/scenarios/) |
| `repeat` | how often the function runs, `1` by default, at least `1` |
| `timeout` | optional: this scenario's deadline, replacing `scenario_timeout`; see [Limits](../limits/#scenario-deadline) |

The deadline covers the whole scenario: starting Node, loading the module, every run and writing the
result. A scenario that exceeds it is stopped and the check ends with exit code 2.
