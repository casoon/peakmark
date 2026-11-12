# Contributing

Thanks for helping. peakmark is a small Rust CLI; the layout is described in
[docs/architecture.md](docs/architecture.md).

## Before you push

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests that run scenarios need `node` on the `PATH`. Lints are configured in `Cargo.toml`
(`clippy::pedantic` with a few documented exceptions); a new `allow` needs a comment saying why.

## Changes

- Keep a change to one purpose. Unrelated cleanups go in their own pull request.
- Add a test with the change: a unit test next to the code, or an integration test in `tests/`
  (`tests/library.rs` for the API, `tests/cli.rs` for the command).
- Changes users can see (output, options, configuration, baseline format) get a line under
  `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md).
- No new dependencies without a reason in the pull request.
- Commit messages follow Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`).
