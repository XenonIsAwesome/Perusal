# Progress
- M0 Foundation: done except GPUI hello-window / diff spike (deferred to M3, see DECISIONS.md).
- M1 Core engine: partial. Done: diff stats index (git numstat), noisy rules, grouping, filter language (aggregates, multi-key sort, autocomplete; 10k files filter+sort ~2 ms release), presets (TOML, global/project), SQLite store (viewed, drafts, journal, markdown export), gitops (partial clone, MR-ref fetch, worktree, commits), CLI (gen, files, view, draft, export).
  Remaining: tree-sitter highlighting, line/word diff (imara-diff), full-size synthetic MR benchmark of open time, pin-and-update flow.
- M2 Providers: done against mocks: GitHub+GitLab trait impls, submit atomicity, CI, threads, errors (401, 429, offline), device flow, custom CA client, file token store, `perusal-cli mr <url>`.
