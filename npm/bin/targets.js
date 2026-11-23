'use strict'
// The release targets the package ships, by the platform and architecture Node
// reports. They are the targets the Release workflow builds; scripts and the
// launcher read them from here and nowhere else.
const targets = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-arm64': 'aarch64-unknown-linux-musl',
  'linux-x64': 'x86_64-unknown-linux-musl',
}

// The release target for this platform and architecture, or undefined if the
// package has no binary for it.
function targetFor(platform, arch) {
  return targets[`${platform}-${arch}`]
}

module.exports = { targets, targetFor }
