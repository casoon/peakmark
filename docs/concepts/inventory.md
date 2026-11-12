---
title: Inventory
description: What is inside a module and what it requires, read from the file without running it.
order: 3
---

peakmark reads the inventory of every module it checks. It comes from the bytes of the artifact
alone: nothing is instantiated, no glue runs, and Node is not involved. `check --details` prints it
and `check --json` carries it under `inventory` for each module. The inventory alone never fails a check.
`accept` keeps a compact part of it in the baseline, `check` reports what differs from it, and only
an optional [policy](#policy) can turn the inventory into an error.

The artifact is validated first. A module that is invalid, uses a feature outside WebAssembly 3.0,
or is a WebAssembly component stops the check with exit code 2 before an inventory exists; see
[Validation comes first](../measurements/#validation-comes-first). Components are not analysed
recursively.

## What it lists

| Part | JSON key | Content |
|---|---|---|
| Artifact | `artifact` | `kind` (`core_module`) and the binary `version` |
| Counts | `counts` | types, imported and defined functions, imports, exports, data and element segments |
| Imports, exports | `imports`, `exports` | module and name, and the type: function signature, table, memory, global or tag |
| Entities | `memories`, `tables`, `globals`, `tags` | every entity in index order, imported ones first, with `imported_from`, `exported_as` and its type |
| Limits | `type` of a memory | `minimum` and `maximum` in pages (`null`: unbounded), `shared`, `memory64`, `page_size` in bytes |
| Features | `features` | what the binary needs, see below |
| Sections | `sections` | every section in file order with `header` and `payload` bytes |
| Custom sections | `custom_sections` | `name`, `kind` and `bytes` |
| Composition | `composition` | the bytes of the file by purpose, see [Where the bytes are](#where-the-bytes-are) |
| Functions | `functions` | `count`, `body_bytes` and the `largest` bodies with their raw size, name and exports |
| Data | `data` | `bytes` and the `largest` data segments by size and location |
| Names | `names` | `absent`, `present` (module name, named functions) or `unreadable` |
| Metadata | `producers`, `target_features` | what the toolchain wrote |
| Coverage | `coverage` | what the inventory could not decide |

Function signatures list value types: `(i32, i32) -> i32`. Reference types keep their text form,
for example `funcref` or `(ref null 3)`. A memory without a maximum is reported as unbounded; whether
that is acceptable is a decision for a policy, not a finding.

### Names

A module with a `name` section reports how many of its functions are named. A stripped module says
`absent`: its functions are unnamed and the output states that instead of leaving the gap silent.
Imports and exports always have names, because the format requires them; they are listed whether the
module is stripped or not.

### Section bytes

`sections` accounts for every byte of the file. The 8-byte preamble (magic number and version) is
`PREAMBLE_BYTES` in the library; each section adds a `header` (the section id and the LEB128 size
that precede its payload) and a `payload`. The preamble plus all headers and payloads equal the file
size. The payload of a custom section includes its own name. Headers are overhead: they are listed
apart so that an attribution of size can leave them out or show them.

### Custom sections

Custom sections are classified by name: `name` (`names`), `.debug_*`, `external_debug_info` and
`sourceMappingURL` (`debug`), `producers`, `target_features`, and everything else (`other`, not
interpreted). Their presence is reported; it is not a finding. A name section or debug information
is not treated as waste or as a secret: whether to ship them is up to you.

`producers` and `target_features` are read when present. They are the toolchain's claims. They are
listed next to the derived features and never turned into requirements.

## Where the bytes are

`composition` divides the file into parts that add up to its size, exactly:

| Part | Bytes |
|---|---|
| `code` | the code section: function bodies, one size prefix each, and the count |
| `data` | the data section |
| `names` | the `name` section |
| `debug` | `.debug_*`, `external_debug_info`, `sourceMappingURL` |
| `custom` | every other custom section, `producers` and `target_features` included |
| `structure` | types, imports, functions, tables, memories, globals, exports, start, elements, data count, tags |
| `overhead` | the 8-byte preamble and the id and size field in front of every section |

`functions` lists the largest function bodies (20, biggest first). A size is shallow and raw: the
bytes of that body (locals, instructions, end) without the size prefix and without anything it
calls. A function is named by its `name` section entry; a stripped module has none, so its
`exported_as` names and its `index` in this file (`func[12]`, imports first) identify it. The index
means nothing in another build. Raw bytes are not what a function costs after gzip or Brotli:
compression is a property of the whole resource, so the sizes of functions do not add up to a
compressed size and are never presented as a saving. `functions.bodies` in the library holds every
body for later parts; it is not serialized.

`data` lists the largest data segments with their size, the memory they are copied to and the
offset if it is a constant (`null` for a passive segment). The contents are never printed, and
peakmark does not say which asset a segment comes from.

On the first run this is all there is: the composition and the largest bodies and segments, shown
by `check --details` and in `--json`, with no delta and no statement about history.

## Features are derived

`features` lists the core features the binary needs. Each entry has a `feature`, the number of
`declarations` that need it and the number of `instructions`. It is derived from the module, not
taken from advertised metadata:

- **Declarations:** a shared memory, a 64-bit memory or table, more than one memory, a tag, a
  `v128` or GC reference type in a signature, a struct or array type, subtyping, a passive data or
  element segment, a second table or one that is not `funcref`, a mutable global that is imported
  or exported, a function type with several results, imports from `wasm:js-string`.
- **Instructions:** every instruction in function bodies and constant expressions is classified by
  the proposal it comes from, using the list wasmparser keeps. SIMD, relaxed SIMD, atomics, bulk
  memory, reference types, function references, GC, exceptions (both the current and the legacy
  form), tail calls, sign extension, saturating float-to-int, wide arithmetic, memory control, stack
  switching and the other proposals have their own entry. Arithmetic in a constant expression is
  reported as extended constants; a block with a function type as multi-value.

Features appear under snake-case names such as `simd`, `shared_memory`, `atomics`, `bulk_memory`,
`reference_types`, `exception_handling`, `tail_call` and `gc`; the complete list is `Feature` in the
library.

This is a heuristic about the bytes. It can say that the module contains something that needs a
feature; it cannot say that the code runs or that an engine supports it.

### What is not detected

`coverage` states this, and `check --details` repeats it as `not detectable`:

- what the host must provide beyond the imports, for example cross-origin isolation for shared
  memory in a browser (`host_environment`);
- whether the code that needs a feature is ever run (`reachability`): a feature in dead code is
  listed;
- the toolchain's own claims in `target_features` and `producers` (`toolchain_claims`);
- the meaning of custom sections other than names, debug, producers and target features
  (`custom_section_semantics`);
- which engines and versions support a feature (`engine_support`).

Two things lower the coverage further, and are reported as such, never as "not used":

- `unclassified_instructions`: instructions of a proposal this version of peakmark has no feature
  for, counted by proposal;
- `unreadable_custom_sections`: a `name`, `producers` or `target_features` section that is there
  and cannot be read.

`complete` is `true` only when both are empty; the limits above apply anyway. A feature that
validation does not accept does not get this far: it stops the check with exit code 2 as
unsupported.

## Against the baseline

`peakmark accept` stores a snapshot of the inventory with the entry of the module (`inventory` in
[the baseline](../budget-and-baseline/#what-the-baseline-stores)): the imports and exports with
their types, the memories and tables the module keeps to itself, and the names of the derived
features. It is sorted, so reordering a module changes nothing, and an entry accepted before it was
kept has none: `check` notes that (`baseline/inventory-missing`) and compares nothing, never an
empty inventory.

`check` compares the snapshot with the module as it is now and reports each difference as a
warning, with exit code 0, until the baseline is accepted again:

| Rule | Meaning |
|---|---|
| `baseline/interface-added` | an import, export, internal memory or table that was not accepted |
| `baseline/interface-removed` | an accepted one that is gone |
| `baseline/interface-changed` | the same declaration with another signature or other limits; the finding shows both |
| `baseline/feature-added` | the binary needs a feature it did not need when accepted |
| `baseline/feature-removed` | it no longer needs one |
| `baseline/size-sections` | the file changed between its parts (note) |
| `baseline/size-functions` | function bodies changed, with the confidence of the match (note) |
| `baseline/sizes-missing` | the accepted entry has no section or function sizes yet (note) |

### Identity

A declaration is identified by import module (imports only), name and kind, never by the name
alone: `env.f` as a function and `env.f` as a memory are two imports, and `a.f` and `b.f` are not
the same import. Under an identity, the type is what can change. A signature change is one
`interface-changed` finding, not a removal and an addition; a change of kind or module is both. A
module that imports the same module, name and kind several times with different types is compared
as a set of types.

A memory or table that is imported or exported is an import or an export, with its limits as the
type, so a changed maximum is reported once. Only those the module keeps to itself are listed
apart, named by their position (`#0`, `#1`) among them. Limits are written `min..max` in pages for
a memory (`1..` has no maximum, then `shared`, `64-bit` and a non-default `page-size`) and
`funcref 1..16` for a table.

### Attributing a size change

When the file changed size or moved bytes between its parts, `check` reports a note
`baseline/size-sections` with the change of every part that differs (code, data, names, debug,
other custom, declarations, headers and preamble). The deltas add up to the change of the file
size. Its `action` names the first thing to look at, grounded in the part that grew most: a changed
function, the largest data segment and where it is, or, for name or debug information, that the
deployment artifact can be built without it. peakmark does not modify files and does not claim
what leaving something out would save.

`baseline/size-functions` explains the code change per function, but only as far as the match is
trustworthy. Two builds are matched by name (`name` section) or, for an unnamed function, by export
name, and only if that key occurs once. The index is never used: after stripping or reordering it
points at unrelated functions. The snapshot keeps the count, the sum and the 20 largest bodies, so
it stays small however large the module is.

| Field | Meaning |
|---|---|
| `changes` | functions found through their key: `changed` (both sizes), `added`, `removed` |
| `confidence` | `high`: every listed function has a key; `partial`: some do not; `none`: none does (a stripped module) |
| `unmatched_accepted`, `unmatched_now` | functions without a usable key, by local ID of their own build and with their size; they are not compared |
| `body_delta`, `attributed`, `unaccounted` | the change of all bodies, the part the `changes` explain, and the rest, which no listed function explains |
| `status` | `exact` if `confidence` is high, both lists name every function and `unaccounted` is 0; otherwise `estimated` |

If the module has more than 20 functions, a key only on one side may mean the function rose into
or fell out of the list instead of being new or gone; then it is `entered_largest` or
`left_largest`, and its size is not counted as attributed. Sizes are raw body bytes, not gzip or
Brotli savings. Matching the paths that retain a function, and mapping bytes to source, are
separate, later work.

### What a host-interface change means

A new import means the module asks the host for one more binding; a removed export means callers
lose one. That is all the inventory states. An import name does not show what the host does with it:
a `fetch` import may be wired to anything or to nothing, and the filesystem or network access of a
WASI module depends on the host implementation and the grants it is given. An empty import list is
not a safety statement either, and neither is the absence of a violation. peakmark makes no
security statement, and compatibility with a runtime or browser is unknown: it is only tied to what
the file contains, never to a list of engines.

## Policy

A policy turns parts of the inventory into a gate. It is opt-in per module, in
[`[module.policy]`](../../reference/configuration/):

```toml
[module.policy]
require_exports = [{ name = "add", signature = "(i32, i32) -> i32" }]
allow_imports   = [{ module = "env", name = "log", signature = "(i32) -> ()" }]
allow_features  = ["bulk_memory"]
```

A broken rule is an error: exit code 1, and `accept` refuses the module (exit code 2) until the
module or the policy is changed. Accepting a baseline does not waive a policy, just as it does not
waive an absolute size budget. Each finding names the declaration and the action to take, and
`--json` carries both under `violations`.

| Rule | Broken when |
|---|---|
| `policy/export-missing` | a required export is not there |
| `policy/export-signature` | it is there as another signature, or is not a function; the finding shows what it is |
| `policy/import-not-allowed` | `allow_imports` is set and the import is not listed (a listed name from another module does not count) |
| `policy/import-signature` | the import is listed, but not with this type |
| `policy/feature-not-allowed` | `allow_features` is set and the binary needs another feature |
| `policy/feature-unverifiable` | `allow_features` is set and instructions of a proposal peakmark has no feature for exist (`unclassified_instructions`), so it cannot show that all features are allowed |

Strict means: what is not listed is refused, and what peakmark cannot classify is not assumed to be
allowed. An absent key gates nothing; a present key, even an empty one, is a complete list. A
policy is about the bytes of the file. It does not say that the host grants an allowed import
anything, that a required export works, or that the module is secure.

## JSON schema

The document has a `schema_version`, currently `1`. It is deterministic: lists keep the order of
the module and features keep the order of `Feature`. Adding a field keeps the version; renaming or
removing one raises it. The library exposes the same data as `peakmark::inventory::Inventory`
(`Inventory::from_bytes` reads one from bytes), so other parts of peakmark and other programs use
the model and not the text.
