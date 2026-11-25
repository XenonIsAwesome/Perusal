# Perusal — Product and Technical Spec (for Claude Code)

Nov 24, 2026 · @Xenon

## Read this first (instructions for Claude Code)

This spec is addressed to you, Claude Code. You are building the app from an empty repository in a cloud session, and the owner is not watching. Build it end to end, make the decisions yourself, and keep going.

**Do not ask the owner questions.** Everything below is already decided, and where the spec leaves a detail open you choose the simplest option that meets the goals and record it in `DECISIONS.md` (one line: decision and reason). Where this spec names a recommendation or a default, take it. Stop and ask only if you are completely blocked, meaning no further progress is possible on any part of the work. The only realistic case is that a step needs a real credential or account you cannot get, and even then you continue with fixtures and mocks and do not wait.

**Working environment**

- The sandbox is Linux with network access limited to package registries (crates.io, npm, PyPI) and GitHub. GitLab and GitHub APIs for real MRs are probably unreachable, and you have no tokens. Develop and test providers against recorded JSON fixtures and a local mock HTTP server (for example wiremock); never rely on a live host.
- There is probably no display or GPU. You cannot assume you can run the GPUI window. Keep all logic in UI-free crates with unit and integration tests that run headless, and keep the UI crate thin. For the UI, require `cargo check` and `cargo clippy` to pass, and try headless rendering (Xvfb with a software Vulkan driver such as lavapipe) on a best-effort basis; do not block on it.
- Use git. Commit after each logical step with clear messages, keep a `PROGRESS.md` log of milestone status, and do not push anywhere unless the repository already has a remote configured for you.

**Repository layout (create it)**

```
Cargo.toml            workspace
crates/core/          review session, types, persistence (SQLite), settings (TOML)
crates/gitops/        git CLI wrapper, partial clone cache, in-process diff
crates/diff/          line and word diff, tree-sitter highlighting, noisy-file rules
crates/filter/        expression language: lexer, parser, evaluator, aggregates, autocomplete data
crates/providers/     Provider trait; github, gitlab, mock implementations
crates/agents/        ACP client, CLI adapters, review skill
crates/app/           GPUI application (thin)
crates/cli/           headless test harness over core
extension/            browser extension (Manifest V3, Chromium and Firefox)
packaging/            AppImage, Flatpak, deb, rpm, .desktop and URL handler
docs/                 DECISIONS.md, PROGRESS.md, themes/agents.md
```

**Name.** The product is called Perusal, and the crate, binary and config names use `perusal`; this is final. Keep the product name, binary name, config directory and the deep-link scheme (`perusal://`) in one place (`crates/core/src/brand.rs` plus workspace metadata) so a rename stays a single change.

**GPUI.** Use one GPUI distribution throughout and never mix forks. Prefer the version that the latest `gpui-component` release depends on; pin exact versions and record the choice in `DECISIONS.md`. If that combination does not build, fall back to a pinned git revision of Zed's GPUI.

**Order of work.** Follow the milestones in the Roadmap section. Each has acceptance criteria; a milestone is done when those pass and are committed.

## Overview and goals

Perusal is a native desktop app for reviewing GitLab merge requests and GitHub pull requests quickly, built around filtering, batching and a commit graph instead of one long scrolling page. It targets reviewers who handle large MRs, starts on Linux, and uses the user's own AI agent CLIs instead of its own accounts.

What it adds over the web UIs:

- A Wireshark-style filter and multi-key sort over per-file stats, content and review state, with global and per-project presets.
- A batch-based flow: one file or one related group at a time, with next, back and a 'mark as viewed' toggle.
- A git graph of the MR's commits, where clicking a commit reviews just that commit.
- Optional grouping of related files (header and source, test and implementation) into automatic batches.
- AI through installed agent CLIs: ask about a selection, or run a full AI review that lands as tagged drafts.
- Draft-only reviews that survive crashes and can be resumed from a recent-reviews page.

**Goals**

1. Fast: a 10,000-file, 500,000-line MR is usable in under a second.
2. A review is never lost: autosave, crash recovery, and a warning on quit.
3. Works on self-hosted GitLab and GitHub Enterprise Server, not just the cloud hosts.
4. No telemetry and no network traffic except the user's hosts and the agents they launch.

