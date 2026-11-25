//! Markdown to a small block model that the UI renders. UI-free.
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Span { pub text: String, pub bold: bool, pub italic: bool, pub strike: bool, pub code: bool, pub link: Option<String> }

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Span>), Para(Vec<Span>), Code { lang: String, text: String },
    List { ordered: bool, start: u64, items: Vec<Vec<Block>> }, Quote(Vec<Block>), Rule,
    Table { head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
}

pub fn parse(src: &str) -> Vec<Block> {
    let mut stack: Vec<Vec<Block>> = vec![vec![]];
    let mut spans: Vec<Span> = vec![];
    let (mut bold, mut italic, mut strike) = (0u32, 0u32, 0u32); let mut link: Option<String> = None;
    let mut code: Option<(String, String)> = None;
    let mut lists: Vec<(bool, u64, Vec<Vec<Block>>)> = vec![];
    let mut head: Vec<Vec<Span>> = vec![]; let mut rows: Vec<Vec<Vec<Span>>> = vec![]; let mut cur_row: Vec<Vec<Span>> = vec![]; let mut in_head = false;
    let push_text = |spans: &mut Vec<Span>, t: &str, code: bool, bold: u32, italic: u32, strike: u32, link: &Option<String>| {
        spans.push(Span { text: t.to_string(), bold: bold > 0, italic: italic > 0, strike: strike > 0, code, link: link.clone() });
    };
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(Tag::CodeBlock(k)) => { code = Some((match k { CodeBlockKind::Fenced(l) => l.to_string(), _ => String::new() }, String::new())); }
            Event::End(TagEnd::CodeBlock) => { if let Some((lang, text)) = code.take() { stack.last_mut().unwrap().push(Block::Code { lang, text: text.trim_end_matches('\n').to_string() }); } }
            Event::Text(t) if code.is_some() => { code.as_mut().unwrap().1.push_str(&t); }
            Event::Start(Tag::Paragraph) | Event::Start(Tag::Heading { .. }) | Event::Start(Tag::HtmlBlock) => spans.clear(),
            Event::End(TagEnd::HtmlBlock) => { let s = std::mem::take(&mut spans); if !s.is_empty() { stack.last_mut().unwrap().push(Block::Para(s)); } }
            Event::End(TagEnd::Paragraph) => { let s = std::mem::take(&mut spans); stack.last_mut().unwrap().push(Block::Para(s)); }
            Event::End(TagEnd::Heading(l)) => { let s = std::mem::take(&mut spans); let n = match l { HeadingLevel::H1 => 1, HeadingLevel::H2 => 2, HeadingLevel::H3 => 3, HeadingLevel::H4 => 4, HeadingLevel::H5 => 5, HeadingLevel::H6 => 6 }; stack.last_mut().unwrap().push(Block::Heading(n, s)); }
            Event::Start(Tag::BlockQuote(_)) => stack.push(vec![]),
            Event::End(TagEnd::BlockQuote(_)) => { let b = stack.pop().unwrap(); stack.last_mut().unwrap().push(Block::Quote(b)); }
            Event::Start(Tag::List(start)) => lists.push((start.is_some(), start.unwrap_or(1), vec![])),
            Event::Start(Tag::Item) => { spans.clear(); stack.push(vec![]); }
            Event::End(TagEnd::Item) => {
                if !spans.is_empty() { let s = std::mem::take(&mut spans); stack.last_mut().unwrap().push(Block::Para(s)); }
                let it = stack.pop().unwrap(); if let Some(l) = lists.last_mut() { l.2.push(it); }
            }
            Event::End(TagEnd::List(_)) => { if let Some((ordered, start, items)) = lists.pop() { stack.last_mut().unwrap().push(Block::List { ordered, start, items }); } }
            Event::Rule => stack.last_mut().unwrap().push(Block::Rule),
            Event::Start(Tag::Emphasis) => italic += 1, Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Strong) => bold += 1, Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => strike += 1, Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()), Event::End(TagEnd::Link) => link = None,
            Event::Start(Tag::Image { .. }) => push_text(&mut spans, "[image] ", false, bold, italic, strike, &link),
            Event::Text(t) => push_text(&mut spans, &t, false, bold, italic, strike, &link),
            Event::Code(t) => push_text(&mut spans, &t, true, bold, italic, strike, &link),
            Event::Html(t) | Event::InlineHtml(t) => push_text(&mut spans, &t, false, bold, italic, strike, &link),
            Event::SoftBreak => push_text(&mut spans, " ", false, bold, italic, strike, &link),
            Event::HardBreak => push_text(&mut spans, "\n", false, bold, italic, strike, &link),
            Event::TaskListMarker(done) => push_text(&mut spans, if done { "☑ " } else { "☐ " }, false, 0, 0, 0, &None),
            Event::Start(Tag::Table(_)) => { head.clear(); rows.clear(); }
            Event::Start(Tag::TableHead) => { in_head = true; cur_row.clear(); }
            Event::End(TagEnd::TableHead) => { head = std::mem::take(&mut cur_row); in_head = false; }
            Event::Start(Tag::TableRow) => cur_row.clear(),
            Event::End(TagEnd::TableRow) => { if !in_head { rows.push(std::mem::take(&mut cur_row)); } }
            Event::Start(Tag::TableCell) => spans.clear(),
            Event::End(TagEnd::TableCell) => cur_row.push(std::mem::take(&mut spans)),
            Event::End(TagEnd::Table) => { let (h, r) = (std::mem::take(&mut head), std::mem::take(&mut rows)); stack.last_mut().unwrap().push(Block::Table { head: h, rows: r }); }
            _ => {}
        }
    }
    if !spans.is_empty() { let s = std::mem::take(&mut spans); stack[0].push(Block::Para(s)); }
    stack.swap_remove(0)
}

