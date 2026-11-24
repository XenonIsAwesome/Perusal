# Decisions
- Name `perusal`, isolated in `crates/core/src/brand.rs` (spec).
- License MIT OR Apache-2.0; LICENSE-APACHE holds a pointer to the canonical text rather than the full text (fetch full text before release).
- GPUI not wired in M0: app crate is a stub so `cargo build --workspace` stays fast and green; the GPUI pin is chosen when M3 starts, per spec.
- Synthetic MR generator lives in `perusal-cli` (`perusal-cli gen <dir>`), using the git CLI.