**Non-goals for v1:** merging or closing MRs, editing MR titles or labels, Gitea/Forgejo and other hosts, macOS and Windows builds, RTL text, screen-reader support, an in-app updater.

## Decisions at a glance

These are the choices made in the question rounds; later sections expand on them.

| Area | Decision |
| --- | --- |
| Code source | Local cache clone (partial clone, fetching only MR refs) for remote MRs |
| Hosts, v1 | GitHub.com, GitLab.com, self-hosted GitLab, GitHub Enterprise Server |
| Hosts, later | Gitea, Forgejo and other hosts with MR review; provider abstraction built from day one |
| OS | Linux first; spec lists the changes needed for Windows and macOS |
| MR updates mid-review | Review pinned to a head; banner offers to move to the new head, keeping comments and viewed marks where lines survive |
| Auth | Personal access token (OS keychain), OAuth/device flow, git credential helper or SSH for cloning only; `gh`/`glab` login reuse as a low-priority fallback |
| Existing discussions | Full parity: view, reply, resolve, suggestions, reactions, edit and delete own comments |
| Pending comments | Local only until submit |
| Filter | Text expressions with autocomplete, math, regex and cross-file aggregates; presets |
| Sort | Any expression, multi-key, ascending or descending |
| Presets | Global and project scope for filter and sort; project default auto-applies, global is the fallback |
| Viewed state | Local always; synced to the host where supported |
| File grouping | Built-in name-based rules, user-editable; toggleable |
| Git graph | MR commits plus target-branch context; click a commit or range to review just that |
| Noisy files | Auto-collapsed with one-click expand; patterns configurable |
| Diff rendering | Tree-sitter syntax highlighting plus word-level diff |
| Progress | Bar across the top of the review panel showing viewed files |
| UI stack | Rust, GPUI, with a UI-independent core crate |
| Git engine | `git` CLI for clone, fetch and worktrees; diffs computed in-process |
| AI protocol | Agent Client Protocol (ACP) where supported, headless CLI fallback |
| AI Q&A | Docked chat per review, saved with the review, answers convertible to draft comments |
| Full AI review | Comments land as AI-tagged drafts you accept, edit or discard |
| Agent permissions | Read-only by default, plus web search and reading related repositories |
| MR actions | Approve or request changes; assign reviewers and re-request review; link out for everything else |
| CI widget | Status icon only (breathing while running, check or cross when done); click opens the pipeline in the browser |
| Browser extension | Opens the MR in the app; user-maintained domain list for self-hosted sites |
| Multiple MRs | Tabs in one window |
| Local branch review | Base is merge-base with the default branch; comments stay local, exportable or attachable to a found MR |
| Quit and crash | Continuous autosave; quit prompt with Submit, Keep for later, Discard |
| Stale MR on reopen | Banner, drafts kept, submit as plain comments where the host allows |
| Themes | Built-ins plus custom TOML theme files, with an `agents.md` explaining how an AI agent can customize them |
| Customization | Separate UI and code fonts, live zoom, follow OS dark/light, rebindable keys |
| Input | Keyboard-first navigation |
| Performance budget | 10k files / 500k lines: usable under 1 s, filter under 100 ms, steady scroll at display refresh rate, under about 1 GB memory |
| Packaging | AppImage, Flatpak, .deb and .rpm; no in-app updater |
| License | Open source, free |
| Privacy | No telemetry; network only to configured hosts and launched agents |

## Architecture and tech stack

Perusal is a Rust workspace in which all review logic sits in UI-free crates, and a thin GPUI app renders it. That keeps the logic testable without a window and lets the UI layer be replaced if GPUI becomes a problem.

&#91;embedded content: architecture · app, core crates, external systems\]

The app calls only into the core; the core alone talks to hosts, the git CLI, agent processes and the keychain.

