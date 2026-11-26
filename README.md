<img src="assets/perusal.svg" alt="Perusal icon" width="96" align="left">

# Perusal

A native desktop reviewer for GitLab merge requests and GitHub pull requests, built around filtering, batching and a
commit graph instead of one long scrolling page. Licensed MIT OR Apache-2.0. No telemetry.

<br clear="left">

## Status

The review engine is implemented and tested headless (git indexing, filter/sort language, presets, SQLite-backed
reviews, GitHub/GitLab providers against mocks, ACP agent integration, themes, keymap, deep links, extension,
packaging scripts). The GPUI app reviews local branches and GitHub/GitLab merge requests end to end: filtering and sorting, batches, commit graph, inline threads (reply, resolve, react, edit), drafts with a quit prompt, AI assistance through your own agent CLIs, a keymap with a command palette, and moving a review to a new head. It has been exercised against mock hosts only, never a real one: `DECISIONS.md` lists what is untested or missing.

```bash
target/release/perusal                              # Home: recent reviews, open local, open from URL, browse remote MRs
target/release/perusal /path/to/repo my-branch      # straight into a local review (--base <ref> optional)
target/release/perusal https://github.com/o/r/pull/1  # straight into a remote review
```
Home: click the tabs (pull request URL, local branch, browse remote), paste with Ctrl+V, Enter opens. Tab / Shift+Tab move between fields. Esc in a review returns Home.
Remote reviews need a token for private repos (the app asks and stores it in the OS keychain when `secret-tool` is installed, otherwise under `~/.config/perusal/tokens`, mode 0600).
Data lives in `~/.local/share/perusal` (override with `PERUSAL_HOME`; the system CA store is trusted; `PERUSAL_CA_FILE` adds one more CA certificate).
Keys: `n`/`p` file, `v` viewed, `Enter` next batch, `Backspace` back, `/` filter, `c` comment, `g` group, `x` clear commit, `Ctrl+P` command palette, `?` all shortcuts, `Ctrl +/-/0` zoom. Rebind them in `~/.config/perusal/keymap.toml` (`[keys]`, e.g. `comment = "z"`).
Click a file to open it or a commit to scope the review to it. On Linux you need `libxkbcommon-dev`, `libxkbcommon-x11-dev`
and `libwayland-dev` to build. `perusal-cli` is the headless harness.

## Roadmap

Planned, roughly in this order of interest:

| Area | Item | Notes |
| --- | --- | --- |
| Platforms | **Windows** support | GPUI runs on Windows; the work is paths, the agent/CLI lookup, tokens storage and an installer. |
| Platforms | **macOS** support | Same, plus an app bundle, signing and notarisation. |
| Hosts | **Gitea** and **Forgejo** | New provider next to GitHub and GitLab (pull requests, review comments, CI status). |

Linux (X11 and Wayland) is the supported platform today. Suggestions are welcome as issues.

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
