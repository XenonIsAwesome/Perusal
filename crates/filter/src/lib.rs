//! Wireshark-style expression language for filtering and sorting the file list.
use perusal_diff::FileStat;
use regex::Regex;
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub enum Val { Num(f64), Str(String), Bool(bool) }

impl Val {
    fn num(&self) -> f64 { match self { Val::Num(n) => *n, Val::Bool(b) => *b as u8 as f64, Val::Str(s) => s.parse().unwrap_or(0.0) } }
    fn truthy(&self) -> bool { match self { Val::Bool(b) => *b, Val::Num(n) => *n != 0.0, Val::Str(s) => !s.is_empty() } }
    fn text(&self) -> String { match self { Val::Str(s) => s.clone(), Val::Num(n) => n.to_string(), Val::Bool(b) => b.to_string() } }
}

#[derive(Debug, Clone)]
enum Tok { Num(f64), Str(String), Re(String), Id(String), Op(&'static str), LP, RP, Comma }

const OPS: [&str; 17] = ["&&", "||", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "!", "~", "=", "?"];

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = src.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() { i += 1; continue; }
        if ch.is_ascii_digit() {
            let s = i; while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') { i += 1; }
            out.push(Tok::Num(c[s..i].iter().collect::<String>().parse().map_err(|_| format!("bad number at {s}"))?));
        } else if ch.is_alphabetic() || ch == '_' {
            let s = i; while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') { i += 1; }
            out.push(Tok::Id(c[s..i].iter().collect()));
        } else if ch == '"' {
            i += 1; let s = i; while i < c.len() && c[i] != '"' { i += 1; }
            if i >= c.len() { return Err(format!("unterminated string at {}", s - 1)); }
            out.push(Tok::Str(c[s..i].iter().collect())); i += 1;
        } else if ch == '/' && matches!(out.last(), Some(Tok::Op("~")) ) {
            i += 1; let s = i; while i < c.len() && c[i] != '/' { if c[i] == '\\' { i += 1; } i += 1; }
            if i >= c.len() { return Err(format!("unterminated regex at {}", s - 1)); }
            out.push(Tok::Re(c[s..i].iter().collect())); i += 1;
        } else if ch == '(' { out.push(Tok::LP); i += 1; }
        else if ch == ')' { out.push(Tok::RP); i += 1; }
        else if ch == ',' { out.push(Tok::Comma); i += 1; }
        else {
            let rest: String = c[i..].iter().take(2).collect();
            let op = OPS.iter().find(|o| rest.starts_with(**o)).ok_or_else(|| format!("unexpected '{ch}' at {i}"))?;
            out.push(Tok::Op(if *op == "=" { "==" } else { op })); i += op.len();
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum Expr {
    Num(f64), Str(String), Re(Regex), Field(&'static str),
    Un(&'static str, Box<Expr>), Bin(&'static str, Box<Expr>, Box<Expr>),
    /// aggregate id, function name, args
    Agg(usize, String, Vec<Expr>),
}

pub const FIELDS: [&str; 24] = ["name", "path", "extension", "ext", "language", "status", "binary", "generated", "additions", "deletions",
    "net", "ratio", "hunks", "size", "viewed", "comments", "unresolved", "ai", "noisy", "added", "removed", "changed", "whitespace", "meaningful"];
pub const FUNCS: [&str; 7] = ["avg", "sum", "max", "min", "count", "top", "bottom"];

struct Parser { t: Vec<Tok>, p: usize, aggs: usize, depth: usize }
const MAX_DEPTH: usize = 64;

impl Parser {
    fn peek_op(&self) -> Option<&'static str> { if let Some(Tok::Op(o)) = self.t.get(self.p) { Some(o) } else { None } }
    fn bin(&mut self, level: usize) -> Result<Expr, String> {
        if level == 0 { self.depth += 1; if self.depth > MAX_DEPTH { return Err("expression nested too deeply".into()); } }
        let r = self.bin_inner(level);
        if level == 0 { self.depth -= 1; }
        r
    }
    fn bin_inner(&mut self, level: usize) -> Result<Expr, String> {
        const LV: [&[&str]; 5] = [&["||"], &["&&"], &["==", "!=", "<", ">", "<=", ">=", "~"], &["+", "-"], &["*", "/", "%"]];
        if level == LV.len() { return self.unary(); }
        let mut l = self.bin(level + 1)?;
        while let Some(o) = self.peek_op() {
            if !LV[level].contains(&o) { break; }
            self.p += 1;
            let r = self.bin(level + 1)?;
            l = Expr::Bin(o, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek_op() {
            Some(o @ ("!" | "-")) => {
                self.p += 1; self.depth += 1; if self.depth > MAX_DEPTH { return Err("expression nested too deeply".into()); }
                let r = self.unary(); self.depth -= 1; Ok(Expr::Un(o, Box::new(r?)))
            }
            _ => self.atom(),
        }
    }
    fn atom(&mut self) -> Result<Expr, String> {
        let tok = self.t.get(self.p).cloned().ok_or("unexpected end of expression")?;
        self.p += 1;
        match tok {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Re(r) => Regex::new(&r).map(Expr::Re).map_err(|e| format!("invalid regex: {e}")),
            Tok::LP => { let e = self.bin(0)?; self.expect_rp()?; Ok(e) }
            Tok::Id(id) => {
                if matches!(self.t.get(self.p), Some(Tok::LP)) {
                    if !FUNCS.contains(&id.as_str()) { return Err(format!("unknown function '{id}' (try: {})", FUNCS.join(", "))); }
                    self.p += 1;
                    let mut args = Vec::new();
                    if !matches!(self.t.get(self.p), Some(Tok::RP)) {
                        loop { args.push(self.bin(0)?); if matches!(self.t.get(self.p), Some(Tok::Comma)) { self.p += 1 } else { break } }
                    }
                    self.expect_rp()?;
                    let need = if id == "top" || id == "bottom" { 2 } else { 1 };
                    if args.len() != need { return Err(format!("{id}() takes {need} argument(s)")); }
                    self.aggs += 1;
                    Ok(Expr::Agg(self.aggs - 1, id, args))
                } else {
                    match id.as_str() {
                        "true" => Ok(Expr::Num(1.0)), "false" => Ok(Expr::Num(0.0)),
                        _ => FIELDS.iter().find(|f| **f == id).map(|f| Expr::Field(f))
                            .ok_or_else(|| format!("unknown field '{id}'")),
                    }
                }
            }
            Tok::RP => Err("unexpected ')'".into()),
            Tok::Comma => Err("unexpected ','".into()),
            Tok::Op(o) => Err(format!("unexpected operator '{o}'")),
        }
    }
    fn expect_rp(&mut self) -> Result<(), String> {
        if matches!(self.t.get(self.p), Some(Tok::RP)) { self.p += 1; Ok(()) } else { Err("expected ')'".into()) }
    }
}

fn language(ext: &str) -> &'static str {
    match ext { "rs" => "rust", "py" => "python", "ts" | "tsx" => "typescript", "js" | "jsx" => "javascript", "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp", "go" => "go", "java" => "java", "md" => "markdown", "toml" => "toml", "json" => "json", _ => "" }
}

fn field(name: &str, f: &FileStat) -> Val {
    match name {
        "name" => Val::Str(f.name().into()), "path" => Val::Str(f.path.clone()),
        "extension" | "ext" => Val::Str(f.ext().into()), "language" => Val::Str(language(f.ext()).into()),
        "status" => Val::Str(f.status.as_str().into()), "binary" => Val::Bool(f.binary), "generated" => Val::Bool(f.generated),
        "additions" | "added" => Val::Num(f.additions as f64), "deletions" | "removed" => Val::Num(f.deletions as f64),
        "net" => Val::Num(f.net() as f64), "ratio" => Val::Num(f.change_ratio()), "hunks" => Val::Num(f.hunks as f64),
        "size" => Val::Num(f.size as f64), "viewed" => Val::Bool(f.viewed), "comments" => Val::Num(f.comments as f64),
        "unresolved" => Val::Num(f.unresolved as f64), "ai" => Val::Bool(f.ai_flagged), "noisy" => Val::Bool(f.noisy()),
        "changed" => Val::Num(f.changed() as f64),
        "whitespace" => Val::Bool(f.ws_only()), "meaningful" => Val::Num(f.meaningful as f64),
        _ => Val::Bool(false),
    }
}

/// A compiled expression. Aggregates are computed in a second pass over the index.
#[derive(Debug, Clone)]
pub struct Compiled { expr: Expr, n_aggs: usize }

pub fn compile(src: &str) -> Result<Compiled, String> {
    let t = lex(src)?;
    if t.is_empty() { return Err("empty expression".into()); }
    let mut p = Parser { t, p: 0, aggs: 0, depth: 0 };
    let expr = p.bin(0)?;
    if p.p < p.t.len() { return Err(format!("unexpected token at position {}", p.p)); }
    Ok(Compiled { expr, n_aggs: p.aggs })
}

enum AggVal { Scalar(f64), Threshold(f64, bool) }

fn eval(e: &Expr, f: &FileStat, aggs: &[AggVal]) -> Val {
    match e {
        Expr::Num(n) => Val::Num(*n), Expr::Str(s) => Val::Str(s.clone()), Expr::Re(r) => Val::Str(r.as_str().into()),
        Expr::Field(n) => field(n, f),
        Expr::Un("!", x) => Val::Bool(!eval(x, f, aggs).truthy()),
        Expr::Un(_, x) => Val::Num(-eval(x, f, aggs).num()),
        Expr::Bin(op, a, b) => {
            if *op == "&&" { return Val::Bool(eval(a, f, aggs).truthy() && eval(b, f, aggs).truthy()); }
            if *op == "||" { return Val::Bool(eval(a, f, aggs).truthy() || eval(b, f, aggs).truthy()); }
            if *op == "~" {
                let l = eval(a, f, aggs).text();
                return Val::Bool(match &**b { Expr::Re(r) => r.is_match(&l), o => l.contains(&eval(o, f, aggs).text()) });
            }
            let (l, r) = (eval(a, f, aggs), eval(b, f, aggs));
            let both_num = matches!((&l, &r), (Val::Num(_), Val::Num(_)) | (Val::Bool(_), Val::Num(_)) | (Val::Num(_), Val::Bool(_)));
            match *op {
                "+" => Val::Num(l.num() + r.num()), "-" => Val::Num(l.num() - r.num()), "*" => Val::Num(l.num() * r.num()),
                "/" => Val::Num(l.num() / r.num()), "%" => Val::Num(l.num() % r.num()),
                _ => {
                    let ord = if both_num { l.num().partial_cmp(&r.num()) } else { Some(l.text().cmp(&r.text())) };
                    let o = ord.unwrap_or(Ordering::Less);
                    Val::Bool(match *op { "==" => ord == Some(Ordering::Equal), "!=" => ord != Some(Ordering::Equal),
                        "<" => o == Ordering::Less, ">" => o == Ordering::Greater, "<=" => o != Ordering::Greater, _ => o != Ordering::Less })
                }
            }
        }
        Expr::Agg(id, name, args) => match &aggs[*id] {
            AggVal::Scalar(v) => Val::Num(*v),
            AggVal::Threshold(t, top) => { let _ = name; let v = eval(&args[1], f, aggs).num(); Val::Bool(if *top { v >= *t } else { v <= *t }) }
        },
    }
}

fn collect<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    match e { Expr::Un(_, x) => collect(x, out), Expr::Bin(_, a, b) => { collect(a, out); collect(b, out) }
        Expr::Agg(_, _, args) => { for a in args { collect(a, out); } out.push(e) } _ => {} }
}

impl Compiled {
    fn prepare(&self, files: &[FileStat]) -> Vec<AggVal> {
        let mut nodes = Vec::new();
        collect(&self.expr, &mut nodes);
        nodes.sort_by_key(|n| if let Expr::Agg(i, ..) = n { *i } else { 0 });
        let mut vals: Vec<AggVal> = Vec::new();
        // inner aggregates have larger ids than outer ones do not occur in practice; compute in id order with progressive fill
        for _ in 0..self.n_aggs { vals.push(AggVal::Scalar(0.0)); }
        for n in nodes {
            let Expr::Agg(id, name, args) = n else { continue };
            let col_expr = args.last().unwrap();
            let col: Vec<f64> = files.iter().map(|f| eval(col_expr, f, &vals).num()).collect();
            vals[*id] = match name.as_str() {
                "avg" => AggVal::Scalar(if col.is_empty() { 0.0 } else { col.iter().sum::<f64>() / col.len() as f64 }),
                "sum" => AggVal::Scalar(col.iter().sum()),
                "max" => AggVal::Scalar(col.iter().cloned().fold(f64::MIN, f64::max)),
                "min" => AggVal::Scalar(col.iter().cloned().fold(f64::MAX, f64::min)),
                "count" => AggVal::Scalar(files.iter().filter(|f| eval(col_expr, f, &vals).truthy()).count() as f64),
                _ => {
                    let k = eval(&args[0], &files[0], &vals).num().max(0.0) as usize;
                    let mut s = col.clone();
                    let top = name == "top";
                    s.sort_by(|a, b| if top { b.partial_cmp(a).unwrap() } else { a.partial_cmp(b).unwrap() });
                    AggVal::Threshold(if k == 0 || s.is_empty() { if top { f64::INFINITY } else { f64::NEG_INFINITY } } else { s[(k - 1).min(s.len() - 1)] }, top)
                }
            };
        }
        vals
    }
}

/// Returns indexes of files matching the filter, in original order.
pub fn filter(files: &[FileStat], src: &str) -> Result<Vec<usize>, String> {
    if src.trim().is_empty() { return Ok((0..files.len()).collect()); }
    let c = compile(src)?;
    let aggs = c.prepare(files);
    Ok(files.iter().enumerate().filter(|(_, f)| eval(&c.expr, f, &aggs).truthy()).map(|(i, _)| i).collect())
}

/// Multi-key sort: "additions + deletions desc, name asc". Stable. Sorts `idx` in place.
pub fn sort(files: &[FileStat], idx: &mut [usize], src: &str) -> Result<(), String> {
    if src.trim().is_empty() { return Ok(()); }
    let mut keys = Vec::new();
    for part in split_top(src) {
        let p = part.trim();
        let (e, desc) = if let Some(x) = p.strip_suffix(" desc") { (x, true) } else if let Some(x) = p.strip_suffix(" asc") { (x, false) } else { (p, false) };
        let c = compile(e)?; let a = c.prepare(files); keys.push((c, a, desc));
    }
    idx.sort_by(|&a, &b| {
        for (c, aggs, desc) in &keys {
            let (x, y) = (eval(&c.expr, &files[a], aggs), eval(&c.expr, &files[b], aggs));
            let o = match (&x, &y) { (Val::Str(p), Val::Str(q)) => p.cmp(q), _ => x.num().partial_cmp(&y.num()).unwrap_or(Ordering::Equal) };
            let o = if *desc { o.reverse() } else { o };
            if o != Ordering::Equal { return o; }
        }
        Ordering::Equal
    });
    Ok(())
}

/// Splits a sort expression into tiers: ("additions + deletions", true) for "additions + deletions desc, name asc".
pub fn parse_tiers(src: &str) -> Vec<(String, bool)> {
    split_top(src).into_iter().filter_map(|p| {
        let p = p.trim();
        if p.is_empty() { return None; }
        Some(if let Some(x) = p.strip_suffix(" desc") { (x.trim().to_string(), true) } else if let Some(x) = p.strip_suffix(" asc") { (x.trim().to_string(), false) } else { (p.to_string(), false) })
    }).collect()
}
/// Inverse of [`parse_tiers`]; empty expressions are dropped.
pub fn format_tiers(t: &[(String, bool)]) -> String {
    t.iter().filter(|(e, _)| !e.trim().is_empty()).map(|(e, d)| format!("{} {}", e.trim(), if *d { "desc" } else { "asc" })).collect::<Vec<_>>().join(", ")
}

fn split_top(s: &str) -> Vec<String> {
    let (mut d, mut cur, mut out) = (0, String::new(), Vec::new());
    for ch in s.chars() {
        match ch { '(' => d += 1, ')' => d -= 1, ',' if d == 0 => { out.push(std::mem::take(&mut cur)); continue } _ => {} }
        cur.push(ch);
    }
    out.push(cur); out
}

/// Autocomplete candidates for the token being typed at the end of `prefix`.
pub fn complete(prefix: &str) -> Vec<&'static str> {
    let word: String = prefix.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
    if word.is_empty() { return vec![]; }
    FIELDS.iter().chain(FUNCS.iter()).copied().filter(|c| c.starts_with(&word) && *c != word).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion { pub label: String, pub detail: &'static str, /// text that replaces the word being typed
    pub insert: String, pub kind: &'static str }

fn field_doc(f: &str) -> &'static str {
    match f {
        "name" => "file name", "path" => "full path", "extension" | "ext" => "extension without the dot", "language" => "detected language",
        "status" => "added, modified, deleted, renamed", "binary" => "binary file", "generated" => "generated or vendored file", "noisy" => "collapsed by default (lockfiles, generated, huge)",
        "additions" | "added" => "lines added", "deletions" | "removed" => "lines removed", "net" => "additions minus deletions", "changed" => "additions plus deletions",
        "ratio" => "balance of adds and removes, 0 to 0.5", "hunks" => "number of hunks", "size" => "file size in bytes",
        "viewed" => "marked as viewed", "comments" => "number of comments", "unresolved" => "unresolved threads", "ai" => "flagged by AI",
        "whitespace" => "only whitespace changed", "meaningful" => "lines changed ignoring whitespace",
        _ => "",
    }
}
fn func_doc(f: &str) -> &'static str {
    match f { "avg" => "average over all files", "sum" => "total over all files", "max" => "largest value", "min" => "smallest value", "count" => "how many files match", "top" => "top(n, expr): the n largest", "bottom" => "bottom(n, expr): the n smallest", _ => "" }
}

/// Suggestions for the text before the caret, plus how many chars before the caret the chosen suggestion replaces.
pub fn suggest(before: &str) -> (Vec<Suggestion>, usize) {
    let word: String = before.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
    let wl = word.chars().count();
    let lower = word.to_lowercase();
    // value suggestions after `field ==`
    let head = &before[..before.len() - word.len()];
    let h = head.trim_end().trim_end_matches('"').trim_end();
    for (field, values) in [("status", &["added", "modified", "deleted", "renamed"][..]), ("language", &["rust", "python", "typescript", "javascript", "c", "cpp", "go", "java", "markdown", "toml", "json"][..])] {
        for op in ["==", "!=", "="] {
            if h.ends_with(&format!("{field} {op}")) || h.ends_with(&format!("{field}{op}")) {
                let quoted = head.trim_end().ends_with('"');
                let out: Vec<Suggestion> = values.iter().filter(|v| v.starts_with(&lower)).map(|v| Suggestion { label: v.to_string(), detail: "value", insert: if quoted { v.to_string() } else { format!("\"{v}\"") }, kind: "value" }).collect();
                return (out, wl);
            }
        }
    }
    if word.is_empty() {
        // at the start of an expression, offer the common fields
        if before.trim().is_empty() { return (["additions", "name", "path", "status", "viewed", "noisy", "deletions", "language"].iter().map(|f| Suggestion { label: f.to_string(), detail: field_doc(f), insert: f.to_string(), kind: "field" }).collect(), 0); }
        return (vec![], 0);
    }
    if word.chars().all(|c| c.is_ascii_digit()) { return (vec![], wl); }
    let mut out: Vec<Suggestion> = vec![];
    for f in FIELDS.iter().filter(|f| f.starts_with(&lower) && **f != lower) { out.push(Suggestion { label: f.to_string(), detail: field_doc(f), insert: f.to_string(), kind: "field" }); }
    for f in FUNCS.iter().filter(|f| f.starts_with(&lower) && **f != lower) { out.push(Suggestion { label: format!("{f}()"), detail: func_doc(f), insert: format!("{f}("), kind: "function" }); }
    if out.is_empty() { for f in FIELDS.iter().filter(|f| f.contains(&lower) && **f != lower) { out.push(Suggestion { label: f.to_string(), detail: field_doc(f), insert: f.to_string(), kind: "field" }); } }
    out.truncate(8);
    (out, wl)
}

/// Suggestions for the sort field: expressions as in the filter, then `asc`/`desc` after a finished term.
pub fn suggest_sort(before: &str) -> (Vec<Suggestion>, usize) {
    let word: String = before.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
    let wl = word.chars().count();
    let dir = |l: &str, d: &'static str| Suggestion { label: l.into(), detail: d, insert: l.into(), kind: "order" };
    let trimmed = before.trim_end();
    let after_term = before.ends_with(' ') && !trimmed.is_empty() && !trimmed.ends_with([',', '+', '-', '*', '/', '%', '(', '&', '|', '<', '>', '=', '!', '~']);
    if after_term { return (vec![dir("desc", "largest first"), dir("asc", "smallest first")], 0); }
    let (mut s, n) = suggest(before);
    for (l, d) in [("asc", "smallest first"), ("desc", "largest first")] { if !word.is_empty() && l.starts_with(&word.to_lowercase()) && l != word { s.insert(0, dir(l, d)); } }
    if before.trim().is_empty() { s = vec![Suggestion { label: "additions + deletions".into(), detail: "total changed lines", insert: "additions + deletions desc".into(), kind: "example" }, Suggestion { label: "name".into(), detail: "alphabetical", insert: "name asc".into(), kind: "example" }, Suggestion { label: "path".into(), detail: "by path", insert: "path asc".into(), kind: "example" }]; return (s, 0); }
    let n = n.max(if s.first().map(|x| x.kind == "order").unwrap_or(false) { wl } else { 0 });
    (s, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn sort_suggestions() {
        let (s, _) = suggest_sort("additions + deletions "); assert_eq!(s[0].insert, "desc");
        let (s, n) = suggest_sort("name a"); assert!(s.iter().any(|x| x.insert == "asc") && n == 1);
        assert!(suggest_sort("").0.iter().any(|x| x.insert.contains("desc"))); assert!(suggest_sort("additions, ").0.is_empty() || suggest_sort("additions, ").0[0].kind != "order");
    }
    #[test] fn suggestions() {
        let (s, n) = suggest("name ~ /x/ && add"); assert_eq!(n, 3); assert!(s.iter().any(|x| x.insert == "additions" && x.kind == "field"));
        let (s, _) = suggest("av"); assert_eq!(s[0].insert, "avg(");
        let (s, n) = suggest("status == \"ad"); assert_eq!((s[0].insert.as_str(), n), ("added", 2));
        let (s, _) = suggest("status == "); assert!(s.iter().any(|x| x.insert == "\"deleted\""));
        assert!(!suggest("").0.is_empty()); assert!(suggest("additions > ").0.is_empty()); assert!(suggest("additions > 5").0.is_empty());
        assert!(suggest("zzz").0.is_empty()); let (s, _) = suggest("tions"); assert!(s.iter().any(|x| x.label == "additions"));
    }
    
    use perusal_diff::Status;
    fn files() -> Vec<FileStat> {
        let mut v = vec![
            FileStat::new("src/a.rs", Status::Modified, 100, 10), FileStat::new("tests/a_test.rs", Status::Added, 5, 0),
            FileStat::new("src/b.py", Status::Modified, 1, 1), FileStat::new("Cargo.lock", Status::Modified, 900, 800),
        ];
        v[1].viewed = true; v
    }
    #[test] fn basic() { assert_eq!(filter(&files(), "additions > 50").unwrap(), vec![0, 3]); }
    #[test] fn regex_and_not() { assert_eq!(filter(&files(), "name ~ /test/ && !viewed").unwrap(), Vec::<usize>::new());
        assert_eq!(filter(&files(), "path ~ /^src/ && !viewed").unwrap(), vec![0, 2]); }
    #[test] fn strings() { assert_eq!(filter(&files(), "status == \"added\"").unwrap(), vec![1]); assert_eq!(filter(&files(), "ext = \"py\"").unwrap(), vec![2]); }
    #[test] fn aggregates() {
        assert_eq!(filter(&files(), "additions > avg(additions)").unwrap(), vec![3]);
        assert_eq!(filter(&files(), "top(2, additions + deletions)").unwrap(), vec![0, 3]);
        assert_eq!(filter(&files(), "bottom(1, additions)").unwrap(), vec![2]);
    }
    #[test] fn noisy_target() { assert_eq!(filter(&files(), "noisy").unwrap(), vec![3]); }
    #[test] fn tiers_roundtrip() {
        let t = parse_tiers("top(2, additions) desc, name"); assert_eq!(t, vec![("top(2, additions)".to_string(), true), ("name".to_string(), false)]);
        assert_eq!(format_tiers(&t), "top(2, additions) desc, name asc"); assert!(parse_tiers("").is_empty());
    }
    #[test] fn whitespace_fields() { let mut f = files(); f[0].meaningful = 0; assert_eq!(filter(&f, "whitespace").unwrap(), vec![0]); assert_eq!(filter(&f, "meaningful == 0").unwrap(), vec![0]); }
    #[test] fn reference_lists_everything() { let r = reference(); assert!(FIELDS.iter().all(|f| r.contains(f)) && FUNCS.iter().all(|f| r.contains(f)) && r.contains("whitespace")); }
    #[test] fn sorting() {
        let f = files(); let mut i: Vec<usize> = (0..4).collect();
        sort(&f, &mut i, "additions + deletions desc, name asc").unwrap(); assert_eq!(i, vec![3, 0, 1, 2]);
    }
    #[test] fn errors() {
        assert!(filter(&files(), "bogus > 1").unwrap_err().contains("unknown field"));
        assert!(filter(&files(), "(additions").is_err()); assert!(filter(&files(), "name ~ /[/").is_err());
        assert!(filter(&files(), "foo(1)").unwrap_err().contains("unknown function"));
    }
    #[test] fn autocomplete() { assert!(complete("x && add").contains(&"additions")); }
    #[test] fn never_panics_on_garbage() {
        for s in ["", "&&", "))((", "\"", "/", "~", "1 +", "top(", "top(1)", "a b c", "@@", "avg()", "!!!!"] { let _ = filter(&files(), s); }
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use perusal_diff::Status;
    struct Rng(u64);
    impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str { v[(self.next() % v.len() as u64) as usize] } }
    const PIECES: [&str; 40] = ["additions", "name", "path", "viewed", "noisy", "top(", "avg(", "count(", ")", "(", ",", "&&", "||", "!", "~", "/x+/", "/", "\"", "\"a\"", "1", "2.5", ".", "+", "-", "*", "%", ">", "<=", "==", "=", " ", "\u{0}", "é", "😀", "\\", "[", "]", "{", "bottom(", "status"];
    #[test] fn random_inputs_never_panic_or_hang() {
        let files = vec![FileStat::new("a.rs", Status::Added, 3, 1), FileStat::new("b/c.py", Status::Modified, 0, 9)];
        let mut r = Rng(0x9E3779B97F4A7C15);
        for _ in 0..30_000 {
            let n = (r.next() % 12) as usize; let s: String = (0..n).map(|_| r.pick(&PIECES)).collect();
            let _ = filter(&files, &s); let mut idx = vec![0, 1]; let _ = sort(&files, &mut idx, &s); let _ = complete(&s);
        }
        // mutations of valid expressions
        let valid = ["additions > avg(additions) && name ~ /test/ && !viewed", "top(10, additions + deletions)", "status == \"added\" || noisy"];
        for v in valid { for i in 0..v.len() { if !v.is_char_boundary(i) { continue; } let mut m = v.to_string(); m.truncate(i); let _ = filter(&files, &m); let mut m2 = v.to_string(); m2.insert_str(i, r.pick(&PIECES)); let _ = filter(&files, &m2); } }
    }
    #[test] fn deep_nesting_is_rejected_not_overflowed() {
        let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000)); assert!(filter(&files_one(), &deep).unwrap_err().contains("too deeply"));
        assert!(filter(&files_one(), &"!".repeat(100_000)).is_err());
        assert!(filter(&files_one(), &format!("{}1", "-".repeat(100_000))).is_err());
    }
    fn files_one() -> Vec<FileStat> { vec![FileStat::new("a", Status::Added, 1, 1)] }
}

