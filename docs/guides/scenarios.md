---
title: Writing a scenario
description: Drive a module the way a page would, so peakmark can read the memory it holds.
order: 1
---

A scenario is an ES module whose default export is a function. It gets the loaded module and drives
it the way a page would:

```js
export default async function (engine) {
  const orders = new engine.Engine();
  // load data, run queries, pivot …
  orders.free();
}
```

peakmark runs the function `repeat` times in one fresh Node process, then reads the module's linear
memory. A scenario that throws stops the check and its message is shown. A scenario that does not
finish within its [deadline](../../reference/limits/#scenario-deadline) is stopped the same way.

## What the function receives

- With `glue` set, the glue is imported as an ES module. A default export that is a function is
  called as `wasm-bindgen --target web` expects, and the scenario gets the resulting module.
- Without `glue`, the module is instantiated without imports and the scenario gets its exports.

## Choosing a realistic flow

Linear memory only grows, so the value that matters is the highest one a flow reaches. Load the
amount of data a page would load and run the operations it would run. A scenario that touches
nothing proves nothing about memory.

## Release what you allocate

`growth_per_run` is what each run after the first adds, averaged. Free what the scenario creates
(`free()` on wasm-bindgen objects) and a run that returns to its starting point adds nothing. A
budget of `"0"` then fails when a leak appears.

## Several scenarios

Each scenario runs in its own Node process. With several scenarios on a module, the module's value
is the highest one, and a finding names the scenario it came from.

## Time and output

A scenario has 60 seconds by default. Set `timeout = "5m"` on a scenario, or `scenario_timeout` at
the top of `peakmark.toml`, if a flow needs longer. The deadline is for the whole scenario, so with a
large `repeat` count all runs. Awaiting a promise that never resolves, or a loop that never ends, ends
in a failure that names the scenario. Log as much as you like; only the end of stderr is kept for
failure messages.
