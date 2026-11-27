//! Markdown to a small block model that the UI renders. UI-free.
use perusal_diff::text::{Class, Kind};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::ops::Range;

/// How a piece of rendered Markdown differs from the other side of a change (rich diff).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Change { #[default] None, Added, Removed, Modified }

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Span { pub text: String, pub bold: bool, pub italic: bool, pub strike: bool, pub code: bool, pub link: Option<String>, pub change: Change }

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Span>), Para(Vec<Span>), Code { lang: String, text: String },
    List { ordered: bool, start: u64, items: Vec<Vec<Block>> }, Quote(Vec<Block>), Rule,
    /// A table, or when commenting is per row (`parse_located`) one piece of it: the header alone, or a single body row with an empty `head`.
    Table { head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
    /// One line of a code block, highlighted once at parse time; `first`/`last` round the block's corners.
    CodeLine { text: String, runs: Vec<(Range<usize>, Class)>, first: bool, last: bool, change: Change },
}

/// The text runs of rendered blocks, in the order they are drawn: one per heading/paragraph/table cell (its spans joined) and one per
/// line of a code block. Suggestion blocks are drawn as a diff and have none. Ctrl+F counts and highlights matches per run.
pub fn block_texts(blocks: &[Block]) -> Vec<String> {
    let join = |sp: &[Span]| sp.iter().map(|s| s.text.as_str()).collect::<String>();
    let mut out = vec![];
    for b in blocks {
        match b {
            Block::Heading(_, sp) | Block::Para(sp) => out.push(join(sp)),
            Block::Code { lang, text } => if !lang.trim().starts_with("suggestion") { out.extend(text.split('\n').map(String::from)); },
            Block::List { items, .. } => for it in items { out.extend(block_texts(it)); },
            Block::Quote(inner) => out.extend(block_texts(inner)),
            Block::Rule => {}
            Block::CodeLine { text, .. } => out.push(text.clone()),
            Block::Table { head, rows } => { for c in head { out.push(join(c)); } for r in rows { for c in r { out.push(join(c)); } } }
        }
    }
    out
}

