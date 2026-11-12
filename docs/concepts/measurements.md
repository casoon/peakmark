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

## Why memory

Linear memory only grows. A module that once needed 30 MiB holds 30 MiB for as long as the instance
lives, so the highest value a scenario reaches is the number that matters, and what is held after
the last run is that peak. peakmark reads it from the instance itself, not from the process around
it.

Memory is counted in pages of 64 KiB.

## Reported, not gated

`--json` also carries the memory the module declares, the size of its code, data and custom
sections, and the time instantiation took. These values are reported and never fail a check.

## Compression

gzip and brotli sizes come from peakmark's own encoders. brotli is a Rust implementation: on the
same settings it lands about 0.1 % above the reference encoder, not on the same byte. Set budgets
from peakmark's numbers.
