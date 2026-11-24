# Benchmarks

`cargo run --release -p perusal-cli -- bench` generates a synthetic MR (10,000 files, 500,000 changed lines), then times
indexing (open), a filter with aggregates, and a two-key sort, and checks the spec budgets.
Unit-level 10k-file filter+sort timing is also tested in `crates/filter` (`perf`).

Measured in the development container (release build):

| Measure | Budget | Result |
|---|---|---|
| Open (index 10,000 files) | < 1 s | ~0.3 s |
| Filter with aggregates | < 100 ms | ~1 ms |
| Two-key sort | < 100 ms | ~21 ms |
| Peak memory (CLI) | < ~1 GB | ~7 MB |

Not measured: diff rendering and scroll rate (no window yet), per-file diff cost with tree-sitter on large files.
