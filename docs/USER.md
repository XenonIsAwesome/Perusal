# Using Perusal

## Headless review (`perusal-cli`)
```bash
perusal-cli files <repo> <branch> --base main --filter 'additions > 50 && !viewed' --sort 'additions + deletions desc, name asc'
perusal-cli files <repo> <branch> --group          # related files batched (foo.h + foo.cpp, foo.ts + foo.test.ts)
perusal-cli view <repo> <branch> <path>            # mark viewed (--unset to clear)
perusal-cli draft <repo> <branch> "text" --file f --line 12 [--ai]
perusal-cli export <repo> <branch>                 # Markdown of drafts
perusal-cli mr https://github.com/o/r/pull/3       # needs PERUSAL_TOKEN
```
State lives in `perusal.sqlite` (override with `--db`). Drafts and viewed marks persist across crashes.

## Filter language
Fields: name path ext language status binary generated additions deletions net ratio hunks size viewed comments unresolved ai noisy changed.
Operators: `&& || ! == != < <= > >= + - * / %`, regex match `~ /re/`, substring `~ "text"`.
Aggregates over all files: `avg() sum() max() min() count() top(n, expr) bottom(n, expr)`.
Examples: `additions > avg(additions)`, `name ~ /test/ && !viewed`, `top(10, additions + deletions)`.
Sort: any expressions, comma separated, each optionally `asc` or `desc`.

## Presets, keys, themes
Presets are TOML (global and per project, keyed by normalized remote URL); a project default auto-applies.
Keymap overrides go in `keymap.toml` under `[keys]`. Themes: see `crates/app/themes/AGENTS.md`; check with `perusal theme check`.

## Hosts and tokens
GitHub/GitLab (cloud and self-hosted). Tokens are stored in a mode-0600 file outside the database (keychain support pending).
Self-hosted hosts with a private CA work once the CA is in the system trust store (`update-ca-certificates`, or `SSL_CERT_FILE`/`SSL_CERT_DIR`); `PERUSAL_CA_FILE` adds one more. SSH keys only cover git transport; commenting needs a token.

## Agents
Agents run read-only in a checkout of the reviewed head, with a minimal environment (no tokens). Their output is only ever
saved as AI-tagged drafts. Every prompt (chat, review, filter/sort/rule generation) is an editable Markdown template in `~/.config/perusal/prompts/` (Settings, AI, Open prompts folder); see the `AGENTS.md` there.

## Browser extension
`extension/build.sh` builds Chromium and Firefox zips. Add self-hosted domains in the extension options.

## Files panel
Switch between **List** and **Tree** in the Files header (or press `t`; the choice is remembered). Tree mode groups files by directory,
folders first, compresses single-child chains (`crates/ui/src`), and shows `viewed/total` and `+/−` for each folder.
Click a folder to collapse or expand it; **Collapse all** and **Expand all** act on every folder. Moving with `n`/`p`/Enter follows the
order shown in the tree and expands folders to reveal the current file.

## Settings
Open **Settings** in the review header (or press `s`). Everything is saved to `~/.config/perusal/settings.toml`.
- **Appearance:** theme (the four built-ins plus any valid file in `~/.config/perusal/themes/`), and an option to follow the OS light/dark setting with a chosen pair.
- **Review:** group related files automatically, and List or Tree for the Files panel.
- **Diff:** default diff view (Unified or Split) and whether Markdown files open as a source diff or rendered. Both can still be switched per file in the file header (`u` toggles Unified/Split, `m` toggles Markdown Diff/Preview).
- **AI:** which agent CLI to use (Claude Code, Gemini CLI, Codex, opencode) or a custom command.

## Filter autocomplete
Press `/` and type. Suggestions with descriptions appear for fields, functions and values (`status == "ad` offers `added`). Up/Down choose, Tab accepts, Enter runs the filter, Esc closes.

## Selecting lines, grouping files
Click a diff line, drag across several, or Shift+click to extend. A bar offers **Comment** and **Ask AI** for exactly those lines (`c` and `a` also work).
In the Files panel, Ctrl+click toggles files and Shift+click selects a range; **Review together** turns them into one batch shown stacked with per-file headers, and **Ungroup** undoes it. Manual groups last for the session.

