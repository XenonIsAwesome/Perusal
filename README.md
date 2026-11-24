# Perusal

A native desktop reviewer for GitLab merge requests and GitHub pull requests, built around filtering, batching and a
commit graph instead of one long scrolling page. Licensed MIT OR Apache-2.0. No telemetry.

## Status

The review engine is implemented and tested headless (git indexing, filter/sort language, presets, SQLite-backed
reviews, GitHub/GitLab providers against mocks, ACP agent integration, themes, keymap, deep links, extension,
packaging scripts). A first GPUI window reviews a local branch. It is early: see `DECISIONS.md` for what it lacks.

```bash
cargo run --release -p perusal-ui -- /path/to/repo my-branch --base main
```
Keys: `n`/`p` file, `v` viewed, `Enter` next batch, `Backspace` back, `/` filter, `c` comment, `g` group, `x` clear commit, `Ctrl +/-/0` zoom.
Click a file to open it or a commit to scope the review to it. On Linux you need `libxkbcommon-dev`, `libxkbcommon-x11-dev`
and `libwayland-dev` to build. `perusal-cli` is the headless harness.

## Build and test

```bash
cargo build --workspace            # everything except the GUI
cargo build --release -p perusal-ui  # the GUI
cargo test --workspace
cargo run --release -p perusal-cli -- bench     # checks the performance budget
extension/build.sh                              # browser extension zips
packaging/build-deb.sh                          # .deb
```

## Layout

`crates/core` sessions, presets, SQLite, brand · `gitops` git CLI wrapper · `diff` stats, diffs, highlighting ·
`filter` expression language · `providers` GitHub/GitLab · `agents` ACP and safety · `app` view models · `cli` harness.
Product naming lives in `crates/core/src/brand.rs`.
