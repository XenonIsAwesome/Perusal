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
Keymap overrides go in `keymap.toml` under `[keys]`. Themes: see `crates/app/themes/agents.md`; check with `perusal theme check`.

## Hosts and tokens
GitHub/GitLab (cloud and self-hosted). Tokens are stored in a mode-0600 file outside the database (keychain support pending).
Custom CA files per host. SSH keys only cover git transport; commenting needs a token.

## Agents
Agents run read-only in a checkout of the reviewed head, with a minimal environment (no tokens). Their output is only ever
saved as AI-tagged drafts. Edit `skills/review.md` to change the review prompt.

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
**AI review** runs the review prompt in `skills/review.md` and adds its findings as AI-tagged drafts you can edit or remove. Nothing is posted to the host.
Agents run headless in a read-only snapshot of the reviewed head, with a minimal environment: only PATH, HOME, LANG, TERM, TMPDIR and XDG_RUNTIME_DIR are passed, and any variable whose name looks like a credential (TOKEN, KEY, SECRET...) is dropped, including API keys such as `ANTHROPIC_API_KEY`. Agents that need such a variable must use their own stored login.
