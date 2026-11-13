---
title: What is measured
description: The five values peakmark reads, and what the memory values do and do not say.
order: 1
---

| Value | Rule | How |
|---|---|---|
| `raw` | `size/raw` | bytes of the file |
| `gzip` | `size/gzip` | gzip at level 9 |
| `brotli` | `size/brotli` | brotli at quality 11, 22-bit window |
| `peak_memory` | `mem/peak` | highest capacity of the module's observed linear memories in a scenario |
| `growth_per_run` | `mem/growth` | capacity each run after the first adds, averaged |

## Validation comes first

Before any value is measured, peakmark validates the whole artifact with wasmparser's validator,
function bodies included. An artifact that does not validate stops the check with exit code 2 and
an error naming the file; nothing is measured, and Node is not started. Validation does not
certify anything about the module's security.

Only core modules are measured, with the features of WebAssembly 3.0 (bulk memory, reference
types, SIMD, threads, exception handling, multiple and 64-bit memories, tail calls, GC and the
rest of that release). Two cases are told apart from an invalid binary:

- a module that is valid but uses a feature outside that set, such as a custom page size;
- a WebAssembly component, which is refused rather than measured as one module.

## What the memory values are

peakmark reads the **capacity** of the module's linear memories: the byte length of the memory
object, counted in pages of 64 KiB. Capacity only grows, so the highest value a scenario reaches
is the number that matters. It is read from the instance, not from the process around it.

Capacity is not use. A module that reserved 30 MiB holds 30 MiB whether 3 or 29 of them are in
use. That has consequences:

- A leak that takes up pages the module already reserved does not change capacity, and a cache
  that fills legitimately does. The metric cannot tell the two apart.
- Capacity moves in whole pages. Growth below a page per run shows only once it crosses one.
- `growth_per_run` is derived from a finite trace of `repeat` samples. A longer run can show
  growth a short one hides; a trace without growth does not prove the absence of a leak or a
  plateau.
- peakmark does not claim a leak from a missing `free()`. It sees only what the memory did.

### What is covered

The values cover one configured module: the instances created from the configured artifact.

- **Which instances.** Every `WebAssembly.instantiate`, `new WebAssembly.Instance`, `compile`,
  `new WebAssembly.Module` and streaming variant is watched, whoever calls it (the glue, the
  scenario, the runner). An instance counts when its module was compiled from the bytes of the
  configured artifact. A second instance of it is counted too.
- **Instances of other modules** are counted in `coverage.unrelated_instances` and their memory is
  left out of every value.
- **Which memories.** The memories an instance exports, and the memories it imports and was given
  in its imports, even if it does not export them. A memory used by several instances, or
  imported by one and exported by another, is counted once; `instances` on its entry says how many
  use it.
- **The value** for a sample is the sum of the capacities of the memories that are still
  reachable at that moment. It is the capacity the observed instances hold, and is not a
  process peak: memories of instances with different lifetimes are added up as if they lived
  together. peakmark holds memories weakly, so it does not keep a memory alive that the module and
  the scenario let go of; a collected memory leaves the sum (its entry in the trace turns `null`).
  Whether and when that happens depends on the garbage collector.

### What is not observed

An observation path peakmark cannot watch is reported under `coverage.unobserved` in `--json`
and as a `memory/coverage` note in the report. It is never counted as zero.

- A memory the module declares and neither exports nor imports (`internal_memory`).
- A module run in a worker thread (`workers`): the memory there is not seen.
- Memory of the JavaScript heap, runtime-native allocations, WebAssembly GC objects and
  allocator state. These are not linear memory capacity at all.

A module of which no memory can be observed stops the check with exit code 2 and says so.

### Observed, estimated, not available

| Quantity | Status | May fail a check |
|---|---|---|
| `raw`, `gzip` | observed on the file | yes |
| `brotli` | observed; a Rust port about 0.1 % above the reference | yes, with budgets set from peakmark's numbers |
| `peak_memory` | observed capacity of the covered memories, with the coverage above | yes |
| `growth_per_run` | derived from a finite trace, in whole pages; proves neither a leak nor a plateau | yes, read as capacity growth |
| first-run growth (`warmup_growth`) | observed, reported next to the later growth | no |
| initialization time | observed, noisy; includes importing the glue, compiling and instantiating | no |
| live allocations, forgotten `free()`, wrapper lifetimes | not available | no |
| JS heap, WebAssembly GC objects, runtime-native memory | not available | no |
| fragmentation, allocation sites, bytes crossing the JS/WebAssembly boundary | not available | no |

### Reading a trace

`--json` keeps, per scenario, the capacity after instantiation (`after_instantiate`) and after every
run (`after_run`), the growth of the first run (`warmup_growth`), and for each observed memory its
own trace (`memories`). `check --details` prints the same split: capacity after instantiation,
growth in the first run, growth per run after it.

The first run is a warm-up: it may fill caches and pools, so it is not part of `growth_per_run`.
Growth that appears from the second run on is what the later runs added, averaged. A scenario
that grows once, late, shows as a small average over many runs; look at `after_run` for where.

## Measurement contract

`accept` records with the values what they were measured under: the measurement protocol and
warm-up (the first run of a scenario is not counted in `growth_per_run`), the gzip and brotli
settings, the Node version, the SHA-256 of the artifact, and per scenario the SHA-256 of its script
and its `repeat`. Which of these decide whether a later check can compare against the baseline is
described under [Budget and baseline](../budget-and-baseline/#what-can-still-be-compared).

## Reported, not gated

`--json` also carries the memory the module declares, the size of its code, data and custom
sections, and the time instantiation took. These values are reported and never fail a check.
What the module imports, exports and requires is the [inventory](../inventory/), read from the file
without running it.

## Allocator probe

A Rust module that installs the opt-in [allocator probe](../../guides/allocator-probe/) also
reports what it requested from its allocator: live bytes at the end, their peak, and the numbers
of allocations and frees. These are observed values of a different thing than capacity, they are
`null` ("not observed") without the probe, and they never decide a check.

## Compression

gzip and brotli sizes come from peakmark's own encoders. brotli is a Rust implementation: on the
same settings it lands about 0.1 % above the reference encoder, not on the same byte. Set budgets
from peakmark's numbers.