/// A top-level block with the 1-based source lines it covers (for commenting on rendered markdown).
#[derive(Debug, Clone, PartialEq)]
pub struct Located { pub block: Block, pub first_line: u32, pub last_line: u32 }

fn is_block_tag(t: &Tag) -> bool { matches!(t, Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote(_) | Tag::CodeBlock(_) | Tag::List(_) | Tag::Table(_) | Tag::HtmlBlock) }
fn is_block_end(t: &TagEnd) -> bool { matches!(t, TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::BlockQuote(_) | TagEnd::CodeBlock | TagEnd::List(_) | TagEnd::Table | TagEnd::HtmlBlock) }

pub fn parse_located(src: &str) -> Vec<Located> {
    let blocks = parse(src);
    let line_of = |off: usize| src[..off.min(src.len())].bytes().filter(|b| *b == b'\n').count() as u32 + 1;
    let mut ranges: Vec<(u32, u32)> = vec![]; let mut depth = 0usize; let mut ldepth = 0usize;
    // source lines of each item of a top-level list, by block index
    let mut items: std::collections::HashMap<usize, Vec<(u32, u32)>> = Default::default();
    for (ev, r) in Parser::new_ext(src, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS).into_offset_iter() {
        match &ev {
            Event::Start(t) if is_block_tag(t) => { if depth == 0 { let end = src[..r.end.min(src.len())].trim_end().len(); ranges.push((line_of(r.start), line_of(end.saturating_sub(1)))); } depth += 1; if matches!(t, Tag::List(_)) { ldepth += 1; } }
            Event::End(t) if is_block_end(t) => { depth = depth.saturating_sub(1); if matches!(t, TagEnd::List(_)) { ldepth = ldepth.saturating_sub(1); } }
            Event::Start(Tag::Item) if ldepth == 1 && depth == 1 => { let end = src[..r.end.min(src.len())].trim_end().len(); items.entry(ranges.len().saturating_sub(1)).or_default().push((line_of(r.start), line_of(end.saturating_sub(1)))); }
            Event::Rule if depth == 0 => ranges.push((line_of(r.start), line_of(r.start))),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (i, block) in blocks.into_iter().enumerate() {
        let (a, b) = ranges.get(i).copied().unwrap_or((0, 0));
        match block {
            // each bullet or numbered item can be commented on by itself
            Block::List { ordered, start, items: its } if items.get(&i).map(|r| r.len() == its.len() && its.len() > 1).unwrap_or(false) => {
                let rs = &items[&i];
                for (k, it) in its.into_iter().enumerate() { out.push(Located { block: Block::List { ordered, start: start + k as u64, items: vec![it] }, first_line: rs[k].0, last_line: rs[k].1.max(rs[k].0) }); }
            }
            block => out.push(Located { block, first_line: a, last_line: b.max(a) }),
        }
    }
    out
}

pub fn is_markdown(path: &str) -> bool { let l = path.to_lowercase(); l.ends_with(".md") || l.ends_with(".markdown") || l.ends_with(".mdx") }

#[cfg(test)]
mod tests {
    use super::*;
    fn plain(s: &[Span]) -> String { s.iter().map(|x| x.text.as_str()).collect() }
    #[test] fn blocks() {
        let b = parse("# Title\n\nHello **bold** and `code` and [link](http://x).\n\n- one\n- two\n\n1. a\n2. b\n\n> quote\n\n```rust\nfn x() {}\n```\n\n---\n");
        assert!(matches!(&b[0], Block::Heading(1, s) if plain(s) == "Title"));
        let Block::Para(p) = &b[1] else { panic!() };
        assert!(p.iter().any(|s| s.bold && s.text == "bold")); assert!(p.iter().any(|s| s.code && s.text == "code")); assert!(p.iter().any(|s| s.link.as_deref() == Some("http://x")));
        let Block::List { ordered: false, items, .. } = &b[2] else { panic!("{:?}", b[2]) }; assert_eq!(items.len(), 2);
        assert!(matches!(&b[3], Block::List { ordered: true, items, .. } if items.len() == 2));
        assert!(matches!(&b[4], Block::Quote(q) if matches!(&q[0], Block::Para(s) if plain(s) == "quote")));
        assert!(matches!(&b[5], Block::Code { lang, text } if lang == "rust" && text == "fn x() {}")); assert_eq!(b[6], Block::Rule);
    }
    #[test] fn tables_tasks_nested() {
        let b = parse("| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n- [x] done\n- [ ] todo\n  - nested\n");
        let Block::Table { head, rows } = &b[0] else { panic!() }; assert_eq!(head.len(), 2); assert_eq!(rows.len(), 2); assert_eq!(plain(&rows[1][0]), "3");
        let Block::List { items, .. } = &b[1] else { panic!() }; assert!(matches!(&items[0][0], Block::Para(s) if plain(s).starts_with('☑')));
        assert!(items[1].iter().any(|x| matches!(x, Block::List { .. })));
    }
    #[test] fn located_blocks_know_their_source_lines() {
        let src = "# Title\n\nfirst para\nsecond line\n\n- a\n- b\n\n```rust\nlet x = 1;\n```\n\n---\n\n> quote\n";
        let l = parse_located(src);
        let got: Vec<(u32, u32)> = l.iter().map(|x| (x.first_line, x.last_line)).collect();
        // the two bullets are separate, individually commentable blocks
        assert_eq!(got, vec![(1, 1), (3, 4), (6, 6), (7, 7), (9, 11), (13, 13), (15, 15)]);
        assert!(matches!(l[1].block, Block::Para(_)) && matches!(l[6].block, Block::Quote(_)));
    }
    #[test] fn robust_on_garbage() { for s in ["", "***", "```", "| x", "> > >", "[a](", "# \n#"] { let _ = parse(s); } assert!(is_markdown("a/README.MD") && !is_markdown("a.rs")); }
}
