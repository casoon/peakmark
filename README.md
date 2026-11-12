# peakmark

Gate WebAssembly modules on size and memory against a baseline kept in the
repository.

peakmark measures a built `.wasm` module — its size on disk and on the wire,
and the linear memory it holds after a scenario — and compares the result with
a budget and with the values you last accepted. Over the budget, the check
fails. Past the baseline, it warns and names the command that accepts the
growth on purpose.

> Status: first release. Built for modules made with `wasm-bindgen`; the
> commands are `check` and `accept`.

## Why memory

Linear memory only grows. A module that once needed 30 MiB holds 30 MiB for as
long as the instance lives, so the highest value a scenario reaches is the
number that matters, and what is held after the last run is that peak.
peakmark reads it from the instance itself, not from the process around it.

## Install

```bash
cargo install peakmark
```

Or build from a checkout with `cargo install --path .`.

Official binaries come only from the GitHub releases of `casoon/peakmark`,
built there by the release workflow, never on a developer machine. To verify a
download by hand, check the hash and the signed build provenance:

```bash
shasum -a 256 -c peakmark-<version>-<target>.tar.gz.sha256
gh attestation verify peakmark-<version>-<target>.tar.gz --repo casoon/peakmark
```

Scenarios run in Node.js, so `node` has to be on the `PATH`. A configuration
without scenarios needs no Node.

## Quick start

Describe the modules in `peakmark.toml`, next to the baseline it will keep:

```toml
[[module]]
name     = "engine"
artifact = "pkg/engine_bg.wasm"
glue     = "pkg/engine.js"

[module.budget]
brotli         = "258 kB"
peak_memory    = "24 MiB"
growth_per_run = "0"

[[scenario]]
module = "engine"
file   = "bench/pivot.mjs"
repeat = 20
```

A scenario is an ES module whose default export gets the loaded module and
drives it the way a page would:

```js
export default async function (engine) {
  const view = new engine.Engine();
  view.free();
}
```

Accept the first measurement, then check on every change:

```bash
peakmark accept --all --reason "first measurement"
```

```bash
peakmark check
```

```text
[FAIL] peakmark check: over budget: engine

  Modules: 1   Over budget: 1   Grown: 0

* engine (1)
  - Peak memory 31.2 MiB, budget 24.0 MiB [mem/peak] at bench/pivot.mjs

Next steps:
  - Shrink the module, or raise its budget in the configuration on purpose
```

## What is measured

| Value | Rule | How |
|---|---|---|
| `raw` | `size/raw` | bytes of the file |
| `gzip` | `size/gzip` | gzip at level 9 |
| `brotli` | `size/brotli` | brotli at quality 11, 22-bit window |
| `peak_memory` | `mem/peak` | linear memory held after the last run of a scenario |
| `growth_per_run` | `mem/growth` | what each run after the first adds, averaged |

Each scenario runs in a fresh Node process. With several scenarios, a module's
value is the highest one, and the finding names the scenario it came from.

`--json` also carries the memory the module declares, the size of its code,
data and custom sections, and the time instantiation took. These are reported,
not gated.

## Budget and baseline

- The **budget** in `peakmark.toml` is the limit. A value over it fails the
  check with exit code 1.
- The **baseline** in `peakmark.baseline.json` is what you last accepted. A
  value past it by more than the tolerance (`tolerance = "1%"` by default)
  is a warning.
- `peakmark accept <module>... --reason "<text>"` stores the measured values
  as the new baseline and appends the reason to `peakmark.history.jsonl`.
  It refuses a module that is over its budget: raise the budget first.

Commit all three files. Growth then shows up in review as a decision with a
reason, not as drift.

## Commands

```text
peakmark check [--details] [--json]
peakmark accept (<module>... | --all) --reason <text>
```

| Option | Meaning |
|---|---|
| `--config <file>` | configuration file, `peakmark.toml` by default; paths in it are relative to it |
| `--details` | `check`: also list the measured values |
| `--json` | `check`: print JSON instead of the report |
| `--color <mode>` | `auto`, `always` or `never` |

Exit codes: `0` within budget, `1` over budget, `2` could not run.

## Limits

- gzip and brotli sizes come from peakmark's own encoders. Another encoder at
  the same settings lands within a fraction of a percent, not on the same
  byte, so set budgets from peakmark's numbers.
- Memory is counted in pages of 64 KiB. Growth smaller than a page per run
  shows only once it crosses a page; raise `repeat` to see it.
- `glue` is loaded as an ES module. A default export that is a function is
  called as `wasm-bindgen --target web` expects. Without `glue`, the module is
  instantiated without imports and the scenario gets its exports.
- A memory the module imports instead of exporting is not seen.

## License

MIT
