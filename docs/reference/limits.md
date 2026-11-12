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
- Only core modules are measured. A component is refused with exit code 2, as is a module that
  uses a feature outside WebAssembly 3.0.
- peakmark measures finished artifacts and does not build them.
