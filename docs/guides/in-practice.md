---
title: peakmark in practice
description: What gating real WebAssembly modules brought in four projects, with their measured values.
order: 2
---

These are the modules of four projects that use peakmark, measured with the version described in
this documentation. The values are the ones in each project's `peakmark.baseline.json`.

| Project | Module | Raw | gzip | brotli | Memory |
| --- | --- | --- | --- | --- | --- |
| opengrid | `elements` | 1.08 MB | 450.6 kB | 355.8 kB | not run |
| opengrid | `engine` | 481.4 kB | 202.0 kB | 163.5 kB | 23.6 MiB peak, 0 B growth per run |
| opengrid | `xlsx` | 672.7 kB | 299.1 kB | 246.7 kB | 56.9 MiB peak, 0 B growth per run |
| opengrid | `collation` | 1.19 MB | 441.3 kB | 333.2 kB | not run |
| chartlet | `chartlet` | 2.46 MB | 932.0 kB | 658.6 kB | 1.4 MiB peak, 0 B growth per run |
| lightweight-pdf | `lightweight-pdf` | 1.26 MB | 597.3 kB | 400.5 kB | 3.2 MiB peak, 0 B growth per run |
| barrierlab | `a11y-wasm` | 473.6 kB | 198.6 kB | 160.8 kB | 1.6 MiB peak, 0 B growth per run |

## opengrid: every change of size has a reason

opengrid ships four modules and gates each one with a brotli budget, plus a memory scenario for the
engine and the workbook writer. The latest entries of the history file show what
`peakmark accept` recorded:

- **2026-11-11:** header texts in the workbook writer grew `xlsx` by 1.8 KiB; the budget was raised
  to 250000 B on purpose.
- **2026-11-12:** `wasm-opt --low-memory-unused` made `elements`, `engine` and `xlsx` smaller; the
  budgets were set to about 1 % over the new size.
- **2026-11-12:** the first memory scenario (50k rows in five pieces) found a peak of 56.9 MiB and
  no growth per run, and set the budget at 64 MiB.
- **2026-11-13:** locale-aware sorting added 1.4 KiB to `elements`, 4.2 KiB to `engine` and a
  new module `collation` of 333152 B, almost all of it data (1.08 MB raw of 1.19 MB).

Each entry names the cause and the number, so a later reader sees why a budget moved. The memory
scenario of `engine` also shows how the memory is used: 1.1 MiB after instantiation, +22.6 MiB in
the first run, then 0 B per run over five runs.

Checking the same files with the current peakmark also showed what the older baseline could not
say: the `xlsx` entry had no inventory and no record for its scenario, reported as the warnings
`baseline/inventory-missing` and `baseline/scenario-new` with exit code 0 until the baseline is
accepted again.

## chartlet, lightweight-pdf and barrierlab: a first gate

The other three projects had no gate. Each got a `peakmark.toml` with one module and a brotli
budget of about 1 % over the first measurement, and the baseline was accepted once. The numbers in
the example are the current ones; the first chartlet baseline was wrong, see below:

```toml
[[module]]
name     = "chartlet"
artifact = "packages/chartlet/src/chartlet.wasm"

[module.budget]
brotli = "665500 B"
```

From then on `peakmark check` fails when the module grows past its budget, and warns when the
baseline no longer matches.

chartlet and lightweight-pdf then got one scenario each. chartlet's module has no glue, so the
scenario copies a request in, renders the monthly-revenue chart as SVG and HTML and frees both
buffers, which is what the package's own wrapper does; 20 runs peak at 1.4 MiB, with 64 KiB added in
the first run and nothing after. lightweight-pdf's scenario renders a 40-row invoice table; 10 runs
peak at 3.2 MiB, with 1.4 MiB added in the first run and nothing after. In both, the growth budget
is `"0"` and the peak budget about 10 % over the measurement. barrierlab got a scenario afterwards:
its module takes the columns of a flat tree, so the scenario builds a synthetic page and scans it, no
DOM needed; 20 runs peak at 1.6 MiB, and the growth is 0 B. Because the tree is synthetic, this
measures the scan of that tree, not a real page.

For lightweight-pdf, `check --details` also shows where the 1.26 MB come from: 852 kB are data, of
which two segments of about 230 kB each are the largest, and 401 kB are code. peakmark reports the
sizes, not what the data is.

barrierlab's first measurement pointed at the output of `cargo build`, before `wasm-bindgen` ran. peakmark
measured it without complaint: 877.6 kB raw, with 1399 exports named `__wbindgen_describe_*`. Pointing
`artifact` at the published file, `packages/a11y-wasm/pkg/a11y_wasm_bg.wasm`, gives 403.0 kB, and
`check` reported what changed against the first baseline: 1409 exports removed, 20 added, and the
sizes of five functions and the sections. peakmark cannot tell which of two files is the one that
ships; the `artifact` path is the project's decision. Since then the module has grown from 403.0 kB
to 473.6 kB raw (138.3 kB to 160.8 kB brotli); I did not attribute the growth to individual changes.

### The first chartlet baseline was a stale build

The first chartlet measurement, 2.43 MB raw and 647.2 kB brotli, was taken from a `chartlet.wasm`
that was a few weeks older than the source. When CI built the module from the current source it came
out at 2.46 MB and 658.8 kB brotli and failed the budget. A rebuild on the local machine gave
2.46 MB and 658.6 kB brotli. peakmark measures the file it is pointed at; nothing in the file says
it is out of date. Build the module before `check`, locally as in CI, and take the first baseline
from that build.

## A forgotten `free()`: the growth budget on a real module

To see the growth budget fail on a real module, the lightweight-pdf scenario was run once with the
final `pdf.free()` removed, the mistake the guide
[Release what you allocate](../scenarios/#release-what-you-allocate) warns about. The module and the
rest of the scenario are the same:

| Scenario | Peak memory | Growth per run | Verdict |
| --- | --- | --- | --- |
| with `pdf.free()` | 3.2 MiB | 0 B | within budget |
| without it (10 runs) | 11.1 MiB | 896.0 KiB | over budget, exit code 1 |

`check` names both findings: `Memory growth per run 896.0 KiB, budget 0 B [mem/growth]` and
`Peak memory 11.1 MiB, budget 3.5 MiB [mem/peak]`. The leak is a construction of this page, not a
bug found in lightweight-pdf, and it shows page growth only: a leak that fits in pages the module
already reserved would not appear (see the note above).

## What this shows

- A budget with a written reason per change turns "the module grew" into a record that can be read
  months later.
- The memory scenario catches page growth per run, not leaks: a growth of `0` says the capacity
  stayed the same over the runs, nothing more (see
  [Release what you allocate](../scenarios/#release-what-you-allocate)).
- Size-only gating is a useful start; the scenario is the second step.
