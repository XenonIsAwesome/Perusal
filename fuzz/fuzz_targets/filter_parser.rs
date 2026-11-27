#![no_main]
//! Coverage-guided fuzzing of the filter/sort language: no input may panic, hang or overflow the stack.
//! Run with: cargo +nightly fuzz run filter_parser   (from this directory; needs `cargo install cargo-fuzz`)
use libfuzzer_sys::fuzz_target;
use perusal_diff::{FileStat, Status};

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };
    let files = vec![FileStat::new("a.rs", Status::Added, 3, 1), FileStat::new("b/c.py", Status::Modified, 0, 9)];
    let _ = perusal_filter::filter(&files, s);
    let mut idx = vec![0, 1];
    let _ = perusal_filter::sort(&files, &mut idx, s);
    let _ = perusal_filter::complete(s);
});