## AI
**Ask AI** (`a`) opens a chat in the right panel with the selected lines as context. Answers can be saved as AI-tagged drafts with **Add as draft**.
**AI review** runs the review prompt template (`prompts/review.md`) and adds its findings as AI-tagged drafts you can edit or remove. Nothing is posted to the host. The agent is also told about the discussion already on the merge request and your local drafts (so it does not repeat them) and gets the merge request link to fetch the overview itself.
Agents run headless in a read-only snapshot of the reviewed head, with a minimal environment: only PATH, HOME, LANG, TERM, TMPDIR and XDG_RUNTIME_DIR are passed, and any variable whose name looks like a credential (TOKEN, KEY, SECRET...) is dropped, including API keys such as `ANTHROPIC_API_KEY`. Agents that need such a variable must use their own stored login.

## Layout and controls (v2)
- **Floating bar** at the bottom of the diff: previous/next batch, a comment on the whole merge request, Unified/Split, Diff/Preview for Markdown, and **Finish review**.
- **Publish comments instantly** (Settings, off by default): on remote reviews each new comment is posted as soon as you add it and Finish review (and `f`) is hidden; turn it off to approve or to post a summary. Leftover drafts show a publish icon.
- **Finish review** (`f`) posts your drafts plus an optional summary and a verdict (Comment, Approve, Request changes; GitLab has no Request changes). You can finish before viewing every file. For a local review it copies your drafts as Markdown and saves a file.
- **Sidebars** collapse with their chevron buttons and resize by dragging the divider; the right panel's Commits and Drafts sections collapse too. Layout is remembered.
- **Commit scope:** click a commit to review only that commit. A chip above the file list shows it; click it again or the close button to show everything.
- **Symlinks** are shown in italics with a link icon and a note about the old and new target.
- **Markdown preview** supports commenting: click a block (Shift+click to extend) and press `c`. The comment anchors to the block's source lines.
- **Filters:** `ai` lists files with AI drafts and `comments > 0` files with any draft.
- **Text fields** support mouse selection: click to place the caret, drag to select, double-click for a word, triple-click for all.
- **AI:** while an agent works you see a loading indicator, the elapsed time and what it is doing (for Claude Code: files read, searches, web lookups). **Stop** ends it immediately. Settings has the agent, model, effort/variant and the response timeout (15 minutes by default, or No limit).

## Grouping rules
Settings has the built-in rules (C/C++ headers and sources, TS/JS tests, Python tests, Rust tests, Protobuf with generated files) with an on/off switch each, and lets you add your own: a regular expression with a named `(?P<stem>...)` group. Files matching the same rule with the same stem, even in different folders, are batched. The first matching rule wins and very generic names (main, lib, index...) are ignored.

## Custom themes
Settings > Appearance > **Open themes folder** opens `~/.config/perusal/themes/` in your file manager and creates an `AGENTS.md` there that explains the format to an AI agent. Open your agent in that folder and ask, for example: "Read AGENTS.md, then create a warmer dark theme called warm.toml and validate it with `perusal theme check warm.toml`."

