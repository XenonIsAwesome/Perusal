# Customizing Perusal themes (for AI agents)

A theme is one TOML file placed in `~/.config/perusal/themes/`. Edits reload live. Start from a built-in
(`dark`, `light`, `github-dark`, `github-light`; copies are in the Perusal source under `crates/app/themes/`) and change colors.

## Schema (every key; all colors are `#rrggbb`)
```toml
name = "My Theme"      # shown in the picker
dark = true            # true for dark themes (used when following OS appearance)

[ui]       # required keys: background, foreground, accent, border
[syntax]   # optional keys: keyword, string, comment, number, type, function (tree-sitter capture classes)
[diff]     # required keys: added, removed (line backgrounds), added_word, removed_word (word-level highlights)
[status]   # required keys: ci_running, ci_passed, ci_failed, viewed, ai
```
Unknown keys are errors. Missing required keys are errors.

## Contrast rules
- `ui.foreground` on `ui.background`: at least 4.5:1.
- each `syntax.*` color on `ui.background`: at least 3:1.
- Keep `added`/`removed` backgrounds subtle so syntax colors stay readable; word highlights are stronger variants of them.

## Validate
```
perusal theme check path/to/theme.toml
```
Exit code 0 means valid. Errors name the table and key and say what to change. Fix every message, then re-run.

## Worked example: "a warmer dark theme"
1. Copy `dark.toml` to `~/.config/perusal/themes/warm.toml`, set `name = "Warm Dark"`.
2. Shift hues toward orange: `background = "#1c1714"`, `foreground = "#efe6dd"`, `border = "#3a302a"`, `accent = "#e8a25c"`.
3. Warm the syntax colors: `keyword = "#ff8a65"`, `string = "#ffd180"`, `comment = "#9c8b7e"`, `number = "#f6c177"`.
4. Re-tint diffs: `added = "#1f2a1a"`, `removed = "#331a17"`.
5. Run `perusal theme check warm.toml` and fix any contrast message by lightening the named color.
