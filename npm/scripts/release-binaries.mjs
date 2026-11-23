// The binaries of the npm package are built by the Release workflow and
// attached, as archives, to the GitHub release of the matching version tag.
//
//   node scripts/release-binaries.mjs fetch   download the archives and SHA256SUMS
//                                             of this version into .release/
//   node scripts/release-binaries.mjs verify  fail unless every archive matches the
//                                             release checksums and its build
//                                             provenance attestation, then unpack
//                                             each binary into vendor/<target>/
//
// The targets are the ones in bin/targets.js. verify runs before every publish
// (prepublishOnly), so what is published is always unpacked from checked
// archives, never a binary built elsewhere. Set PEAKMARK_SKIP_ATTESTATION=1 to
// skip the attestation for local runs; the checksums are still compared.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const { targets } = require('../bin/targets.js')

const packageDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const { version } = JSON.parse(fs.readFileSync(path.join(packageDir, 'package.json'), 'utf8'))
const tag = `v${version}`
const releaseDir = path.join(packageDir, '.release')
const vendorDir = path.join(packageDir, 'vendor')
const checksumFile = 'SHA256SUMS'
const repository = 'casoon/peakmark'

const archiveName = (target) => `peakmark-${version}-${target}.tar.gz`

function fail(message) {
  console.error(`release-binaries: ${message}`)
  process.exit(1)
}

function fetchArchives() {
  fs.rmSync(releaseDir, { recursive: true, force: true })
  execFileSync(
    'gh',
    [
      'release', 'download', tag,
      '--repo', repository,
      '--dir', releaseDir,
      '--pattern', `peakmark-${version}-*.tar.gz`,
      '--pattern', checksumFile,
    ],
    { stdio: 'inherit' },
  )
}

// Needs `gh` with a token (GH_TOKEN in CI). The attestation proves the archive
// was built by this repository's workflow; the checksum alone only proves it
// matches SHA256SUMS, which is attached to the same release.
function verifyAttestation(file) {
  try {
    execFileSync('gh', ['attestation', 'verify', file, '--repo', repository], { stdio: 'pipe' })
  } catch (error) {
    const detail = error.stderr ? `\n${error.stderr.toString().trim()}` : ''
    fail(`${path.basename(file)} has no valid build provenance attestation for ${repository}${detail}`)
  }
}

function verifyAndUnpack() {
  const checksumPath = path.join(releaseDir, checksumFile)
  if (!fs.existsSync(checksumPath)) {
    fail(`${checksumFile} of ${tag} is missing; run \`npm run fetch:binaries\` first`)
  }
  const expected = new Map(
    fs.readFileSync(checksumPath, 'utf8')
      .trim()
      .split('\n')
      .map((line) => {
        const [hash, name] = line.trim().split(/\s+/)
        return [name, hash]
      }),
  )

  const names = Object.values(targets).map(archiveName)
  for (const name of names) {
    const file = path.join(releaseDir, name)
    if (!fs.existsSync(file)) fail(`${name} is missing; run \`npm run fetch:binaries\``)
    const actual = createHash('sha256').update(fs.readFileSync(file)).digest('hex')
    if (!expected.has(name)) fail(`${name} is not listed in the ${checksumFile} of ${tag}`)
    if (expected.get(name) !== actual) fail(`${name} does not match the ${checksumFile} of ${tag}`)
  }

  if (process.env.PEAKMARK_SKIP_ATTESTATION === '1') {
    console.warn('release-binaries: PEAKMARK_SKIP_ATTESTATION=1, build provenance not checked')
  } else {
    for (const name of names) verifyAttestation(path.join(releaseDir, name))
  }

  fs.rmSync(vendorDir, { recursive: true, force: true })
  for (const target of Object.values(targets)) {
    const unpacked = fs.mkdtempSync(path.join(releaseDir, 'unpack-'))
    execFileSync('tar', ['-xzf', path.join(releaseDir, archiveName(target)), '-C', unpacked])
    const binary = path.join(unpacked, `peakmark-${version}-${target}`, 'peakmark')
    if (!fs.existsSync(binary)) fail(`${archiveName(target)} has no peakmark binary`)
    fs.mkdirSync(path.join(vendorDir, target), { recursive: true })
    fs.copyFileSync(binary, path.join(vendorDir, target, 'peakmark'))
    fs.chmodSync(path.join(vendorDir, target, 'peakmark'), 0o755)
    fs.rmSync(unpacked, { recursive: true, force: true })
  }

  console.log(`All ${names.length} binaries match the ${tag} release and are unpacked into vendor/.`)
}

const command = process.argv[2]
if (command === 'fetch') fetchArchives()
else if (command === 'verify') verifyAndUnpack()
else fail('usage: release-binaries.mjs fetch|verify')
