# peakmark

Gate WebAssembly modules on size and memory against a baseline kept in the
repository.

peakmark measures a built `.wasm` module — its size on disk and on the wire,
and the capacity of its linear memory after a scenario — and compares the result
with a budget and with the values you last accepted. Over the budget, the check
fails. Past the baseline, it warns and names the command that accepts the growth
on purpose.

Documentation: <https://casoon.github.io/peakmark/>

## Install

```bash
npm install --save-dev peakmark
```

or run it without installing:

```bash
npx peakmark check
```

The package ships the peakmark binaries for macOS (arm64, x64) and Linux (arm64,
x64; statically linked, so any distribution). On another platform, install with
`cargo install peakmark`. Scenarios run in Node.js, which a project installing
this package already has.

## Use

```bash
npx peakmark accept --all --reason "first measurement"
npx peakmark check
npx peakmark history
```

The configuration, `peakmark.toml`, is described in the
[documentation](https://casoon.github.io/peakmark/docs/reference/configuration/).

## Where the binaries come from

The binaries in this package are the ones attached to the GitHub release of the
same version of `casoon/peakmark`, built there by the release workflow, never on
a developer machine. Before publishing, each archive is checked against the
release's `SHA256SUMS` and its signed build provenance; the package itself is
published from that workflow with npm provenance. To check the package:

```bash
npm audit signatures
```

## License

MIT
