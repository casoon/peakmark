---
title: Installation
description: Install peakmark from npm, from crates.io or from a verified release binary.
order: 1
---

## From npm

```bash
npm install --save-dev peakmark
npx peakmark check
```

The package `peakmark` ships the binaries of the GitHub release of the same version for macOS
(arm64, x64) and Linux (arm64, x64, statically linked). They are checked against the release's
`SHA256SUMS` and build provenance before the package is published, and the package is published by
the release workflow with npm provenance (`npm audit signatures` checks it). On another platform,
install from crates.io.

## From crates.io

```bash
cargo install peakmark
```

## From a release binary

Official binaries come only from the GitHub releases of `casoon/peakmark`, built there by the
release workflow, never on a developer machine. Each archive has a `.sha256` next to it, the release
carries a combined `SHA256SUMS`, and every archive has signed build provenance. To verify a
download by hand, check the hash and the provenance:

```bash
shasum -a 256 -c peakmark-<version>-<target>.tar.gz.sha256
gh attestation verify peakmark-<version>-<target>.tar.gz --repo casoon/peakmark
```

## From a checkout

```bash
cargo install --path .
```

## Requirements

Scenarios run in Node.js, so `node` has to be on the `PATH`. A configuration without scenarios
needs no Node.
