import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const { targets, targetFor } = require('../bin/targets.js')
const packageDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

test('every target is one the Release workflow builds', () => {
  const workflow = fs.readFileSync(path.join(packageDir, '../.github/workflows/release.yml'), 'utf8')
  const built = [...workflow.matchAll(/- target: (\S+)/g)].map((match) => match[1]).sort()
  assert.deepEqual(Object.values(targets).sort(), built)
})

test('maps platform and architecture to a release target', () => {
  assert.equal(targetFor('darwin', 'arm64'), 'aarch64-apple-darwin')
  assert.equal(targetFor('linux', 'x64'), 'x86_64-unknown-linux-musl')
  assert.equal(targetFor('win32', 'x64'), undefined)
})

test('the version is the crate version', () => {
  const cargo = fs.readFileSync(path.join(packageDir, '../Cargo.toml'), 'utf8')
  const crate = cargo.match(/^version = "(.+)"$/m)[1]
  const { version } = JSON.parse(fs.readFileSync(path.join(packageDir, 'package.json'), 'utf8'))
  assert.equal(version, crate)
})

test('runs the binary of this platform and passes on arguments and exit code', (t) => {
  const target = targetFor(process.platform, process.arch)
  if (!target) return t.skip(`no target for ${process.platform}-${process.arch}`)
  // A copy of the package with a stand-in binary, so the test needs no release.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'peakmark-npm-'))
  fs.cpSync(path.join(packageDir, 'bin'), path.join(dir, 'bin'), { recursive: true })
  fs.mkdirSync(path.join(dir, 'vendor', target), { recursive: true })
  const binary = path.join(dir, 'vendor', target, 'peakmark')
  fs.writeFileSync(binary, '#!/bin/sh\necho "args: $*"\nexit 3\n', { mode: 0o644 })

  const run = spawnSync(process.execPath, [path.join(dir, 'bin', 'peakmark.js'), 'check', '--json'], {
    encoding: 'utf8',
  })
  assert.equal(run.stdout.trim(), 'args: check --json')
  assert.equal(run.status, 3, 'exit code passed on; the executable bit was restored')
  fs.rmSync(dir, { recursive: true, force: true })
})