## v3 additions
- **Remembered:** My reviews filters are remembered between runs; each review remembers its own file filter and sort. **Leaving a review:** closing a tab or the window with unsubmitted drafts or a partly viewed list asks to Submit/Finish, Keep for later or Discard. **Find:** Ctrl+F searches the diff or the overview (Enter / Shift+Enter or F3 step through matches, Esc closes). **Reordering:** drag tabs to change their order.
- **Publish now:** the comment box on a pull or merge request has a Publish now button next to Add draft; it posts that comment to the host immediately instead of keeping it as a draft (if posting fails it stays a draft).
- **Panels:** drag the bar between Commits and Drafts in the review panel to resize them. **Errors:** every error message has a copy button.
- **Refresh:** the circular arrow in the review header (or R / F5) fetches new commits, CI and comments and brings the review up to date, keeping drafts, filter and sort; a spinner and a progress bar show it is working. Files you had marked viewed that changed in the update are unmarked (also when you reopen a review whose request has new commits).
- **Tabs:** every review opens in its own tab next to Home (Ctrl+Tab and Ctrl+Shift+Tab switch, Ctrl+W closes). Esc goes to Home; reviews keep running in the background and are saved continuously.
- **Home:** the start tab (Recent reviews or My reviews) is a setting under Settings, Home; with no recent reviews it always starts on My reviews. *Recent reviews* and *My reviews* tabs. My reviews lists every pull and merge request in the projects you are a member of and can review in (GitHub: repositories you can push to; GitLab: projects where you are at least Developer; the 30 newest per project, 40 most recently active projects), across every account you signed in to. Filters, modelled on GitHub's issue browser: Author, Assignee and Label dropdowns (multi-select; choices in one dropdown are OR-ed, different filters are AND-ed; Me is the first choice of the people dropdowns), an *Assigned to me as reviewer* toggle (GitLab reviewer lists; GitHub has no dependable reviewer list so it still lists everything you can review), Remote (All/GitHub/GitLab) and State chips. Dropdowns only offer values that exist on the requests left after the other filters, and have a Filter box. Recent reviews has the same Remote/State filters. Rename, pin and delete reviews from the list; rename the open review with the pencil next to its title.
- **Open a review:** URL, local branch (with an optional base branch or commit), or browse a repository. Public repositories work without a token.
- **Accounts and setup:** Settings > Accounts, or the first-run setup. Sign in with a pasted token, your `gh`/`glab` login, or the browser (OAuth device flow; needs an OAuth app client ID for the host). Settings is also available from Home.
- **Sort and presets:** next to the filter there is a sort field (`additions + deletions desc, name asc`) with autocomplete, and a presets menu to save filter+sort per project or globally and make one the default.
- **Comments:** a Markdown editor (bold, italic, code, quote, list, link, suggestions, Preview tab). Comments, including other people's, appear under their line in the diff; collapse one or all of them, and switch a comment between rich and raw. Hovering a comment highlights the lines it is about. A collapsed comment shrinks to a comment icon in the line-number gutter (click it to expand), and resolved threads start collapsed. Edit your own drafts, including AI ones, from the card or the Drafts list.
- **Next vs viewed:** Next no longer marks files viewed. The eye box in the floating bar does.
- **CI and updates:** a CI icon in the header (click for the checks page). Merged, closed, new commits and CI results appear as in-app notifications and as desktop notifications (Settings > Notifications).

## Testing against a mock host
`PERUSAL_API_BASE` replaces the API base URL, `PERUSAL_GIT_BASE` replaces the clone base (for example `file:///srv/git`), and `PERUSAL_POLL_SECS` sets the update interval. Useful for GitHub Enterprise proxies and for tests.

## v4 additions
- **Sort** is a button that opens a tier panel: add tiers from field chips or write an expression, flip asc/desc, reorder, remove. **Generate with AI** (sparkle next to the filter, inside the sort panel, and in Settings > Review > Grouping rules) turns a plain description into a filter, sort or rule using your agent.
- **Hide whitespace-only changes** (space-bar button in the filter row, default in Settings > Review). Filter fields `whitespace` and `meaningful` work in filters and sorts.
- Settings has tabs: Appearance (themes, built-in title bar), Accounts (Import all logins from gh/glab, including enterprise/self-hosted hosts), Notifications (per-event desktop notifications), Review, Presets (global and per-project), AI (model and effort dropdowns with a Custom option).
- Grouping rules can be duplicated (built-ins too) and custom ones edited; the C/C++ rule covers .h .hh .hpp .hxx .inl .ipp .tpp .txx .c .cc .cpp .cxx .cu .cuh .m .mm.
- Files in a grouped batch can be collapsed from their header. Next wraps from the last batch to the first. A symlink always shows "Go to the original" (it says so when the target is not part of the change). The merge request link in the header opens it in the browser.
- Comments render bold, italic, mono code, clickable links and highlighted code blocks; suggestions are shown as a diff against the original lines.
- Settings > AI has an Agent executable field (full path) that overrides the PATH lookup, with a line showing which executable will be used.
- With opencode selected, the Model dropdown lists the models from `opencode models` (free opencode/ ones first); Custom still lets you type any name.

