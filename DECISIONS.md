# Decisions
- Name `perusal`, isolated in `crates/core/src/brand.rs` (spec).
- License MIT OR Apache-2.0; LICENSE-APACHE holds a pointer to the canonical text rather than the full text (fetch full text before release).
- GPUI not wired in M0: app crate is a stub so `cargo build --workspace` stays fast and green; the GPUI pin is chosen when M3 starts, per spec.
- Synthetic MR generator lives in `perusal-cli` (`perusal-cli gen <dir>`), using the git CLI.
- Line/word diff uses `similar` (inline feature) instead of imara-diff: stable API, built-in word-level inline changes.
- Tree-sitter highlighting ships rust and python grammars first; other languages load lazily in later work (return no spans now).
- Re-anchor of drafts across a head move uses a line-diff map; ranges where either end changed are flagged for manual placement.
