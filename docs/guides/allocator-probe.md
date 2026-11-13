---
title: Allocator probe
description: An opt-in allocator wrapper for Rust modules that lets peakmark report what the module requested next to the capacity it reserved.
order: 4
---

Capacity says how many pages a module reserved, not how much it asked its allocator for. The
allocator probe, the crate `peakmark-probe` in this repository (`probe/`), closes part of that
gap for Rust modules. It is a prototype: opt-in, four numbers, no verdict.

## Installing it

Add the crate to the module's WebAssembly build and wrap the global allocator:

```toml
[dependencies]
peakmark-probe = { git = "https://github.com/casoon/peakmark", package = "peakmark-probe" }
```

```rust
use std::alloc::System;
use peakmark_probe::Probe;

#[global_allocator]
static ALLOCATOR: Probe<System> = Probe::new(System);
```

Put both behind a feature of your own if the probe should not ship. The crate has no
dependencies and adds five small exports to the module. Nothing else about the module changes:
peakmark finds the exports on its own, and a module without them is simply measured as before.

## What it observes

| Value | Meaning |
|---|---|
| `live` | Bytes requested and not yet freed, at the end of the scenario |
| `peak` | The highest `live` reached |
| `allocations` | Allocation requests that succeeded, each `realloc` counted once |
| `frees` | Frees, each `realloc` counted once |

A `realloc` changes `live` by the difference of the sizes. Sizes are what was requested, not what
the allocator reserved: padding, headers and fragmentation are not in them. Fragmentation and
call sites are deliberately not observed.

The values are exported as `peakmark_probe_live`, `peakmark_probe_peak`,
`peakmark_probe_allocations` and `peakmark_probe_frees` (functions without parameters that return
an `i64`) and `peakmark_probe_version`. peakmark reads them after the last run of the scenario.
A scenario can call them as well; in JavaScript they return `BigInt`.

## In the report

`check --details` adds one line per scenario:

```text
render.mjs: allocator probe, requested live 90 B at the end, peak 1.2 MiB, 119941 allocations,
119931 frees, in 1 of 1 instance (requested bytes, not capacity)
```

`--json` carries the same values as `probe` of each scenario:
`{ "instances", "live", "peak", "allocations", "frees" }`. When no observed instance carries the
probe, `probe` is `null` ("not observed") and the details have no probe line. That is not zero: a module
without the probe has not reported that it allocated nothing. With several instances the values
are summed, and `instances` says how many of the observed ones carried the probe; the sum of
peaks is an upper bound of the peak of the sum.

## What it is not

- It has no budget, no baseline and no verdict. A check never fails or warns because of a probe
  value, and `accept` does not store them.
- It does not see allocations that bypass the Rust global allocator, nor memory a module
  manages itself.
- It does not explain capacity. A peak of 1.2 MiB requested next to 3.4 MiB of capacity is a
  fact about two different things.

## What it costs

The no-go test from the proposal was a capacity trace that differs by more than one page, or a
doubled runtime. Measured on two real modules, each built twice from the same commit (with and
without the probe, `wasm-opt -Oz`), the scenario of the project's own `peakmark.toml` on Node 24:

| Module | Capacity trace | Scenario loop, median of 9 | File size |
|---|---|---|---|
| chartlet | identical, 1.4 MiB at every run | 83.9 ms plain, 78.4 ms probed (300 runs) | 2,425,288 B → 2,424,124 B |
| lightweight-pdf | identical, 3.4 MiB at every run | 156.1 ms plain, 161.7 ms probed (100 runs) | 1,257,425 B → 1,255,584 B |

The traces matched page for page, and the runtime differences are within the spread of the
runs in both directions. The two probed files happen to be about 1–2 KB smaller than the
plain ones; that is an observation on these two builds, with no cause attached.
