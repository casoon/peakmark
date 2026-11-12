---
title: Limits
description: What peakmark does not see.
order: 3
---

- gzip and brotli sizes come from peakmark's own encoders. Another encoder at the same settings
  lands within a fraction of a percent, not on the same byte.
- Memory is counted in pages of 64 KiB. Growth smaller than a page per run shows only once it
  crosses a page; raise `repeat` to see it.
- A memory the module imports instead of exporting is not seen.
- Scenarios run in Node, not in a browser: DOM and layout are not part of the measurement.
- peakmark measures finished artifacts and does not build them.

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
