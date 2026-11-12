# peakmark

Gate WebAssembly modules on size and memory against a baseline kept in the
repository.

peakmark measures a built `.wasm` module — its size on disk and on the wire,
and the capacity of its linear memory after a scenario — and compares the result with
a budget and with the values you last accepted. Over the budget, the check
fails. Past the baseline, it warns and names the command that accepts the
growth on purpose.

> Built for modules made with `wasm-bindgen`; the commands are `check` and
> `accept`.

Documentation: <https://casoon.github.io/peakmark/>

## Install

```bash
cargo install peakmark
```

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
brotli         = "170 kB"
peak_memory    = "26 MiB"
growth_per_run = "0"

[[scenario]]
module = "engine"
file   = "bench/pivot.mjs"
repeat = 5
```

Accept the first measurement, then check on every change:

```bash
peakmark accept --all --reason "first measurement"
peakmark check
```

With the peak-memory budget lowered to 20 MiB, the check fails and names the
scenario (a real run, on the engine module of opengrid):

```text
✖ peakmark check: over budget: engine

  Modules: 1   Over budget: 1   Grown: 0

● engine (1)
  - Peak memory 23.6 MiB, budget 20.0 MiB [mem/peak] at bench/pivot.mjs

Next steps:
  • Shrink the module, or raise its budget in the configuration on purpose
```

How scenarios are written, what is measured and how budget and baseline work
together is in the [documentation](https://casoon.github.io/peakmark/docs/).

## License

MIT
