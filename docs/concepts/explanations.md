---
title: Explanations
description: The up to three evidence-backed explanations `check` attaches to a finding, how they are ranked, what they never do, and what they cost.
order: 4
---

`peakmark check` explains a change by itself. Under the findings of a module it lists up to three
explanations: which part of the artifact or of a scenario moved, the value before and after, how
sure that statement is, what the evidence does not cover and what to look at next. No extra command
and no option: `--json` carries the same list under `explanations`.

```text
* Explanations: fixture (3) [advisory]
  - 1. The module imports 1 more binding (exact; import env.log func () -> (); declarations bytes 37 B -> 48 B). Next: Make sure the host provides env.log with the declared type before this build is deployed; peakmark does not instantiate or run the module. Coverage: Compared with the accepted inventory by module, name and kind; why the import was added and what it costs is not known. [explain/import-added]
  - 2. Embedded data grew by 399 B (exact; data bytes 8 B -> 407 B; largest data segment now 400 B). Next: Examine the embedded data: the largest segment sits at memory 0, offset 0x0. peakmark does not show what it holds, so find it in the source or the build. Coverage: The data section is accounted exactly; the segments of the accepted build are not stored, so the new bytes are not tied to one segment. [explain/data-growth]
  - 3. Debug information grew by 212 B (exact; debug bytes 0 B -> 212 B; custom section .debug_info now 212 B). Next: If this is the artifact you deploy, build the deployment artifact without debug information and keep the debug build separately. peakmark does not modify files. Coverage: The sections are accounted exactly by name; their content is not interpreted. 1 further explanation was left out; the findings above stay complete and `--json` lists them in full. [explain/debug-growth]
```

## What an explanation is made of

