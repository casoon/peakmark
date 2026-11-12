---
title: Inventory
description: What is inside a module and what it requires, read from the file without running it.
order: 3
---

peakmark reads the inventory of every module it checks. It comes from the bytes of the artifact
alone: nothing is instantiated, no glue runs, and Node is not involved. `check --details` prints it
and `check --json` carries it under `inventory` for each module. The inventory never fails a check
and is not part of the baseline.

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

## JSON schema

The document has a `schema_version`, currently `1`. It is deterministic: lists keep the order of
the module and features keep the order of `Feature`. Adding a field keeps the version; renaming or
removing one raises it. The library exposes the same data as `peakmark::inventory::Inventory`
(`Inventory::from_bytes` reads one from bytes), so other parts of peakmark and other programs use
the model and not the text.