pub fn parse(src: &str) -> Vec<Block> {
    let mut stack: Vec<Vec<Block>> = vec![vec![]];
    let mut spans: Vec<Span> = vec![];
    let (mut bold, mut italic, mut strike) = (0u32, 0u32, 0u32); let mut link: Option<String> = None;
    let mut code: Option<(String, String)> = None;
    let mut lists: Vec<(bool, u64, Vec<Vec<Block>>)> = vec![];
    let mut head: Vec<Vec<Span>> = vec![]; let mut rows: Vec<Vec<Vec<Span>>> = vec![]; let mut cur_row: Vec<Vec<Span>> = vec![]; let mut in_head = false;
    let push_text = |spans: &mut Vec<Span>, t: &str, code: bool, bold: u32, italic: u32, strike: u32, link: &Option<String>| {
        spans.push(Span { text: t.to_string(), bold: bold > 0, italic: italic > 0, strike: strike > 0, code, link: link.clone(), change: Change::None });
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
pub struct Located { pub block: Block, pub first_line: u32, pub last_line: u32, pub change: Change }

fn line_starts(src: &str) -> Vec<usize> { let mut v = vec![0usize]; for (i, b) in src.bytes().enumerate() { if b == b'\n' { v.push(i + 1); } } v }

/// Highlight classes per line of `text` (highlighted as a whole so multi-line comments and strings work): byte ranges within each line.
pub fn line_classes(lang: &str, text: &str) -> Vec<Vec<(Range<usize>, Class)>> {
    let starts = line_starts(text); let n = starts.len();
    let mut out: Vec<Vec<(Range<usize>, Class)>> = vec![vec![]; n];
    if lang.is_empty() { return out; }
    let mut spans = perusal_diff::text::highlight(lang, text); spans.sort_by_key(|s| s.0);
    let mut last_end = 0usize;
    for (a, b, c) in spans {
        if a < last_end { continue; } last_end = b;
        let mut li = starts.partition_point(|s| *s <= a).saturating_sub(1);
        while li < n && starts[li] < b {
            let ls = starts[li]; let le = if li + 1 < n { starts[li + 1] - 1 } else { text.len() };
            let (sa, sb) = (a.max(ls), b.min(le));
            if sa < sb && text.is_char_boundary(sa) && text.is_char_boundary(sb) { out[li].push((sa - ls..sb - ls, c)); }
            li += 1;
        }
    }
    out
}

/// Language for a Markdown fence label (```rust, ```js, ```sh ...); empty for none, "generic" for labels we do not know.
pub fn fence_language(label: &str) -> &'static str {
    let l = label.split(|c: char| c.is_whitespace() || c == ',' || c == '{').next().unwrap_or("").to_lowercase();
    if l.is_empty() || l == "text" || l == "txt" || l == "plain" || l == "diff" { return ""; }
    let ext = match l.as_str() { "rust" => "rs", "python" | "python3" => "py", "javascript" | "node" => "js", "typescript" => "ts", "shell" | "console" | "sh" | "zsh" | "bash" => "sh", "golang" => "go", "c++" | "cplusplus" => "cpp", "markup" | "xml" | "svg" => "html", "yml" | "toml" | "yaml" => "generic", o => o };
    match perusal_diff::text::language_for_ext(ext) { "" => "generic", o => o }
}

fn is_block_tag(t: &Tag) -> bool { matches!(t, Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote(_) | Tag::CodeBlock(_) | Tag::List(_) | Tag::Table(_) | Tag::HtmlBlock) }
fn is_block_end(t: &TagEnd) -> bool { matches!(t, TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::BlockQuote(_) | TagEnd::CodeBlock | TagEnd::List(_) | TagEnd::Table | TagEnd::HtmlBlock) }

pub fn parse_located(src: &str) -> Vec<Located> {
    let blocks = parse(src);
    let starts = line_starts(src);
    let line_of = |off: usize| starts.partition_point(|s| *s <= off.min(src.len())) as u32;
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
                for (k, it) in its.into_iter().enumerate() { out.push(Located { block: Block::List { ordered, start: start + k as u64, items: vec![it] }, first_line: rs[k].0, last_line: rs[k].1.max(rs[k].0), change: Change::None }); }
            }
            // a table is commented on row by row: the header (with its delimiter line), then one piece per body row
            Block::Table { head, rows } if a > 0 => {
                out.push(Located { block: Block::Table { head, rows: vec![] }, first_line: a, last_line: (a + 1).min(b.max(a)), change: Change::None });
                let n = rows.len();
                for (k, r) in rows.into_iter().enumerate() { let l = (a + 2 + k as u32).min(b.max(a)); out.push(Located { block: Block::Table { head: vec![], rows: vec![r] }, first_line: l, last_line: if k + 1 == n { b.max(l) } else { l }, change: Change::None }); }
            }
            // and a code block line by line (suggestions stay whole: they are drawn as a diff); the fences belong to the first and last line
            Block::Code { lang, text } if a > 0 && !text.is_empty() && !lang.trim().starts_with("suggestion") => {
                let fenced = src[starts[a as usize - 1]..].lines().next().map(|l| { let t = l.trim_start(); t.starts_with("```") || t.starts_with("~~~") }).unwrap_or(false);
                let classes = line_classes(fence_language(&lang), &text);
                let lines: Vec<&str> = text.split('\n').collect(); let n = lines.len();
                for (k, l) in lines.into_iter().enumerate() {
                    let ln = (if fenced { a + 1 + k as u32 } else { a + k as u32 }).min(b.max(a));
                    let (first, last) = (k == 0, k + 1 == n);
                    out.push(Located { block: Block::CodeLine { text: l.to_string(), runs: classes.get(k).cloned().unwrap_or_default(), first, last, change: Change::None }, first_line: if first { a } else { ln }, last_line: if last { b.max(ln) } else { ln }, change: Change::None });
                }
            }
            block => out.push(Located { block, first_line: a, last_line: b.max(a), change: Change::None }),
        }
    }
    out
}

// ---- rich diff: rendered Markdown with additions, deletions and word-level changes marked

fn inline_of(b: &Block) -> Option<&Vec<Span>> {
    match b { Block::Heading(_, s) | Block::Para(s) => Some(s), Block::List { items, .. } if items.len() == 1 => items[0].first().and_then(inline_of), Block::Quote(i) => i.first().and_then(inline_of), _ => None }
}
fn inline_of_mut(b: &mut Block) -> Option<&mut Vec<Span>> {
    match b { Block::Heading(_, s) | Block::Para(s) => Some(s), Block::List { items, .. } if items.len() == 1 => items[0].first_mut().and_then(inline_of_mut), Block::Quote(i) => i.first_mut().and_then(inline_of_mut), _ => None }
}