#[cfg(test)]
mod perf {
    use super::*;
    use perusal_diff::Status;
    /// Budget from the spec: filter/sort over a 10,000-file index in under 100 ms (release build).
    #[test]
    fn filter_and_sort_10k_files() {
        let files: Vec<FileStat> = (0..10_000).map(|i| FileStat::new(&format!("src/m{}/f{i}.rs", i % 100), Status::Modified, (i % 97) as u32, (i % 31) as u32)).collect();
        let t = std::time::Instant::now();
        let mut idx = filter(&files, "additions > avg(additions) && path ~ /m4/ || top(50, additions + deletions)").unwrap();
        sort(&files, &mut idx, "additions + deletions desc, name asc").unwrap();
        let ms = t.elapsed().as_millis();
        eprintln!("10k filter+sort: {ms} ms");
        assert!(ms < if cfg!(debug_assertions) { 2000 } else { 100 }, "{ms} ms");
    }
}

/// Plain-text documentation of the expression language, generated from the field and function tables (used in AI prompts).
pub fn reference() -> String {
    let mut s = String::from("Fields:\n");
    for f in FIELDS { s.push_str(&format!("- {f}: {}\n", field_doc(f))); }
    s.push_str("Functions (aggregates over all files):\n");
    for f in FUNCS { s.push_str(&format!("- {f}: {}\n", func_doc(f))); }
    s.push_str("Operators: && || ! == != < > <= >= + - * / % and `x ~ /regex/` (or `x ~ \"substring\"`). Booleans such as viewed or noisy can be used directly. Examples: `additions > 50 && !viewed`, `name ~ /test/`, `changed > avg(changed)`, `top(5, changed)`.\n");
    s.push_str("Sorting: comma separated tiers, each an expression followed by asc or desc (default asc), e.g. `changed desc, path asc`.\n");
    s
}