| Crate | Responsibility |
| --- | --- |
| `core` | Review session (pinned head, drafts, viewed marks, journal), settings, brand |
| `gitops` | Partial-clone cache, MR-ref fetch, worktrees, git CLI wrapper, object reads |
| `diff` | Line and word diff, rename detection, noisy-file rules, tree-sitter highlighting, per-file stats index |
| `filter` | Expression lexer, parser and evaluator; aggregates; multi-key sort; autocomplete data; presets |
| `providers` | `Provider` trait with GitHub, GitLab and mock implementations |
| `agents` | ACP client, CLI adapters, review skill, safety rules |
| `app` | GPUI views, view models, tabs, keymap, themes |
| `cli` | Headless harness used by tests and for debugging |

**Default libraries** (change only for a concrete reason, and log it): `tokio` for I/O, `rayon` for CPU-bound diff and index work, `reqwest` with rustls for HTTP, `serde`, `rusqlite`, `toml`, `regex`, `tree-sitter` with grammars for common languages loaded lazily, `imara-diff` for line diffs, `gix` for object reads and tree diffs, the `keyring` crate for secrets, `clap` for the CLI, and `wiremock` for provider tests.

**Git handling.** The `git` CLI performs network and worktree operations (clone with a blob filter, fetch of MR refs, `git worktree` for the agent's file tree), so credential helpers, SSH config and partial clones behave as the user expects. Diffs and object reads happen in-process.

**Concurrency.** UI work stays on the GPUI thread. Everything else runs on background tasks and returns immutable snapshots through channels. A snapshot is replaced as a whole, so the UI never sees a half-built index.

**Persistence.** One SQLite database holds hosts, repositories, reviews, drafts, the journal, viewed marks and AI chats. Tokens never go in it. Settings, presets, keymap and themes are TOML files the user can edit.

## Hosts, authentication and data flow

The app talks to hosts through one provider interface, and gets code through a local cache clone rather than through host APIs.

**Providers.** v1 implements GitHub (github.com and Enterprise Server) and GitLab (gitlab.com and self-hosted). Each provider supplies: list and filter MRs, MR metadata, discussions, submit review, approve and request changes, reviewers, CI status, and viewed-state sync where it exists. Gitea, Forgejo and others plug into the same interface after v1.

**Code source.** For a remote MR the app keeps a partial clone per repository in a cache directory and fetches only the MR's refs (for example GitLab's merge request head refs and GitHub's pull refs). Diffs, the git graph, content filters and the agent's file tree all come from this clone. Computing diffs locally also avoids host API limits on large changes; GitHub's list-files endpoint, for instance, is documented as capping at 3,000 files (to verify at implementation time).

**Authentication**

| Method | Role |
| --- | --- |
| Personal access token | Baseline. Works on every host including self-hosted. Stored in the OS keychain, with an encrypted-file fallback where no keyring exists. |
| OAuth / device flow | Where the host supports it (cloud hosts first); self-hosted needs an app registered per instance. |
| git credential helper / SSH | Cloning and fetching only. |
| `gh` / `glab` login | Low priority: used only when the user has not logged in to the app. |

**SSH keys cannot post comments.** GitHub and GitLab do not support API authentication with SSH keys, so an SSH key covers git transport only and a token or OAuth login is always needed to read discussions, post comments and submit reviews. One GitLab shortcut: gitlab-shell can mint a personal access token over SSH (`ssh git@host personal_access_token <name> <scopes> [ttl_days]`), so the app can offer to bootstrap a token from an existing SSH key where the instance allows it. No GitHub equivalent was found.

**Opening an MR**

1. Paste an MR or PR URL.
2. Pick a local repository and branch (see local review in the workflow section).
3. Browse an MR list from a connected account, filtered like GitLab's merge requests page (state, author, assignee, reviewer, label).
4. Open from the browser extension through a deep link.

Self-hosted hosts with a private certificate authority take an optional CA file path in their host entry.

## Review workflow and UI

The review window is three columns: files on the left, the diff in the middle, the commit graph on the right. A progress bar spans the top of the review panel and shows how many files have been marked viewed.

**Layout**

- Left sidebar: the filtered, sorted file list with viewed marks, per-file stats and comment counts.
- Main area: one file by default. The user can open more files at once and either split them into panes or stack them in one scrolling column.
- Right sidebar: the git graph of the MR's commits, with some target-branch context.
- Top: the viewed-files progress bar, MR title and host link, CI status icon, and the submit-review button. Several MRs open as tabs in one window.

**Batch flow.** Each batch is one file, a manual selection, or an auto-group (below). When done, the user presses a button that goes to the next batch. It carries a 'mark as viewed' toggle, and a back button covers a forgotten comment. The next batch follows the current filter and sort.

**Progress bar.** It counts viewed files out of the files in the MR and updates on every mark. Rules: the denominator is all files in the MR, a batch marks all its files together, and a setting can exclude auto-collapsed noisy files. The same progress appears on the recent-reviews page.

**Diff views.** Unified (inline) or side-by-side with whole old and new files, switchable at any time. Rendering uses tree-sitter syntax highlighting plus word-level highlights inside changed lines. Large and generated files render through virtualized views so only visible lines are laid out.

**Git graph.** Shows the MR's commits plus some context from the target branch. Clicking a commit, or selecting a range, limits the file list and diff to that selection, which supports commit-by-commit review.

**File grouping.** A toggle. When on, related files are batched automatically even if the user never batches by hand. Built-in rules are name-based, such as `foo.h` with `foo.cpp`, and `foo.ts` with `foo.test.ts`, even across folders. Rules are editable in settings and can be extended with user patterns. Code-aware grouping through includes and imports is not in v1.

**Noisy files.** Lockfiles, generated code, minified files, huge files and binaries are collapsed by default with a size banner and a one-click expand. Patterns are configurable, and the filter language can target them.

**When the MR changes during review.** The review stays pinned to the head it started on. A banner reports new commits or a force-push and offers to move to the new head, carrying over comments and viewed marks where the lines survive; comments that cannot be re-anchored are flagged for the user to place by hand.

**Local branch review.** Open a local repository, pick a branch, and the base defaults to the merge-base with the default branch (changeable). With no MR on a host, comments stay local; they can be exported as Markdown or sent to an agent. If an open MR exists for the branch, the app offers to attach to it so comments can be posted. Uncommitted working-tree changes are out of scope for v1.

**Recent reviews.** A page listing saved reviews with MR title, project, viewed progress, draft count, MR state (open, merged, force-pushed) and last-opened time, with search, pin and delete. It is the way back after a crash or an early close.

## Filtering, sorting and presets

The file list is driven by one expression language used for both filtering and sorting, in a bar with autocomplete and inline errors, in the style of Wireshark display filters.

**Attributes available in v1**

| Group | Attributes |
| --- | --- |
| File identity | name, path, extension, language, status (added, modified, deleted, renamed, moved), binary, generated |
| Change stats | additions, deletions, net, change ratio, hunk count, file size |
| Content | text or regex in changed lines, or in the whole file (before and after) |
| Review state | viewed, has comments, unresolved threads, AI-flagged, touched by the selected commit |

**Language.** Arithmetic, comparison, boolean logic and regex match, plus cross-file aggregates. Examples:

```
additions - deletions > 20 && name ~ /test/ && !viewed
additions > avg(additions)
top(10, additions + deletions)
```

**Sorting** takes any expression, with several keys and ascending or descending order, for example `additions + deletions desc, name asc`.

**Evaluation.** Per-file stats are computed once into an index when the MR loads. Aggregates are a second pass over that index, so filters stay under the 100 ms budget. Whole-file content search reads from the local clone.

**Presets**

- A preset stores a filter, a sort, or both.
- Two scopes: global, and per project. A project is identified by its normalized remote URL, so several clones of one repository share presets.
- Each project can mark one preset as its default and it auto-applies when an MR from that project opens. If none is set, the global default applies. The user can override it for a single MR, or clear it with one click.
- Presets are stored locally in editable files. Sharing presets through a file in the repository is not in v1.

## Comments, threads and submission

Nothing reaches the host until the user submits; every pending comment lives on the user's machine.

**Writing comments.** Select one or more lines and choose 'Comment' from the context menu to add an inline draft. A separate button adds a global comment on the MR itself. Drafts can be edited and deleted before submit. The same menu holds the AI actions (see the AI section).

**Existing discussions.** Threads from authors, reviewers and bots appear inline in the diff. The app supports full parity with the host: reply, resolve and unresolve, suggestion blocks, emoji reactions, and editing or deleting the user's own comments. Replies and resolves go to the host immediately, outside the pending-review model.

**Submitting.** The submit dialog takes an optional summary and a verdict: approve, request changes, or comment only. GitHub has all three as review events; on GitLab, approval is a separate action from posting the review. The user can also assign reviewers and re-request review after the author pushes. Everything else, such as merging, labels and the description, is done by following the link to the MR in the browser.

Implementation note, to verify per host: on GitHub, submit as one review with its comments in a single call; on GitLab, create draft notes and bulk-publish them, so a failed submit leaves nothing half-posted.

**Leaving mid-review**

- Every change is journaled to disk as it happens.
- Quitting with an unsubmitted review shows a prompt: Submit, Keep for later, or Discard.
- After a crash, the recent-reviews page offers to resume.
- Reopening a review whose MR was merged, closed or force-pushed shows a banner, keeps the drafts, and allows submitting them as plain comments where the host allows comments on that MR. A force-push offers the re-anchor flow described in the workflow section.
- Because drafts are local only, they are not visible in the host's web UI or on other machines until submitted.

## AI integration

AI runs through agent tools already installed on the user's machine, so the app holds no AI accounts or keys. Claude Code, Gemini, Codex and opencode are the first adapters.

**Adapters.** Each agent has an adapter that launches its existing executable. Where an agent speaks the Agent Client Protocol (ACP), the app uses it for streaming, structured replies and permission prompts. Where it does not, the adapter falls back to the agent's headless CLI mode. The agent is started inside the MR's checked-out file tree in the local cache clone, so it sees the code at the reviewed head.

**Ask AI about a selection.** Selecting lines and choosing 'Ask AI' opens a docked chat panel for the review, with the selection as context. The conversation is saved with the review. Any answer can be turned into a draft comment.

**Full AI review.** A custom skill or command, shipped with the app and editable by the user, has the agent review the MR. The output becomes draft comments tagged AI in the app. The user accepts, edits or discards each one, and then submits the review in their own name. An optional footer on posted comments is available and is off by default.

**Agent permissions.** The decision is read-only by default, with the ability to search the web and read other related repositories, such as dependencies. No edit or shell-execution permissions unless the user grants them for a session.

Security rule: MR content is untrusted. An agent that reads attacker-controlled code and can also reach the web is the classic setup for prompt-injection data theft. Required mitigations:

- Start agents with a minimal environment: no host tokens, no keychain access, no repository secrets.
- Read access to other repositories only from an allowlist the user approves per review.
- Show every web request and extra repository read in the chat so it is visible.
- Treat the agent's output as untrusted text: never auto-post, always land as drafts.

## CI widget, browser extension and deep links

**CI widget.** A small status icon in the review header. It shows state only; clicking it opens the pipeline or checks page in the default browser.

| State | Look |
| --- | --- |
| Running | Circle that breathes (slow opacity and scale pulse, about 2 s per cycle) |
| Passed | Static green circle with a check mark |
| Failed | Static red circle with a cross |
| Pending or queued | Grey circle that breathes more slowly |
| Canceled or skipped | Static grey circle with a slash |
| No CI | Hidden |

With the OS reduced-motion setting on, the breathing becomes a static ring. Source of truth is GitLab's head pipeline for the MR and GitHub's check runs and statuses for the head commit. Polling: every 15 s while the state is running and the tab is visible, every 60 s otherwise, paused when the window is unfocused.

**Browser extension.** One codebase, Manifest V3, for Chromium-based browsers and Firefox.

- Defaults to github.com and gitlab.com. The user adds self-hosted domains in the extension options and picks GitHub or GitLab for each; the browser asks for permission per domain, so the extension never needs broad host access.
- On an MR or PR page it adds an 'Open in Perusal' button. It detects pages by path (`/<owner>/<repo>/pull/<n>` on GitHub, `/<group>/<project>/-/merge_requests/<n>` on GitLab).
- The button navigates to `perusal://open?url=<url-encoded MR URL>`.

**Deep links.** The app registers the `perusal://` scheme.

- Linux: a `.desktop` file declares `x-scheme-handler/perusal`. The deb, rpm and Flatpak packages install it. For AppImage, first run offers to write the `.desktop` entry into the user's applications directory.
- Windows, later: a per-user registry entry for the scheme.
- A second launch hands its URL to the running instance (Unix socket on Linux, named pipe on Windows) and exits; the running instance opens the MR in a new tab.
- If the URL's host is unknown or not logged in, the app shows its add-host and login flow first.
- A deep link only opens an MR for viewing. It never submits, posts or runs anything. Only `https` MR URLs are accepted.

## Customization, themes and keyboard

**Themes.** Built-in themes are dark, light, GitHub dark and GitHub light, defined in the same TOML format users can write themselves. Custom themes live in the user's config directory under `themes/`, and edits reload live. A theme file covers UI colors, syntax colors (keyed by tree-sitter capture names), diff colors (added, removed, and word-level highlights), and status colors (CI and review state).

Ship an `agents.md` in that `themes/` directory. It is written for an AI agent and explains: the TOML schema with every key and its meaning, how to start from a built-in theme, contrast rules to respect, how to validate a file (`perusal theme check <file>`), and a worked example of turning a user request such as 'warmer dark theme' into a file. The validator must return clear messages an agent can act on.

**Fonts and size.** The UI font and the code font are set separately, with font size, line height and optional ligatures for code. Live zoom uses Ctrl+plus, Ctrl+minus, Ctrl+0 and Ctrl+scroll, and applies immediately.

**Follow OS appearance.** When on, the app switches between a chosen light theme and a chosen dark theme using the freedesktop appearance setting on Linux.

**Keyboard-first.** Every action is reachable without a mouse, through a command palette (Ctrl+P) and direct keys. Default keys, all rebindable in `keymap.toml`, with an optional vim-style preset:

| Action | Default key |
| --- | --- |
| Next or previous hunk | `]` and `[` |
| Next or previous file in the batch | `n` and `p` |
| Toggle viewed | `v` |
| Next batch (applies the viewed toggle) | `Enter` |
| Back to the previous batch | `Backspace` |
| Comment on selection | `c` |
| Ask AI about selection | `a` |
| Focus the filter bar | `/` |
| Focus the git graph | `g` |
| Next or previous tab | Ctrl+Tab and Ctrl+Shift+Tab |
| Show all shortcuts | `?` |

## Platforms, packaging, performance and privacy

v1 ships for Linux (X11 and Wayland). Keep platform code behind small interfaces so the other systems are additions, not rewrites.

**Changes needed for Windows and macOS later**

| Area | Windows | macOS |
| --- | --- | --- |
| Rendering | GPUI's Win32 and DirectWrite backend | GPUI's Metal backend |
| Deep link | Per-user registry entry for `perusal://` | `CFBundleURLTypes` in the app bundle's Info.plist |
| Single instance | Named pipe | Unix socket |
| Secrets | Windows Credential Manager | Keychain |
| Packaging | MSI or MSIX installer | Signed and notarized .app in a .dmg |
| Keys | Ctrl-based keymap | Cmd-based keymap |
| Process launch | Find `git.exe`; resolve agent `.cmd` shims; long paths | Find `git` outside the GUI's PATH |

**Linux packaging.** AppImage, Flatpak, .deb and .rpm, with build scripts in `packaging/`. There is no in-app updater; updates come through the package or by downloading a new release. The Flatpak build must run the host's `git` and agent executables, which needs a host-spawn permission that weakens the sandbox. Do it, and document the trade-off in `docs/`.

**Performance budget**

| Measure | Target |
| --- | --- |
| Open an MR of 10,000 files and 500,000 changed lines | Window usable in under 1 s (file list and first diff visible; the rest loads lazily) |
| Filter or sort result | Under 100 ms |
| Scrolling a large diff | Steady at the display's refresh rate |
| Memory on that MR | Under about 1 GB |

Design rules that follow: virtualized lists and diff views; diffs computed per visible file on demand; the per-file stats index built in a background pass; syntax highlighting only for the viewport; file contents read on demand from the git object store and never all held in memory. Verify with benchmarks in `crates/diff` and `crates/filter` against a generated synthetic MR of that size, and keep them in the repo.

**Privacy and security**

- No telemetry, no crash reporter, no update pings. The only network traffic is to configured hosts and to agents the user launches.
- Tokens live in the OS keychain only, are never logged, and are never passed to agents.
- Self-hosted hosts may use a private certificate authority: each host entry takes an optional CA file path, on top of the system trust store.
- Logs stay local and redact tokens and URLs' credentials.

## Roadmap and milestones

Build in this order. Logic comes before UI so each milestone can be verified headless, and v1 includes everything except the items listed under post-v1. Commit when a milestone's acceptance criteria pass, and record status in `docs/PROGRESS.md`.

| Milestone | Scope | Acceptance |
| --- | --- | --- |
| M0 Foundation | Workspace and crate skeletons; `brand.rs`; license (MIT OR Apache-2.0, files included); `DECISIONS.md`; synthetic-MR generator (10,000 files, 500,000 changed lines); GPUI hello-window and a virtualized-diff spike | `cargo build --workspace` and `cargo test --workspace` pass; generator produces the repo; spike compiles |
| M1 Core engine | `gitops` (partial-clone cache, MR-ref fetch, worktrees); `diff` (line, word, rename detection, noisy-file rules, tree-sitter highlighting); per-file stats index; `filter` (full language, aggregates, sort, autocomplete data); presets store; grouping rules; SQLite persistence with journaling; review session with pin and update flow; `cli` harness | CLI can open a local branch, list files with stats, filter, sort, group, mark viewed, add drafts, export Markdown. Benchmarks on the synthetic MR meet the budget: open under 1 s, filter under 100 ms |
| M2 Providers | Provider trait; GitHub and GitLab implementations (MR list and filters, metadata, discussions, submit, approve and request changes, reviewers and re-request, CI status, viewed sync, custom CA); PAT, OAuth device flow, credential-helper clone; keychain storage; mock server | Contract tests pass for both providers against recorded fixtures; a failed submit leaves nothing half-posted; CLI can open an MR from a URL against the mock |
| M3 UI shell | Three-column layout; file list; virtualized unified and side-by-side diff; inline and global comments; submit dialog; viewed-progress bar; batch flow; tabs; recent-reviews page; quit prompt and crash resume; built-in themes, fonts, zoom, OS appearance; keymap and command palette | Compiles; clippy clean; UI logic covered by view-model tests; headless render smoke test if the environment allows |
| M4 Review depth | Filter bar and preset UI; git graph with per-commit and range review; file grouping and batches; noisy-file collapse; full thread parity; MR-update banner and re-anchor; local-branch review and attach | View-model tests for each flow; re-anchor tests on rewritten history |
| M5 AI | ACP client; CLI adapters for Claude Code, Gemini, Codex and opencode; docked chat per review; selection to context; answer to draft; full-review skill and AI-tagged drafts; agent safety rules | A fake-agent test double drives chat and review end to end; permission and environment rules have tests |
| M6 Integrations and packaging | CI widget; browser extension; `perusal://` handling and single instance; AppImage, Flatpak, deb and rpm scripts; theme validator and `agents.md` | Widget states tested; extension builds for Chromium and Firefox; packaging scripts run as far as the sandbox allows |
| M7 Hardening | Fuzz the filter parser; offline, 401 and rate-limit behavior; benchmarks kept in-repo; README and user docs | Fuzz run clean; benchmarks documented; docs complete |

**Post-v1:** Gitea, Forgejo and other hosts; Windows and macOS builds; RTL text and screen-reader support; code-aware grouping through includes and imports; structural (syntax-tree) diff; presets shared through a repository file; reviewing uncommitted working-tree changes; merge and MR editing actions.

## Risks and resolved defaults

Every item that earlier sections call open, proposed or 'to verify' is settled here. Treat these as final; do not ask the owner about them.

**Resolved defaults**

| Question | Decision |
| --- | --- |
| Progress bar denominator | All files in the MR. A batch marks all its files together. A setting can exclude auto-collapsed noisy files. |
| Recent-reviews retention | Kept until submitted or discarded; submitted or merged reviews expire after 30 days; pinned reviews never expire. |
| Replies and resolves on existing threads | Sent to the host immediately, outside the pending review. |
| Agent safety mitigations | Adopt all four listed in the AI section: minimal agent environment, per-review repository allowlist, visible web and repository access, drafts only. |
| GitLab viewed-state sync | Implement for GitHub. For GitLab, sync only if the API exposes it when you check; otherwise keep it local and note it in `DECISIONS.md`. |
| Host feature gaps | If a host lacks a feature, degrade gracefully, hide the control, and log the gap in `DECISIONS.md`. |
| Storage | SQLite for reviews, drafts, caches and journals; TOML for settings, presets, keymap and themes. |
| Submit mechanics | GitHub: one review call with its comments. GitLab: draft notes then bulk publish. |
| Name | `perusal`, isolated in `brand.rs`. |
| License | MIT OR Apache-2.0. |

**Defaults for details not specified earlier**

- Noisy-file patterns: lockfiles (`Cargo.lock`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `poetry.lock`, `go.sum`), `*.min.js`, `*.min.css`, `*.map`, `vendor/`, `dist/`, files with an `@generated` or `DO NOT EDIT` marker, binaries, and any file over 1 MB or over 5,000 changed lines. Collapsed files stay filterable.
- Built-in grouping rules: same stem across `.h`, `.hpp`, `.c`, `.cc`, `.cpp`; `foo.ts` with `foo.test.ts` or `foo.spec.ts`; `foo.py` with `test_foo.py`; `foo.rs` with `foo_test.rs`; `foo.proto` with its generated files. Stem matching works across folders.
- Full-review skill output: a JSON array of `{file, start_line, end_line, severity, body}`, where severity is one of `blocker`, `major`, `minor`, `nit`. The skill asks the agent to check correctness, security, tests and readability, to prefer few high-signal comments, and to say nothing when there is nothing to say. The skill file ships in the repo and is user-editable.
- Draft comments marked AI are visually distinct and exportable; discarded AI drafts are not stored.

**Risks**

| Risk | Mitigation |
| --- | --- |
| GPUI is pre-1.0, has sparse docs and several forks | One pinned distribution; all logic in UI-free crates so the UI layer can change |
| v1 scope is large | Milestones order the work so a usable reviewer exists after M4; M5 and M6 are additive |
| The build sandbox likely cannot run a window | Headless-testable core, view-model tests, best-effort virtual-display runs |
| Prompt injection through MR content | Agent safety rules; drafts only; no tokens in the agent environment |
| Host API differences (GitHub and GitLab) | Provider interface with contract tests and graceful degradation |
| Flatpak weakens the sandbox to run host tools | Documented trade-off; AppImage and native packages remain |
| Name and trademark clashes | One-place rename; owner checks domains and trademarks before release |

## Naming

The name is **Perusal**, chosen by the owner on 2026-11-24 over Diffwise. A rename is still a single change in `brand.rs`.

The original name, revu, collides with Bluebeam Revu, a long-established commercial PDF product, and with an existing GitHub project, eddmann/revu, that is a desktop git diff reviewer for AI agents. It was dropped for those reasons.

| Candidate | Findings (searched 2026-11-24) |
| --- | --- |
| Perusal | 85 GitHub repos, none in code review (top: a 2017 essay grader, an abandoned 6-star security tool). One letter from Perusall, an education reading platform. Free on crates.io; taken on npm and PyPI. |
| Diffwise | 22 GitHub repos, mostly created in the last two months, including a VS Code extension and a repo of privacy and support pages for a product of that name. Free on crates.io, npm and PyPI. |
| Taken, rejected | Nitpick, Verdict, Redpen, Hunk, Glint, Squint, Scrutiny, Peruse (a KDE application), Reviu |

These checks used registry and GitHub search only. Domains and trademarks were not checked; the owner does that before any public release.
