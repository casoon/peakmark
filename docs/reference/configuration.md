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
| `[module.policy]` | optional compatibility rules for the [inventory](../../concepts/inventory/#policy): `require_exports`, `allow_imports`, `allow_features` |

Sizes take `B`, `kB`, `MB`, `KiB` or `MiB`; a plain number is bytes. A `peak_memory` or
`growth_per_run` budget needs at least one scenario for the module.

## `[module.policy]`

Without it, the inventory never fails a check: a difference from the accepted inventory is a
warning. A policy is a decision you write down, so breaking it fails the check with exit code 1,
and accepting the baseline does not waive it. Every key is optional.

```toml
[module.policy]
require_exports = [
  { name = "add", signature = "(i32, i32) -> i32" },
  { name = "memory" },
]
allow_imports = [
  { module = "env", name = "log", signature = "(i32) -> ()" },
]
allow_features = ["bulk_memory", "sign_extension", "mutable_globals"]
```

| Key | Meaning |
|---|---|
| `require_exports` | exports the module must have. With a `signature`, the export must be a function with exactly that signature; without, any export of that name satisfies it |
| `allow_imports` | the only imports the module may have, named by `module` and `name` together. With a `signature`, only a function of that signature is allowed; without, any type of that module and name is. Once the key is present, every import not listed is a violation, and `[]` allows none |
| `allow_features` | the only features the binary may need, by the names in [Features are derived](../../concepts/inventory/#features-are-derived). Once present, any other feature is a violation, and `[]` allows none |

A signature is written as `check --details` prints it, `(i32, i32) -> i32`; spaces do not matter.
Unknown keys, an unknown feature name and a rule without `name` (or, for imports, `module`) are a
configuration error. What counts as a violation where peakmark cannot be sure is in
[Policy](../../concepts/inventory/#policy).

## `[[scenario]]`

| Key | Meaning |
|---|---|
| `module` | the module it drives |
| `file` | the ES module, see [Writing a scenario](../../guides/scenarios/); once per module, because it is the scenario's identity in the baseline |
| `repeat` | how often the function runs, `1` by default, at least `1` |
| `timeout` | optional: this scenario's deadline, replacing `scenario_timeout`; see [Limits](../limits/#scenario-deadline) |

The deadline covers the whole scenario: starting Node, loading the module, every run and writing the
result. A scenario that exceeds it is stopped and the check ends with exit code 2.
