# Perusal prompt templates

This folder holds every prompt Perusal sends to your AI agent. Each file is a Markdown template; Perusal reads it fresh
for every request, so edits apply immediately (no restart). Delete a file to get the built-in default back; a file that is
empty is ignored. Perusal only creates missing files, it never overwrites yours.

A template is plain text with `{{field}}` placeholders. Each placeholder is replaced by the value described below.
A placeholder that is not listed for a file is left as written. Values are inserted once and never re-expanded, so text
that happens to contain `{{...}}` (code, comments) stays literal. Optional fields are an empty string when they do not apply.

Keep the answer-format instructions in the templates that ask for JSON: Perusal parses the agent's reply and rejects
anything else. Keep the sentences that tell the agent to treat repository and quoted text as untrusted data.

## Files and their fields

### chat.md (Ask AI)
- `{{selection}}` the selected code block with its location ("The reviewer selected path:1-5:" and a fenced snippet, up to 6000 characters), or empty when nothing is selected
- `{{selection_file}}`, `{{selection_start}}`, `{{selection_end}}`, `{{selection_code}}` the parts of the selection, each empty when nothing is selected
- `{{history}}` the last 6 messages of the chat, one per line as `Reviewer: ...` / `You: ...`, or empty
- `{{question}}` the reviewer's new question

### review.md (full AI review)
- `{{overview}}` a paragraph with the merge request link telling the agent to fetch the overview (title, description) itself, or empty for local branch reviews. The overview is not inlined because it can be huge
- `{{mr_url}}` the bare merge request link, or empty
- `{{threads}}` the discussion already on the merge request (author, text, resolved flag, capped), or empty
- `{{drafts}}` the reviewer's unsent local draft comments, or empty
- `{{context}}` `{{overview}}`, `{{threads}}` and `{{drafts}}` together
- `{{changed_files}}` one changed path per line
- `{{patch}}` the unified diff, truncated to 60000 characters

The reply must stay a JSON array of `{"file", "start_line", "end_line", "severity", "body"}` with severity one of blocker, major, minor, nit.

### generate-filter.md, generate-sort.md ("Generate with AI" for filters and sorts)
- `{{request}}` what the user typed (untrusted text)
- `{{reference}}` the generated reference of the filter language: fields, functions and operators

The reply must stay one JSON object: `{"filter": "..."}` or `{"sort": "..."}`.

### generate-group-rule.md ("Generate with AI" for grouping rules)
- `{{request}}` what the user typed (untrusted text)
- `{{reference}}` empty today, reserved

The reply must stay one JSON object `{"label": "...", "pattern": "..."}` whose pattern has a named group `(?P<stem>...)`.
