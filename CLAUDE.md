# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Perusal is a native Rust + GPUI desktop reviewer for GitHub pull requests and GitLab merge requests. Linux (X11/Wayland) is the only supported platform today.

## Commands

```bash
cargo build --workspace                          # all crates, including the GUI (crates/ui, binary `perusal`)
cargo test --workspace
cargo test -p perusal-filter sorting             # one crate / one test by name substring
cargo clippy --workspace -- -D warnings          # CI runs exactly this on pull requests
cargo run -p perusal-ui -- /path/to/repo branch  # GUI straight into a local review (no args = Home screen)
cargo run --release -p perusal-cli -- bench      # headless harness; also `gen`, `files`, `mr`, ... (see crates/cli)
extension/build.sh                               # browser extension zips; packaging/build-*.sh build deb/rpm/AppImage/Flatpak
```

Building the GUI on Linux needs `libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libwayland-dev` (plus the other X11/font/vulkan dev packages listed in `.github/workflows/ci.yml`).
GPUI is pinned to a specific Zed git revision in `crates/ui/Cargo.toml` (the crates.io release and newer Zed heads do not build here); do not bump it casually.

Useful environment overrides: `PERUSAL_HOME` (data/config root), `PERUSAL_API_BASE` / `PERUSAL_GIT_BASE` (point providers at a mock host), `PERUSAL_POLL_SECS`, `PERUSAL_CA_FILE`.

## Architecture

The rule that shapes everything: **logic lives in UI-free crates and is tested headless; `crates/ui` only renders and wires events.** When adding behavior, put it in a logic crate with a unit test and call it from the UI.

- `gitops` wraps the git CLI. `diff` builds per-file stats (`FileStat`, including whitespace-ignoring counts), line diffs, tree-sitter highlighting and grouping rules (`diff::rules`, regexes with a `stem` group). `filter` is the expression language used for filtering and sorting (fields/functions tables are the single source: autocomplete, docs and the AI prompt reference are generated from them).
- `core` holds the SQLite store (reviews, viewed marks, drafts, chat), presets (global and per project) and product naming (`brand.rs`).
- `providers` implements GitHub and GitLab behind one `Provider` trait (submit is atomic, CI, threads, my-reviews listing); contract tests run against mock servers in `crates/providers/tests`.
- `agents` launches the user's own agent CLIs (claude, gemini, codex, opencode) read-only with a minimal environment (`safety`), streams their activity, and builds/parses prompts. Program lookup goes through `find_program`/`search_dirs` (process PATH, login-shell PATH, usual install dirs), not bare `PATH`.
- `app` has the UI-free view models: `vm::ReviewVm` (visible files, batches, grouping, cursor), `settings`, `theme`, `events` (MR change detection and desktop-notification keys), `markdown`, `textfield`, `tree`, `hosts`.
- `ui` is the GPUI app: `Root` (tab strip, Home, toasts, window chrome) owns one `Entity<View>` per open review and a shared `SettingsView` (also first-run onboarding). `View` (review screen) emits `ViewEvent`s up to `Root`. `model.rs` glues the logic crates to the UI (opening reviews, polling, background jobs); `kit.rs` is the design kit (palette, icons, inputs, markdown renderer). Diff rows live in a variable-height GPUI `list`; comment cards are rows in the same list.

State on disk: settings in `~/.config/perusal/settings.toml`, presets in `presets.toml`, tokens as 0600 files under `tokens/`, the SQLite DB in the data dir.

## Conventions worth knowing

- `DECISIONS.md` records design decisions and known gaps; `docs/PROGRESS.md` records milestone status; `docs/USER.md` is the user-facing behavior doc. Update them when behavior changes.
- UI work is verified with screenshots on a headless X display (Xvfb + xdotool + ImageMagick `import`), since GPUI cannot be unit tested.
- `assets/perusal.svg` is the icon; the app embeds a 256 px PNG render of it (`crates/ui/assets/perusal-256.png`) that must be regenerated when the SVG changes.
- Releases are tag driven (`v1.2.3` builds every package; the version comes from the tag), see `.github/workflows/release.yml`.
