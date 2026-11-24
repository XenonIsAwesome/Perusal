//! Line diffs with word-level highlights, and tree-sitter syntax classes.
use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind { Context, Added, Removed }

#[derive(Debug, Clone)]
pub struct Line {
    pub kind: Kind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
    /// Byte ranges within `text` changed at word level (only for Added/Removed lines paired with a counterpart).
    pub words: Vec<(usize, usize)>,
}

/// Unified line diff with `context` lines around changes.
pub fn diff_lines(old: &str, new: &str, context: usize) -> Vec<Line> {
    let d = TextDiff::from_lines(old, new);
    let mut out = Vec::new();
    for group in d.grouped_ops(context) {
        for op in group {
            for ch in d.iter_inline_changes(&op) {
                let kind = match ch.tag() { ChangeTag::Equal => Kind::Context, ChangeTag::Insert => Kind::Added, ChangeTag::Delete => Kind::Removed };
                let mut text = String::new(); let mut words = Vec::new();
                for (emph, s) in ch.iter_strings_lossy() {
                    if emph && kind != Kind::Context { words.push((text.len(), text.len() + s.len())); }
                    text.push_str(&s);
                }
                let text = text.trim_end_matches('\n').to_string();
                words.retain(|w| w.0 < text.len()); for w in &mut words { w.1 = w.1.min(text.len()); }
                out.push(Line { kind, old_no: ch.old_index().map(|i| i as u32 + 1), new_no: ch.new_index().map(|i| i as u32 + 1), text, words });
            }
        }
    }
    out
}

/// Number of hunks in a diff (contiguous change groups).
pub fn hunk_count(old: &str, new: &str) -> u32 { TextDiff::from_lines(old, new).grouped_ops(0).len() as u32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class { Keyword, String, Comment, Number, Type, Function, Other }

/// Syntax classes per byte range, from tree-sitter leaf nodes. Supported: rust, python. Others return empty.
pub fn highlight(language: &str, src: &str) -> Vec<(usize, usize, Class)> {
    let mut p = tree_sitter::Parser::new();
    let lang = match language { "rust" => tree_sitter_rust::LANGUAGE.into(), "python" => tree_sitter_python::LANGUAGE.into(), _ => return vec![] };
    if p.set_language(&lang).is_err() { return vec![]; }
    let Some(tree) = p.parse(src, None) else { return vec![] };
    let mut out = Vec::new();
    let mut cur = tree.walk();
    'walk: loop {
        let n = cur.node();
        let k = n.kind();
        let class = match k {
            "string_literal" | "string" | "raw_string_literal" | "char_literal" => Some(Class::String),
            "line_comment" | "block_comment" | "comment" => Some(Class::Comment),
            "integer_literal" | "float_literal" | "integer" | "float" => Some(Class::Number),
            "type_identifier" | "primitive_type" => Some(Class::Type),
            _ if !n.is_named() && k.chars().all(|c| c.is_ascii_lowercase() || c == '_') && k.len() > 1 => Some(Class::Keyword),
            _ => None,
        };
        let leafish = matches!(class, Some(Class::String | Class::Comment | Class::Number)) ;
        if let Some(c) = class { out.push((n.start_byte(), n.end_byte(), c)); }
        if n.child_count() > 0 && !leafish && cur.goto_first_child() { continue; }
        loop {
            if cur.goto_next_sibling() { continue 'walk; }
            if !cur.goto_parent() { break 'walk; }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn line_and_word_diff() {
        let l = diff_lines("a\nhello world\nc\n", "a\nhello there\nc\n", 1);
        let rem = l.iter().find(|x| x.kind == Kind::Removed).unwrap(); let add = l.iter().find(|x| x.kind == Kind::Added).unwrap();
        assert_eq!(rem.text, "hello world"); assert_eq!(add.text, "hello there");
        assert!(!rem.words.is_empty() && !add.words.is_empty());
        assert_eq!(&add.text[add.words[0].0..add.words[0].1], "there");
        assert_eq!(add.new_no, Some(2)); assert_eq!(rem.old_no, Some(2));
    }
    #[test] fn hunks() { assert_eq!(hunk_count("1\n2\n3\n4\n5\n", "x\n2\n3\n4\ny\n"), 2); }
    #[test] fn rust_highlight() {
        let src = "fn main() { let x: u32 = 42; // hi\n let s = \"a\"; }";
        let h = highlight("rust", src);
        let has = |c: Class, t: &str| h.iter().any(|(a, b, k)| *k == c && &src[*a..*b] == t);
        assert!(has(Class::Keyword, "fn") && has(Class::Keyword, "let")); assert!(has(Class::Number, "42"));
        assert!(has(Class::Comment, "// hi")); assert!(has(Class::String, "\"a\"")); assert!(has(Class::Type, "u32"));
        assert!(highlight("cobol", src).is_empty());
    }
    #[test] fn python_highlight() { let s = "def f():\n    return 'x'\n"; assert!(highlight("python", s).iter().any(|(_, _, c)| *c == Class::String)); }
}

/// Maps a 1-based old line to the new file; `None` if the line was changed or removed (needs manual placement).
pub fn map_line(old: &str, new: &str, line: u32) -> Option<u32> {
    let d = TextDiff::from_lines(old, new);
    for ch in d.iter_all_changes() {
        if ch.tag() == ChangeTag::Equal && ch.old_index().map(|i| i as u32 + 1) == Some(line) { return ch.new_index().map(|i| i as u32 + 1); }
    }
    None
}

/// Re-anchors a line range; both ends must survive.
pub fn reanchor(old: &str, new: &str, range: (u32, u32)) -> Option<(u32, u32)> {
    Some((map_line(old, new, range.0)?, map_line(old, new, range.1)?))
}

#[cfg(test)]
mod reanchor_tests {
    use super::*;
    #[test] fn shifts_and_flags() {
        let old = "a\nb\nc\nd\n"; let new = "x\ny\na\nB\nc\nd\n";
        assert_eq!(map_line(old, new, 1), Some(3)); assert_eq!(map_line(old, new, 2), None);
        assert_eq!(reanchor(old, new, (3, 4)), Some((5, 6))); assert_eq!(reanchor(old, new, (2, 3)), None);
    }
}
