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

/// Pairs lines for a side-by-side view: context on both sides, runs of removals next to runs of additions.
/// Entries are indexes into `lines`: (left, right).
pub fn split_pairs(lines: &[Line]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut out = Vec::new(); let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == Kind::Context { out.push((Some(i), Some(i))); i += 1; continue; }
        let (mut rem, mut add) = (vec![], vec![]);
        while i < lines.len() && lines[i].kind == Kind::Removed { rem.push(i); i += 1; }
        while i < lines.len() && lines[i].kind == Kind::Added { add.push(i); i += 1; }
        for k in 0..rem.len().max(add.len()) { out.push((rem.get(k).copied(), add.get(k).copied())); }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class { Keyword, String, Comment, Number, Type, Function, Other }

/// Language name for a file extension, as understood by `highlight`. Unknown extensions get "generic"
/// (a lexical highlighter for comments, strings, numbers and common keywords).
pub fn language_for_ext(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "rs" => "rust", "py" | "pyi" => "python", "js" | "mjs" | "cjs" | "jsx" => "javascript", "ts" | "mts" | "cts" => "typescript", "tsx" => "tsx",
        "json" | "jsonc" => "json", "go" => "go", "c" | "h" => "c", "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp", "java" => "java",
        "sh" | "bash" | "zsh" => "bash", "css" | "scss" => "css", "html" | "htm" | "xml" | "svg" => "html",
        "" => "", _ => "generic",
    }
}

fn ts_language(name: &str) -> Option<tree_sitter::Language> {
    Some(match name {
        "rust" => tree_sitter_rust::LANGUAGE.into(), "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(), "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "json" => tree_sitter_json::LANGUAGE.into(), "go" => tree_sitter_go::LANGUAGE.into(), "c" => tree_sitter_c::LANGUAGE.into(), "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(), "bash" => tree_sitter_bash::LANGUAGE.into(), "css" => tree_sitter_css::LANGUAGE.into(), "html" => tree_sitter_html::LANGUAGE.into(),
        _ => return None,
    })
}

fn classify(n: &tree_sitter::Node) -> Option<Class> {
    let k = n.kind();
    if k.contains("comment") { return Some(Class::Comment); }
    if matches!(k, "string" | "string_literal" | "raw_string_literal" | "char_literal" | "template_string" | "interpreted_string_literal" | "raw_string" | "string_value" | "quoted_attribute_value" | "attribute_value" | "heredoc_body" | "character_literal" | "rune_literal" | "regex" | "string_fragment" | "concatenated_string") {
        // JSON object keys are strings too: colour them as names, not values
        if k == "string" && n.parent().map(|p| p.kind() == "pair" && p.child(0).map(|c| c.id() == n.id()).unwrap_or(false)).unwrap_or(false) { return Some(Class::Type); }
        return Some(Class::String);
    }
    if matches!(k, "number" | "integer" | "float" | "integer_literal" | "float_literal" | "number_literal" | "decimal_integer_literal" | "decimal_floating_point_literal" | "hex_integer_literal" | "int_literal" | "imaginary_literal" | "integer_value" | "float_value" | "true" | "false" | "null" | "boolean" | "none" | "nil") { return Some(Class::Number); }
    if matches!(k, "type_identifier" | "primitive_type" | "predefined_type" | "builtin_type" | "sized_type_specifier" | "tag_name" | "class_name" | "id_name" | "property_name" | "attribute_name") { return Some(if matches!(k, "tag_name") { Class::Keyword } else { Class::Type }); }
    if matches!(k, "identifier" | "property_identifier" | "field_identifier" | "function_identifier" | "shorthand_property_identifier" | "word") {
        let p = n.parent()?;
        let is_field = |parent: &tree_sitter::Node, f: &str| parent.child_by_field_name(f).map(|c| c.id() == n.id()).unwrap_or(false);
        let defs = ["function_item", "function_definition", "function_declaration", "method_definition", "method_declaration", "function_declarator", "generator_function_declaration", "method_signature", "function_signature_item", "command_name"];
        let calls = ["call_expression", "call", "method_invocation", "macro_invocation"];
        if (defs.contains(&p.kind()) && (is_field(&p, "name") || is_field(&p, "declarator"))) || (calls.contains(&p.kind()) && (is_field(&p, "function") || is_field(&p, "name"))) || p.kind() == "command_name" { return Some(Class::Function); }
        // obj.method(...) : the property/field/attribute of a member expression that is the callee
        if matches!(p.kind(), "member_expression" | "field_expression" | "attribute" | "selector_expression" | "scoped_identifier") {
            if let Some(gp) = p.parent() { if calls.contains(&gp.kind()) && gp.child_by_field_name("function").map(|f| f.id() == p.id()).unwrap_or(false) && p.child_by_field_name("property").or_else(|| p.child_by_field_name("field")).or_else(|| p.child_by_field_name("attribute")).or_else(|| p.child_by_field_name("name")).map(|c| c.id() == n.id()).unwrap_or(false) { return Some(Class::Function); } }
        }
        return None;
    }
    // keywords are anonymous word-like nodes: fn, let, if, function, const, return...
    (!n.is_named() && k.len() > 1 && k.chars().all(|c| c.is_ascii_lowercase() || c == '_')).then_some(Class::Keyword)
}

