---
title: What is measured
description: The five values peakmark reads, and why memory is read from the instance.
order: 1
---

| Value | Rule | How |
|---|---|---|
| `raw` | `size/raw` | bytes of the file |
| `gzip` | `size/gzip` | gzip at level 9 |
| `brotli` | `size/brotli` | brotli at quality 11, 22-bit window |
| `peak_memory` | `mem/peak` | linear memory held after the last run of a scenario |
| `growth_per_run` | `mem/growth` | what each run after the first adds, averaged |

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

## Why memory

Linear memory only grows. A module that once needed 30 MiB holds 30 MiB for as long as the instance
lives, so the highest value a scenario reaches is the number that matters, and what is held after
the last run is that peak. peakmark reads it from the instance itself, not from the process around
it.

Memory is counted in pages of 64 KiB.

## Measurement contract

`accept` records with the values what they were measured under: the measurement protocol and
warm-up (the first run of a scenario is not counted in `growth_per_run`), the gzip and brotli
settings, the Node version, the SHA-256 of the artifact, and per scenario the SHA-256 of its script
and its `repeat`. Which of these decide whether a later check can compare against the baseline is
described under [Budget and baseline](../budget-and-baseline/#what-can-still-be-compared).

## Reported, not gated

`--json` also carries the memory the module declares, the size of its code, data and custom
sections, and the time instantiation took. These values are reported and never fail a check.

## Compression

gzip and brotli sizes come from peakmark's own encoders. brotli is a Rust implementation: on the
same settings it lands about 0.1 % above the reference encoder, not on the same byte. Set budgets
from peakmark's numbers.
