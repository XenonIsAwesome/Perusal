# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Perusal is a native Rust + GPUI desktop reviewer for GitHub pull requests and GitLab merge requests. Linux (X11/Wayland) is the primary platform; Windows 10/11 x86_64 also works (see the Windows section of `DECISIONS.md`).

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

Useful environment overrides: `PERUSAL_HOME` (data/config root), `PERUSAL_API_BASE` / `PERUSAL_GIT_BASE` (point providers at a mock host), `PERUSAL_POLL_SECS`, `PERUSAL_CA_FILE`, `PERUSAL_DB` (database path).

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

## How the owner wants work done

- Work autonomously end to end. Do not ask for approval or confirmation of decisions; decide, record a one-line decision and reason in `DECISIONS.md`, and keep going. Stop only when truly blocked (for example a real credential is needed), and then continue with fixtures and mocks.
- Develop on the session branch, commit in logical steps, push there; open a pull request only when asked.
- The product spec is `docs/SPEC.md` (exported from the Claude Docs original, https://claude.ai/artifact/GGpC5ERECBrhWYTBrJy4AB); its decisions below are final unless the owner changes them.
- Windows: spawn subprocesses through `perusal_gitops::command` (no console flash), use `find_program` for agents (`.cmd` shims), and gate Unix-only code and tests with `cfg(unix)`. The GUI needs an interactive desktop session; over a service/SSH shell run it through a scheduled task with `/IT`.
- Sandbox limits: real GitHub/GitLab hosts are unreachable (develop against mock servers/fixtures, see `crates/providers/tests`), there is no `notify-send`, and the GUI is only checkable on a headless X display.

## Product spec essentials

Goal: review very large MRs fast (10k files / 500k lines usable in under 1 s, filter under 100 ms) with filtering, batching and a commit graph instead of one scrolling page. Goals that constrain design: a review is never lost (continuous autosave, crash recovery, quit prompt with Submit / Keep for later / Discard), works with self-hosted GitLab and GitHub Enterprise, no telemetry, network only to the user's hosts and the agents they launch. Non-goals for v1: merging/closing MRs, editing titles/labels, Gitea/Forgejo, macOS/Windows, RTL, screen readers, an in-app updater.

- **Drafts are local only** until submit. Submit is atomic (GitHub: one review call; GitLab: draft notes then bulk publish, deleting drafts on failure). Replies, resolves, reactions and edits of existing threads go to the host immediately. GitLab approval is a separate call from the review.
- **Review is pinned to a head**; new commits or a force-push produce a banner and an offer to move, keeping drafts and viewed marks where lines survive.
- **Batch flow:** one file, a manual group, or an auto-group per step; Next follows the current filter and sort; Back covers a forgotten comment; viewed progress bar across the top. Local branch reviews use the merge-base as default base and keep comments local (exportable as Markdown).
- **Filter language** (Wireshark style): fields for identity, stats, content and review state, arithmetic/regex/boolean, cross-file aggregates (`avg`, `top`, ...), multi-key sort with asc/desc, presets global and per project (project default wins, global is the fallback; projects are keyed by normalized remote URL).
- **File grouping** is rule based (name stems across folders, e.g. foo.h + foo.cpp, foo.ts + foo.test.ts), editable in settings, toggleable. No import-graph grouping in v1. Noisy files (lockfiles, generated, minified, huge, binary) collapse by default with a one-click expand.
- **Git graph:** MR commits plus target-branch context; clicking a commit or range scopes files and diff to it.
- **AI** runs the user's installed agent CLIs (claude, gemini, codex, opencode), read-only, in a copy of the code at the reviewed head; Ask AI about a selection (chat saved with the review, answers convertible to drafts) and a full AI review whose output lands as AI-tagged drafts the user accepts, edits or discards. No AI accounts or keys are stored by the app.
- **CI widget** is an icon only (breathing while running, check/cross when done); clicking it opens the pipeline in the browser. Themes are TOML files with an `AGENTS.md` in the themes folder telling an agent how to make one. Keyboard-first navigation, rebindable keys, separate UI and code fonts, live zoom.

## UI direction given during development

- Look and feel: a modern toolset in the spirit of GitLab's and GitHub's review UIs (collapsible sections, inline threads, clean light/dark themes); icon-driven (Material Icons, icon-only buttons where the meaning is obvious, tooltips for the rest), not text-heavy.
- Controls live where the hand is: a floating bar at the bottom of the diff (previous/next, viewed eye checkbox, global comment, unified/split, markdown source/rendered, Finish review); a toolbar floating above the selected lines (Comment, Ask AI); a floating Done in settings. Next never marks a file viewed by itself; the eye checkbox does.
- Settings are the home for every default (diff mode, markdown mode, grouping, files tree/list, whitespace, notifications per event, agent/model/effort, hosts, presets, window title bar) and are split into tabs; anything the spec calls "configurable" gets a settings control.
- The Files panel is a flat list or a collapsible directory tree; grouping by ctrl/shift-click is manual; sidebars are collapsible and resizable. Notifications are one in-app toast queue plus optional desktop notifications chosen per event.
- Comments: inline cards that are collapsible (also other people's), rich Markdown editor with Write/Preview, suggestion blocks shown as a diff (GitLab style), rich/raw toggle, edit and delete own drafts (AI drafts included). AI work shows activity and a Stop button; model and effort are dropdowns with a Custom option; "Generate with AI" exists for filters, sorts and grouping rules.
- Interaction details that were explicitly requested: text fields behave like normal inputs (click-away unfocuses, selection/caret by mouse, horizontal scroll while typing); mouse cursors change on interactive elements; dropdowns support type-to-search; a clickable MR link and CI icon in the header; the app works without arguments (Home with recent reviews, local open, open by URL, browse remote, "my reviews"), with tabs for several reviews, onboarding on first run, and an optional built-in title bar with window buttons.
- The app icon is a magnifier over a diff (`assets/perusal.svg`), used for the window, Home, onboarding, README and packaging.
