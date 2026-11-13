---
title: Limits
description: What peakmark does not see.
order: 3
---

- gzip and brotli sizes come from peakmark's own encoders. Another encoder at the same settings
  lands within a fraction of a percent, not on the same byte.
- Memory means the capacity of the module's linear memories, not live allocations. Pages the
  module already reserved can fill up without any change, so a leak inside them is invisible; JS
  heap, runtime-native allocations and WebAssembly GC objects are not measured.
  [Details](../../concepts/measurements/#what-the-memory-values-are).
- Memory is counted in pages of 64 KiB. Growth smaller than a page per run shows only once it
  crosses a page; raise `repeat` to see it. More runs can expose growth, they cannot prove there
  is none.
- Only instances of the configured artifact are measured. Memory of other modules the scenario
  loads is left out, and a memory held by several instances is counted once.
- A memory the module declares and neither exports nor imports, and a module run in a worker
  thread, are not seen. They are reported as not observed, never as zero.
- The sum over memories adds capacities of instances that may not have lived together. It is not
  the peak of the process.
- Scenarios run in Node, not in a browser: DOM and layout are not part of the measurement.
- Only core modules are measured. A component is refused with exit code 2, as is a module that
  uses a feature outside WebAssembly 3.0.
- peakmark measures finished artifacts and does not build them.
- [Explanations](../../concepts/explanations/) are limited to three per module and rest on the
  stored inventory: against a baseline from before it, a growth is only named as not attributable.
  They state what the file or the trace contains, not a saving, a cause in the source or a security
  property.

## Scenario deadline

A scenario that does not finish within its deadline (60 seconds unless `scenario_timeout` or the
scenario's `timeout` says otherwise) is stopped, and the check ends with exit code 2. The message
names the module, the scenario and the deadline and shows the end of what the scenario wrote to
stderr. The deadline is enforced by peakmark itself, so a loop in JavaScript or WebAssembly that
never yields cannot get past it. peakmark does not run a stopped scenario again.

- **What is stopped.** On Unix, Node runs in its own process group and the whole group is killed,
  so processes the scenario started die with it. A process that left the group (`setsid`, a
  daemon) is not reached. On Windows peakmark ends the process tree with `taskkill /T /F`; that path
  is not covered by tests.
- **Leftover handles.** Node exits as soon as the result is written, so a timer or open socket the
  scenario leaves behind does not hold up the check.
- **Output.** peakmark keeps the last 16 KiB of stderr and discards the rest of what the runner
  prints, so a scenario that logs without end cannot exhaust memory. Ordinary output is fine.
- **A promise that never settles** on its own makes Node exit with an error right away; with a
  timer or socket still alive it runs into the deadline.
- This is hygiene for scenarios you trust, not a sandbox: the scenario can still read and write
  files and reach the network.
