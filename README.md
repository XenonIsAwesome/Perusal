<img src="assets/perusal.svg" alt="Perusal icon" width="96" align="left">

# Perusal

A native desktop reviewer for GitLab merge requests and GitHub pull requests, built around filtering, batching and a
commit graph instead of one long scrolling page. Licensed MIT OR Apache-2.0. No telemetry.

<br clear="left">

## Status

The review engine is implemented and tested headless (git indexing, filter/sort language, presets, SQLite-backed
reviews, GitHub/GitLab providers against mocks, ACP agent integration, themes, keymap, deep links, extension,
packaging scripts). A first GPUI window reviews a local branch. It is early: see `DECISIONS.md` for what it lacks.

```bash
target/release/perusal                              # Home: recent reviews, open local, open from URL, browse remote MRs
target/release/perusal /path/to/repo my-branch      # straight into a local review (--base <ref> optional)
target/release/perusal https://github.com/o/r/pull/1  # straight into a remote review
```
Home: click the tabs (pull request URL, local branch, browse remote), paste with Ctrl+V, Enter opens. Tab / Shift+Tab move between fields. Esc in a review returns Home.
Remote reviews need a token for private repos (the app asks and stores it under `~/.config/perusal/tokens`, mode 0600).
<<<<<<< HEAD
Data lives in `~/.local/share/perusal` (`%LOCALAPPDATA%\perusal` on Windows, config in `%APPDATA%\perusal`; override with `PERUSAL_HOME`; `PERUSAL_CA_FILE` adds a CA certificate).
=======
Data lives in `~/.local/share/perusal` (override with `PERUSAL_HOME`; the system CA store is trusted; `PERUSAL_CA_FILE` adds one more CA certificate).
>>>>>>> origin/main
Keys: `n`/`p` file, `v` viewed, `Enter` next batch, `Backspace` back, `/` filter, `c` comment, `g` group, `x` clear commit, `Ctrl +/-/0` zoom.
Click a file to open it or a commit to scope the review to it. On Linux you need `libxkbcommon-dev`, `libxkbcommon-x11-dev`
and `libwayland-dev` to build. `perusal-cli` is the headless harness. On Windows you need the Rust MSVC toolchain, Visual Studio Build Tools (C++) and Git for Windows; the GUI needs a Direct3D 11 capable display adapter. Windows releases ship an MSI, an MSIX (self-signed: import `Perusal-test-signing.cer` into Trusted People first) and a zip; build them with `packaging/windows/build-msi.ps1` and `build-msix.ps1`. Tokens are kept in the Windows Credential Manager.

## Roadmap

Planned, roughly in this order of interest:

| Area | Item | Notes |
| --- | --- | --- |
| Platforms | **macOS** support | Same, plus an app bundle, signing and notarisation. |
| Hosts | **Gitea** and **Forgejo** | New provider next to GitHub and GitLab (pull requests, review comments, CI status). |

Linux (X11 and Wayland) and Windows 10/11 (x86_64) are supported today. Suggestions are welcome as issues.

## Build and test

```bash
cargo build --workspace
cargo build --release                 # builds the GUI (target/release/perusal) and perusal-cli
cargo test --workspace
cargo run --release -p perusal-cli -- bench     # checks the performance budget
extension/build.sh                              # browser extension zips
packaging/build-deb.sh                          # .deb
```

## Layout

`crates/core` sessions, presets, SQLite, brand · `gitops` git CLI wrapper · `diff` stats, diffs, highlighting ·
`filter` expression language · `providers` GitHub/GitLab · `agents` ACP and safety · `app` view models · `cli` harness.
Product naming lives in `crates/core/src/brand.rs`.