## Releases
Push a tag like `v1.2.3` to build and publish: `.deb`, `.rpm`, AppImage, Flatpak bundle, a tarball, both browser extensions and `SHA256SUMS`. The package version is the tag without the `v`. Pull requests run the test suite and clippy.

## v5 additions
- **Keys:** `?` lists every shortcut and `Ctrl+P` opens a command palette (type to filter, Enter runs). Rebind keys in `~/.config/perusal/keymap.toml`:
  ```toml
  [keys]
  comment = "z"
  next_file = "j"
  ```
  A key bound twice or an unknown action name makes the whole file ignored, with a warning. Settings > Appearance > Keyboard has a vim preset (j/k) and opens the config folder.
- **Quitting:** closing the window (or Ctrl+Q) with an unfinished review asks: Submit (remote reviews), Keep for later (drafts are saved either way), Discard.
- **Existing threads:** each thread card has Reply, Resolve/Unresolve and a thumbs-up; your own comments and replies also have Edit and Delete. These go to the host at once and are not part of the pending review. The person icon in the header requests a review from the usernames you type.
- **New commits:** when the host head changes a banner offers Move to latest. Drafts follow their lines where those survive (the toast says how many need a new place), and files that changed are marked unviewed again. A force-push is reported as such.
- **Commits:** Shift+click a second commit in the graph to scope the review to the range.
- **Comments on removed lines** are posted on the old side of the diff.
- **Manual file groups** are saved with the review.
- **Fonts and motion:** Settings > Appearance has UI font, code font, code line height, ligatures and Reduce motion.
- **Tokens** go to the OS keychain when `secret-tool` is available (GNOME Keyring, KWallet); existing token files are moved there on first use.
- **Links:** `perusal <link>` opens in the running window. For an AppImage, Settings > Appearance > Links registers it as the `perusal://` handler.
- **GitLab sign-in** also works through the browser (OAuth device flow) once you enter the client ID of an OAuth application with device flow enabled.
- **Overview:** pull/merge request reviews have two screens in the centre pane, switched with the Overview / Changes tabs or the `O` key. The Overview shows title, state, branches, the Markdown description and the general conversation (comments not tied to a line, plus your own global drafts), with a button to add a global comment and one to jump to the changes. It is the first screen by default; Settings > Review > First screen can open on the code instead. Selecting a file, `n`/`p`, `Enter` and the like jump to the code. Local branch reviews have no overview.

- **User pictures:** people show with their host picture (overview author, assignees and reviewers, comment authors and replies, commits in the graph, My reviews rows). When a picture is unavailable or does not apply, such as a local git committer, you get a generated badge: two letters of the name (`XenonIsAwesome` is `XA`) on a colour picked from the name.
- **My reviews search:** free text also searches the overview text; qualifiers `author:bob`, `assignee:alice`, `reviewer:carol`, `label:bug`, `project:grp/app`, `host:corp`, `state:open`, `kind:gitlab`, `is:draft`, `user:name` (any role) work too, and a leading `-` excludes.
- **Publish now:** the comment box on a pull or merge request has a Publish now button next to Add draft; it posts that comment to the host immediately instead of keeping it as a draft (if posting fails it stays a draft).
- **Panels:** drag the bar between Commits and Drafts in the review panel to resize them. **Errors:** every error message has a copy button.
- **Refresh:** the circular arrow in the review header (or R / F5) fetches new commits, CI and comments and brings the review up to date, keeping drafts, filter and sort; a spinner and a progress bar show it is working. Files you had marked viewed that changed in the update are unmarked (also when you reopen a review whose request has new commits).
- **Tabs:** middle-click a review tab to close it. **Whole-request comments** (the overview Comment button) are drafts like any other and are posted as separate conversation threads when you finish the review.
- **Search on Home:** Recent reviews and My reviews each have a search box above the filters. Every word you type must match (case-insensitive) the title, repository/branch or host, project, number, author or state, and it combines with the Remote and State chips. The clear button empties it.