Explanations use only what the check measured anyway: the [inventory](../inventory/), the section and
function differences against the baseline with the confidence of the match, the interface
differences, the [reference graph](../inventory/#why-a-function-is-in-the-module) of the largest
functions and the memory trace of the scenarios. Nothing is analysed for them, nothing is run, and
a statement that the evidence does not carry is not made.

| Field in `--json` | Meaning |
|---|---|
| `rule` | stable id, `explain/...` |
| `status` | `exact`, `estimated` or `hypothesis` |
| `subject` | what it is about: a scenario, a function, a part of the file |
| `summary` | the statement |
| `evidence` | `what`, `was` and `now`, in bytes; a side that is not known is `null` |
| `reached` | for a function: the `reachability`, the `root`, the `path` and the number of direct `callers` |
| `coverage` | what the evidence does not cover |
| `action` | what to look at or do next; it names evidence and never a saving |

`status` says how far the statement goes. **Exact**: the numbers are what the file or the trace
contains and nothing is left out, for example a section that grew by 212 bytes. **Estimated**: read
from the data, but the match or the coverage is partial, for example a function matched while other
functions could not be, or a memory trace that leaves out a memory nobody exports. **Hypothesis**:
the data allows the statement and does not prove it, for example a function that entered the list of
the largest and may be new or may have been smaller.

## The rules

| Rule | When | Status |
|---|---|---|
| `explain/composition` | first run, no baseline: what the file is made of, no growth claimed | exact |
| `explain/not-comparable` | a scenario or value is measured but cannot be compared (edited script, changed `repeat`, other settings) or was accepted and is not measured now | exact |
| `explain/insufficient` | a size grew and the baseline has no section or function sizes | exact |
| `explain/import-added` | the module imports more than it did | exact |
| `explain/feature-added` | the binary needs a feature it did not | exact |
| `explain/function-growth` | the function that grew most, found by name or export, with its path from a root | exact, estimated or hypothesis |
| `explain/code-growth` | the code grew and no function can be named | exact |
| `explain/data-growth` | the data section grew | exact |
| `explain/debug-growth` | debug information grew | exact |
| `explain/names-growth` | the `name` section grew | exact |
| `explain/custom-growth` | other custom sections grew | exact |
| `explain/memory-growth` | a scenario's capacity grew, located at instantiation, in the first run, per run or later | exact or estimated |

Data growth, debug growth and a function that grew are three different explanations with three
different next steps: data is examined in the source or the build, debug information is a question
about the deployment artifact, a function is reviewed.

Memory is located in the trace of the scenario. It names the first place where the capacity differs
from the accepted trace, and keeps saying that capacity is reserved pages: a capacity that grows
with every run is not by itself a leak. A baseline from before the trace was kept only allows the
peak to be compared, so that explanation is `estimated` and says so.

## Order, cap and what stays visible

Within a module the list is ordered by

1. input first: what cannot be compared or attributed comes before any optimization hint,
2. then changes of the interface, which can stop a module from loading,
3. then size and memory;

within these by status (exact before estimated before hypothesis), then by the bytes involved
(largest first), then by rule and subject. The order is the same on every run, and the same rule for
the same subject is listed once.

At most three are listed. The cap applies to explanations only: every hard failure, every
baseline, coverage and policy finding stays in the report and in `--json` (`issues`, `changes`,
`violations`), and the explanations are printed in a group of their own after all findings. The
number left out is in `explanations_omitted`.

## What they never do

Explanations do not change the verdict or the exit code, do not change a budget, do not accept a
baseline, do not modify, strip or rebuild the artifact, do not install anything, do not use the
network and do not run discovered scripts. A default check of sizes stays offline and does not need
Node. There is no statement about a saving: the sizes are raw bytes, and gzip and Brotli do not add
up from them. There is no security statement either; what a new import is used for is not known,
and an empty list of explanations is not an all-clear.

## When there is no explanation

No evidence means no explanation. A size issue with only a changed set of declarations or headers,
a shrinking part, a change inside the tolerance and a module that equals its baseline list nothing.
Parts that are not explained on purpose are the declarations (the interface differences carry them)
and the headers. A module without names or exports is never matched by index: its growth is
reported as `explain/code-growth` for the part, without a function.

## Cost

Inventory, section accounting and the reference graph of the largest functions are computed for
every check; the explanations only read their results. Measured on one machine (release build,
single run, Apple silicon), the explanations take well under a millisecond, and the reference graph
is a small share of the work:

| Artifact | Functions | Whole measurement | Inventory | Reference graph | Explanations |
|---|---|---|---|---|---|
| 202 kB, real | 497 | 178 ms | 3.4 ms | 0.9 ms | < 0.01 ms |
| 266 kB, real | 1 190 | 269 ms | 2.4 ms | 2.2 ms | < 0.01 ms |
| 1.1 MB, real | 2 460 | 1 359 ms | 10.0 ms | 4.9 ms | < 0.01 ms |
| 1.4 MB, real | 2 461 | 1 832 ms | 5.1 ms | 2.6 ms | < 0.01 ms |
| 3.1 MB, synthetic | 40 001 | 5 476 ms | 43.2 ms | 29.6 ms | 0.5 ms |

The whole measurement is dominated by Brotli at quality 11. Peak resident memory of the process
grows by the reference graph by about 0.3 MiB (202 kB), 1 MiB (1.4 MB) and 20 MiB (3.1 MB with
40 000 functions) on top of the inventory; the whole measurement of the 1.4 MB module peaks at
65 MiB, of which almost all is the compression. Since the graph costs 1 to 2 percent of the
measurement it is built for every check; a deeper analysis would be requested explicitly. There is
none yet.

## Noise and how it is held back

Evaluated on a declared corpus: the fixtures of the test suite (one module per explanation, each
checked against the evidence it must cite and against the case where it must stay silent), four
finished modules from the author's own projects (202 kB to 1.4 MB) for the first run and the cost
above, and a synthetic module of 40 000 functions. There is no external user feedback yet; this
page is to be revised when there is.

Rules that keep it quiet, each with a test:

- A size explanation needs a size finding (`size/raw`, `size/gzip` or `size/brotli`): a few bytes
  inside the tolerance explain nothing.
- Only a part that grew is explained; a part that shrank is not.
- Declarations and headers are not explained by size; the interface differences speak for them.
- A function must be matched by name or export and occur once; otherwise it is not named.
- A memory value is only located when the scenario script, `repeat` and measurement protocol are the
  ones that were accepted.
- One explanation per rule and subject, at most three per module.

Noise seen so far: with `tolerance` left at 1 percent, a build with one more function can produce
`explain/function-growth` and `explain/code-growth` candidates for the same code growth. They are
folded into one: the code part appears as evidence of the function. A module whose imports changed
lists `explain/import-added` until the baseline is accepted again; that is the intended reminder,
and `[module.policy]` is the place to say which imports are expected.
