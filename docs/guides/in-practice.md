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
| chartlet | `chartlet` | 2.43 MB | 916.6 kB | 647.2 kB | 1.4 MiB peak, 0 B growth per run |
| lightweight-pdf | `lightweight-pdf` | 1.26 MB | 597.3 kB | 400.5 kB | 3.2 MiB peak, 0 B growth per run |
| barrierlab | `a11y-wasm` | 403.0 kB | 168.3 kB | 138.3 kB | not run |

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
budget of about 1 % over the first measurement, and the baseline was accepted once:

```toml
[[module]]
name     = "chartlet"
artifact = "packages/chartlet/src/chartlet.wasm"

[module.budget]
brotli = "653800 B"
```

From then on `peakmark check` fails when the module grows past its budget, and warns when the
baseline no longer matches.

chartlet and lightweight-pdf then got one scenario each. chartlet's module has no glue, so the
scenario copies a request in, renders the monthly-revenue chart as SVG and HTML and frees both
buffers, which is what the package's own wrapper does; 20 runs peak at 1.4 MiB, with 64 KiB added in
the first run and nothing after. lightweight-pdf's scenario renders a 40-row invoice table; 10 runs
peak at 3.2 MiB, with 1.4 MiB added in the first run and nothing after. In both, the growth budget
is `"0"` and the peak budget about 10 % over the measurement. barrierlab has no scenario yet, so its
memory is not measured.

For lightweight-pdf, `check --details` also shows where the 1.26 MB come from: 852 kB are data, of
which two segments of about 230 kB each are the largest, and 401 kB are code. peakmark reports the
sizes, not what the data is.

barrierlab's first measurement pointed at the output of `cargo build`, before `wasm-bindgen` ran. peakmark
measured it without complaint: 877.6 kB raw, with 1399 exports named `__wbindgen_describe_*`. Pointing
`artifact` at the published file, `packages/a11y-wasm/pkg/a11y_wasm_bg.wasm`, gives 403.0 kB, and
`check` reported what changed against the first baseline: 1409 exports removed, 20 added, and the
sizes of five functions and the sections. That is why the table shows the second value. peakmark
cannot tell which of two files is the one that ships; the `artifact` path is the project's decision.

## What this shows

- A budget with a written reason per change turns "the module grew" into a record that can be read
  months later.
- The memory scenario catches page growth per run, not leaks: a growth of `0` says the capacity
  stayed the same over the runs, nothing more (see
  [Release what you allocate](../scenarios/#release-what-you-allocate)).
- Size-only gating is a useful start; the scenario is the second step.
