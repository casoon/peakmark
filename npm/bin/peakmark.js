#!/usr/bin/env node
'use strict'
// Runs the peakmark binary of this platform with the arguments given, and exits
// with its exit code. The binaries are the ones attached to the GitHub release
// of this version, checked against its SHA256SUMS and build provenance before
// the package was published.
const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')
const { targetFor } = require('./targets.js')

const target = targetFor(process.platform, process.arch)
if (!target) {
  console.error(
    `peakmark: no binary for ${process.platform}-${process.arch} in this package; ` +
      'install it with `cargo install peakmark` instead',
  )
  process.exit(2)
}

const binary = path.join(__dirname, '..', 'vendor', target, 'peakmark')
const run = () => spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' })

let result = run()
if (result.error && result.error.code === 'EACCES') {
  // A package manager that dropped the executable bit; restore it once.
  fs.chmodSync(binary, 0o755)
  result = run()
}
if (result.error) {
  console.error(`peakmark: cannot run ${binary}: ${result.error.message}`)
  process.exit(2)
}
if (result.signal) {
  process.kill(process.pid, result.signal)
}
process.exit(result.status ?? 2)
