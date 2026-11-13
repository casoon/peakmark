---
title: Recipes
description: Compare build variants and guard against stale memory views with what peakmark has today.
order: 3
---

Two situations come up often and need no new command.

## Compare build variants

To see what a build setting (`opt-level`, `wasm-opt` flags, a feature) does to size and memory,
list each build as a module of its own. peakmark does not build anything: produce the variants
yourself, then point one module at each artifact.

```toml
[[module]]
name     = "engine-size"
artifact = "target/variants/size/engine_bg.wasm"
glue     = "target/variants/size/engine.js"

[module.budget]
brotli = "170 kB"

[[module]]
name     = "engine-speed"
artifact = "target/variants/speed/engine_bg.wasm"
glue     = "target/variants/speed/engine.js"

[module.budget]
brotli = "190 kB"

[[scenario]]
module = "engine-size"
file   = "scripts/engine.mjs"

[[scenario]]
module = "engine-speed"
file   = "scripts/engine.mjs"
```

Both modules run the same scenario file, which is allowed: a scenario file may be listed once
per module. Run `peakmark check --details` and read the values side by side. Accept only the
variant you keep, or give each its own budget and accept them separately.

What this does not do: peakmark does not rank the variants, and it does not say which setting
made the difference. Compare the numbers yourself, and compare modules built from the same source.

## Check that a binding survives memory growth

Linear memory can move when it grows, and a JavaScript view created before the growth is then
empty. peakmark cannot see whether a binding keeps using such a view, so it does not claim to
detect it. Assert the behavior in the scenario instead: do the same read before and after an
operation that grows memory, and fail when the result differs.

The method names below stand for your own binding.

```js
export default async function (engine) {
  const probe = engine.summarize("a,b\n1,2\n");
  const before = JSON.stringify(probe);

  const big = engine.load(largeInput()); // grows linear memory
  const after = JSON.stringify(engine.summarize("a,b\n1,2\n"));
  big.free();

  if (after !== before) {
    throw new Error(`result changed after memory growth: ${before} != ${after}`);
  }
}
```

A scenario that throws stops the check with exit code 2 and shows the message. The scenario only
covers the calls it makes: it shows that these calls survive growth, not that every binding does.
Make sure the input is large enough to grow the memory, which the per-scenario trace in `--json`
(`after_run`) confirms.