/// Marks every piece of text in a block as added or removed.
pub fn mark_block(b: &mut Block, c: Change) {
    match b {
        Block::Heading(_, s) | Block::Para(s) => for x in s { x.change = c; },
        Block::CodeLine { change, .. } => *change = c,
        Block::List { items, .. } => for it in items { for x in it { mark_block(x, c); } },
        Block::Quote(i) => for x in i { mark_block(x, c); },
        Block::Table { head, rows } => for cell in head.iter_mut().chain(rows.iter_mut().flatten()) { for x in cell { x.change = c; } },
        Block::Code { .. } | Block::Rule => {}
    }
}

/// The new spans with the word-level changes against `old` marked: added words flagged, removed words inserted as removed spans.
/// None when the two share too little to be read as the same text edited (fraction of unchanged text below `min_sim`).
fn word_spans(old: &[Span], new: &[Span], min_sim: f32) -> Option<Vec<Span>> {
    let join = |s: &[Span]| s.iter().map(|x| x.text.as_str()).collect::<String>();
    let (o, n) = (join(old), join(new));
    let ch = perusal_diff::text::word_changes(&o, &n);
    let eq: usize = ch.iter().filter(|c| c.0 == Kind::Context).map(|c| c.1.len()).sum();
    if (eq as f32) / (o.len().max(n.len()).max(1) as f32) < min_sim { return None; }
    let mut flags = vec![false; n.len()]; let mut dels: Vec<(usize, String)> = vec![]; let mut pos = 0usize;
    for (k, t) in ch { match k { Kind::Context => pos += t.len(), Kind::Added => { for f in &mut flags[pos..pos + t.len()] { *f = true; } pos += t.len(); } Kind::Removed => dels.push((pos, t)) } }
    let del_span = |t: &str| Span { text: t.to_string(), change: Change::Removed, ..Default::default() };
    let (mut out, mut off, mut di) = (vec![], 0usize, 0usize);
    for s in new {
        let (start, end) = (off, off + s.text.len()); off = end;
        let mut a = start;
        loop {
            while di < dels.len() && dels[di].0 <= a { if !dels[di].1.trim().is_empty() { out.push(del_span(&dels[di].1)); } di += 1; }
            if a >= end { break; }
            let mut b = a + 1;
            while b < end && flags[b] == flags[a] && dels.get(di).map(|d| d.0) != Some(b) { b += 1; }
            out.push(Span { text: s.text[a - start..b - start].to_string(), change: if flags[a] { Change::Added } else { Change::None }, ..s.clone() });
            a = b;
        }
    }
    for (_, t) in &dels[di..] { if !t.trim().is_empty() { out.push(del_span(t)); } }
    Some(out)
}

/// Marks `new` against the block `old` it replaces, when both are the same kind of text block. True when it did.
fn try_pair(new: &mut Block, old: &Block) -> bool {
    if let (Some(ns), Some(os)) = (inline_of(new), inline_of(old)) {
        if std::mem::discriminant(new) != std::mem::discriminant(old) { return false; }
        let Some(r) = word_spans(os, ns, 0.35) else { return false };
        *inline_of_mut(new).unwrap() = r; return true;
    }
    // a table row piece: cell by cell
    if let (Block::Table { head: h1, rows: r1 }, Block::Table { head: h2, rows: r2 }) = (&mut *new, old) {
        if h1.is_empty() && h2.is_empty() && r1.len() == 1 && r2.len() == 1 && r1[0].len() == r2[0].len() {
            for (nc, oc) in r1[0].iter_mut().zip(&r2[0]) { if let Some(sp) = word_spans(oc, nc, 0.0) { *nc = sp; } }
            return true;
        }
    }
    false
}

