---
title: Overview
description: What peakmark gates, where it stops, and how this documentation is organised.
order: 0
---

peakmark measures a built `.wasm` module — its size on disk and on the wire, and the capacity of its
linear memory after a scenario — and compares the result with a budget and with the values you last
accepted. Over the budget, the check fails. Past the baseline, it warns and names the command that
accepts the growth on purpose.

## What it covers

- Size of the file: raw, gzip and brotli.
- Peak memory capacity and its growth per run, read from the module's own linear memory by running
  a scenario in Node. Capacity is not live allocation; see
  [what the memory values are](concepts/measurements/#what-the-memory-values-are).
- An inventory of what each module imports, exports, declares and requires, read from the file
  without running it; see [Inventory](concepts/inventory/).
- Up to three evidence-backed explanations of what changed, attached to `check` by itself; see
  [Explanations](concepts/explanations/).
- A budget per module in `peakmark.toml` and a baseline in the repository, with the reason for
  every change.

## Where it stops

peakmark does not optimise a module, does not time it, and does not measure in a browser with DOM
and layout. It is built for modules made with `wasm-bindgen`, and it measures finished artifacts:
building them stays in your own pipeline.

## How the docs are organised

- **Getting started**: install peakmark and gate a first module.
- **Concepts**: what is measured, what the inventory says, and how budget and baseline work
  together.
- **Guides**: writing a scenario.
- **Reference**: the configuration file, the commands and the known limits.