/// Syntax classes per byte range. Tree-sitter grammars cover rust, python, javascript, typescript, tsx, json, go, c, cpp,
/// java, bash, css and html; "generic" uses a lexical highlighter; an empty name returns nothing.
pub fn highlight(language: &str, src: &str) -> Vec<(usize, usize, Class)> {
    if language == "generic" { return generic_highlight(src); }
    let Some(lang) = ts_language(language) else { return vec![] };
    let mut p = tree_sitter::Parser::new();
    if p.set_language(&lang).is_err() { return vec![]; }
    let Some(tree) = p.parse(src, None) else { return vec![] };
    let mut out = Vec::new();
    let mut cur = tree.walk();
    'walk: loop {
        let n = cur.node();
        let class = classify(&n);
        let leafish = matches!(class, Some(Class::String | Class::Comment | Class::Number));
        if let Some(c) = class { if n.end_byte() > n.start_byte() { out.push((n.start_byte(), n.end_byte(), c)); } }
        if n.child_count() > 0 && !leafish && cur.goto_first_child() { continue; }
        loop {
            if cur.goto_next_sibling() { continue 'walk; }
            if !cur.goto_parent() { break 'walk; }
        }
    }
    out
}

const GENERIC_KEYWORDS: &[&str] = &["fn", "func", "function", "def", "class", "struct", "enum", "interface", "trait", "impl", "const", "let", "var", "val", "pub", "public", "private", "protected", "static",
    "if", "else", "elif", "for", "while", "do", "loop", "switch", "case", "match", "break", "continue", "return", "yield", "try", "catch", "except", "finally", "throw", "raise", "import", "from", "use",
    "package", "module", "export", "require", "include", "new", "defer", "async", "await", "in", "is", "not", "and", "or", "extern", "comptime", "pub_fn", "typedef", "namespace"];

/// A small lexical highlighter for languages without a grammar: comments (`//`, `/* */`, `#`, `--`), strings, numbers, keywords.
pub fn generic_highlight(src: &str) -> Vec<(usize, usize, Class)> {
    let b = src.as_bytes(); let mut out = vec![]; let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'/' && b.get(i + 1) == Some(&b'/') || c == b'#' && !(b.get(i + 1) == Some(&b'[') || b.get(i + 1) == Some(&b'!')) || c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b[i] != b'\n' { i += 1; } out.push((start, i, Class::Comment));
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2; while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') { i += 1; } i = (i + 2).min(b.len()); out.push((start, i, Class::Comment));
        } else if c == b'"' || c == b'\'' || c == b'`' {
            i += 1; while i < b.len() && b[i] != c && b[i] != b'\n' { if b[i] == b'\\' { i += 1; } i += 1; }
            i = (i + 1).min(b.len()); out.push((start, i, Class::String));
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') { i += 1; } out.push((start, i, Class::Number));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            if GENERIC_KEYWORDS.contains(&&src[start..i]) { out.push((start, i, Class::Keyword)); }
        } else { i += src[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1); }
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
    #[test] fn split_pairs_align_blocks() {
        let l = diff_lines("a\nb\nc\nd\n", "a\nB\nX\nc\nd\ne\n", 3);
        let p = split_pairs(&l);
        let name = |i: Option<usize>| i.map(|i| l[i].text.clone()).unwrap_or_else(|| "-".into());
        let got: Vec<(String, String)> = p.iter().map(|(a, b)| (name(*a), name(*b))).collect();
        assert_eq!(got, vec![("a".into(), "a".into()), ("b".into(), "B".into()), ("-".into(), "X".into()), ("c".into(), "c".into()), ("d".into(), "d".into()), ("-".into(), "e".into())]);
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
    #[test] fn more_languages_are_highlighted() {
        let has = |lang: &str, src: &str, c: Class, t: &str| highlight(lang, src).iter().any(|(a, b, k)| *k == c && &src[*a..*b] == t);
        let js = "const x = 42; // hi\nfunction go() { return \"a\"; }\nfoo.bar(1);";
        assert!(has("javascript", js, Class::Keyword, "const") && has("javascript", js, Class::Number, "42") && has("javascript", js, Class::Comment, "// hi") && has("javascript", js, Class::String, "\"a\""));
        assert!(has("javascript", js, Class::Function, "go") && has("javascript", js, Class::Function, "bar"));
        assert!(has("typescript", "interface A { n: number }\nlet a: string = 'x';", Class::Keyword, "interface") && has("tsx", "const a = <div className=\"x\">hi</div>;", Class::String, "\"x\""));
        assert!(has("json", "{\"k\": \"v\", \"n\": 3}", Class::Type, "\"k\"") && has("json", "{\"k\": \"v\"}", Class::String, "\"v\""));
        assert!(has("go", "package main\nfunc add(a int) int { return a }", Class::Keyword, "func") && has("c", "int main() { return 0; }", Class::Keyword, "return"));
        assert!(has("cpp", "class A { public: int x; };", Class::Keyword, "class") && has("java", "public class A { void f() {} }", Class::Keyword, "public"));
        assert!(has("bash", "if [ -f x ]; then echo \"hi\"; fi", Class::Keyword, "if") && has("css", "a { color: red; }", Class::Type, "color") && has("html", "<div class=\"a\">x</div>", Class::Keyword, "div"));
        let zig = "const std = @import(\"std\"); // zig\npub fn main() void { var x: u32 = 5; }";
        assert!(has("generic", zig, Class::Keyword, "const") && has("generic", zig, Class::String, "\"std\"") && has("generic", zig, Class::Comment, "// zig") && has("generic", zig, Class::Number, "5") && has("generic", zig, Class::Keyword, "pub"));
        assert_eq!(language_for_ext("TSX"), "tsx"); assert_eq!(language_for_ext("zig"), "generic"); assert_eq!(language_for_ext(""), "");
    }
    #[test] fn multi_line_constructs_work_on_whole_files() {
        let src = "/* one\n two */\nlet s = `a\nb`;\n"; let h = highlight("javascript", src);
        assert!(h.iter().any(|(a, b, k)| *k == Class::Comment && src[*a..*b].contains("two")) && h.iter().any(|(a, b, k)| *k == Class::String && src[*a..*b].contains('b')));
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