/// The rendered blocks of `new` with its changes against `old` marked: added blocks, edited blocks (changed words flagged),
/// and blocks only the old text had, inserted where they were as removed blocks. Blank lines do not count as changes.
pub fn parse_diff(old: &str, new: &str) -> Vec<Located> {
    let mut nb = parse_located(new);
    if old == new { return nb; }
    let ob = parse_located(old);
    // diff the non-blank lines only, so blank lines cannot pull the alignment onto the wrong paragraph
    let compact = |s: &str| -> (String, Vec<u32>) {
        let (mut t, mut map) = (String::new(), vec![]);
        for (i, l) in s.lines().enumerate() { if !l.trim().is_empty() { t.push_str(l); t.push('\n'); map.push(i as u32 + 1); } }
        (t, map)
    };
    let ((oc, omap), (nc, nmap)) = (compact(old), compact(new));
    let lines = perusal_diff::text::diff_lines(&oc, &nc, usize::MAX / 2);
    let (nn, on) = (new.lines().count() + 2, old.lines().count() + 2);
    let (mut added, mut removed, mut nblank, mut oblank) = (vec![false; nn], vec![false; on], vec![true; nn], vec![true; on]);
    for &n in &nmap { nblank[n as usize] = false; } for &o in &omap { oblank[o as usize] = false; }
    let (mut nreg, mut oreg) = (vec![0u32; nn], vec![0u32; on]);
    let mut anchor = vec![u32::MAX; on]; let mut pending: Vec<usize> = vec![];
    let (mut region, mut in_change) = (0u32, false);
    for l in &lines {
        let (o, n) = (l.old_no.map(|o| omap[o as usize - 1] as usize), l.new_no.map(|n| nmap[n as usize - 1]));
        match l.kind {
            Kind::Context => in_change = false,
            Kind::Added => {
                if !in_change { region += 1; in_change = true; }
                if let Some(n) = n { added[n as usize] = true; nreg[n as usize] = region; }
            }
            Kind::Removed => {
                if !in_change { region += 1; in_change = true; }
                if let Some(o) = o { removed[o] = true; oreg[o] = region; pending.push(o); }
            }
        }
        if let (Some(n), false) = (n, pending.is_empty()) { for o in pending.drain(..) { anchor[o] = n; } }
    }
    // a range counts as wholly changed when it has text and every non-blank line in it changed
    let whole = |flags: &[bool], blank: &[bool], a: u32, b: u32| { let mut any = false; for i in a as usize..=(b as usize).min(flags.len() - 1) { if !blank[i] { if !flags[i] { return false; } any = true; } } any };
    // the old blocks that lost lines, by change region
    let mut by_region: std::collections::HashMap<u32, Vec<usize>> = Default::default();
    for (j, b) in ob.iter().enumerate() {
        let mut seen: Vec<u32> = vec![];
        for o in b.first_line..=b.last_line { if (o as usize) < on && removed[o as usize] && !seen.contains(&oreg[o as usize]) { seen.push(oreg[o as usize]); by_region.entry(oreg[o as usize]).or_default().push(j); } }
    }
    let mut consumed = vec![false; ob.len()]; let mut paired_at = vec![usize::MAX; ob.len()];
    for (i, lb) in nb.iter_mut().enumerate() {
        if lb.first_line == 0 { continue; }
        let range = lb.first_line as usize..=(lb.last_line as usize).min(nn - 1);
        let Some(first_added) = range.clone().find(|&n| added[n]) else { continue };
        let paired = by_region.get(&nreg[first_added]).and_then(|js| js.iter().copied().find(|&j| !consumed[j] && try_pair(&mut lb.block, &ob[j].block)));
        if let Some(j) = paired { consumed[j] = true; paired_at[j] = i; lb.change = Change::Modified; continue; }
        if whole(&added, &nblank, lb.first_line, lb.last_line) { mark_block(&mut lb.block, Change::Added); lb.change = Change::Added; } else { lb.change = Change::Modified; }
    }
    // an empty line inside a code block that is wholly added (or removed) is part of it
    let blank_code = |b: &Block| matches!(b, Block::CodeLine { text, .. } if text.is_empty());
    for i in 1..nb.len().saturating_sub(1) {
        if nb[i].change == Change::None && blank_code(&nb[i].block) && nb[i - 1].change == Change::Added && nb[i + 1].change == Change::Added { mark_block(&mut nb[i].block, Change::Added); nb[i].change = Change::Added; }
    }
    // blocks that only the old text had
    let gone_ok: Vec<bool> = ob.iter().enumerate().map(|(j, b)| !consumed[j] && b.first_line != 0 && whole(&removed, &oblank, b.first_line, b.last_line)).collect();
    let mut gone: Vec<(usize, Located)> = vec![];
    let mut after: Option<usize> = None; // removed blocks that follow an edited block stay right after it
    for (j, b) in ob.iter().enumerate() {
        let inner = j > 0 && j + 1 < ob.len() && blank_code(&b.block) && !consumed[j] && gone_ok[j - 1] && gone_ok[j + 1];
        if consumed[j] { after = Some(paired_at[j] + 1); continue; }
        if !gone_ok[j] && !inner { after = None; continue; }
        let mut lb = b.clone(); mark_block(&mut lb.block, Change::Removed); lb.change = Change::Removed;
        let at = anchor[b.first_line as usize];
        let pos = after.unwrap_or_else(|| nb.partition_point(|x| x.last_line < at)); after = Some(pos);
        gone.push((pos, lb));
    }
    gone.sort_by_key(|g| g.0); // stable: removed blocks keep their order
    let mut out = Vec::with_capacity(nb.len() + gone.len()); let mut gi = 0;
    for (i, lb) in nb.into_iter().enumerate() {
        while gi < gone.len() && gone[gi].0 <= i { out.push(gone[gi].1.clone()); gi += 1; }
        out.push(lb);
    }
    for (_, lb) in &gone[gi..] { out.push(lb.clone()); }
    out
}

