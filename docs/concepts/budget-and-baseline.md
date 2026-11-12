---
title: Budget and baseline
description: The budget is the limit, the baseline is what you last accepted.
order: 2
---

- The **budget** in `peakmark.toml` is the limit. A value over it fails the check with exit code 1.
- The **baseline** in `peakmark.baseline.json` is what you last accepted. A value past it by more
  than the tolerance (`tolerance = "1%"` by default) is a warning, and the report names the command
  that accepts the growth on purpose.
- `peakmark accept <module>... --reason "<text>"` stores the measured values as the new baseline and
  appends the reason to `peakmark.history.jsonl`. It refuses a module that is over its budget:
  raise the budget first.

Commit all three files. Growth then shows up in review as a decision with a reason, not as drift.

## Without a baseline

A module without a baseline warns with `baseline/missing` and names the next step:

```text
⚠ peakmark check: no baseline for engine

  Modules: 1   Over budget: 0   Grown: 0

● engine (1)
  - No baseline accepted yet [baseline/missing]

Next steps:
  • Accept the measured values as the new baseline
    $ peakmark accept engine --reason "..."
```
