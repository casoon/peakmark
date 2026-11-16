---
title: In CI
description: Run peakmark locally and as a step in GitHub Actions, with a verified binary.
order: 5
---

peakmark is a single binary that reads `peakmark.toml` from the directory it runs in. It builds
nothing: the module you list under `artifact` has to exist when `peakmark check` runs. That is the
one thing to get right, locally and in CI.

## Locally

```bash
cargo install peakmark
# build the module the way you ship it, then:
peakmark check
```

`check` exits 0 within budget, 1 over budget, 2 when it could not run. If a change is intended,
`peakmark accept <module> --reason "..."` writes the new baseline and history; commit both with the
code. Scenarios run in Node, so `node` has to be on the `PATH`.

Measure the file you ship. For `wasm-bindgen` that is the processed `pkg/*_bg.wasm`, not the output
of `cargo build`; `check` notes it when the artifact still carries `__wbindgen_describe_*` exports.

## In GitHub Actions

Add a step after the one that builds the module. This installs the release binary, verifies its
hash and its signed build provenance first, and then checks:

```yaml
      - name: Install peakmark
        env:
          GH_TOKEN: ${{ github.token }}
          PEAKMARK_VERSION: "0.1.1"
        run: |
          name="peakmark-${PEAKMARK_VERSION}-x86_64-unknown-linux-musl"
          gh release download "v${PEAKMARK_VERSION}" --repo casoon/peakmark \
            --pattern "${name}.tar.gz*" --dir "$RUNNER_TEMP"
          cd "$RUNNER_TEMP"
          sha256sum -c "${name}.tar.gz.sha256"
          gh attestation verify "${name}.tar.gz" --repo casoon/peakmark
          tar -xzf "${name}.tar.gz"
          echo "$RUNNER_TEMP/${name}" >> "$GITHUB_PATH"

      - name: peakmark
        run: peakmark check --details
```

A failing `check` fails the job. Pin `PEAKMARK_VERSION` and raise it on purpose: a newer peakmark can
read an older baseline, but it may report more than the version that wrote it.

Things that make the step flap, and what to do about them:

- **The build differs from your local one.** Size depends on the Rust toolchain and the `wasm-opt`
  version. Pin both in CI to what you build with locally, or the CI file differs by a few hundred
  bytes and trips a tight budget. Leave the budget some headroom, and let `accept` record the
  version you actually ship.
- **The module is built on one runner only.** Add the step to the job that builds it, not to every
  entry of an OS matrix.
- **The baseline is stale.** `check` compares with the committed baseline; a pull request that
  changes the module commits the matching `accept` with it.

`--json` prints the same result for other tools, for example to post a comment on the pull request.