pub fn is_markdown(path: &str) -> bool { let l = path.to_lowercase(); l.ends_with(".md") || l.ends_with(".markdown") || l.ends_with(".mdx") }

#[cfg(test)]
mod tests {
    #[test] fn block_texts_follow_drawing_order() {
        let t = block_texts(&parse("# Title\n\nSome **bold** text\n\n- one\n- two\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n\n| h1 | h2 |\n|---|---|\n| c1 | c2 |\n"));
        assert_eq!(t, vec!["Title", "Some bold text", "one", "two", "let a = 1;", "let b = 2;", "h1", "h2", "c1", "c2"]);
    }
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
    #[test] fn tables_and_code_are_commentable_per_line() {
        let src = "intro\n\n| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n";
        let got: Vec<(u32, u32)> = parse_located(src).iter().map(|x| (x.first_line, x.last_line)).collect();
        // intro, table header + delimiter, two rows, then the code lines (fences belong to the first and last)
        assert_eq!(got, vec![(1, 1), (3, 4), (5, 5), (6, 6), (8, 9), (10, 11)]);
        let l = parse_located(src);
        assert!(matches!(&l[1].block, Block::Table { head, rows } if head.len() == 2 && rows.is_empty()));
        assert!(matches!(&l[2].block, Block::Table { head, rows } if head.is_empty() && rows.len() == 1 && plain(&rows[0][1]) == "2"));
        assert!(matches!(&l[4].block, Block::CodeLine { text, first: true, last: false, .. } if text == "let a = 1;"));
        assert!(matches!(&l[5].block, Block::CodeLine { last: true, .. }));
    }
    #[test] fn rich_diff_marks_added_removed_and_edited_words() {
        let old = "# T\n\nThe quick brown fox.\n\ngone para\n\nkeep\n";
        let new = "# T\n\nThe quick red fox.\n\nkeep\n\nbrand new\n";
        let d = parse_diff(old, new);
        let kinds: Vec<Change> = d.iter().map(|x| x.change).collect();
        assert_eq!(kinds, vec![Change::None, Change::Modified, Change::Removed, Change::None, Change::Added]);
        let Block::Para(sp) = &d[1].block else { panic!() };
        assert!(sp.iter().any(|s| s.change == Change::Added && s.text.trim() == "red") && sp.iter().any(|s| s.change == Change::Removed && s.text.trim() == "brown"));
        assert!(matches!(&d[2].block, Block::Para(s) if s.iter().all(|x| x.change == Change::Removed) && plain(s) == "gone para"));
        assert!(matches!(&d[4].block, Block::Para(s) if s.iter().all(|x| x.change == Change::Added)));
        assert_eq!(parse_diff("a\n", "a\n").iter().filter(|x| x.change != Change::None).count(), 0);
    }
    #[test] fn rich_diff_table_rows_and_unrelated_rewrites() {
        let d = parse_diff("| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n", "| a | b |\n|---|---|\n| 1 | 9 |\n| 3 | 4 |\n| 5 | 6 |\n");
        let kinds: Vec<Change> = d.iter().map(|x| x.change).collect();
        assert_eq!(kinds, vec![Change::None, Change::Modified, Change::None, Change::Added]);
        // a paragraph replaced by something unrelated reads as removed + added, not as a word soup
        let d = parse_diff("alpha beta gamma\n", "totally different words here now\n");
        assert_eq!(d.iter().map(|x| x.change).collect::<Vec<_>>(), vec![Change::Removed, Change::Added]);
    }
    #[test] fn robust_on_garbage() { for s in ["", "***", "```", "| x", "> > >", "[a](", "# \n#"] { let _ = parse(s); } assert!(is_markdown("a/README.MD") && !is_markdown("a.rs")); }
}
