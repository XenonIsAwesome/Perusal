//! In-page search (Ctrl+F): finds every occurrence of a query in a text.
use std::ops::Range;

/// Byte ranges of every non-overlapping occurrence of `query` in `text`. ASCII letters match regardless of case; other characters
/// match exactly (so byte positions stay valid for highlighting). An empty query matches nothing.
pub fn find_all(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() || query.len() > text.len() { return vec![]; }
    let (h, n) = (text.to_ascii_lowercase(), query.to_ascii_lowercase());
    let mut out = vec![]; let mut from = 0;
    while let Some(i) = h[from..].find(&n) { let s = from + i; out.push(s..s + n.len()); from = s + n.len(); }
    out
}

/// Next/previous hit index with wrap-around; `None` when there are no hits.
pub fn step(cur: usize, count: usize, forward: bool) -> Option<usize> {
    if count == 0 { None } else if forward { Some((cur + 1) % count) } else { Some((cur + count - 1) % count) }
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn finds_case_insensitively_without_overlap() {
        assert_eq!(find_all("Foo foo FOO", "foo"), vec![0..3, 4..7, 8..11]);
        assert_eq!(find_all("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(find_all("abc", "").is_empty() && find_all("abc", "abcd").is_empty() && find_all("abc", "x").is_empty());
    }
    #[test] fn ranges_stay_on_char_boundaries() {
        let t = "héllo wörld héllo"; let r = find_all(t, "héllo"); assert_eq!(r.len(), 2); for x in r { assert!(t.is_char_boundary(x.start) && t.is_char_boundary(x.end)); }
    }
    #[test] fn stepping_wraps() { assert_eq!(step(2, 3, true), Some(0)); assert_eq!(step(0, 3, false), Some(2)); assert_eq!(step(0, 0, true), None); }
}
