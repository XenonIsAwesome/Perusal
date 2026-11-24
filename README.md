# Perusal

A native desktop reviewer for GitLab merge requests and GitHub pull requests, built around filtering, batching and a
commit graph instead of one long scrolling page. Licensed MIT OR Apache-2.0. No telemetry.

## Status

The review engine is implemented and tested headless: git indexing, the filter/sort language, presets, SQLite-backed
reviews with crash-safe drafts, GitHub and GitLab providers (mock-tested), agent integration over ACP, view models,
themes, keymap, deep links, a browser extension and packaging scripts. **The GPUI window is not built yet**: `gpui`
does not compile in the development sandbox (see `DECISIONS.md`). `perusal` today supports `perusal theme check <file>`
and validates `perusal://` links. Use `perusal-cli` for headless review.

See `docs/PROGRESS.md` for milestone status, `docs/USER.md` for usage, `docs/BENCHMARKS.md` for performance.

## Build and test

```bash
cargo build --workspace
cargo test --workspace
cargo run --release -p perusal-cli -- bench     # checks the performance budget
extension/build.sh                              # browser extension zips
packaging/build-deb.sh                          # .deb
```

## Layout

`crates/core` sessions, presets, SQLite, brand · `gitops` git CLI wrapper · `diff` stats, diffs, highlighting ·
`filter` expression language · `providers` GitHub/GitLab · `agents` ACP and safety · `app` view models · `cli` harness.
Product naming lives in `crates/core/src/brand.rs`.
