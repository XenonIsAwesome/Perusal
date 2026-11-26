use crate::icons as ic;
use crate::kit::{self, badge, ci_badge, diff_squares, field_key, icon, icon_btn, icon_text_btn, input_text, kbd, loading_dots, markdown_view, new_bounds, seg_box, seg_icon, tip, wire_input, Btn, FieldBounds, Fonts, Input, Pal};
use crate::model::{self, Model};
use gpui::{deferred, div, list, point, prelude::*, px, relative, uniform_list, AnyElement, ClickEvent, Context, FontWeight, HighlightStyle, KeyDownEvent, ListAlignment, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, SharedString, StyledText};
use perusal_agents::run::{agent_env, run_streaming, AgentEvent, RunError};
use perusal_agents::{CtxDraft, CtxThread, ReviewContext, launch_with, on_path, parse_review, supports_effort, Options, Selection};
use perusal_app::events::{self, Snapshot};
use perusal_app::markdown::{self, Located};
use perusal_app::notify::Kind as NKind;
use perusal_app::settings::{DiffMode, MdMode, Settings};
use perusal_app::theme::{self, Theme};
use perusal_app::tree::{self, TreeRow};
use perusal_core::presets::{Preset, Presets};
use perusal_diff::text::{diff_lines, language_for_ext, split_pairs, Kind, Line};
use perusal_diff::Status;
use perusal_filter::Suggestion;
use perusal_providers::{InlineComment, Submission, Thread, Verdict};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(PartialEq, Clone, Copy)]
enum Fid { Filter, Sort, Comment, Ask, Summary, Preset, Rename, Gen, Find }
#[derive(PartialEq, Clone, Copy)]
enum RightTab { Review, Ai }
#[derive(PartialEq, Clone, Copy)]
enum Side { Left, Right }
#[derive(Clone, Copy)]
enum Job { Chat, Review }
/// The centre pane: the MR overview or the code (files and diff).
#[derive(Clone, Copy, PartialEq)]
enum Page { Overview, Code }
/// One occurrence found by Ctrl+F: the list row (diff) or page section (overview), which text of that row, and where in it.
#[derive(Clone)]
struct Hit { row: usize, part: usize, range: Range<usize> }
struct Comp { editing: Option<i64>, global: bool, preview: bool }

#[derive(Clone)]
struct RLine { line: Line, runs: Vec<(Range<usize>, HighlightStyle)> }
#[derive(Clone)]
struct CItem { key: String, author: String, body: String, draft: Option<i64>, ai: bool, resolved: bool, outdated: bool, replies: Vec<(String, String)>, /// the code a suggestion in the body replaces
    orig: Option<String>, /// new-file lines the comment is about
    range: Option<(u32, u32)> }
/// A collapsed comment: drawn as an icon in the line-number gutter of the line it belongs to.
#[derive(Clone)]
struct Mark { key: String, draft: bool, resolved: bool, range: Option<(u32, u32)> }
#[derive(Clone)]
enum RowKind { FileHeader, Rendered(Vec<markdown::Block>), Collapsed(String, bool), Note(String), Hunk(String), Uni(RLine), Split(Option<RLine>, Option<RLine>), Comment(CItem) }
#[derive(Clone)]
struct Row { fi: usize, kind: RowKind, /// ordinal among selectable rows
    ord: Option<usize>, new_line: Option<u32>, marks: Vec<Mark> }

pub enum ViewEvent { Home, OpenSettings, Notice(NKind, String, Option<&'static str>), Renamed(String), Ci(String) }
impl gpui::EventEmitter<ViewEvent> for View {}

pub struct View {
    pub m: Model, settings: Settings, theme: Theme, pal: Pal, fonts: Fonts,
    find_open: bool, find_in: Input, find_hits: Vec<Hit>, find_cur: usize, find_page: Page, ov_scroll: gpui::ScrollHandle,
    focus: Option<Fid>, gen: Input, sort_open: bool, gen_for: Option<perusal_agents::GenKind>, gen_busy: bool, gen_cancel: Arc<AtomicBool>, gen_msg: Option<String>, list_all: bool, forced: HashSet<usize>, collapsed_files: HashSet<usize>, sym_path: Option<String>, filter: Input, sort: Input, comment: Input, ask: Input, summary: Input, preset_name: Input, rename: Input,
    b_body: FieldBounds, b_composer: FieldBounds,
    error: Option<String>, zoom: f32, composer: Option<Comp>, finish_open: bool, presets_open: bool,
    shown: Option<Vec<usize>>, nav_changed: bool, base: Vec<Row>, rows: Vec<Row>, list: ListState, md: Option<Vec<Located>>, md_src: String, md_rendered: bool, md_sel: Option<(usize, usize)>, diff_mode: DiffMode, sym_target: Option<usize>,
    collapsed_c: HashSet<String>, raw_c: HashSet<String>,
    sel: Option<(usize, usize)>, anchor: Option<usize>, dragging: bool, row_y: HashMap<usize, f32>, sel_x: f32, sb_drag: Option<f32>, hx: f32, hb_drag: Option<f32>, hx_text: std::rc::Rc<std::cell::Cell<f32>>,
    tree: bool, collapsed: HashSet<String>, trows: Vec<TreeRow>, multi: BTreeSet<usize>, last_clicked: Option<usize>,
    sug: Vec<Suggestion>, sug_replace: usize, sug_idx: Option<usize>, sug_for: Fid,
    left_open: bool, right_open: bool, left_w: f32, right_w: f32, resizing: Option<Side>, commits_open: bool, drafts_open: bool, page: Page, info: Option<perusal_providers::MrInfo>, hover_c: Option<(usize, u32, u32)>, auto_c: HashSet<String>,
    tab: RightTab, chat: Vec<(String, String)>, ai_ctx: Option<Selection>, ai_notice: Option<String>,
    ai_job: Option<Job>, ai_events: Arc<Mutex<Vec<AgentEvent>>>, ai_cancel: Arc<AtomicBool>, ai_started: Option<Instant>, ai_activity: Vec<String>, ai_partial: String,
    verdict: Verdict, submit_busy: bool, submit_msg: Option<(bool, String)>,
    threads: Vec<Thread>, ci: String, snap: Snapshot, local_mr: Option<String>, notices: Vec<(NKind, String, Option<&'static str>)>, presets: Presets, project: String,
}

fn status_style(s: Status, p: &Pal) -> (&'static str, gpui::Hsla) {
    match s { Status::Added => ("A", p.success), Status::Modified => ("M", p.warn), Status::Deleted => ("D", p.danger), Status::Renamed => ("R", p.accent) }
}

fn merge_words(mut runs: Vec<(Range<usize>, HighlightStyle)>, line: &Line, p: &Pal) -> Vec<(Range<usize>, HighlightStyle)> {
    for w in &line.words {
        if w.0 < w.1 && w.1 <= line.text.len() && line.text.is_char_boundary(w.0) && line.text.is_char_boundary(w.1) {
            runs.retain(|r| r.0.end <= w.0 || r.0.start >= w.1);
            runs.push((w.0..w.1, HighlightStyle { background_color: Some(if line.kind == Kind::Added { p.added_word } else { p.removed_word }), color: Some(p.fg), ..Default::default() }));
            runs.sort_by_key(|r| r.0.start);
        }
    }
    runs.retain(|r| r.0.end <= line.text.len() && line.text.is_char_boundary(r.0.start) && line.text.is_char_boundary(r.0.end));
    runs
}

/// Resolves a symlink target relative to the link's directory, collapsing `.` and `..`.
pub fn resolve_link(link_path: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') { vec![] } else { link_path.rsplit_once('/').map(|x| x.0.split('/').collect()).unwrap_or_default() };
    for t in target.trim().split('/') { match t { "" | "." => {} ".." => { parts.pop(); } o => parts.push(o) } }
    parts.join("/")
}

/// Pins a collapsed comment's icon on `out[anchor]`; false when that row kind draws no icons (the comment then stays a full card so it is never unreachable).
fn add_mark(out: &mut [Row], anchor: usize, c: &CItem) -> bool {
    match out.get_mut(anchor) {
        Some(row) if matches!(row.kind, RowKind::Uni(_) | RowKind::Split(..) | RowKind::FileHeader) => { row.marks.push(Mark { key: c.key.clone(), draft: c.draft.is_some(), resolved: c.resolved, range: c.range }); true }
        _ => false,
    }
}

impl View {
    pub fn new(mut m: Model, settings: Settings, fonts: Fonts) -> Self {
        let theme = theme::load(&settings.theme, Some(&model::themes_dir()));
        m.vm.dir_order = settings.files_tree; m.vm.grouping = settings.group_related; m.vm.rules = settings.group_rules.clone(); m.vm.mark_on_next = false; m.vm.hide_whitespace = settings.hide_whitespace;
        let presets = model::load_presets(); let project = model::project_of(&m);
        let (mut f0, mut s0) = (String::new(), String::new());
        if let Some(d) = presets.effective_default(&project) { f0 = d.filter.clone(); s0 = d.sort.clone(); let _ = m.vm.set_filter(&f0); let _ = m.vm.set_sort(&s0); }
        // what the user last typed in this review wins over the preset default (even when they cleared it)
        if let Ok(Some((f, s))) = m.store.ui(m.vm.review) { f0 = f; s0 = s; let _ = m.vm.set_filter(&f0); let _ = m.vm.set_sort(&s0); }
        m.vm.sync_review_state(&m.store); let _ = m.vm.refresh();
        let chat = m.store.messages(m.vm.review).unwrap_or_default();
        let (title, mr_state) = (m.title.clone(), m.mr_state.clone());
        View { find_open: false, find_in: Input::new(""), find_hits: vec![], find_cur: 0, find_page: Page::Code, ov_scroll: gpui::ScrollHandle::new(), pal: Pal::new(&theme), theme, fonts, focus: None, gen: Input::new(""), sort_open: false, gen_for: None, gen_busy: false, gen_cancel: Arc::new(AtomicBool::new(false)), gen_msg: None, list_all: false, forced: HashSet::new(), collapsed_files: HashSet::new(), sym_path: None, filter: Input::new(&f0), sort: Input::new(&s0), comment: Input::multi(""), ask: Input::new(""), summary: Input::multi(""), preset_name: Input::new(""), rename: Input::new(&title),
            b_body: new_bounds(), b_composer: new_bounds(), error: None, zoom: 1.0, composer: None, finish_open: false, presets_open: false,
            shown: None, nav_changed: true, base: vec![], rows: vec![], list: ListState::new(0, ListAlignment::Top, px(600.0)), md: None, md_src: String::new(), md_rendered: false, md_sel: None, diff_mode: settings.diff_mode, sym_target: None,
            collapsed_c: HashSet::new(), raw_c: HashSet::new(), sel: None, anchor: None, dragging: false, row_y: HashMap::new(), sel_x: 0.0, sb_drag: None, hx: 0.0, hb_drag: None, hx_text: Default::default(),
            tree: settings.files_tree, collapsed: HashSet::new(), trows: vec![], multi: BTreeSet::new(), last_clicked: None, sug: vec![], sug_replace: 0, sug_idx: None, sug_for: Fid::Filter,
            left_open: settings.left_open, right_open: settings.right_open, left_w: settings.left_width, right_w: settings.right_width, resizing: None, commits_open: true, drafts_open: true, page: if m.info.is_some() && settings.first_view == perusal_app::settings::FirstView::Overview { Page::Overview } else { Page::Code }, info: m.info.clone(), hover_c: None, auto_c: HashSet::new(),
            tab: RightTab::Review, chat, ai_ctx: None, ai_notice: None, ai_job: None, ai_events: Arc::new(Mutex::new(vec![])), ai_cancel: Arc::new(AtomicBool::new(false)), ai_started: None, ai_activity: vec![], ai_partial: String::new(),
            verdict: Verdict::Comment, submit_busy: false, submit_msg: None, threads: vec![], ci: String::new(), snap: Snapshot { state: mr_state, ..Default::default() }, local_mr: None, notices: vec![], presets, project, settings, m }
    }
    pub fn key_id(&self) -> String { self.m.remote.clone().map(|u| format!("remote:{u}")).unwrap_or_else(|| format!("local:{}#{}", self.m.repo.display(), self.m.branch)) }
    fn current_file(&self) -> Option<usize> { self.m.vm.current_batch().first().copied() }
    fn save_settings(&self) { model::save_settings(&self.settings); }
    fn note(&mut self, kind: NKind, t: impl Into<String>, key: Option<&'static str>) { self.notices.push((kind, t.into(), key)); }
    fn flush(&mut self, cx: &mut Context<Self>) { for (k, t, d) in std::mem::take(&mut self.notices) { cx.emit(ViewEvent::Notice(k, t, d)); } }
    fn resync(&mut self) { self.m.vm.sync_review_state(&self.m.store); let _ = self.m.vm.refresh(); self.rebuild_rows(); }

    /// Applies new settings from the Settings panel to this review.
    /// (drafts, viewed, total) when leaving this review would lose work; `None` when there is nothing unsaved or it was already submitted.
    pub fn unfinished(&self) -> Option<(usize, u32, u32)> {
        if !self.m.store.unfinished(self.m.vm.review).unwrap_or(false) { return None; }
        let (v, t) = self.m.vm.progress(); Some((self.m.store.drafts(self.m.vm.review).map(|d| d.len()).unwrap_or(0), v as u32, t as u32))
    }
    pub fn review_id(&self) -> i64 { self.m.vm.review }
    pub fn is_remote(&self) -> bool { self.m.mr.is_some() }
    /// Opens the Finish review dialog (used by the leave prompt's Submit choice).
    pub fn begin_finish(&mut self, cx: &mut Context<Self>) { self.finish_open = true; self.focus = Some(Fid::Summary); self.submit_msg = None; cx.notify(); }
    pub fn apply_settings(&mut self, s: Settings) {
        let regroup = s.group_related != self.settings.group_related || s.group_rules != self.settings.group_rules || s.files_tree != self.settings.files_tree;
        let diff_changed = s.diff_mode != self.settings.diff_mode;
        self.theme = theme::load(&s.theme, Some(&model::themes_dir())); self.pal = Pal::new(&self.theme);
        self.tree = s.files_tree; self.m.vm.dir_order = s.files_tree; self.m.vm.grouping = s.group_related; self.m.vm.rules = s.group_rules.clone();
        if diff_changed { self.diff_mode = s.diff_mode; }
        self.settings.left_width = self.left_w; self.settings.right_width = self.right_w;
        let ws = s.hide_whitespace != self.settings.hide_whitespace;
        self.presets = model::load_presets();
        self.settings = s; self.shown = None;
        if ws { self.m.vm.hide_whitespace = self.settings.hide_whitespace; let _ = self.m.vm.refresh(); }
        if regroup { let keep = self.current_file(); let _ = self.m.vm.refresh(); if let Some(fi) = keep { self.m.vm.select_file(fi); } }
    }
    fn reload(&mut self) {
        self.shown = None; self.nav_changed = true; self.clear_selection();
        if self.tree { for fi in self.m.vm.current_batch().to_vec() { tree::expand_to(&mut self.collapsed, &self.m.vm.files[fi].path); } }
    }
    fn display_order(&self) -> Vec<usize> {
        if self.tree { self.trows.iter().filter_map(|r| if let TreeRow::File { fi, .. } = r { Some(*fi) } else { None }).collect() } else { self.m.vm.visible.clone() }
    }
    fn toggle_tree(&mut self) {
        let keep = self.current_file();
        self.tree = !self.tree; self.settings.files_tree = self.tree; self.save_settings();
        self.m.vm.dir_order = self.tree; let _ = self.m.vm.refresh();
        if let Some(fi) = keep { self.m.vm.select_file(fi); } self.reload();
    }
    fn toggle_dir(&mut self, path: &str) { if !self.collapsed.remove(path) { self.collapsed.insert(path.to_string()); } }
    fn busy(&self) -> bool { self.ai_job.is_some() }

    // ---------- diff rows
    fn load_diff(&mut self) {
        let batch = self.m.vm.current_batch().to_vec();
        if batch.is_empty() { self.base.clear(); self.rows.clear(); self.md = None; self.shown = None; self.list.reset(0); return; }
        if self.shown.as_ref() == Some(&batch) { return; }
        self.hx = 0.0; self.hx_text.set(0.0);
        if self.nav_changed { self.md_rendered = self.settings.markdown == MdMode::Rendered && batch.iter().any(|&i| markdown::is_markdown(&self.m.vm.files[i].path) && !self.m.vm.files[i].symlink); self.nav_changed = false; }
        self.shown = Some(batch.clone()); self.md = None; self.sym_target = None; self.sym_path = None;
        let p = self.pal;
        let mut rows: Vec<Row> = Vec::new();
        let mk = |fi: usize, kind: RowKind| Row { fi, kind, ord: None, new_line: None, marks: vec![] };
        for &fi in &batch {
            let f = self.m.vm.files[fi].clone();
            let old = perusal_gitops::show(&self.m.repo, &self.m.base_sha, &f.path).unwrap_or_default();
            let new = perusal_gitops::show(&self.m.repo, &self.m.branch, &f.path).unwrap_or_default();
            if batch.len() == 1 && self.md_rendered && !f.symlink {
                let src = if new.is_empty() { old } else { new }; self.md = Some(markdown::parse_located(&src)); self.md_src = src; self.base.clear(); self.rows.clear(); return;
            }
            if batch.len() > 1 { rows.push(mk(fi, RowKind::FileHeader)); if self.collapsed_files.contains(&fi) { continue; } }
            if f.symlink {
                let t = |s: &str| s.trim().to_string();
                rows.push(mk(fi, RowKind::Note(match (old.is_empty(), new.is_empty()) { (true, false) => format!("Symbolic link to {}", t(&new)), (false, true) => format!("Symbolic link to {} was removed", t(&old)), _ => format!("Symbolic link retargeted: {} → {}", t(&old), t(&new)) })));
                if batch.len() == 1 { let tgt = resolve_link(&f.path, if new.is_empty() { &old } else { &new }); self.sym_path = Some(tgt.clone()); self.sym_target = self.m.vm.files.iter().position(|x| x.path == tgt && !x.symlink); }
            }
            if batch.len() > 1 && self.md_rendered && !f.symlink && markdown::is_markdown(&f.path) && !f.noisy() {
                let src = if new.is_empty() { old.clone() } else { new.clone() };
                rows.push(mk(fi, RowKind::Rendered(markdown::parse(&src)))); continue;
            }
            if f.noisy() && !f.symlink && !self.forced.contains(&fi) {
                let why = if f.binary { "is a binary file, so there is no text to show" } else if f.generated { "is collapsed because it looks generated or vendored" } else { "is collapsed because it is very large" };
                rows.push(mk(fi, RowKind::Collapsed(format!("{} {why}.", f.path), !f.binary))); continue;
            }
            let lang = language_for_ext(f.ext());
            let (old_sp, new_sp) = (kit::line_spans(lang, &old, &p), kit::line_spans(lang, &new, &p));
            let lines = if self.m.vm.hide_whitespace { perusal_diff::text::diff_lines_ws(&old, &new, 3) } else { diff_lines(&old, &new, 3) };
            let rl = |l: &Line| -> RLine {
                let spans = if l.kind == Kind::Removed { l.old_no.and_then(|n| old_sp.get(n as usize - 1)) } else { l.new_no.and_then(|n| new_sp.get(n as usize - 1)) }.cloned().unwrap_or_default();
                RLine { line: l.clone(), runs: merge_words(spans, l, &p) }
            };
            let (mut last_old, mut last_new, mut first) = (0u32, 0u32, true);
            let mut hunk = |rows: &mut Vec<Row>, o: u32, n: u32| {
                let jump = (o > 0 && o > last_old + 1) || (n > 0 && n > last_new + 1) || (first && (o > 1 || n > 1));
                if jump { rows.push(Row { fi, kind: RowKind::Hunk(format!("@@ -{} +{} @@", o.max(1), n.max(1))), ord: None, new_line: None, marks: vec![] }); }
                first = false;
                if o > 0 { last_old = o; }
                if n > 0 { last_new = n; }
            };
            // added or deleted files have nothing on one side, so they always use a single column
            let split = self.diff_mode == DiffMode::Split && !matches!(f.status, Status::Added | Status::Deleted);
            if !split {
                for l in &lines { hunk(&mut rows, l.old_no.unwrap_or(0), l.new_no.unwrap_or(0)); rows.push(Row { fi, kind: RowKind::Uni(rl(l)), ord: None, new_line: l.new_no, marks: vec![] }); }
            } else {
                for (a, b) in split_pairs(&lines) {
                    hunk(&mut rows, a.and_then(|i| lines[i].old_no).unwrap_or(0), b.and_then(|i| lines[i].new_no).unwrap_or(0));
                    rows.push(Row { fi, kind: RowKind::Split(a.map(|i| rl(&lines[i])), b.map(|i| rl(&lines[i]))), ord: None, new_line: b.and_then(|i| lines[i].new_no).or_else(|| a.and_then(|i| lines[i].new_no)), marks: vec![] });
                }
            }
        }
        let mut ord = 0; for r in &mut rows { if matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..)) { r.ord = Some(ord); ord += 1; } }
        self.base = rows; self.rebuild_rows();
    }

    /// Comments for a file: (end line it belongs to, card). Remote threads (anyone's) and your own drafts.
    fn comments_for(&self, fi: usize) -> Vec<(Option<u32>, CItem)> {
        let path = &self.m.vm.files[fi].path; let mut out = vec![];
        for t in self.threads.iter().filter(|t| t.path.as_deref() == Some(path.as_str())) {
            let (author, body) = t.comments.first().cloned().unwrap_or_default();
            out.push((t.line, CItem { key: format!("t:{}", t.id), author, body, draft: None, ai: false, resolved: t.resolved, outdated: false, replies: t.comments.iter().skip(1).cloned().collect(), orig: None, range: t.line.map(|l| (l, l)) }));
        }
        for d in self.m.store.drafts(self.m.vm.review).unwrap_or_default().into_iter().filter(|d| d.file.as_deref() == Some(path.as_str())) {
            let orig = (d.body.contains("```suggestion")).then(|| d.start_line.zip(d.end_line).and_then(|(a, b)| {
                let t = perusal_gitops::show(&self.m.repo, &self.m.branch, path)?;
                let v: Vec<&str> = t.lines().skip(a.saturating_sub(1) as usize).take((b + 1).saturating_sub(a) as usize).collect(); (!v.is_empty()).then(|| v.join("\n"))
            })).flatten();
            out.push((d.end_line.or(d.start_line), CItem { key: format!("d:{}", d.id), author: "You".into(), body: d.body, draft: Some(d.id), ai: d.ai, resolved: false, outdated: false, replies: vec![], orig, range: d.end_line.or(d.start_line).map(|e| (d.start_line.unwrap_or(e).min(e), e)) }));
        }
        out
    }
    /// Inserts comment cards into the diff rows next to the lines they belong to.
    fn rebuild_rows(&mut self) {
        let top = self.list.logical_scroll_top();
        let mut out: Vec<Row> = Vec::with_capacity(self.base.len() + 8);
        let files: Vec<usize> = { let mut f: Vec<usize> = vec![]; for r in &self.base { if !f.contains(&r.fi) { f.push(r.fi); } } f };
        for fi in files {
            let mut pending = self.comments_for(fi);
            // resolved threads start collapsed (once; expanding one by hand sticks)
            for (_, c) in &pending { if c.resolved && self.auto_c.insert(c.key.clone()) { self.collapsed_c.insert(c.key.clone()); } }
            let group: Vec<&Row> = self.base.iter().filter(|r| r.fi == fi).collect();
            if self.collapsed_files.contains(&fi) { out.extend(group.into_iter().cloned()); continue; }
            let mut placed_top = false; let mut anchor = out.len();
            let card = |c: CItem| Row { fi, kind: RowKind::Comment(c), ord: None, new_line: None, marks: vec![] };
            for r in group {
                out.push(r.clone());
                if matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..) | RowKind::FileHeader) { anchor = out.len() - 1; }
                let header_like = matches!(r.kind, RowKind::FileHeader | RowKind::Note(_) | RowKind::Collapsed(..) | RowKind::Rendered(_));
                if !placed_top && !header_like { placed_top = true; }
                if header_like && !placed_top {
                    // file-level comments (no line) sit right under the file header
                    let (file_level, rest): (Vec<_>, Vec<_>) = pending.into_iter().partition(|(l, _)| l.is_none()); pending = rest;
                    for (_, c) in file_level {
                        if self.collapsed_c.contains(&c.key) && add_mark(&mut out, anchor, &c) { continue; }
                        out.push(card(c));
                    }
                }
                if let (Some(n), true) = (r.new_line, matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..))) {
                    let (here, rest): (Vec<_>, Vec<_>) = pending.into_iter().partition(|(l, _)| *l == Some(n)); pending = rest;
                    for (_, c) in here {
                        if self.collapsed_c.contains(&c.key) && add_mark(&mut out, anchor, &c) { continue; }
                        out.push(card(c));
                    }
                }
            }
            for (l, mut c) in pending {
                c.outdated = l.is_some();
                if self.collapsed_c.contains(&c.key) && add_mark(&mut out, anchor, &c) { continue; }
                out.push(card(c));
            }
        }
        self.rows = out;
        if self.find_open && self.page == Page::Code { self.recompute_find(); }
        // measure every row up front so the scroll range is exact (otherwise it keeps growing as unseen rows are measured); huge diffs stay lazy
        let all = self.rows.len() <= 4000;
        if all != self.list_all { self.list = ListState::new(self.rows.len(), ListAlignment::Top, px(600.0)); if all { self.list = self.list.clone().measure_all(); } self.list_all = all; } else { self.list.reset(self.rows.len()); }
        self.list.scroll_to(top);
    }
    fn toggle_comment(&mut self, key: &str) { if !self.collapsed_c.remove(key) { self.collapsed_c.insert(key.to_string()); } self.rebuild_rows(); }
    /// Collapse (or expand) every comment on the current file(s).
    fn collapse_all_comments(&mut self) {
        let keys: Vec<String> = self.rows.iter().filter_map(|r| if let RowKind::Comment(c) = &r.kind { Some(c.key.clone()) } else { None }).collect();
        if keys.is_empty() { return; }
        let all = keys.iter().all(|k| self.collapsed_c.contains(k));
        for k in keys { if all { self.collapsed_c.remove(&k); } else { self.collapsed_c.insert(k); } }
        self.rebuild_rows();
    }
    fn comment_count(&self) -> usize { self.rows.iter().filter(|r| matches!(r.kind, RowKind::Comment(_))).count() }

    // ---------- selection
    fn in_sel(&self, ord: usize) -> bool { self.sel.map(|(a, b)| ord >= a && ord <= b).unwrap_or(false) }
    fn selection(&self) -> Option<Selection> {
        if let (Some(blocks), Some((a, b))) = (&self.md, self.md_sel) {
            let fi = self.current_file()?; let (lo, hi) = (a.min(b), a.max(b));
            let (first, last) = (blocks.get(lo)?.first_line, blocks.get(hi)?.last_line);
            let snippet: String = self.md_src.lines().skip(first.saturating_sub(1) as usize).take((last + 1 - first) as usize).collect::<Vec<_>>().join("\n");
            return Some(Selection { file: self.m.vm.files[fi].path.clone(), start: first, end: last, snippet });
        }
        let (a, b) = self.sel?; let fi = self.rows.iter().find(|r| r.ord == Some(a))?.fi;
        let (mut lo, mut hi, mut snippet) = (u32::MAX, 0u32, String::new());
        for r in self.rows.iter().filter(|r| r.fi == fi && r.ord.map(|o| o >= a && o <= b).unwrap_or(false)) {
            let lines: Vec<&Line> = match &r.kind { RowKind::Uni(l) => vec![&l.line], RowKind::Split(l, rr) => l.iter().chain(rr.iter()).map(|x| &x.line).collect(), _ => vec![] };
            for l in lines {
                if let Some(n) = l.new_no.or(l.old_no) { lo = lo.min(n); hi = hi.max(n); }
                snippet.push_str(match l.kind { Kind::Added => "+", Kind::Removed => "-", Kind::Context => " " }); snippet.push_str(&l.text); snippet.push('\n');
            }
        }
        (hi > 0).then(|| Selection { file: self.m.vm.files[fi].path.clone(), start: lo, end: hi, snippet })
    }
    /// The selected lines as they will look after the change (for suggested edits).
    fn selection_new_text(&self) -> Option<String> {
        let s = self.selection()?;
        let t: Vec<String> = s.snippet.lines().filter(|l| !l.starts_with('-')).map(|l| l.chars().skip(1).collect()).collect();
        (!t.is_empty()).then(|| t.join("\n"))
    }
    fn row_hit(&mut self, ord: usize, shift: bool) {
        if shift { if let Some(a) = self.anchor { self.sel = Some((a.min(ord), a.max(ord))); return; } }
        self.anchor = Some(ord); self.sel = Some((ord, ord)); self.dragging = true; self.row_y.clear();
    }
    fn clear_selection(&mut self) { self.sel = None; self.anchor = None; self.md_sel = None; self.row_y.clear(); }
    fn has_selection(&self) -> bool { self.sel.is_some() || self.md_sel.is_some() }

    // ---------- filter, sort, presets
    fn apply_filter(&mut self) {
        match self.m.vm.set_filter(self.filter.f.text.trim()) { Ok(()) => { self.error = None; self.sug.clear(); self.remember_ui(); self.reload(); } Err(e) => self.error = Some(format!("Filter: {e}")) }
    }
    fn apply_sort(&mut self) {
        match self.m.vm.set_sort(self.sort.f.text.trim()) { Ok(()) => { self.error = None; self.sug.clear(); self.remember_ui(); self.reload(); } Err(e) => self.error = Some(format!("Sort: {e}")) }
    }
    fn set_tiers(&mut self, t: Vec<(String, bool)>) { let text = perusal_filter::format_tiers(&t); self.sort.f.set(&text); self.apply_sort(); }
    /// Asks the agent to write the filter or sort described in the generate box.
    fn start_gen(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.gen_for else { return };
        if self.gen_busy { return; }
        let req = self.gen.f.text.trim().to_string();
        if req.is_empty() { self.gen_msg = Some("Describe what you want first.".into()); return; }
        self.gen_busy = true; self.gen_msg = None; self.gen_cancel.store(false, Ordering::Relaxed);
        let (st, cancel) = (self.settings.clone(), self.gen_cancel.clone());
        let task = cx.background_executor().spawn(async move { model::run_gen(&st, kind, &req, &cancel) });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| {
            t.gen_busy = false;
            match r {
                Ok(v) => {
                    if kind == perusal_agents::GenKind::Filter { t.filter.f.set(&v[0]); t.apply_filter(); } else { let tiers = perusal_filter::parse_tiers(&v[0]); t.set_tiers(tiers); }
                    t.gen_for = None; t.gen.f.clear(); t.note(NKind::Success, format!("Applied: {}", v[0]), None); t.flush(cx);
                }
                Err(e) if e.contains("stopped") || e.contains("cancel") => t.gen_msg = Some("Stopped.".into()),
                Err(e) => t.gen_msg = Some(e),
            }
            cx.notify(); }); }).detach();
        cx.notify();
    }
    /// The "Generate with AI" box shared by the filter and sort popovers.
    fn gen_box(&self, kind: perusal_agents::GenKind, hint: &'static str, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.pal; let focused = self.focus == Some(Fid::Gen);
        div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border)
            .child(div().flex().items_center().gap_2().text_sm().font_weight(FontWeight::SEMIBOLD).child(icon(ic::AUTO_AWESOME, 16.0, p.ai)).child("Generate with AI"))
            .child(self.winp(Fid::Gen, div().id("gen-box").flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).child(input_text(&self.gen, focused, hint, &p, false, 1)), cx))
            .children(self.gen_msg.clone().map(|m| div().text_xs().text_color(p.danger).child(m)))
            .child(div().flex().items_center().gap_2().when(self.gen_busy, |d| d.child(loading_dots(&p)).child(div().text_xs().text_color(p.muted).child(format!("{} is writing it…", self.settings.agent_label()))))
                .child(div().flex_1())
                .child(if self.gen_busy { icon_text_btn("gen-stop", ic::STOP, "Stop", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.gen_cancel.store(true, Ordering::Relaxed); cx.notify(); })).into_any_element() }
                    else { icon_text_btn("gen-go", ic::AUTO_AWESOME, "Generate", Btn::Primary, &p).on_click(cx.listener(move |this, _, _, cx| { this.gen_for = Some(kind); this.start_gen(cx); cx.stop_propagation(); cx.notify(); })).into_any_element() }))
    }
    fn update_suggestions(&mut self) {
        let (s, n) = if self.sug_for == Fid::Sort { perusal_filter::suggest_sort(self.sort.f.before_caret()) } else { perusal_filter::suggest(self.filter.f.before_caret()) };
        self.sug = s; self.sug_replace = n; self.sug_idx = None;
    }
    fn accept_suggestion(&mut self, i: usize) {
        if let Some(s) = self.sug.get(i).cloned() {
            let f = if self.sug_for == Fid::Sort { &mut self.sort.f } else { &mut self.filter.f };
            f.replace_before_caret(self.sug_replace, &s.insert); self.update_suggestions();
        }
    }
    fn apply_preset(&mut self, p: &Preset) {
        self.filter.f.set(&p.filter); self.sort.f.set(&p.sort);
        let (a, b) = (self.m.vm.set_filter(&p.filter), self.m.vm.set_sort(&p.sort));
        self.error = a.err().or(b.err()); self.remember_ui(); self.reload();
    }
    /// Saves the applied filter and sort with this review so reopening it restores them.
    fn remember_ui(&self) { let _ = self.m.store.set_ui(self.m.vm.review, self.filter.f.text.trim(), self.sort.f.text.trim()); }
    fn save_preset(&mut self, project_scope: bool) {
        let name = self.preset_name.f.text.trim().to_string(); if name.is_empty() { return; }
        let pr = Preset { filter: self.filter.f.text.trim().into(), sort: self.sort.f.text.trim().into() };
        self.presets.save(if project_scope { Some(&self.project) } else { None }, &name, pr);
        model::save_presets(&self.presets); self.preset_name.f.clear();
        self.note(NKind::Success, format!("Saved the preset \"{name}\"."), None);
    }

    // ---------- actions
    fn open_composer(&mut self, global: bool, editing: Option<i64>) {
        let text = editing.and_then(|id| self.m.store.drafts(self.m.vm.review).ok().and_then(|d| d.into_iter().find(|x| x.id == id)).map(|d| d.body)).unwrap_or_default();
        self.comment.f.set(&text); self.comment.f.multiline = true;
        self.composer = Some(Comp { editing, global, preview: false }); self.focus = Some(Fid::Comment);
    }
    fn save_composer(&mut self, cx: &mut Context<Self>) {
        let text = self.comment.f.text.trim().to_string(); if text.is_empty() { return; }
        let Some(c) = self.composer.take() else { return };
        let mut added: Option<i64> = None;
        if let Some(id) = c.editing { let _ = self.m.store.edit_draft(id, &text); self.note(NKind::Success, "Comment updated.", None); }
        else {
            let sel = if c.global { None } else { self.selection() };
            match (sel, self.current_file(), c.global) {
                (Some(s), _, _) => { added = self.m.store.add_draft(self.m.vm.review, Some(&s.file), Some((s.start, s.end)), &text, false).ok(); self.note(NKind::Success, format!("Draft added on {}:{}-{}", s.file, s.start, s.end), None); }
                (None, Some(fi), false) => { let path = self.m.vm.files[fi].path.clone(); added = self.m.store.add_draft(self.m.vm.review, Some(&path), None, &text, false).ok(); self.note(NKind::Success, format!("Draft added on {path}"), None); }
                _ => { added = self.m.store.add_draft(self.m.vm.review, None, None, &text, false).ok(); self.note(NKind::Success, "Draft added on the merge request.", None); }
            }
        }
        self.comment.f.clear(); self.focus = None; self.clear_selection(); self.resync();
        // Saved locally first so nothing is lost if the host is unreachable; then posted right away.
        if let (Some(id), true) = (added, self.instant()) { self.publish_draft(id, cx); }
    }
    fn toggle_viewed(&mut self) { if let Some(fi) = self.current_file() { let v = !self.m.vm.files[fi].viewed; for i in self.m.vm.current_batch().to_vec() { self.m.vm.set_viewed(&self.m.store, i, v); } } }
    fn next(&mut self) { self.m.vm.next(&self.m.store); self.reload(); }
    fn back(&mut self) { self.m.vm.back(); self.reload(); }
    fn step(&mut self, d: i32) { let n = self.m.vm.batch_count() as i32; if n > 0 { self.m.vm.cursor = (self.m.vm.cursor as i32 + d).rem_euclid(n) as usize; self.reload(); } }
    fn clear_scope(&mut self) { self.m.graph.clear(); self.m.vm.commit_scope = None; let _ = self.m.vm.refresh(); self.reload(); }
    fn pick_commit(&mut self, i: usize) { self.page = Page::Code;
        if self.m.graph.selection == Some((i, i)) { self.clear_scope(); return; }
        self.m.graph.click(i); self.m.vm.commit_scope = self.m.graph.scope(&self.m.repo).ok().flatten(); let _ = self.m.vm.refresh(); self.reload();
    }
    fn set_diff_mode(&mut self, m: DiffMode) { if self.diff_mode != m { self.diff_mode = m; self.shown = None; self.clear_selection(); } }
    fn click_file(&mut self, fi: usize, mods: gpui::Modifiers) {
        if mods.control || mods.platform {
            if !self.multi.remove(&fi) { if self.multi.is_empty() { if let Some(c) = self.current_file() { if c != fi { self.multi.insert(c); } } } self.multi.insert(fi); }
            self.last_clicked = Some(fi);
        } else if mods.shift {
            let order = self.display_order(); let from = self.last_clicked.or_else(|| self.current_file()).unwrap_or(fi);
            if let (Some(a), Some(b)) = (order.iter().position(|x| *x == from), order.iter().position(|x| *x == fi)) { for x in &order[a.min(b)..=a.max(b)] { self.multi.insert(*x); } }
        } else { self.multi.clear(); self.last_clicked = Some(fi); self.m.vm.select_file(fi); self.page = Page::Code; self.reload(); }
    }
    fn group_multi(&mut self) {
        let order = self.display_order(); let mut members: Vec<usize> = order.into_iter().filter(|i| self.multi.contains(i)).collect();
        if members.len() < 2 { return; }
        members.dedup(); self.m.vm.group_files_manually(members); self.multi.clear(); self.reload();
        self.note(NKind::Info, "Grouped into one batch. Viewing it shows every file together.", None);
    }
    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let t = self.rename.f.text.trim().to_string();
        if !t.is_empty() { let _ = self.m.store.rename_review(self.m.vm.review, &t); self.m.title = t.clone(); cx.emit(ViewEvent::Renamed(t)); }
    }
    /// Clicking away from every field: unfocus, close dropdowns and menus, discard unapplied filter text.
    fn blur(&mut self, cx: &mut Context<Self>) {
        match self.focus {
            Some(Fid::Filter) => { let t = self.m.vm.filter.clone(); self.filter.f.set(&t); }
            Some(Fid::Sort) => { let t = self.m.vm.sort.clone(); self.sort.f.set(&t); }
            Some(Fid::Rename) => { self.commit_rename(cx); }
            _ => {}
        }
        self.focus = None; self.sug.clear(); self.presets_open = false; self.error = None; self.sort_open = false; if !self.gen_busy { self.gen_for = None; }
    }
    fn goto_symlink_target(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.sym_target { self.m.vm.select_file(t); self.page = Page::Code; self.reload(); }
        else if let Some(p) = self.sym_path.clone() { self.note(NKind::Info, format!("The original, {p}, is not changed in this merge request, so there is nothing to show."), None); self.flush(cx); }
    }

    // ---------- live updates: threads, CI, MR state
    pub fn start_polling(&mut self, cx: &mut Context<Self>) {
        let mr = self.m.mr.clone(); let (repo, branch, hosts) = (self.m.repo.clone(), self.m.branch.clone(), self.settings.hosts.clone());
        cx.spawn(async move |this, cx| {
            let mut delay = 1u64; let fixed: Option<u64> = std::env::var("PERUSAL_POLL_SECS").ok().and_then(|v| v.parse().ok());
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(delay)).await;
                if let Some(mr) = mr.clone() {
                    let r = cx.background_executor().spawn(async move { model::fetch_remote_state(&mr) }).await;
                    let alive = this.update(cx, |t, cx| { if let Ok(s) = r { t.apply_remote(s, cx); } t.flush(cx); cx.notify(); t.ci == "running" }).ok();
                    let Some(running) = alive else { break }; delay = fixed.unwrap_or(if running { 15 } else { 60 });
                } else {
                    let (rp, br, hs) = (repo.clone(), branch.clone(), hosts.clone());
                    let r = cx.background_executor().spawn(async move { model::find_mr_for_branch(&rp, &br, &hs) }).await;
                    let alive = this.update(cx, |t, cx| { if let Some((_, web)) = r { if t.local_mr.is_none() { t.note(NKind::Info, "A pull request or merge request is now open for this branch.", Some("new_commits")); t.local_mr = Some(web); } } t.flush(cx); cx.notify(); t.local_mr.is_some() }).ok();
                    let Some(found) = alive else { break }; delay = if found { 600 } else { 90 };
                }
            }
        }).detach();
    }
    fn apply_remote(&mut self, s: model::RemoteState, cx: &mut Context<Self>) {
        let now = Snapshot { state: model::norm_state(&s.info.state), head: s.info.head_sha.clone(), ci: Some(model::ci_label(&s.ci).to_string()).filter(|c| c != "none") };
        let title = self.m.title.clone();
        // the pinned head is the one this review was opened on; a different host head means new commits
        let mut prev = self.snap.clone(); if prev.head.is_empty() { prev.head = self.m.branch.clone(); }
        for e in events::diff(&prev, &now, &title, &|_, _| true) {
            let kind = match e.level { events::Level::Info => NKind::Info, events::Level::Success => NKind::Success, events::Level::Warning => NKind::Warning, events::Level::Error => NKind::Error };
            self.note(kind, e.body.clone(), e.important.then_some(e.key));
        }
        if now.head != self.m.branch && prev.head == self.m.branch { /* surfaced once as a notice above; the review stays pinned */ }
        self.m.mr_state = now.state.clone(); let _ = self.m.store.set_mr_state(self.m.vm.review, &now.state);
        let ci = now.ci.clone().unwrap_or_default();
        if ci != self.ci { self.ci = ci.clone(); cx.emit(ViewEvent::Ci(ci)); }
        self.snap = now; self.info = Some(s.info.clone());
        if s.threads.len() != self.threads.len() || s.threads.iter().zip(&self.threads).any(|(a, b)| a.comments.len() != b.comments.len() || a.resolved != b.resolved) { self.threads = s.threads; self.rebuild_rows(); } else { self.threads = s.threads; }
    }
    fn ci_url(&self) -> Option<String> { let w = self.m.web_url.clone()?; Some(perusal_providers::ci_page(&self.m.mr.as_ref()?.kind, &w)) }

    // ---------- AI
    fn ai_options(&self) -> Options { Options { model: self.settings.model.clone(), effort: self.settings.effort.clone(), path: self.settings.agent_path.clone() } }
    fn start_ai(&mut self, job: Job, question: String, cx: &mut Context<Self>) {
        if self.busy() { return; }
        let opts = self.ai_options();
        let Some(probe) = launch_with(&self.settings.agent, &self.settings.agent_command, "x", &opts) else { self.ai_notice = Some("No agent configured. Choose one in Settings.".into()); return };
        if !on_path(&probe.program) { self.ai_notice = Some(format!("`{}` was not found on your PATH. Install it, pick another agent in Settings, or set a custom command.", probe.program)); return; }
        self.ai_notice = None; self.ai_partial.clear(); self.ai_activity.clear();
        let sel = self.ai_ctx.clone();
        let (repo, rev, base, review) = (self.m.repo.clone(), self.m.branch.clone(), self.m.base_sha.clone(), self.m.vm.review);
        let (agent, custom) = (self.settings.agent.clone(), self.settings.agent_command.clone());
        let timeout = self.settings.agent_timeout().unwrap_or(std::time::Duration::MAX);
        let changed: Vec<String> = self.m.vm.files.iter().map(|f| f.path.clone()).collect();
        let rctx = ReviewContext {
            mr_url: self.m.web_url.clone(),
            threads: self.threads.iter().map(|t| CtxThread { path: t.path.clone(), line: t.line, resolved: t.resolved, comments: t.comments.clone() }).collect(),
            drafts: self.m.store.drafts(review).unwrap_or_default().into_iter().map(|d| CtxDraft { file: d.file, start: d.start_line, end: d.end_line, body: d.body, ai: d.ai }).collect(),
        };
        let prompts = model::prompts();
        let prompt_chat = prompts.chat(sel.as_ref(), &self.chat, &question);
        if let Job::Chat = job { self.chat.push(("user".into(), question.clone())); let _ = self.m.store.add_message(review, "user", &question); }
        self.ai_job = Some(job); self.ai_started = Some(Instant::now());
        let events = self.ai_events.clone(); events.lock().unwrap().clear();
        let cancel = self.ai_cancel.clone(); cancel.store(false, Ordering::Relaxed);
        let is_review = matches!(job, Job::Review);
        let task = cx.background_executor().spawn(async move {
            let push = |e: AgentEvent| events.lock().unwrap().push(e);
            push(AgentEvent::Activity("Preparing a read-only copy of the code".into()));
            let dest = model::snapshot_dir(review, &rev);
            if !dest.join(".perusal-snapshot").exists() { perusal_gitops::export_tree(&repo, &rev, &dest).map_err(RunError::Failed)?; let _ = std::fs::write(dest.join(".perusal-snapshot"), "1"); }
            let prompt = if is_review { push(AgentEvent::Activity("Collecting the patch".into())); let patch = perusal_gitops::patch(&repo, &base, &rev).map_err(RunError::Failed)?; prompts.review(&rctx, &changed, &patch) } else { prompt_chat };
            let l = launch_with(&agent, &custom, &prompt, &opts).ok_or_else(|| RunError::Failed("no agent configured".into()))?;
            push(AgentEvent::Activity(format!("Starting {}", l.program)));
            run_streaming(&l, &dest, &agent_env(), timeout, &cancel, push)
        });
        cx.spawn(async move |this, cx| { let r: Result<String, RunError> = task.await; let _ = this.update(cx, |t, cx| t.finish_ai(job, r, cx)); }).detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                let alive = this.update(cx, |t, cx| { t.drain_ai_events(); cx.notify(); t.busy() }).unwrap_or(false);
                if !alive { break; }
            }
        }).detach();
        cx.notify();
    }
    fn drain_ai_events(&mut self) {
        let evs: Vec<AgentEvent> = std::mem::take(&mut *self.ai_events.lock().unwrap());
        for e in evs {
            match e {
                AgentEvent::Activity(a) => { if self.ai_activity.last() != Some(&a) { self.ai_activity.push(a); if self.ai_activity.len() > 40 { self.ai_activity.remove(0); } } }
                AgentEvent::Text(t) => { self.ai_partial.push_str(&t); self.ai_partial.push('\n'); if self.ai_activity.last().map(|s| s.as_str()) != Some("Writing the answer") { self.ai_activity.push("Writing the answer".into()); } }
                AgentEvent::Final(_) => {}
            }
        }
    }
    fn stop_ai(&mut self) { self.ai_cancel.store(true, Ordering::Relaxed); self.ai_activity.push("Stopping…".into()); }
    fn finish_ai(&mut self, job: Job, r: Result<String, RunError>, cx: &mut Context<Self>) {
        self.drain_ai_events(); self.ai_job = None; self.ai_started = None; self.ai_partial.clear();
        let review = self.m.vm.review;
        match (job, r) {
            (Job::Chat, Ok(text)) => { let t = text.trim().to_string(); self.chat.push(("agent".into(), t.clone())); let _ = self.m.store.add_message(review, "agent", &t); }
            (Job::Review, Ok(text)) => match parse_review(&text) {
                Ok(cs) => {
                    for c in &cs { let _ = self.m.store.add_draft(review, Some(&c.file), Some((c.start_line, c.end_line)), &perusal_agents::draft_body(c), true); }
                    let msg = if cs.is_empty() { "AI review finished with nothing to flag.".to_string() } else { format!("AI review added {} draft comment{}, tagged AI. Review, edit or remove them in Drafts.", cs.len(), if cs.len() == 1 { "" } else { "s" }) };
                    self.chat.push(("agent".into(), msg.clone())); let _ = self.m.store.add_message(review, "agent", &msg); self.tab = RightTab::Review; self.resync();
                    self.note(NKind::Success, msg, Some("ai_done"));
                }
                Err(e) => self.ai_notice = Some(format!("The agent's answer was not a usable review ({e}). First 300 characters: {}", text.chars().take(300).collect::<String>())),
            },
            (_, Err(RunError::Cancelled)) => self.note(NKind::Info, "Stopped. Nothing was added.", None),
            (_, Err(e)) => self.ai_notice = Some(e.to_string()),
        }
        self.flush(cx); cx.notify();
    }
    // ---------- find in page (Ctrl+F)
    fn open_find(&mut self, cx: &mut Context<Self>) {
        self.find_open = true; self.focus = Some(Fid::Find); self.find_in.f.select_all(); self.recompute_find(); self.scroll_to_hit(); cx.notify();
    }
    fn close_find(&mut self) { self.find_open = false; self.find_hits.clear(); self.find_cur = 0; if self.focus == Some(Fid::Find) { self.focus = None; } }
    /// The searchable texts of a diff row (one per part: the two sides of a split row, a comment and its replies).
    fn row_texts(&self, r: &Row) -> Vec<String> {
        match &r.kind {
            RowKind::Uni(rl) => vec![rl.line.text.clone()],
            RowKind::Split(a, b) => vec![a.as_ref().map(|x| x.line.text.clone()).unwrap_or_default(), b.as_ref().map(|x| x.line.text.clone()).unwrap_or_default()],
            RowKind::FileHeader => vec![self.m.vm.files[r.fi].path.clone()],
            RowKind::Hunk(t) | RowKind::Note(t) | RowKind::Collapsed(t, _) => vec![t.clone()],
            RowKind::Comment(c) => { let mut v = vec![format!("{} {}", c.author, c.body)]; v.extend(c.replies.iter().map(|(a, t)| format!("{a} {t}"))); v }
            RowKind::Rendered(_) => vec![],
        }
    }
    /// Finds every occurrence on the current page (diff rows; the overview recomputes itself while drawing).
    fn recompute_find(&mut self) {
        self.find_page = self.page;
        let q = self.find_in.f.text.clone();
        let mut hits = vec![];
        if !q.is_empty() && self.page == Page::Code {
            for (ix, r) in self.rows.iter().enumerate() { for (part, t) in self.row_texts(r).iter().enumerate() { for range in perusal_app::find::find_all(t, &q) { hits.push(Hit { row: ix, part, range }); } } }
        }
        self.find_hits = hits; if self.find_cur >= self.find_hits.len() { self.find_cur = 0; }
    }
    fn find_step(&mut self, forward: bool) {
        if self.find_page != self.page { self.recompute_find(); }
        if let Some(i) = perusal_app::find::step(self.find_cur, self.find_hits.len(), forward) { self.find_cur = i; self.scroll_to_hit(); }
    }
    fn scroll_to_hit(&mut self) {
        let Some(h) = self.find_hits.get(self.find_cur) else { return };
        if self.page == Page::Code { self.list.scroll_to(gpui::ListOffset { item_ix: h.row.saturating_sub(3), offset_in_item: px(0.0) }); } else { self.ov_scroll.scroll_to_item(h.row); }
    }
    /// Hits inside one row as (part, range, is the current hit).
    fn hits_in_row(&self, row: usize) -> Vec<(usize, Range<usize>, bool)> {
        if !self.find_open || self.find_hits.is_empty() || self.page != Page::Code { return vec![]; }
        let start = self.find_hits.partition_point(|h| h.row < row);
        self.find_hits[start..].iter().enumerate().take_while(|(_, h)| h.row == row).map(|(i, h)| (h.part, h.range.clone(), start + i == self.find_cur)).collect()
    }
    /// Background for rows whose text is not drawn as a plain code line (headers, comments, hunks), so hits there are still visible.
    fn row_tint(&self, ix: usize) -> Option<gpui::Hsla> {
        let kind_plain = self.rows.get(ix).map(|r| matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..))).unwrap_or(true);
        if kind_plain { return None; }
        let hits = self.hits_in_row(ix); if hits.is_empty() { None } else { Some(self.pal.warn.opacity(if hits.iter().any(|h| h.2) { 0.35 } else { 0.16 })) }
    }
    /// The floating find box (top right of the centre pane).
    fn find_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find_open { return None; }
        let p = self.pal; let focused = self.focus == Some(Fid::Find);
        let n = self.find_hits.len(); let has_q = !self.find_in.f.text.is_empty();
        let count = if !has_q { String::new() } else if n == 0 { "No results".to_string() } else { format!("{} of {n}", self.find_cur + 1) };
        let input = self.winp(Fid::Find, div().id("find-box").flex().items_center().w(px(220.0)).px_2().py(px(4.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).child(input_text(&self.find_in, focused, "Find in page", &p, false, 1)), cx);
        Some(div().absolute().top(px(8.0)).right(px(20.0)).flex().items_center().gap_1().px_2().py(px(5.0)).rounded_lg().bg(p.surface).border_1().border_color(p.border).shadow_lg()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(input).child(div().min_w(px(62.0)).px_1().text_xs().text_color(if has_q && n == 0 { p.danger } else { p.muted }).child(count))
            .child(icon_btn("find-prev", ic::KEYBOARD_ARROW_UP, "Previous match (Shift+Enter)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.find_step(false); cx.notify(); })))
            .child(icon_btn("find-next", ic::KEYBOARD_ARROW_DOWN, "Next match (Enter)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.find_step(true); cx.notify(); })))
            .child(icon_btn("find-close", ic::CLOSE, "Close (Esc)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.close_find(); cx.notify(); }))).into_any_element())
    }
    fn draft_from_answer(&mut self, text: &str) {
        let (f, l) = match &self.ai_ctx { Some(s) => (Some(s.file.clone()), Some((s.start, s.end))), None => (self.current_file().map(|i| self.m.vm.files[i].path.clone()), None) };
        let _ = self.m.store.add_draft(self.m.vm.review, f.as_deref(), l, text, true); self.note(NKind::Success, "Saved the answer as an AI draft.", None); self.resync();
    }

    // ---------- finishing the review
    fn finish_counts(&self) -> (usize, usize, usize, usize) {
        let d = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let (v, t) = self.m.vm.progress();
        (d.len(), d.iter().filter(|x| x.ai).count(), t - v, t)
    }
    fn build_submission(&self) -> Submission {
        let drafts = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        self.submission_of(drafts, self.summary.f.text.trim().to_string(), self.verdict.clone())
    }
    fn submission_of(&self, drafts: Vec<perusal_core::store::Draft>, mut summary: String, verdict: Verdict) -> Submission {
        let (mut comments, mut general) = (vec![], vec![]);
        for d in drafts {
            match (&d.file, d.start_line, d.end_line) {
                (Some(f), Some(a), Some(b)) => {
                    let (old, new) = (perusal_gitops::show(&self.m.repo, &self.m.base_sha, f).unwrap_or_default(), perusal_gitops::show(&self.m.repo, &self.m.branch, f).unwrap_or_default());
                    let (start_old, end_old) = (perusal_diff::text::old_line_for_new(&old, &new, a), perusal_diff::text::old_line_for_new(&old, &new, b));
                    // hosts reject inline comments on lines outside the diff ("Line could not be resolved"), so those go into the summary instead
                    let shown: HashSet<u32> = diff_lines(&old, &new, 3).iter().filter_map(|l| l.new_no).collect();
                    if self.m.mr.is_some() && !(shown.contains(&a) && shown.contains(&b)) {
                        let at = if a == b { format!("{f}:{a}") } else { format!("{f}:{a}-{b}") };
                        summary.push_str(&format!("\n\n**{at}**: {}", d.body));
                    } else {
                        comments.push(InlineComment { path: f.clone(), start_line: a, end_line: b, body: d.body, start_old, end_old })
                    }
                }
                (Some(f), _, _) => { summary.push_str(&format!("\n\n**{f}**: {}", d.body)); }
                _ => general.push(d.body),
            }
        }
        Submission { head_sha: self.m.branch.clone(), summary: summary.trim().to_string(), verdict, comments, general }
    }
    /// True when comments go straight to the host and Finish review is not shown (remote reviews only).
    /// Name to draw an avatar under: the signed-in user's login for "You", the name itself otherwise.
    fn who(&self, name: &str) -> String { if name == "You" { self.m.mr.as_ref().and_then(|r| crate::avatars::me(&r.host)).unwrap_or_else(|| name.to_string()) } else { name.to_string() } }
    fn instant(&self) -> bool { self.settings.publish_instantly && self.m.mr.is_some() }
    /// Posts one local draft to the host now. The draft is only deleted once the host accepted it.
    fn publish_draft(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(mr) = self.m.mr.clone() else { return };
        let Some(d) = self.m.store.drafts(self.m.vm.review).unwrap_or_default().into_iter().find(|d| d.id == id) else { return };
        let sub = self.submission_of(vec![d], String::new(), Verdict::Comment);
        let task = cx.background_executor().spawn(async move { model::submit_review(&mr, &sub) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |t, cx| {
                match r {
                    Ok(()) => { let _ = t.m.store.delete_draft(id); t.note(NKind::Success, "Comment published.", None); t.resync(); }
                    Err(e) => t.note(NKind::Error, format!("Could not publish the comment (kept as a draft): {e}"), None),
                }
                t.flush(cx); cx.notify();
            });
        }).detach();
    }
    fn start_submit(&mut self, cx: &mut Context<Self>) {
        let Some(mr) = self.m.mr.clone() else { return };
        if self.submit_busy { return; }
        let sub = self.build_submission();
        if sub.comments.is_empty() && sub.summary.is_empty() && sub.verdict == Verdict::Comment { self.submit_msg = Some((false, "Add a summary or at least one comment, or choose Approve or Request changes.".into())); return; }
        self.submit_busy = true; self.submit_msg = None;
        let task = cx.background_executor().spawn(async move { model::submit_review(&mr, &sub) });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| t.finish_submit(r, cx)); }).detach();
        cx.notify();
    }
    fn finish_submit(&mut self, r: Result<(), String>, cx: &mut Context<Self>) {
        self.submit_busy = false;
        match r {
            Ok(()) => {
                for d in self.m.store.drafts(self.m.vm.review).unwrap_or_default() { let _ = self.m.store.delete_draft(d.id); }
                let _ = self.m.store.set_state(self.m.vm.review, "submitted"); self.resync();
                self.summary.f.clear(); self.finish_open = false; self.focus = None; self.note(NKind::Success, "Review posted to the host. Your drafts were cleared.", Some("posted"));
            }
            Err(e) => self.submit_msg = Some((false, e)),
        }
        self.flush(cx); cx.notify();
    }
    fn export_local(&mut self, cx: &mut Context<Self>) {
        match self.m.store.export_markdown(self.m.vm.review).map_err(|e| e.to_string()).and_then(|md| { cx.write_to_clipboard(gpui::ClipboardItem::new_string(md.clone())); model::export_markdown_file(&self.m.title, &md) }) {
            Ok(p) => { let _ = self.m.store.set_state(self.m.vm.review, "submitted"); self.submit_msg = Some((true, format!("Copied to the clipboard and saved to {}", p.display()))) }
            Err(e) => self.submit_msg = Some((false, e)),
        }
    }

    // ---------- keys
    pub fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke; let key = ks.key.as_str();
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        if ctrl && key == "f" { self.open_find(cx); return; }
        if self.finish_open && self.focus != Some(Fid::Summary) { if key == "escape" { self.finish_open = false; cx.notify(); } return; }
        if let Some(f) = self.focus {
            match f {
                Fid::Filter | Fid::Sort => {
                    let is_sort = f == Fid::Sort; self.sug_for = f;
                    match key {
                        "escape" => { if self.sug.is_empty() { self.blur(cx); } else { self.sug.clear(); } }
                        "enter" => { if let Some(i) = self.sug_idx { self.accept_suggestion(i); } else if is_sort { self.apply_sort(); } else { self.apply_filter(); } }
                        "tab" => { if !self.sug.is_empty() { self.accept_suggestion(self.sug_idx.unwrap_or(0)); } else { self.focus = Some(if is_sort { Fid::Filter } else { Fid::Sort }); self.sug_for = self.focus.unwrap(); self.update_suggestions(); } }
                        "down" => { if !self.sug.is_empty() { self.sug_idx = Some(self.sug_idx.map(|i| (i + 1) % self.sug.len()).unwrap_or(0)); } }
                        "up" => { if !self.sug.is_empty() { self.sug_idx = Some(self.sug_idx.map(|i| (i + self.sug.len() - 1) % self.sug.len()).unwrap_or(self.sug.len() - 1)); } }
                        _ => { let i = if is_sort { &mut self.sort } else { &mut self.filter }; if field_key(&mut i.f, ks, cx) { self.update_suggestions(); } }
                    }
                }
                Fid::Comment => { if key == "escape" { self.composer = None; self.focus = None; } else if key == "enter" && ctrl { self.save_composer(cx); } else { field_key(&mut self.comment.f, ks, cx); } }
                Fid::Ask => { if key == "escape" { self.focus = None; } else if key == "enter" { let q = self.ask.f.text.trim().to_string(); if !q.is_empty() { self.ask.f.clear(); self.start_ai(Job::Chat, q, cx); } } else { field_key(&mut self.ask.f, ks, cx); } }
                Fid::Summary => { if key == "escape" { self.finish_open = false; self.focus = None; } else if key == "enter" && ctrl { self.start_submit(cx); } else { field_key(&mut self.summary.f, ks, cx); } }
                Fid::Preset => { if key == "escape" { self.presets_open = false; self.focus = None; } else if key == "enter" { self.save_preset(true); } else { field_key(&mut self.preset_name.f, ks, cx); } }
                Fid::Find => match key {
                    "escape" => self.close_find(),
                    "enter" | "f3" => self.find_step(!ks.modifiers.shift),
                    _ => { if field_key(&mut self.find_in.f, ks, cx) { self.find_cur = 0; self.recompute_find(); self.scroll_to_hit(); } }
                },
                Fid::Gen => { if key == "escape" { self.gen_for = None; self.focus = None; } else if key == "enter" { self.start_gen(cx); } else { field_key(&mut self.gen.f, ks, cx); } }
                Fid::Rename => { if key == "escape" { let t = self.m.title.clone(); self.rename.f.set(&t); self.focus = None; } else if key == "enter" { self.commit_rename(cx); self.focus = None; } else { field_key(&mut self.rename.f, ks, cx); } }
            }
            self.flush(cx); cx.notify(); return;
        }
        if ctrl { match key { "=" | "+" => self.zoom = theme::zoom(self.zoom, 1), "-" => self.zoom = theme::zoom(self.zoom, -1), "0" => self.zoom = theme::zoom(self.zoom, 0), _ => {} } cx.notify(); return; }
        // pages: O switches; any code-navigation key from the overview goes to the code first
        if key == "o" && self.info.is_some() { self.page = if self.page == Page::Overview { Page::Code } else { Page::Overview }; cx.notify(); return; }
        if self.page == Page::Overview && matches!(key, "n" | "p" | "enter" | "backspace" | "v" | "u" | "m" | "x") { self.page = Page::Code; }
        match key {
            "escape" => { if self.has_selection() || !self.multi.is_empty() { self.clear_selection(); self.multi.clear(); } else if self.composer.is_some() { self.composer = None; } else if self.busy() { self.stop_ai(); } else { cx.emit(ViewEvent::Home); return; } }
            "/" => { self.focus = Some(Fid::Filter); self.sug_for = Fid::Filter; self.update_suggestions(); }
            "c" => { self.open_composer(self.page == Page::Overview && self.info.is_some(), None); }
            "a" => { self.ai_ctx = self.selection(); self.tab = RightTab::Ai; self.right_open = true; self.focus = Some(Fid::Ask); }
            "s" => { cx.emit(ViewEvent::OpenSettings); }
            "f" if !self.instant() => { self.finish_open = true; self.focus = Some(Fid::Summary); self.submit_msg = None; }
            "m" => { if self.m.vm.current_batch().iter().any(|&i| markdown::is_markdown(&self.m.vm.files[i].path)) { self.md_rendered = !self.md_rendered; self.shown = None; self.clear_selection(); } }
            "u" => { let m = if self.diff_mode == DiffMode::Unified { DiffMode::Split } else { DiffMode::Unified }; self.set_diff_mode(m); }
            "v" => self.toggle_viewed(), "enter" => self.next(), "backspace" => self.back(),
            "t" => self.toggle_tree(), "n" => self.step(1), "p" => self.step(-1), "x" => self.clear_scope(),
            "g" => { self.settings.group_related = !self.settings.group_related; self.save_settings(); self.m.vm.grouping = self.settings.group_related; let _ = self.m.vm.refresh(); self.reload(); }
            _ => {}
        }
        self.flush(cx); cx.notify();
    }
}

fn ago(i: Instant) -> String { let s = i.elapsed().as_secs(); if s < 60 { format!("{s}s") } else { format!("{}m {}s", s / 60, s % 60) } }

impl Render for View {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settings.follow_os {
            let dark = matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark);
            let want = self.settings.effective_theme(dark).to_string();
            if self.theme.name != theme::load(&want, Some(&model::themes_dir())).name { self.theme = theme::load(&want, Some(&model::themes_dir())); self.pal = Pal::new(&self.theme); self.shown = None; }
        }
        self.flush(cx);
        self.load_diff();
        if self.tree { self.trows = tree::build_rows(&self.m.vm.files, &self.m.vm.visible, &self.collapsed); }
        let p = self.pal; let z = self.zoom;
        let (viewed, total) = self.m.vm.progress();
        let frac = if total == 0 { 0.0 } else { viewed as f32 / total as f32 };
        let drafts = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let ndrafts = drafts.len();

        let top = self.top_bar(cx, ndrafts);
        let progress = div().flex().items_center().gap_3().w_full().px_4().py(px(8.0)).bg(p.bg).border_b_1().border_color(p.border)
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).flex_none().child(format!("{viewed} / {total} files viewed")))
            .child(div().flex_1().h(px(8.0)).rounded_full().bg(p.border).child(div().h_full().rounded_full().bg(p.success).w(relative(frac))))
            .child(div().text_sm().text_color(p.muted).flex_none().child(format!("{}%", (frac * 100.0).round() as u32)))
            .child(div().text_sm().text_color(p.muted).flex_none().child(format!("Batch {} of {}", self.m.vm.cursor + 1, self.m.vm.batch_count().max(1))));
        let filter = self.filter_row(cx);
        let left = if self.left_open { self.files_panel(cx).into_any_element() } else { self.rail(Side::Left, cx).into_any_element() };
        let center = self.center(cx);
        let right = if self.right_open { self.right_panel(cx, &drafts).into_any_element() } else { self.rail(Side::Right, cx).into_any_element() };
        let (lw, rw) = (self.left_w * z, self.right_w * z);

        let footer = div().flex().items_center().gap_4().px_4().py(px(5.0)).bg(p.surface).border_t_1().border_color(p.border).text_xs().text_color(p.muted)
            .children([("n / p", "file"), ("v", "viewed"), ("Enter", "next"), ("/", "filter"), ("c", "comment"), ("a", "ask AI"), ("f", "finish"), ("u", "split"), ("m", "markdown"), ("t", "tree"), ("g", "group"), ("o", "overview"), ("s", "settings"), ("Esc", "home")]
                .map(|(k, l)| div().flex().items_center().gap_1().child(kbd(k, &p)).child(l)));
        let overlay = self.finish_open.then(|| self.finish_overlay(cx));

        let side = |w: f32, open: bool, s: Side, cx: &mut Context<Self>, el: AnyElement| {
            let col = div().h_full().flex().flex_col().flex_none().bg(p.surface).overflow_hidden().child(el);
            let col = if open { col.w(px(w)) } else { col.w(px(38.0)) };
            let handle = open.then(|| div().id(if s == Side::Left { "rz-l" } else { "rz-r" }).w(px(5.0)).h_full().flex_none().cursor_col_resize().bg(p.border).hover(move |x| x.bg(p.accent.opacity(0.6)))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| { this.resizing = Some(s); cx.stop_propagation(); cx.notify(); })));
            (col, handle)
        };
        let (lcol, lhandle) = side(lw, self.left_open, Side::Left, cx, left);
        let (rcol, rhandle) = side(rw, self.right_open, Side::Right, cx, right);

        div().relative().size_full().flex().flex_col().bg(p.bg).text_color(p.fg).text_size(px(14.0 * z))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.blur(cx); cx.notify(); }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.dragging = false; for i in [&mut this.filter, &mut this.sort, &mut this.comment, &mut this.ask, &mut this.summary, &mut this.preset_name, &mut this.rename, &mut this.gen] { i.dragging = false; }
                if this.sb_drag.take().is_some() { this.list.scrollbar_drag_ended(); }
                this.hb_drag = None;
                if this.resizing.take().is_some() { this.settings.left_width = this.left_w; this.settings.right_width = this.right_w; this.save_settings(); }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, w, cx| {
                if ev.pressed_button != Some(MouseButton::Left) { return; }
                let zoom = this.zoom; let vw: f32 = w.viewport_size().width.into(); let x: f32 = ev.position.x.into(); let y: f32 = ev.position.y.into();
                match this.resizing { Some(Side::Left) => { this.left_w = (x / zoom).clamp(200.0, 700.0); cx.notify(); } Some(Side::Right) => { this.right_w = ((vw - x) / zoom).clamp(220.0, 700.0); cx.notify(); } None => {} }
                if let (Some(grab), Some((ox, tw, max))) = (this.hb_drag, this.hbar_geom()) {
                    let thumb = (tw * tw / (tw + max)).max(28.0); let travel = (tw - thumb).max(1.0);
                    this.hx = ((x - ox - grab) / travel).clamp(0.0, 1.0) * max; cx.notify();
                }
                if let Some(grab) = this.sb_drag {
                    let vp = this.list.viewport_bounds(); let vh: f32 = vp.size.height.into(); let oy: f32 = vp.origin.y.into(); let max: f32 = this.list.max_offset_for_scrollbar().y.into();
                    let thumb = (vh * vh / (vh + max)).max(28.0); let travel = (vh - thumb).max(1.0);
                    let frac = ((y - oy - grab) / travel).clamp(0.0, 1.0); this.list.set_offset_from_scrollbar(point(px(0.0), px(-frac * max))); cx.notify();
                }
            }))
            .child(top).child(progress).child(filter)
            .child(div().flex().flex_1().min_h_0().child(lcol).children(lhandle).child(center).children(rhandle).child(rcol))
            .child(footer)
            .children(overlay)
    }
}

impl View {
    fn winp(&self, id: Fid, el: gpui::Stateful<gpui::Div>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let (ui, mono) = (self.fonts.ui.clone(), self.fonts.mono.clone());
        match id {
            Fid::Filter => wire_input(el, cx, |t| &mut t.filter, |t, _| { t.focus = Some(Fid::Filter); t.sug_for = Fid::Filter; t.update_suggestions(); }, mono, false),
            Fid::Sort => wire_input(el, cx, |t| &mut t.sort, |t, _| { t.focus = Some(Fid::Sort); t.sug_for = Fid::Sort; t.update_suggestions(); }, mono, false),
            Fid::Comment => wire_input(el, cx, |t| &mut t.comment, |t, _| { t.focus = Some(Fid::Comment); }, ui, false),
            Fid::Ask => wire_input(el, cx, |t| &mut t.ask, |t, _| { if t.ai_ctx.is_none() { t.ai_ctx = t.selection(); } t.focus = Some(Fid::Ask); }, ui, false),
            Fid::Summary => wire_input(el, cx, |t| &mut t.summary, |t, _| { t.focus = Some(Fid::Summary); }, ui, false),
            Fid::Preset => wire_input(el, cx, |t| &mut t.preset_name, |t, _| { t.focus = Some(Fid::Preset); }, ui, false),
            Fid::Gen => wire_input(el, cx, |t| &mut t.gen, |t, _| { t.focus = Some(Fid::Gen); }, ui, false),
            Fid::Rename => wire_input(el, cx, |t| &mut t.rename, |t, _| { t.focus = Some(Fid::Rename); }, ui, false),
            Fid::Find => wire_input(el, cx, |t| &mut t.find_in, |t, _| { t.focus = Some(Fid::Find); }, ui, false),
        }
    }
    fn input_of(&self, id: Fid) -> &Input { match id { Fid::Filter => &self.filter, Fid::Sort => &self.sort, Fid::Comment => &self.comment, Fid::Ask => &self.ask, Fid::Summary => &self.summary, Fid::Preset => &self.preset_name, Fid::Rename => &self.rename, Fid::Gen => &self.gen, Fid::Find => &self.find_in } }
    fn rail(&mut self, s: Side, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let (i, t) = if s == Side::Left { (ic::CHEVRON_RIGHT, "Show the files panel") } else { (ic::CHEVRON_LEFT, "Show the review panel") };
        div().flex().flex_col().items_center().pt_2().size_full().bg(p.surface).child(icon_btn(if s == Side::Left { "open-l" } else { "open-r" }, i, t, Btn::Ghost, &p)
            .on_click(cx.listener(move |this, _, _, cx| { if s == Side::Left { this.left_open = true; this.settings.left_open = true; } else { this.right_open = true; this.settings.right_open = true; } this.save_settings(); cx.notify(); })))
    }

    fn top_bar(&mut self, cx: &mut Context<Self>, ndrafts: usize) -> impl IntoElement {
        let p = self.pal;
        let reviewing = matches!(self.ai_job, Some(Job::Review));
        let renaming = self.focus == Some(Fid::Rename);
        let title: AnyElement = if renaming {
            self.winp(Fid::Rename, div().id("rename-box").flex().items_center().px_3().py(px(4.0)).w(px(360.0)).rounded_md().border_1().border_color(p.accent).bg(p.bg).child(input_text(&self.rename, true, "Name this review", &p, false, 1)), cx).into_any_element()
        } else {
            div().flex().items_center().gap_1().min_w_0().child(div().id("title-text").text_lg().font_weight(FontWeight::SEMIBOLD).truncate().min_w_0().px_2().py(px(2.0)).rounded_md().border_1().border_color(gpui::transparent_black()).cursor_text().hover(move |x| x.bg(p.hover).border_color(p.accent)).child(SharedString::from(self.m.title.clone()))
                .on_click(cx.listener(|this, _, _, cx| { let t = this.m.title.clone(); this.rename.f.set(&t); this.focus = Some(Fid::Rename); cx.stop_propagation(); cx.notify(); })))
                .child(icon_btn("rename", ic::EDIT, "Rename this review", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { let t = this.m.title.clone(); this.rename.f.set(&t); this.focus = Some(Fid::Rename); cx.stop_propagation(); cx.notify(); }))).into_any_element()
        };
        let ci = ci_badge(&self.ci, &p).map(|b| b.on_click(cx.listener(|this, _, _, _| { if let Some(u) = this.ci_url() { let _ = model::open_url(&u); } })));
        let local_mr = self.local_mr.clone().map(|u| icon_text_btn("open-mr", ic::OPEN_IN_NEW, "Open the PR", Btn::Secondary, &p).on_click(cx.listener(move |_, _, _, _| { let _ = model::open_url(&u); })));
        div().flex().items_center().gap_3().px_4().h(px(52.0)).bg(p.surface).border_b_1().border_color(p.border)
            .child(icon_btn("home", ic::HOME, "Back to Home (Esc)", Btn::Ghost, &p).on_click(cx.listener(|_, _, _, cx| cx.emit(ViewEvent::Home))))
            .child(title)
            .children(self.m.remote.clone().map(|u| { let label = u.trim_start_matches("https://").to_string(); let hv = p.hover; let t = tip(div().id("mr-link").flex().items_center().gap_1().px_2().py(px(1.0)).rounded_full().text_xs().border_1().border_color(p.border).text_color(p.accent).cursor_pointer().hover(move |x| x.bg(hv)).child(label).child(icon(ic::OPEN_IN_NEW, 12.0, p.accent)), "Open on the host in your browser", &p); t.on_click(cx.listener(move |_, _, _, _| { let _ = model::open_url(&u); })) }))
            .children(Some(self.m.mr_state.clone()).filter(|s| !s.is_empty()).map(|s| { let (c, l) = match s.as_str() { "merged" => (p.ai, "Merged"), "closed" => (p.danger, "Closed"), _ => (p.success, "Open") }; badge(l, c, c.opacity(0.15)) }))
            .children(ci).children(local_mr)
            .child(div().flex_1())
            .when(ndrafts > 0, |d| d.child(badge(format!("{ndrafts} draft{}", if ndrafts == 1 { "" } else { "s" }), p.accent, p.accent.opacity(0.15))))
            .child(if reviewing { icon_text_btn("ai-review", ic::STOP, "Stop AI review", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.stop_ai(); cx.notify(); })).into_any_element() }
                else { icon_text_btn("ai-review", ic::AUTO_AWESOME, "AI review", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.tab = RightTab::Ai; this.right_open = true; this.start_ai(Job::Review, String::new(), cx); cx.notify(); })).into_any_element() })
            .child(icon_btn("settings", ic::SETTINGS, "Settings (s)", Btn::Secondary, &p).on_click(cx.listener(|_, _, _, cx| cx.emit(ViewEvent::OpenSettings))))
    }

    fn filter_row(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let on_filter = self.focus == Some(Fid::Filter);
        let mkbox = |this: &Self, id: Fid, icon_i: ic::Icon, ph: &str, focused: bool, tipt: &str, cx: &mut Context<Self>| {
            let inp = this.input_of(id); let bad = this.error.as_deref().map(|e| (id == Fid::Filter) == e.starts_with("Filter")).unwrap_or(false);
            let el = div().id(if id == Fid::Filter { "filter" } else { "sort" }).flex().items_center().gap_2().px_3().py(px(6.0)).flex_1().min_w_0().rounded_md().border_1().border_color(if focused { p.accent } else if bad { p.danger } else { p.border }).bg(p.surface)
                .child(tip(div().id(if id == Fid::Filter { "fi" } else { "si" }).child(icon(icon_i, 18.0, p.muted)), tipt.to_string(), &p))
                .child(input_text(inp, focused, ph, &p, false, 1).font_family(this.fonts.mono.clone()))
                .when(!inp.f.is_empty(), |d| d.child(tip(div().id(if id == Fid::Filter { "clr-f" } else { "clr-s" }).cursor_pointer().child(icon(ic::CLOSE, 16.0, p.muted)).on_click(cx.listener(move |this, _, _, cx| {
                    if id == Fid::Filter { this.filter.f.clear(); this.apply_filter(); } else { this.sort.f.clear(); this.apply_sort(); } cx.stop_propagation(); cx.notify(); })), "Clear", &p)));
            this.winp(id, el, cx)
        };
        let fbox = mkbox(self, Fid::Filter, ic::SEARCH, "Filter: additions > 50 && !viewed  ·  ai, comments > 0, status == \"added\"", on_filter, "Filter files with an expression", cx);
        let tiers = perusal_filter::parse_tiers(&self.m.vm.sort);
        let sort_label = if tiers.is_empty() { "Sort".to_string() } else { format!("Sort · {}", tiers.len()) };
        let sort_btn = icon_text_btn("sort-btn", ic::SORT, sort_label, if self.sort_open || !tiers.is_empty() { Btn::Secondary } else { Btn::Ghost }, &p)
            .on_click(cx.listener(|this, _, _, cx| { this.sort_open = !this.sort_open; this.presets_open = false; this.gen_for = None; this.sug.clear(); cx.stop_propagation(); cx.notify(); }));
        let ws_on = self.m.vm.hide_whitespace;
        let ws_btn = icon_btn("ws-btn", ic::SPACE_BAR, if ws_on { "Whitespace-only changes are hidden. Click to show them." } else { "Hide whitespace-only changes" }, if ws_on { Btn::Secondary } else { Btn::Ghost }, &p)
            .on_click(cx.listener(|this, _, _, cx| { this.m.vm.hide_whitespace = !this.m.vm.hide_whitespace; let _ = this.m.vm.refresh(); this.shown = None; this.reload(); cx.stop_propagation(); cx.notify(); }));
        let ai_filter = icon_btn("gen-filter", ic::AUTO_AWESOME, "Generate a filter with AI", if self.gen_for == Some(perusal_agents::GenKind::Filter) { Btn::Secondary } else { Btn::Ghost }, &p)
            .on_click(cx.listener(|this, _, _, cx| { this.gen_for = if this.gen_for == Some(perusal_agents::GenKind::Filter) { None } else { Some(perusal_agents::GenKind::Filter) }; this.sort_open = false; this.presets_open = false; this.gen_msg = None; this.focus = Some(Fid::Gen); cx.stop_propagation(); cx.notify(); }));
        let sort_pop = self.sort_open.then(|| {
            let n = tiers.len();
            let rows: Vec<_> = tiers.iter().enumerate().map(|(i, (e, desc))| {
                let (t1, t2, t3, t4) = (tiers.clone(), tiers.clone(), tiers.clone(), tiers.clone());
                div().flex().items_center().gap_1().px_3().py(px(4.0))
                    .child(div().w(px(18.0)).text_xs().text_color(p.muted).child(format!("{}", i + 1)))
                    .child(div().flex_1().min_w_0().truncate().font_family(self.fonts.mono.clone()).text_sm().child(e.clone()))
                    .child(icon_text_btn(("t-dir", i), if *desc { ic::ARROW_DOWNWARD } else { ic::ARROW_UPWARD }, if *desc { "Descending" } else { "Ascending" }, Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let mut t = t1.clone(); t[i].1 = !t[i].1; this.set_tiers(t); cx.stop_propagation(); cx.notify(); })))
                    .child(icon_btn(("t-up", i), ic::KEYBOARD_ARROW_UP, "Move up (higher priority)", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let mut t = t2.clone(); if i > 0 { t.swap(i, i - 1); this.set_tiers(t); } cx.stop_propagation(); cx.notify(); })))
                    .child(icon_btn(("t-dn", i), ic::KEYBOARD_ARROW_DOWN, "Move down", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let mut t = t3.clone(); if i + 1 < t.len() { t.swap(i, i + 1); this.set_tiers(t); } cx.stop_propagation(); cx.notify(); })))
                    .child(icon_btn(("t-rm", i), ic::DELETE, "Remove this tier", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let mut t = t4.clone(); t.remove(i); this.set_tiers(t); cx.stop_propagation(); cx.notify(); })))
            }).collect();
            let fields = ["changed", "additions", "deletions", "net", "meaningful", "name", "path", "extension", "status", "size", "comments", "unresolved", "viewed", "whitespace"];
            let chips: Vec<_> = fields.iter().enumerate().map(|(i, f)| {
                let (t, f) = (tiers.clone(), f.to_string()); let numeric = !matches!(f.as_str(), "name" | "path" | "extension" | "status" | "viewed" | "whitespace");
                div().id(("t-add", i)).px_2().py(px(2.0)).rounded_md().border_1().border_color(p.border).cursor_pointer().text_xs().font_family(self.fonts.mono.clone()).hover(move |x| x.bg(p.hover)).child(f.clone())
                    .on_click(cx.listener(move |this, _, _, cx| { let mut t = t.clone(); t.push((f.clone(), numeric)); this.set_tiers(t); cx.stop_propagation(); cx.notify(); }))
            }).collect();
            let on = self.focus == Some(Fid::Sort);
            let sug_items: Vec<_> = if on { self.sug.iter().enumerate().map(|(i, s)| { let sel = self.sug_idx == Some(i);
                div().id(("ssug", i)).flex().items_center().gap_3().px_3().py(px(3.0)).cursor_pointer().when(sel, |d| d.bg(p.accent.opacity(0.18))).hover(move |x| x.bg(p.hover))
                    .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation())).on_click(cx.listener(move |this, _, _, cx| { this.accept_suggestion(i); cx.notify(); }))
                    .child(div().font_family(self.fonts.mono.clone()).text_sm().child(s.label.clone())).child(div().flex_1().text_xs().text_color(p.muted).truncate().child(s.detail)) }).collect() } else { vec![] };
            deferred(div().absolute().top(px(42.0)).right(px(48.0)).w(px(480.0)).rounded_md().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(div().px_3().py_2().border_b_1().border_color(p.border).flex().items_center().gap_2().child(div().flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Sort files")).child(div().text_xs().text_color(p.muted).child("Tier 1 wins; later tiers break ties"))
                    .when(n > 0, |d| d.child(icon_text_btn("t-clear", ic::CLOSE, "Clear", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.set_tiers(vec![]); cx.stop_propagation(); cx.notify(); })))))
                .child(div().flex().flex_col().py_1().when(n == 0, |d| d.child(div().px_3().py_2().text_sm().text_color(p.muted).child("No sorting. Add a tier below."))).children(rows))
                .child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border)
                    .child(div().text_xs().text_color(p.muted).child("Add a tier"))
                    .child(div().flex().flex_wrap().gap_1().children(chips))
                    .child(div().text_xs().text_color(p.muted).child("Or write the whole sort as an expression"))
                    .child(self.winp(Fid::Sort, div().id("sort-expr").flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if on { p.accent } else { p.border }).bg(p.bg).font_family(self.fonts.mono.clone()).child(input_text(&self.sort, on, "additions + deletions desc, name asc", &p, false, 1)), cx))
                    .children(sug_items))
                .child(self.gen_box(perusal_agents::GenKind::Sort, "e.g. biggest files first, then alphabetical", cx))).with_priority(15)
        });
        let filter_pop = (self.gen_for == Some(perusal_agents::GenKind::Filter)).then(|| deferred(div().absolute().top(px(42.0)).left_0().w(px(460.0)).rounded_md().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(self.gen_box(perusal_agents::GenKind::Filter, "e.g. big files I have not viewed yet", cx).border_t_0())).with_priority(15));
        let dropdown = (on_filter && !self.sug.is_empty()).then(|| {
            let items: Vec<_> = self.sug.iter().enumerate().map(|(i, s)| {
                let on = self.sug_idx == Some(i);
                div().id(("sug", i)).flex().items_center().gap_3().px_3().py(px(5.0)).cursor_pointer().when(on, |d| d.bg(p.accent.opacity(0.18))).hover(move |x| x.bg(p.hover))
                    .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .on_click(cx.listener(move |this, _, _, cx| { this.accept_suggestion(i); cx.notify(); }))
                    .child(div().font_family(self.fonts.mono.clone()).text_sm().font_weight(FontWeight::MEDIUM).child(s.label.clone()))
                    .child(div().flex_1().text_xs().text_color(p.muted).truncate().child(s.detail))
                    .child(badge(s.kind, p.muted, p.hover))
            }).collect();
            let left = relative(0.0);
            deferred(div().absolute().top(px(42.0)).left(left).w(relative(0.5)).min_w(px(380.0)).rounded_md().border_1().border_color(p.border).bg(p.surface).shadow_lg().py_1().occlude().children(items)
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(div().px_3().py_1().text_xs().text_color(p.muted).child("Up/Down to choose, Tab to accept, Enter to apply"))).with_priority(10)
        });
        // presets menu
        let presets_menu = self.presets_open.then(|| {
            let mut items: Vec<AnyElement> = vec![];
            let add = |scope_label: &'static str, project: bool, map: Option<&perusal_core::presets::Scope>, items: &mut Vec<AnyElement>, cx: &mut Context<Self>| {
                let Some(sc) = map else { return };
                for (name, pr) in sc.presets.iter() {
                    let (n1, n2, n3) = (name.clone(), name.clone(), name.clone()); let pr2 = pr.clone(); let is_def = sc.default.as_deref() == Some(name.as_str());
                    items.push(div().id(SharedString::from(format!("pr-{scope_label}-{name}"))).flex().items_center().gap_2().px_3().py(px(6.0)).cursor_pointer().hover(move |x| x.bg(p.hover))
                        .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                        .on_click(cx.listener(move |this, _, _, cx| { this.apply_preset(&pr2); this.presets_open = false; cx.notify(); }))
                        .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!("{name}  ·  {scope_label}"))).child(div().text_xs().text_color(p.muted).truncate().child(format!("{}{}", if pr.filter.is_empty() { "no filter".to_string() } else { pr.filter.clone() }, if pr.sort.is_empty() { String::new() } else { format!("  |  sort {}", pr.sort) }))))
                        .child(icon_btn(SharedString::from(format!("prd-{scope_label}-{n1}")), if is_def { ic::STAR } else { ic::STAR_BORDER }, if is_def { "Default for new reviews (click to clear)" } else { "Make this the default" }, Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| {
                            let proj = if project { Some(this.project.clone()) } else { None }; let cur_default = if project { this.presets.projects.get(&perusal_core::presets::normalize_remote(&this.project)).and_then(|s| s.default.clone()) } else { this.presets.global.default.clone() };
                            this.presets.set_default(proj.as_deref(), if cur_default.as_deref() == Some(n1.as_str()) { None } else { Some(n1.as_str()) }); model::save_presets(&this.presets); cx.stop_propagation(); cx.notify(); })))
                        .child(icon_btn(SharedString::from(format!("prx-{scope_label}-{n2}")), ic::DELETE, "Delete this preset", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| {
                            let sc = if project { this.presets.projects.get_mut(&perusal_core::presets::normalize_remote(&this.project)) } else { Some(&mut this.presets.global) };
                            if let Some(sc) = sc { sc.presets.remove(&n2); if sc.default.as_deref() == Some(n3.as_str()) { sc.default = None; } } model::save_presets(&this.presets); cx.stop_propagation(); cx.notify(); }))).into_any_element());
                }
            };
            add("project", true, self.presets.projects.get(&perusal_core::presets::normalize_remote(&self.project)), &mut items, cx);
            add("global", false, Some(&self.presets.global), &mut items, cx);
            let empty = items.is_empty();
            deferred(div().absolute().top(px(42.0)).right_0().w(px(420.0)).rounded_md().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(div().px_3().py_2().border_b_1().border_color(p.border).text_sm().font_weight(FontWeight::SEMIBOLD).child("Filter and sort presets"))
                .child(div().max_h(px(280.0)).overflow_hidden().flex().flex_col().when(empty, |d| d.child(div().px_3().py_3().text_sm().text_color(p.muted).child("No presets yet. Save the current filter and sort below."))).children(items))
                .child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border)
                    .child(self.winp(Fid::Preset, div().id("pr-name").flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if self.focus == Some(Fid::Preset) { p.accent } else { p.border }).bg(p.bg).child(input_text(&self.preset_name, self.focus == Some(Fid::Preset), "Name this preset", &p, false, 1)), cx))
                    .child(div().flex().gap_2().child(icon_text_btn("pr-save-proj", ic::BOOKMARK, "Save for this project", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_preset(true); cx.stop_propagation(); cx.notify(); })))
                        .child(icon_text_btn("pr-save-glob", ic::BOOKMARK_BORDER, "Save globally", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_preset(false); cx.stop_propagation(); cx.notify(); })))))).with_priority(15)
        });
        let err = self.error.clone();
        div().flex().flex_col().w_full().px_4().py_2().bg(p.bg).border_b_1().border_color(p.border).gap_1()
            .child(div().relative().w_full().flex().items_center().gap_2().child(fbox).child(ai_filter).child(ws_btn).child(sort_btn)
                .child(icon_btn("presets", ic::BOOKMARK, "Filter and sort presets", if self.presets_open { Btn::Secondary } else { Btn::Ghost }, &p).on_click(cx.listener(|this, _, _, cx| { this.presets_open = !this.presets_open; this.sug.clear(); cx.stop_propagation(); cx.notify(); })))
                .children(dropdown).children(presets_menu).children(sort_pop).children(filter_pop))
            .children((self.tree && !self.m.vm.sort.is_empty()).then(|| div().flex().items_center().gap_2().text_xs().text_color(p.warn).child(icon(ic::WARNING, 14.0, p.warn)).child("Sorting does not apply while files are shown as a directory tree. Switch to the flat list to use it.")))
            .children(err.map(|e| div().text_xs().text_color(p.danger).truncate().child(e)))
    }

    fn files_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let n_files = self.m.vm.visible.len();
        let list_len = if self.tree { self.trows.len() } else { n_files };
        let list = uniform_list("files", list_len, cx.processor(move |this, range: Range<usize>, _w, cx| {
            let p = this.pal; let rh = px(30.0 * this.zoom);
            range.map(|row| {
                if this.tree {
                    if let TreeRow::Dir { path, name, depth, collapsed, total, viewed, additions, deletions } = this.trows[row].clone() {
                        let done = viewed == total; let tp = path.clone();
                        return div().id(row).flex().items_center().gap_2().pl(px(8.0 + 14.0 * depth as f32)).pr_3().h(rh).w_full().cursor_pointer().hover(move |s| s.bg(p.hover))
                            .on_click(cx.listener(move |this, _, _, cx| { this.toggle_dir(&tp); cx.notify(); }))
                            .child(icon(if collapsed { ic::CHEVRON_RIGHT } else { ic::EXPAND_MORE }, 16.0, p.muted))
                            .child(icon(if collapsed { ic::FOLDER } else { ic::FOLDER_OPEN }, 16.0, p.warn))
                            .child(div().min_w_0().flex_1().truncate().font_weight(FontWeight::SEMIBOLD).child(name))
                            .child(div().text_xs().flex_none().flex().items_center().gap_1().child(div().text_color(p.success).child(format!("+{additions}"))).child(div().text_color(p.danger).child(format!("−{deletions}"))).child(diff_squares(additions, deletions, &p, 6.0)))
                            .child(div().flex_none().px_2().rounded_full().text_xs().font_weight(FontWeight::MEDIUM).text_color(if done { p.success } else { p.muted }).bg(if done { p.success.opacity(0.15) } else { p.hover }).child(format!("{viewed}/{total}")));
                    }
                }
                let (fi, depth) = if this.tree { match this.trows[row] { TreeRow::File { fi, depth } => (fi, depth), _ => unreachable!() } } else { (this.m.vm.visible[row], 0) };
                let f = &this.m.vm.files[fi];
                let sel = this.m.vm.current_batch().contains(&fi); let picked = this.multi.contains(&fi); let grouped = this.m.vm.is_manually_grouped(fi);
                let (letter, c) = status_style(f.status, &p);
                let (dir, name) = if this.tree { (String::new(), f.name().to_string()) } else { match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) } };
                let (comments, ai_flag, symlink) = (f.comments, f.ai_flagged, f.symlink);
                div().id(row).flex().items_center().gap_2().pl(px(12.0 + if this.tree { 14.0 * depth as f32 + 14.0 } else { 0.0 })).pr_3().h(rh).w_full().cursor_pointer()
                    .border_l_2().border_color(if sel { p.accent } else if picked { p.warn } else { gpui::transparent_black() })
                    .when(sel, |d| d.bg(p.accent.opacity(0.12))).when(picked, |d| d.bg(p.warn.opacity(0.14))).hover(move |s| s.bg(p.hover))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| { this.click_file(fi, ev.modifiers()); cx.notify(); }))
                    .child(if symlink { icon(ic::LINK, 16.0, p.accent).into_any_element() } else { div().size(px(18.0)).rounded_sm().flex().items_center().justify_center().text_xs().font_weight(FontWeight::BOLD).text_color(c).bg(c.opacity(0.15)).flex_none().child(letter).into_any_element() })
                    .child(div().flex().flex_1().min_w_0().truncate().child(div().text_color(p.muted).child(dir)).child(div().text_color(if f.viewed { p.muted } else { p.fg }).font_weight(FontWeight::MEDIUM).when(symlink, |d| d.italic()).child(name)))
                    .when(symlink, |d| d.child(badge("symlink", p.accent, p.accent.opacity(0.15))))
                    .when(grouped, |d| d.child(icon(ic::LINK, 14.0, p.accent)))
                    .when(ai_flag, |d| d.child(icon(ic::AUTO_AWESOME, 14.0, p.ai)))
                    .when(comments > 0, |d| d.child(div().flex().items_center().gap_1().text_xs().text_color(p.muted).child(icon(ic::COMMENT, 13.0, p.muted)).child(format!("{comments}"))))
                    .when(f.noisy() && !symlink, |d| d.child(div().text_xs().text_color(p.muted).child("collapsed")))
                    .child(div().text_xs().flex_none().flex().items_center().gap_1().child(div().text_color(p.success).child(format!("+{}", f.additions))).child(div().text_color(p.danger).child(format!("−{}", f.deletions))).child(diff_squares(f.additions, f.deletions, &p, 6.0)))
                    .child(div().size(px(16.0)).flex_none().flex().items_center().justify_center().when(f.viewed, |d| d.rounded_full().bg(p.success).child(icon(ic::CHECK, 12.0, gpui::white()))))
            }).collect()
        })).flex_1().min_h_0().w_full();
        let tree_on = self.tree;
        let header = div().flex().items_center().gap_2().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Files")).child(badge(format!("{n_files}"), p.muted, p.hover)).child(div().flex_1())
            .when(tree_on, |d| d
                .child(icon_btn("exp-all", ic::UNFOLD_MORE, "Expand all folders", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.collapsed.clear(); cx.notify(); })))
                .child(icon_btn("col-all", ic::UNFOLD_LESS, "Collapse all folders", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.collapsed = tree::all_dirs(&this.m.vm.files, &this.m.vm.visible).into_iter().collect(); cx.notify(); }))))
            .child(seg_box(&p).child(seg_icon("seg-list", ic::VIEW_LIST, "Flat list", !tree_on, &p).on_click(cx.listener(|this, _, _, cx| { if this.tree { this.toggle_tree(); } cx.notify(); })))
                .child(seg_icon("seg-tree", ic::ACCOUNT_TREE, "Directory tree (t)", tree_on, &p).on_click(cx.listener(|this, _, _, cx| { if !this.tree { this.toggle_tree(); } cx.notify(); }))))
            .child(icon_btn("close-l", ic::CHEVRON_LEFT, "Hide the files panel", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.left_open = false; this.settings.left_open = false; this.save_settings(); cx.notify(); })));
        let n = self.multi.len();
        let bar = (n > 0).then(|| div().flex().items_center().gap_2().px_3().py(px(6.0)).bg(p.warn.opacity(0.12)).border_b_1().border_color(p.border)
            .child(div().text_sm().flex_1().child(format!("{n} file{} selected", if n == 1 { "" } else { "s" })))
            .child(if n >= 2 { icon_text_btn("grp-multi", ic::LINK, "Review together", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.group_multi(); cx.notify(); })) }
                else { tip(icon_text_btn("grp-multi", ic::LINK, "Review together", Btn::Primary, &p).opacity(0.4).cursor_default(), "Select at least two files to review them together", &p) })
            .child(icon_btn("clr-multi", ic::CLOSE, "Clear selection", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.multi.clear(); cx.notify(); }))));
        let hint = (n == 0 && n_files > 1 && self.m.vm.commit_scope.is_none()).then(|| div().px_3().py(px(4.0)).text_xs().text_color(p.muted).border_b_1().border_color(p.border).child("Ctrl+click or Shift+click files to review them together"));
        let scope = self.m.graph.selection.filter(|_| self.m.vm.commit_scope.is_some()).and_then(|(a, _)| self.m.graph.rows.get(a).cloned()).map(|r| div().flex().items_center().gap_2().px_3().py(px(6.0)).bg(p.accent.opacity(0.12)).border_b_1().border_color(p.border)
            .child(icon(ic::COMMIT, 17.0, p.accent)).child(div().flex_1().min_w_0().truncate().text_sm().child(format!("Only commit {} · {} file{}", &r.sha[..7.min(r.sha.len())], n_files, if n_files == 1 { "" } else { "s" })))
            .child(icon_btn("scope-x", ic::CLOSE, "Show all commits (x)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.clear_scope(); cx.notify(); }))));
        div().flex().flex_col().size_full().child(header).children(scope).children(bar).children(hint).child(list)
    }

    /// One row of the diff list (a code line, a hunk marker, a note or a comment card).
    fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(r) = self.rows.get(ix).cloned() else { return div().into_any_element() };
        let p = self.pal; let h = px(20.0 * self.zoom); let fsz_f = 13.0 * self.zoom; let fsz = px(fsz_f); let mono = self.fonts.mono.clone();
        let fi = r.fi; let st = self.m.vm.files[fi].status;
        let selected = r.ord.map(|o| self.in_sel(o)).unwrap_or(false);
        let base = div().id(ix).flex().items_center().h(h).w_full().font_family(mono.clone()).text_size(fsz).text_color(p.fg);
        let hx = self.hx; let hxt = self.hx_text.clone();
        let found = self.hits_in_row(ix);
        let code = |rl: &RLine, part: usize| -> gpui::Div {
            let w = rl.line.text.chars().count() as f32 * fsz_f * 0.6; if w > hxt.get() { hxt.set(w); }
            let mut runs = rl.runs.clone();
            let extra: Vec<(Range<usize>, HighlightStyle)> = found.iter().filter(|h| h.0 == part && h.1.end <= rl.line.text.len())
                .map(|h| (h.1.clone(), HighlightStyle { background_color: Some(if h.2 { p.accent.opacity(0.8) } else { p.warn.opacity(0.5) }), color: Some(p.fg), ..Default::default() })).collect();
            if !extra.is_empty() { runs = gpui::combine_highlights(runs, extra).collect(); }
            div().flex_1().min_w_0().h_full().overflow_hidden().child(div().flex_none().relative().left(px(-hx)).whitespace_nowrap().child(StyledText::new(SharedString::from(rl.line.text.clone())).with_highlights(runs)))
        };
        let cell = |side: Option<&RLine>, left: bool| -> gpui::Div {
            let Some(rl) = side else { return div().flex_1().h_full().bg(if selected { p.accent.opacity(0.22) } else { p.hover.opacity(0.4) }) };
            let l = &rl.line;
            let (bg, sign, gut) = match l.kind { Kind::Added => (p.added, "+", p.added_word.opacity(0.55)), Kind::Removed => (p.removed, "−", p.removed_word.opacity(0.55)), Kind::Context => (gpui::transparent_black(), " ", gpui::transparent_black()) };
            let no = if left { l.old_no } else { l.new_no };
            div().flex().items_center().h_full().flex_1().min_w_0().overflow_hidden().bg(if selected { p.accent.opacity(0.28) } else { bg })
                .child(div().w(px(46.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(no.map(|n| n.to_string()).unwrap_or_default()))
                .child(div().w(px(20.0)).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(sign))
                .child(code(rl, if left { 0 } else { 1 }))
        };
        let marks: Vec<AnyElement> = r.marks.iter().enumerate().map(|(i, m)| {
            let k = m.key.clone(); let range = m.range;
            let color = if m.draft { p.warn } else if m.resolved { p.success } else { p.accent };
            let el = div().id(("mk", ix * 8 + i)).flex_none().size(px(22.0)).flex().items_center().justify_center().rounded_sm().cursor_pointer().bg(p.surface).border_1().border_color(color.opacity(0.6)).hover(move |x| x.bg(p.hover))
                .child(icon(ic::COMMENT, 15.0, color))
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .on_hover(cx.listener(move |this, h: &bool, _, cx| { if *h { this.hover_c = range.map(|(a, b)| (fi, a, b)); } else if this.hover_c.map(|x| Some((x.1, x.2)) == range).unwrap_or(false) { this.hover_c = None; } cx.notify(); }))
                .on_click(cx.listener(move |this, _, _, cx| { this.hover_c = None; this.toggle_comment(&k); cx.stop_propagation(); cx.notify(); }));
            tip(el, "Show this comment", &p).into_any_element()
        }).collect();
        let inner = match &r.kind {
            RowKind::FileHeader => {
                let closed = self.collapsed_files.contains(&fi); let f = &self.m.vm.files[fi];
                return base.bg(p.surface).border_t_1().border_color(p.border).px_2().gap_2().font_weight(FontWeight::SEMIBOLD).cursor_pointer().hover(move |x| x.bg(p.hover))
                    .child(icon(if closed { ic::CHEVRON_RIGHT } else { ic::EXPAND_MORE }, 17.0, p.muted)).child(div().flex_1().min_w_0().truncate().child(f.path.clone()))
                    .children(marks)
                    .child(div().flex().gap_2().text_xs().child(div().text_color(p.success).child(format!("+{}", f.additions))).child(div().text_color(p.danger).child(format!("−{}", f.deletions))))
                    .on_click(cx.listener(move |this, _, _, cx| { if !this.collapsed_files.remove(&fi) { this.collapsed_files.insert(fi); } this.shown = None; this.load_diff(); cx.notify(); })).into_any_element();
            }
            RowKind::Collapsed(t, expandable) => return base.bg(p.accent.opacity(0.10)).px_3().gap_2().text_color(p.fg).child(icon(ic::UNFOLD_MORE, 15.0, p.accent)).child(div().flex_1().min_w_0().truncate().child(t.clone()))
                .when(*expandable, |d| d.child(div().id(("expand", ix)).px_2().rounded_md().border_1().border_color(p.border).cursor_pointer().hover(move |x| x.bg(p.hover)).text_sm().child("Show anyway")
                    .on_click(cx.listener(move |this, _, _, cx| { this.forced.insert(fi); this.shown = None; this.load_diff(); cx.notify(); })))).into_any_element(),
            RowKind::Note(t) => return base.bg(p.accent.opacity(0.10)).px_3().gap_2().text_color(p.fg).child(icon(ic::LINK, 15.0, p.accent)).child(t.clone()).into_any_element(),
            RowKind::Hunk(t) => return base.bg(p.accent.opacity(0.10)).text_color(p.muted).text_xs().child(div().w(px(70.0)).flex_none()).child(t.clone()).into_any_element(),
            RowKind::Comment(c) => return self.comment_card(fi, &c.clone(), cx).into_any_element(),
            RowKind::Rendered(blocks) => return div().id(ix).w_full().px_6().py_3().font_family(self.fonts.ui.clone()).text_size(px(14.0)).child(markdown_view(blocks, &p, &mono))
                .child(div().pt_2().text_xs().text_color(p.muted).child("Rendered preview. Switch to the source diff (m) to comment on lines, or open this file on its own to comment on blocks.")).into_any_element(),
            RowKind::Uni(rl) => {
                let l = &rl.line;
                let bg = match l.kind { Kind::Added => p.added, Kind::Removed => p.removed, Kind::Context => gpui::transparent_black() };
                let gut = match l.kind { Kind::Added => p.added_word.opacity(0.55), Kind::Removed => p.removed_word.opacity(0.55), Kind::Context => gpui::transparent_black() };
                let (show_old, show_new) = (st != Status::Added, st != Status::Deleted);
                base.bg(if selected { p.accent.opacity(0.28) } else { bg })
                    .when(show_old, |d| d.child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.old_no.map(|n| n.to_string()).unwrap_or_default())))
                    .when(show_new, |d| d.child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.new_no.map(|n| n.to_string()).unwrap_or_default())))
                    .child(div().w(px(22.0)).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(match l.kind { Kind::Added => "+", Kind::Removed => "−", _ => " " }))
                    .child(code(rl, 0))
            }
            RowKind::Split(a, b) => base.child(cell(a.as_ref(), true)).child(div().w(px(1.0)).h_full().bg(p.border)).child(cell(b.as_ref(), false)),
        };
        let ord = r.ord.unwrap_or(0);
        let hl = self.hover_c.map(|(hf, a, b)| hf == fi && r.new_line.map(|n| n >= a && n <= b).unwrap_or(false)).unwrap_or(false);
        let inner = inner.when(hl, |d| d.relative().child(div().absolute().inset_0().bg(p.warn.opacity(0.20)).border_l_2().border_color(p.warn)))
            .when(!marks.is_empty(), |d| d.relative().child(div().absolute().top(px(-1.0)).left(px(2.0)).flex().gap(px(2.0)).children(marks)));
        inner.when(selected, |d| d.border_l_2().border_color(p.accent)).cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| { this.row_hit(ord, ev.modifiers.shift); let y: f32 = ev.position.y.into(); this.row_y.insert(ord, y); this.sel_x = ev.position.x.into(); cx.stop_propagation(); cx.notify(); }))
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if this.dragging && ev.pressed_button == Some(MouseButton::Left) { if let Some(a) = this.anchor { let y: f32 = ev.position.y.into(); this.row_y.insert(ord, y); let s = Some((a.min(ord), a.max(ord))); if s != this.sel { this.sel = s; } cx.notify(); } }
            })).into_any_element()
    }

    /// A comment on the diff, GitLab style: collapsible, rich or raw, with edit and delete for your own drafts.
    fn comment_card(&mut self, fi: usize, c: &CItem, cx: &mut Context<Self>) -> impl IntoElement {
        let instant = self.instant();
        let (cfi, hv) = (fi, c.range);
        let p = self.pal; let mono = self.fonts.mono.clone();
        let collapsed = self.collapsed_c.contains(&c.key); let raw = self.raw_c.contains(&c.key);
        let (k1, k2) = (c.key.clone(), c.key.clone()); let id = SharedString::from(c.key.clone());
        let first_line: String = c.body.lines().next().unwrap_or("").chars().take(120).collect();
        let draft = c.draft; let orig = c.orig.clone(); let who_c = self.who(&c.author);
        let body = |text: &str| -> AnyElement { if raw { div().font_family(mono.clone()).text_sm().whitespace_normal().child(text.to_string()).into_any_element() } else { kit::markdown_view_ctx(&markdown::parse(text), &p, &mono, orig.as_deref()).text_sm().into_any_element() } };
        let accent = if c.draft.is_some() { p.warn } else if c.resolved { p.success } else { p.accent };
        div().flex().w_full().px_2().py(px(4.0)).child(
            div().id(SharedString::from(format!("card-{}", c.key))).flex().flex_col().flex_1().min_w_0().rounded_md()
                .on_hover(cx.listener(move |this, h: &bool, _, cx| { if *h { this.hover_c = hv.map(|(a, b)| (cfi, a, b)); } else if this.hover_c.map(|x| Some((x.1, x.2)) == hv).unwrap_or(false) { this.hover_c = None; } cx.notify(); })).border_1().border_color(p.border).border_l_4().border_color(accent).bg(p.surface).overflow_hidden()
                .child(div().flex().items_center().gap_2().px_3().py(px(5.0)).bg(p.hover.opacity(0.5))
                    .child(div().id(SharedString::from(format!("cc-{}", c.key))).flex().items_center().gap_1().cursor_pointer().on_click(cx.listener(move |this, _, _, cx| { this.toggle_comment(&k1); cx.notify(); }))
                        .child(icon(if collapsed { ic::CHEVRON_RIGHT } else { ic::EXPAND_MORE }, 17.0, p.muted)).child(icon(ic::COMMENT, 15.0, accent)))
                    .child(kit::avatar(&who_c, 20.0)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(c.author.clone()))
                    .when(c.draft.is_some(), |d| d.child(badge("Draft", p.warn, p.warn.opacity(0.15)))).when(c.ai, |d| d.child(badge("AI", p.ai, p.ai.opacity(0.15))))
                    .when(c.resolved, |d| d.child(badge("Resolved", p.success, p.success.opacity(0.15)))).when(c.outdated, |d| d.child(badge("Outdated", p.muted, p.hover)))
                    .when(!c.replies.is_empty(), |d| d.child(div().text_xs().text_color(p.muted).child(format!("{} repl{}", c.replies.len(), if c.replies.len() == 1 { "y" } else { "ies" }))))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(p.muted).when(collapsed, |d| d.child(first_line)))
                    .child(icon_btn(SharedString::from(format!("raw-{}", c.key)), if raw { ic::PREVIEW } else { ic::CODE }, if raw { "Show the rendered comment" } else { "Show the raw Markdown" }, Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { if !this.raw_c.remove(&k2) { this.raw_c.insert(k2.clone()); } this.rebuild_rows(); cx.notify(); })))
                    .children(draft.map(|d| icon_btn(SharedString::from(format!("ed-{d}")), ic::EDIT, "Edit this comment", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.open_composer(false, Some(d)); cx.stop_propagation(); cx.notify(); }))))
                    .children(draft.filter(|_| instant).map(|d| icon_btn(SharedString::from(format!("pub-{d}")), ic::SEND, "Publish this draft now", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.publish_draft(d, cx); cx.stop_propagation(); cx.notify(); }))))
                    .children(draft.map(|d| icon_btn(SharedString::from(format!("rm-{d}")), ic::DELETE, "Delete this draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(d); this.resync(); cx.stop_propagation(); cx.notify(); })))))
                .when(!collapsed, |d| d.child(div().flex().flex_col().gap_2().px_3().py_2().child(body(&c.body))
                    .children(c.replies.iter().map(|(a, t)| div().flex().flex_col().gap_1().pt_2().border_t_1().border_color(p.border).child(div().flex().items_center().gap_2().child(kit::avatar(a, 20.0)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(a.clone()))).child(body(t))).collect::<Vec<_>>())))
        ).id(id)
    }

    /// Horizontal scrollbar geometry: (track left x, track width, max offset); None when nothing overflows.
    fn hbar_geom(&self) -> Option<(f32, f32, f32)> {
        let b = self.b_body.get(); let (ox, w): (f32, f32) = (b.origin.x.into(), b.size.width.into());
        let cols = if self.diff_mode == DiffMode::Split { 2.0 } else { 1.0 };
        let max = (self.hx_text.get() - (w - 130.0) / cols).max(0.0);
        (max > 1.0 && w > 60.0 && self.md.is_none()).then_some((ox, w - 12.0, max))
    }

    fn hscrollbar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.pal; let (ox, tw, max) = self.hbar_geom()?;
        let thumb = (tw * tw / (tw + max)).max(28.0); let travel = (tw - thumb).max(1.0); let left = (self.hx / max).clamp(0.0, 1.0) * travel;
        let dragging = self.hb_drag.is_some();
        Some(div().id("hb-track").absolute().bottom(px(70.0)).left_0().w(px(tw)).h(px(12.0)).cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                let x: f32 = ev.position.x.into(); let rel = x - ox;
                let grab = if rel >= left && rel <= left + thumb { rel - left } else { thumb / 2.0 };
                this.hb_drag = Some(grab); this.hx = ((rel - grab) / travel).clamp(0.0, 1.0) * max; cx.stop_propagation(); cx.notify();
            }))
            .child(div().absolute().left(px(left)).bottom(px(2.0)).h(px(8.0)).w(px(thumb)).rounded_full().bg(if dragging { p.muted } else { p.muted.opacity(0.5) }).hover(move |s| s.bg(p.muted))).into_any_element())
    }

    fn scrollbar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.pal;
        let vp = self.list.viewport_bounds(); let vh: f32 = vp.size.height.into(); let max: f32 = self.list.max_offset_for_scrollbar().y.into();
        if max <= 1.0 || vh <= 1.0 { return None; }
        let off: f32 = (-self.list.scroll_px_offset_for_scrollbar().y).into();
        let thumb = (vh * vh / (vh + max)).max(28.0); let travel = (vh - thumb).max(1.0); let top = (off / max).clamp(0.0, 1.0) * travel;
        let oy: f32 = vp.origin.y.into(); let dragging = self.sb_drag.is_some();
        Some(div().id("sb-track").absolute().top_0().right_0().w(px(12.0)).h(px(vh)).cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                let y: f32 = ev.position.y.into(); let rel = y - oy;
                let grab = if rel >= top && rel <= top + thumb { rel - top } else { thumb / 2.0 };
                this.sb_drag = Some(grab); this.list.scrollbar_drag_started();
                let frac = ((rel - grab) / travel).clamp(0.0, 1.0); this.list.set_offset_from_scrollbar(point(px(0.0), px(-frac * max))); cx.stop_propagation(); cx.notify();
            }))
            .child(div().absolute().top(px(top)).right(px(2.0)).w(px(8.0)).h(px(thumb)).rounded_full().bg(if dragging { p.muted } else { p.muted.opacity(0.5) }).hover(move |s| s.bg(p.muted))).into_any_element())
    }

    /// Centre pane: a switch between Overview and Changes (remote reviews only), then the chosen page.
    fn center(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.find_open && self.find_page != self.page { self.recompute_find(); }
        let p = self.pal;
        let tabs = self.info.is_some().then(|| {
            let tab = |id: &'static str, label: &'static str, i: ic::Icon, on: bool, to: Page, cx: &mut Context<Self>| div().id(id).flex().items_center().gap_2().px_4().py(px(7.0)).cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted })
                .border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() }).hover(move |s| s.text_color(p.fg)).child(icon(i, 16.0, if on { p.accent } else { p.muted })).child(label)
                .on_click(cx.listener(move |this, _, _, cx| { this.page = to; cx.notify(); }));
            let on = self.page == Page::Overview;
            div().flex().items_center().gap_1().border_b_1().border_color(p.border).bg(p.surface)
                .child(tab("pg-overview", "Overview", ic::DESCRIPTION, on, Page::Overview, cx)).child(tab("pg-code", "Changes", ic::DIFFERENCE, !on, Page::Code, cx))
                .child(div().flex_1()).child(div().pr_3().text_xs().text_color(p.muted).child("O switches between them"))
        });
        let drafts = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let inner = if self.page == Page::Overview && self.info.is_some() { self.overview_page(&drafts, cx) } else { self.code_center(cx).into_any_element() };
        let find = self.find_bar(cx);
        div().relative().flex().flex_col().flex_1().min_w_0().h_full().children(tabs).child(inner).children(find).into_any_element()
    }

    /// The overview page: what the MR is about and the conversation that is not tied to a line.
    fn overview_page(&mut self, drafts: &[perusal_core::store::Draft], cx: &mut Context<Self>) -> AnyElement {
        let instant = self.instant();
        let p = self.pal; let mono = self.fonts.mono.clone();
        let Some(info) = self.info.clone() else { return div().into_any_element() };
        let global_threads: Vec<Thread> = self.threads.iter().filter(|t| t.path.is_none()).cloned().collect();
        let global_drafts: Vec<&perusal_core::store::Draft> = drafts.iter().filter(|d| d.file.is_none()).collect();
        let me = self.who("You");
        let card = |who: String, body: &str, tag: Option<AnyElement>| div().w_full().max_w(px(900.0)).flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(p.border).bg(p.surface)
            .child(div().flex().items_center().gap_2().child(kit::avatar(if who == "You" { &me } else { &who }, 24.0)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(who)).children(tag)).child(markdown_view(&markdown::parse(body), &p, &mono).text_sm());
        let mut list: Vec<AnyElement> = vec![]; let mut list_texts: Vec<String> = vec![];
        for t in &global_threads {
            for (i, (a, b)) in t.comments.iter().enumerate() { list_texts.push(format!("{a} {b}")); list.push(card(a.clone(), b, (i == 0 && t.resolved).then(|| badge("Resolved", p.success, p.success.opacity(0.15)).into_any_element())).into_any_element()); }
        }
        for d in &global_drafts {
            let id = d.id; list_texts.push(d.body.clone());
            list.push(card("You".into(), &d.body, Some(badge("Draft", p.warn, p.warn.opacity(0.15)).into_any_element()))
                .child(div().flex().justify_end().child(icon_btn(("ovde", id as usize), ic::EDIT, "Edit this comment", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.open_composer(false, Some(id)); cx.notify(); })))
                    .children(instant.then(|| icon_btn(("ovdp", id as usize), ic::SEND, "Publish this draft now", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.publish_draft(id, cx); cx.notify(); }))))
                    .child(icon_btn(("ovdd", id as usize), ic::DELETE, "Delete this draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(id); this.resync(); cx.notify(); })))).into_any_element());
        }
        let state = model::norm_state(&info.state); let dot = match state.as_str() { "merged" => p.ai, "closed" => p.danger, _ => p.success };
        // Ctrl+F on this page: one search text per section, in drawing order (title, meta, description, header, [empty hint], comments..., footer)
        let first_list = 4 + usize::from(list.is_empty());
        if self.find_open {
            let q = self.find_in.f.text.clone();
            let mut texts: Vec<String> = vec![info.title.clone(), String::new(), info.description.clone(), String::new()];
            if list.is_empty() { texts.push(String::new()); } texts.extend(list_texts.iter().cloned());
            let mut hits = vec![];
            if !q.is_empty() { for (ix, t) in texts.iter().enumerate() { for range in perusal_app::find::find_all(t, &q) { hits.push(Hit { row: ix, part: 0, range }); } } }
            self.find_page = Page::Overview; self.find_hits = hits; if self.find_cur >= self.find_hits.len() { self.find_cur = 0; }
        }
        let tint = |ix: usize| -> Option<gpui::Hsla> {
            if !self.find_open || self.page != Page::Overview { return None; }
            let any = self.find_hits.iter().any(|h| h.row == ix); if !any { return None; }
            Some(p.warn.opacity(if self.find_hits.get(self.find_cur).map(|h| h.row == ix).unwrap_or(false) { 0.35 } else { 0.16 }))
        };
        let list: Vec<AnyElement> = list.into_iter().enumerate().map(|(i, el)| match tint(first_list + i) { Some(c) => div().w_full().max_w(px(900.0)).rounded_md().bg(c).child(el).into_any_element(), None => el }).collect();
        let sec = |el: gpui::Div| el.w_full().max_w(px(900.0));
        let page = div().id("overview").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.ov_scroll).flex().flex_col().items_center().gap_4().px_6().pt_6().pb(px(100.0))
            .child(sec(div().text_2xl().font_weight(FontWeight::SEMIBOLD).when_some(tint(0), |d, c| d.bg(c)).child(info.title.clone())))
            .child(sec(div().flex().flex_wrap().items_center().gap_3().text_sm().text_color(p.muted)
                .child(badge(match state.as_str() { "merged" => "Merged", "closed" => "Closed", _ => "Open" }, dot, dot.opacity(0.15))).child(div().flex().items_center().gap_2().child(kit::avatar(&info.author, 22.0)).child(format!("by {}", info.author)))
                .children((!info.assignees.is_empty()).then(|| div().flex().items_center().gap_1().child(div().text_xs().child("Assigned")).children(info.assignees.iter().map(|a| div().flex().items_center().gap_1().child(kit::avatar(&a.login, 20.0)).child(a.login.clone())))))
                .children((!info.reviewers.is_empty()).then(|| div().flex().items_center().gap_1().child(div().text_xs().child("Reviewers")).children(info.reviewers.iter().map(|a| div().flex().items_center().gap_1().child(kit::avatar(&a.login, 20.0)).child(a.login.clone())))))
                .when(!info.source_ref.is_empty(), |d| d.child(div().flex().items_center().gap_1().font_family(mono.clone()).child(info.source_ref.clone()).child(icon(ic::ARROW_FORWARD, 14.0, p.muted)).child(info.base_ref.clone())))))
            .child(sec(div().flex().flex_col().gap_2().p_4().rounded_md().border_1().border_color(p.border).bg(p.surface).when_some(tint(2), |d, c| d.bg(c))
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child("Description"))
                .child(if info.description.trim().is_empty() { div().text_color(p.muted).child("No description.").into_any_element() } else { markdown_view(&markdown::parse(&info.description), &p, &mono).into_any_element() })))
            .child(sec(div().flex().items_center().justify_between().child(div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).child("Conversation")).child(badge(format!("{}", list.len()), p.muted, p.hover)))
                .child(icon_text_btn("ov-add", ic::ADD_COMMENT, "Comment on the whole merge request", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_composer(true, None); cx.notify(); })))))
            .when(list.is_empty(), |d| d.child(sec(div().text_sm().text_color(p.muted).child("No general comments yet."))))
            .children(list)
            .child(sec(div().text_xs().text_color(p.muted).child("Open Changes (O) to review the files. Comments on specific lines are in the diff."))).into_any_element();
        let bar = div().absolute().bottom(px(16.0)).left_0().right_0().flex().justify_center().child(
            div().flex().items_center().gap_2().px_3().py_2().rounded_lg().bg(p.surface).border_1().border_color(p.border).shadow_lg()
                .child(icon_text_btn("ov-code", ic::DIFFERENCE, "Review the changes", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.page = Page::Code; cx.notify(); })))
                .child(icon_btn("ov-gcomment", ic::ADD_COMMENT, "Comment on the whole merge request (c)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_composer(true, None); cx.notify(); })))
                .children((!self.instant()).then(|| icon_text_btn("ov-finish", ic::RATE_REVIEW, "Finish review", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.finish_open = true; this.focus = Some(Fid::Summary); this.submit_msg = None; cx.notify(); })))));
        let composer = self.composer.is_some().then(|| self.composer_panel(cx));
        div().relative().flex().flex_col().flex_1().min_h_0().w_full().child(page).child(bar).children(composer).into_any_element()
    }

    fn code_center(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let mono = self.fonts.mono.clone();
        let batch = self.m.vm.current_batch().to_vec();
        let first = batch.first().map(|&i| self.m.vm.files[i].clone());
        let is_md = batch.iter().any(|&i| markdown::is_markdown(&self.m.vm.files[i].path) && !self.m.vm.files[i].symlink);
        let viewed_all = !batch.is_empty() && batch.iter().all(|&i| self.m.vm.files[i].viewed);
        let manual = batch.first().map(|&i| self.m.vm.is_manually_grouped(i)).unwrap_or(false);
        let (adds, dels): (u32, u32) = batch.iter().fold((0, 0), |a, &i| (a.0 + self.m.vm.files[i].additions, a.1 + self.m.vm.files[i].deletions));
        let md_on = self.md_rendered; let mode = self.diff_mode;
        let one_sided = !batch.is_empty() && batch.iter().all(|&i| matches!(self.m.vm.files[i].status, Status::Added | Status::Deleted) || self.m.vm.files[i].symlink);
        let ncomments = self.comment_count();

        let title = if batch.len() > 1 { div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Batch of {} files", batch.len()))).child(div().text_xs().text_color(p.muted).child(if manual { "grouped by you" } else { "related files" })) }
            else if let Some(f) = &first { let (dir, name) = match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) };
                div().flex().items_center().gap_2().min_w_0().truncate().when(f.symlink, |d| d.child(icon(ic::LINK, 17.0, p.accent))).child(div().flex().font_family(mono.clone()).text_sm().child(div().text_color(p.muted).child(dir)).child(div().font_weight(FontWeight::SEMIBOLD).child(name))) } else { div().child("No files match") };
        let sym = self.sym_path.clone().filter(|_| batch.len() == 1).map(|path| { let known = self.sym_target.is_some();
            icon_text_btn("sym-go", ic::ARROW_FORWARD, "Go to the original", Btn::Secondary, &p).on_click(cx.listener(move |this, _, _, cx| { this.goto_symlink_target(cx); cx.notify(); }))
                .tooltip_text(if known { format!("Open {path}") } else { format!("{path} is not changed in this merge request") }, &p) });
        let header = (!batch.is_empty()).then(|| {
            let mut h = div().flex().items_center().gap_3().px_4().py_2().border_b_1().border_color(p.border).bg(p.surface).child(div().flex_1().min_w_0().child(title))
                .child(div().flex().items_center().gap_2().text_xs().font_family(mono.clone()).child(div().text_color(p.success).child(format!("+{adds}"))).child(div().text_color(p.danger).child(format!("−{dels}"))).child(diff_squares(adds, dels, &p, 10.0)));
            if let Some(s) = sym { h = h.child(s); }
            if ncomments > 0 { h = h.child(icon_btn("collapse-c", ic::UNFOLD_LESS, "Collapse or expand all comments in this file", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.collapse_all_comments(); cx.notify(); }))); }
            if manual { h = h.child(icon_btn("ungroup", ic::LINK_OFF, "Ungroup these files", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { if let Some(fi) = this.current_file() { this.m.vm.ungroup(fi); this.reload(); } cx.notify(); }))); }
            h
        });

        let drafts = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let body: AnyElement = if let Some(blocks) = &self.md {
            let cur_path = first.as_ref().map(|f| f.path.clone()).unwrap_or_default();
            let items: Vec<_> = blocks.iter().enumerate().map(|(i, b)| {
                let on = self.md_sel.map(|(a, c)| i >= a.min(c) && i <= a.max(c)).unwrap_or(false);
                let n = drafts.iter().filter(|d| d.file.as_deref() == Some(cur_path.as_str()) && d.start_line.map(|s| s <= b.last_line && d.end_line.unwrap_or(s) >= b.first_line).unwrap_or(false)).count();
                div().id(("mdb", i)).relative().px_3().py(px(2.0)).rounded_md().cursor_pointer().border_l_2().border_color(if on { p.accent } else { gpui::transparent_black() }).when(on, |d| d.bg(p.accent.opacity(0.12))).hover(move |s| s.bg(p.hover.opacity(0.6)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.md_sel = match (ev.modifiers().shift, this.md_sel) { (true, Some((a, _))) => Some((a, i)), _ => Some((i, i)) };
                        if let Some(pos) = ev.mouse_position() { let y: f32 = pos.y.into(); this.row_y.insert(i, y); this.sel_x = pos.x.into(); } cx.notify();
                    }))
                    .child(markdown_view(std::slice::from_ref(&b.block), &p, &mono))
                    .when(n > 0, |d| d.child(div().absolute().top(px(2.0)).right(px(4.0)).flex().items_center().gap_1().text_xs().text_color(p.accent).child(icon(ic::COMMENT, 14.0, p.accent)).child(format!("{n}"))))
            }).collect();
            div().id("md").flex_1().min_h_0().overflow_y_scroll().p_6().pb(px(90.0)).child(div().max_w(px(880.0)).flex().flex_col().gap_2()
                .child(div().text_xs().text_color(p.muted).pb_2().child("Rendered preview. Click a block to comment on it; Shift+click extends the selection.")).children(items)).into_any_element()
        } else if self.rows.is_empty() {
            div().flex_1().flex().items_center().justify_center().text_color(p.muted).child("Nothing to show for this selection.").into_any_element()
        } else {
            div().id("diff-wrap").flex_1().min_h_0().w_full().pb(px(70.0))
                .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, _, cx| {
                    let dx: f32 = ev.delta.pixel_delta(px(20.0)).x.into();
                    if dx != 0.0 {
                        let max = this.hbar_geom().map(|g| g.2).unwrap_or(0.0);
                        let n = (this.hx - dx).clamp(0.0, max); if n != this.hx { this.hx = n; cx.notify(); }
                    }
                }))
                .child(list(self.list.clone(), cx.processor(move |this, ix: usize, _w, cx| { let el = this.render_row(ix, cx); match this.row_tint(ix) { Some(c) => div().w_full().bg(c).child(el).into_any_element(), None => el } })).size_full()).into_any_element()
        };
        let scrollbar = if self.md.is_none() { self.scrollbar(cx) } else { None };
        let hscrollbar = self.hscrollbar(cx);

        // toolbar that floats above the selected lines
        let sel = self.selection();
        let toolbar = sel.clone().filter(|_| self.composer.is_none()).and_then(|s| {
            let range = if self.md.is_some() { self.md_sel.map(|(a, b)| (a.min(b), a.max(b))) } else { self.sel };
            let (lo, hi) = range?; let miny = self.row_y.iter().filter(|(o, _)| **o >= lo && **o <= hi).map(|(_, y)| *y).fold(f32::MAX, f32::min);
            if miny == f32::MAX { return None; }
            let b = self.b_body.get(); let (ox, oy): (f32, f32) = (b.origin.x.into(), b.origin.y.into()); let w: f32 = b.size.width.into();
            let top = (miny - oy - 50.0).max(4.0); let left = (self.sel_x - ox - 30.0).clamp(8.0, (w - 330.0).max(8.0));
            Some(div().absolute().top(px(top)).left(px(left)).flex().items_center().gap_1().px_2().py_1().rounded_lg().bg(p.surface).border_1().border_color(p.accent).shadow_lg().occlude()
                .child(div().text_xs().text_color(p.muted).px_1().child(format!("{}:{}–{}", s.file.rsplit('/').next().unwrap_or(&s.file), s.start, s.end)))
                .child(icon_text_btn("sel-comment", ic::ADD_COMMENT, "Comment", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_composer(false, None); cx.stop_propagation(); cx.notify(); })))
                .child(icon_text_btn("sel-ai", ic::AUTO_AWESOME, "Ask AI", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = this.selection(); this.tab = RightTab::Ai; this.right_open = true; this.focus = Some(Fid::Ask); cx.stop_propagation(); cx.notify(); })))
                .child(icon_btn("sel-clear", ic::CLOSE, "Clear the selection (Esc)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.clear_selection(); cx.stop_propagation(); cx.notify(); }))))
        });

        // floating action bar
        let sep = || div().w(px(1.0)).h(px(22.0)).bg(p.border).flex_none();
        let bar = div().absolute().bottom(px(16.0)).left_0().right_0().flex().justify_center().child(
            div().flex().items_center().gap_2().px_3().py_2().rounded_lg().bg(p.surface).border_1().border_color(p.border).shadow_lg()
                .child(icon_btn("prev", ic::CHEVRON_LEFT, "Previous batch (Backspace)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.back(); cx.notify(); })))
                .child(icon_btn("next", ic::CHEVRON_RIGHT, "Next batch (Enter)", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.next(); cx.notify(); })))
                .child(tip(div().id("viewed-box").size(px(30.0)).flex().items_center().justify_center().flex_none().rounded_md().border_1().border_color(if viewed_all { p.success } else { p.border }).bg(if viewed_all { p.success } else { p.surface }).cursor_pointer().hover(move |s| s.bg(if viewed_all { p.success } else { p.hover }))
                    .on_click(cx.listener(|this, _, _, cx| { this.toggle_viewed(); cx.notify(); })).child(icon(if viewed_all { ic::VISIBILITY } else { ic::VISIBILITY_OFF }, 18.0, if viewed_all { gpui::white() } else { p.muted })), if viewed_all { "Viewed. Click to mark as not viewed (v)" } else { "Mark this file as viewed (v)" }, &p))
                .child(icon_btn("gcomment", ic::ADD_COMMENT, "Comment on the whole merge request", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_composer(true, None); cx.notify(); })))
                .when(!md_on && !one_sided, |d| d.child(sep()).child(seg_box(&p).child(seg_icon("df-uni", ic::VIEW_AGENDA, "Unified diff (u)", mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Unified); cx.notify(); })))
                    .child(seg_icon("df-split", ic::VERTICAL_SPLIT, "Side-by-side diff (u)", mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Split); cx.notify(); })))))
                .when(is_md, |d| d.child(sep()).child(seg_box(&p).child(seg_icon("md-diff", ic::CODE, "Show the markdown source diff (m)", !md_on, &p).on_click(cx.listener(|this, _, _, cx| { if this.md_rendered { this.md_rendered = false; this.shown = None; this.clear_selection(); } cx.notify(); })))
                    .child(seg_icon("md-prev", ic::VISIBILITY, "Show the rendered markdown (m)", md_on, &p).on_click(cx.listener(|this, _, _, cx| { if !this.md_rendered { this.md_rendered = true; this.shown = None; this.clear_selection(); } cx.notify(); })))))
                .child(sep())
                .children((!self.instant()).then(|| icon_text_btn("finish", ic::RATE_REVIEW, "Finish review", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.finish_open = true; this.focus = Some(Fid::Summary); this.submit_msg = None; cx.notify(); })))));

        let composer = self.composer.is_some().then(|| self.composer_panel(cx));
        let wrap = div().relative().flex().flex_col().flex_1().min_h_0().w_full().child(div().absolute().size_full().child(kit::bounds_probe(&self.b_body))).child(body).children(scrollbar).children(hscrollbar).children(toolbar).child(bar);
        div().relative().flex().flex_col().flex_1().min_w_0().min_h_0().children(header).child(wrap).children(composer)
    }

    /// The rich comment editor: Markdown toolbar, Write and Preview tabs, suggestions.
    fn composer_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let mono = self.fonts.mono.clone();
        let c = self.composer.as_ref().map(|c| (c.editing, c.global, c.preview)).unwrap_or((None, false, false));
        let (editing, global, preview) = c;
        let sel = if global || editing.is_some() { None } else { self.selection() };
        let target: SharedString = if editing.is_some() { "Edit comment".into() } else if global { "Comment on the whole merge request".into() } else { match (&sel, self.current_file()) { (Some(s), _) => format!("Comment on {}:{}–{}", s.file, s.start, s.end), (None, Some(fi)) => format!("Comment on {} (select lines first to anchor it)", self.m.vm.files[fi].path), _ => "General comment".into() }.into() };
        let has_new = sel.is_some() && self.selection_new_text().is_some();
        let focused = self.focus == Some(Fid::Comment);
        let tb = |id: &'static str, i: ic::Icon, t: &'static str, cx: &mut Context<Self>, f: fn(&mut TextField2)| icon_btn(id, i, t, Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.focus = Some(Fid::Comment); f(&mut this.comment.f); cx.stop_propagation(); cx.notify(); }));
        let editor: AnyElement = if preview {
            div().min_h(px(110.0)).max_h(px(260.0)).overflow_hidden().px_3().py_2().child(if self.comment.f.text.trim().is_empty() { div().text_sm().text_color(p.muted).child("Nothing to preview yet.").into_any_element() } else { kit::markdown_view_ctx(&markdown::parse(&self.comment.f.text), &p, &mono, sel.as_ref().and_then(|_| self.selection_new_text()).as_deref()).text_sm().into_any_element() }).into_any_element()
        } else {
            self.winp(Fid::Comment, div().id("cbox").flex().px_3().py(px(6.0)).min_h(px(110.0)).max_h(px(260.0)).overflow_hidden().font_family(self.fonts.ui.clone()).child(input_text(&self.comment, focused, "Write a comment. Markdown is supported: **bold**, `code`, lists, links, and suggestions.", &p, false, 5)), cx).into_any_element()
        };
        let tab = |id: &'static str, label: &'static str, on: bool, cx: &mut Context<Self>, set: bool| div().id(id).px_3().py(px(5.0)).cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() }).child(label)
            .on_click(cx.listener(move |this, _, _, cx| { if let Some(c) = this.composer.as_mut() { c.preview = set; } cx.stop_propagation(); cx.notify(); }));
        let probe = kit::bounds_probe(&self.b_composer);
        div().relative().flex().flex_col().border_t_1().border_color(p.accent).bg(p.surface).child(div().absolute().size_full().child(probe))
            .child(div().flex().items_center().gap_2().px_3().pt_2().child(icon(ic::COMMENT, 17.0, p.accent)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().truncate().child(target))
                .child(icon_btn("cmp-close", ic::CLOSE, "Close (Esc)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.composer = None; this.focus = None; cx.stop_propagation(); cx.notify(); }))))
            .child(div().flex().items_center().gap_1().px_3().child(tab("cmp-write", "Write", !preview, cx, false)).child(tab("cmp-prev", "Preview", preview, cx, true)).child(div().flex_1())
                .when(!preview, |d| d.child(tb("tb-b", ic::FORMAT_BOLD, "Bold", cx, |f| f.wrap_selection("**", "**", "bold")))
                    .child(tb("tb-i", ic::FORMAT_ITALIC, "Italic", cx, |f| f.wrap_selection("_", "_", "italic")))
                    .child(tb("tb-c", ic::CODE, "Inline code", cx, |f| f.wrap_selection("`", "`", "code")))
                    .child(tb("tb-cb", ic::DATA_OBJECT, "Code block", cx, |f| f.insert_block("```\ncode\n```")))
                    .child(tb("tb-q", ic::FORMAT_QUOTE, "Quote", cx, |f| f.prefix_lines("> ")))
                    .child(tb("tb-l", ic::FORMAT_LIST_BULLETED, "Bulleted list", cx, |f| f.prefix_lines("- ")))
                    .child(tb("tb-k", ic::LINK, "Link", cx, |f| f.wrap_selection("[", "](https://)", "text")))
                    .when(has_new, |d| d.child(icon_btn("tb-s", ic::LIGHTBULB, "Insert a suggested change with the selected lines", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(t) = this.selection_new_text() { this.comment.f.insert_block(&format!("```suggestion\n{t}\n```")); } this.focus = Some(Fid::Comment); cx.stop_propagation(); cx.notify(); }))))))
            .child(div().mx_3().rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).child(editor))
            .child(div().flex().items_center().gap_2().px_3().py_2().child(div().flex_1().text_xs().text_color(p.muted).child("Ctrl+Enter saves. Markdown and ```suggestion blocks are supported."))
                .child(icon_text_btn("cmp-cancel", ic::CLOSE, "Cancel", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.composer = None; this.focus = None; cx.stop_propagation(); cx.notify(); })))
                .child(icon_text_btn("cmp-save", ic::CHECK, if editing.is_some() { "Save changes" } else { "Add draft" }, Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_composer(cx); this.flush(cx); cx.stop_propagation(); cx.notify(); }))))
    }

    fn right_panel(&mut self, cx: &mut Context<Self>, drafts: &[perusal_core::store::Draft]) -> impl IntoElement {
        let p = self.pal;
        let tabs = div().flex().items_center().border_b_1().border_color(p.border).px_2()
            .child(self.tab_btn("tab-review", "Review", ic::COMMIT, RightTab::Review, cx)).child(self.tab_btn("tab-ai", "Ask AI", ic::AUTO_AWESOME, RightTab::Ai, cx)).child(div().flex_1())
            .child(icon_btn("close-r", ic::CHEVRON_RIGHT, "Hide this panel", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.right_open = false; this.settings.right_open = false; this.save_settings(); cx.notify(); })));
        let body = if self.tab == RightTab::Review { self.review_tab(cx, drafts).into_any_element() } else { self.ai_tab(cx).into_any_element() };
        div().flex().flex_col().size_full().child(tabs).child(body)
    }
    fn tab_btn(&self, id: &'static str, label: &'static str, i: ic::Icon, t: RightTab, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let on = self.tab == t;
        div().id(id).flex().items_center().gap_1().px_3().py_2().cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() })
            .child(icon(i, 16.0, if on { p.accent } else { p.muted })).child(label).on_click(cx.listener(move |this, _, _, cx| { this.tab = t; cx.notify(); }))
    }
    fn section_header(&self, id: &'static str, title: &'static str, count: Option<usize>, open: bool, cx: &mut Context<Self>, toggle: fn(&mut View)) -> gpui::Stateful<gpui::Div> {
        let p = self.pal;
        div().id(id).flex().items_center().gap_2().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface).cursor_pointer().hover(move |s| s.bg(p.hover))
            .child(icon(if open { ic::EXPAND_MORE } else { ic::CHEVRON_RIGHT }, 18.0, p.muted)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child(title)).children(count.map(|c| badge(format!("{c}"), p.muted, p.hover)))
            .on_click(cx.listener(move |this, _, _, cx| { toggle(this); cx.notify(); }))
    }
    fn review_tab(&mut self, cx: &mut Context<Self>, drafts: &[perusal_core::store::Draft]) -> impl IntoElement {
        let p = self.pal; let mono = self.fonts.mono.clone();
        let commit_rows: Vec<_> = self.m.graph.rows.iter().enumerate().map(|(i, r)| {
            let sel = self.m.graph.selection.map(|(a, b)| i >= a && i <= b).unwrap_or(false) && self.m.vm.commit_scope.is_some(); let last = i + 1 == self.m.graph.rows.len();
            div().id(("c", i)).flex().gap_2().px_3().cursor_pointer().when(sel, |d| d.bg(p.accent.opacity(0.14))).hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| { if this.m.graph.rows.get(i).map(|r| r.in_mr).unwrap_or(false) { this.pick_commit(i); } cx.notify(); }))
                .child(div().flex().flex_col().items_center().w(px(12.0)).flex_none()
                    .child(div().h(px(8.0)).w(px(2.0)).bg(if i == 0 { gpui::transparent_black() } else { p.border }))
                    .child(div().size(px(10.0)).rounded_full().when(r.in_mr, |d| d.bg(p.accent)).when(!r.in_mr, |d| d.border_2().border_color(p.muted)))
                    .child(div().flex_1().w(px(2.0)).bg(if last { gpui::transparent_black() } else { p.border })))
                .child(div().flex().flex_col().min_w_0().flex_1().py_1().child(div().truncate().text_sm().text_color(if r.in_mr { p.fg } else { p.muted }).child(r.subject.clone()))
                    .child(div().flex().items_center().gap_1().child(kit::avatar(&crate::avatars::commit_identity(&r.author, &r.email), 18.0)).child(div().text_xs().text_color(p.muted).truncate().child(r.author.clone())).child(div().font_family(mono.clone()).text_xs().text_color(p.muted).child(r.sha[..7.min(r.sha.len())].to_string()))))
                .when(sel, |d| d.child(div().flex().items_center().child(icon(ic::CHECK, 15.0, p.accent))))
        }).collect();
        let draft_rows: Vec<_> = drafts.iter().enumerate().map(|(i, d)| {
            let id = d.id;
            let loc = match (&d.file, d.start_line, d.end_line) { (Some(f), Some(a), Some(b)) if a != b => format!("{}:{a}–{b}", f.rsplit('/').next().unwrap_or(f)), (Some(f), Some(a), _) => format!("{}:{a}", f.rsplit('/').next().unwrap_or(f)), (Some(f), None, _) => f.rsplit('/').next().unwrap_or(f).to_string(), _ => "Merge request".into() };
            div().id(("d", i)).flex().flex_col().gap_1().px_3().py_2().border_b_1().border_color(p.border)
                .child(div().flex().items_center().justify_between().child(div().flex().items_center().gap_2().child(div().text_xs().font_family(mono.clone()).text_color(p.muted).child(loc)).when(d.ai, |x| x.child(badge("AI", p.ai, p.ai.opacity(0.15)))))
                    .child(div().flex().child(icon_btn(("de", i), ic::EDIT, "Edit this draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.open_composer(false, Some(id)); cx.notify(); })))
                        .child(icon_btn(("dd", i), ic::DELETE, "Remove this draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(id); this.resync(); cx.notify(); })))))
                .child(div().text_sm().child(d.body.clone()))
        }).collect();
        let n = drafts.len();
        let (co, dro) = (self.commits_open, self.drafts_open);
        div().flex().flex_col().flex_1().min_h_0()
            .child(self.section_header("sec-commits", "Commits", Some(self.m.graph.rows.len()), co, cx, |v| v.commits_open = !v.commits_open))
            .when(co, |d| d.child(div().id("commits").max_h(px(260.0)).overflow_y_scroll().children(commit_rows)))
            .child(self.section_header("sec-drafts", "Drafts", Some(n), dro, cx, |v| v.drafts_open = !v.drafts_open))
            .when(dro, |d| d.child(div().id("drafts").flex_1().min_h_0().overflow_y_scroll()
                .when(n == 0, |d| d.child(div().p_4().text_sm().text_color(p.muted).child("No drafts yet. Select lines (click, drag, or Shift+click) and use the Comment button.")))
                .children(draft_rows)))
    }

    fn ai_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let ctx: Option<SharedString> = self.ai_ctx.as_ref().map(|s| format!("{}:{}–{}", s.file, s.start, s.end).into());
        let bubbles: Vec<_> = self.chat.iter().enumerate().map(|(i, (role, text))| {
            let user = role == "user"; let t = text.clone();
            if user {
                div().flex().justify_end().px_3().py(px(6.0)).child(div().max_w(relative(0.86)).px_3().py_2().rounded_lg().bg(p.accent).text_color(gpui::white()).text_sm().child(text.clone()))
            } else {
                div().flex().items_start().gap_2().px_3().py(px(6.0))
                    .child(div().size(px(26.0)).flex_none().rounded_full().bg(p.ai.opacity(0.18)).flex().items_center().justify_center().child(icon(ic::AUTO_AWESOME, 15.0, p.ai)))
                    .child(div().flex().flex_col().gap_1().max_w(relative(0.86)).child(div().px_3().py_2().rounded_lg().bg(p.hover).text_sm().child(markdown_view(&markdown::parse(text), &p, &self.fonts.mono).text_sm()))
                        .child(div().flex().child(icon_btn(("todraft", i), ic::ADD_COMMENT, "Save this answer as an AI draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.draft_from_answer(&t); this.flush(cx); cx.notify(); })))))
            }
        }).collect();
        let focused = self.focus == Some(Fid::Ask); let busy = self.busy();
        let agent = self.settings.agent_label();
        let mut cfg = vec![agent.to_string()];
        if !self.settings.model.is_empty() { cfg.push(self.settings.model.clone()); }
        if !self.settings.effort.is_empty() && supports_effort(&self.settings.agent) { cfg.push(self.settings.effort.clone()); }
        let working = self.ai_job.map(|j| {
            let label = match j { Job::Review => "Reviewing the merge request", Job::Chat => "Thinking" };
            let el = self.ai_started.map(ago).unwrap_or_default();
            let acts: Vec<_> = self.ai_activity.iter().rev().take(6).rev().enumerate().map(|(i, a)| div().flex().items_center().gap_2().text_xs().text_color(if i + 1 == self.ai_activity.len().min(6) { p.fg } else { p.muted }).child(icon(ic::CHEVRON_RIGHT, 13.0, p.muted)).child(div().truncate().child(a.clone()))).collect();
            div().flex().flex_col().gap_2().px_3().py_3().bg(p.accent.opacity(0.08)).border_b_1().border_color(p.accent)
                .child(div().flex().items_center().gap_3().child(loading_dots(&p)).child(div().flex_1().text_sm().font_weight(FontWeight::MEDIUM).text_color(p.accent).child(format!("{label}… {el}")))
                    .child(icon_btn("ai-stop", ic::STOP, "Stop the agent (Esc)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.stop_ai(); cx.notify(); }))))
                .children(acts)
                .when(!self.ai_partial.trim().is_empty(), |d| d.child(div().text_sm().text_color(p.fg).max_h(px(160.0)).overflow_hidden().child(self.ai_partial.trim().to_string())))
        });
        div().flex().flex_col().flex_1().min_h_0()
            .child(div().flex().items_center().gap_1().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface)
                .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("Ask {agent}"))).child(div().text_xs().text_color(p.muted).truncate().child(cfg.join(" · "))))
                .child(icon_btn("new-chat", ic::ADD, "Start a new chat", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { let _ = this.m.store.new_chat(this.m.vm.review); this.chat.clear(); this.ai_ctx = None; this.ai_notice = None; cx.notify(); })))
                .child(icon_btn("ai-settings", ic::TUNE, "Agent, model and effort settings", Btn::Ghost, &p).on_click(cx.listener(|_, _, _, cx| cx.emit(ViewEvent::OpenSettings)))))
            .children(self.ai_notice.clone().map(|n| div().flex().items_start().gap_2().px_3().py_2().text_sm().text_color(p.danger).bg(p.danger.opacity(0.1)).border_b_1().border_color(p.danger).child(icon(ic::ERROR_OUTLINE, 17.0, p.danger)).child(div().flex_1().min_w_0().child(n))
                .child(icon_btn("ai-notice-x", ic::CLOSE, "Dismiss", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.ai_notice = None; cx.notify(); })))))
            .children(working)
            .child(div().id("chat").flex_1().min_h_0().overflow_y_scroll().py_2()
                .when(self.chat.is_empty() && !busy, |d| d.child(div().p_4().flex().flex_col().gap_2().text_sm().text_color(p.muted)
                    .child("Select lines in the diff and use Ask AI, or ask a general question about this merge request.")
                    .child("Answers stay private to you. Nothing is posted to the host. Save one as a draft to keep it.")))
                .children(bubbles))
            .child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border).bg(p.surface)
                .children(ctx.map(|c| div().flex().items_center().gap_2().px_2().py(px(3.0)).rounded_md().bg(p.accent.opacity(0.1)).child(icon(ic::CODE, 15.0, p.accent)).child(div().flex_1().min_w_0().truncate().text_xs().font_family(self.fonts.mono.clone()).child(c))
                    .child(icon_btn("ctx-x", ic::CLOSE, "Remove this context", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = None; cx.stop_propagation(); cx.notify(); })))))
                .child(div().flex().items_center().gap_2()
                    .child(self.winp(Fid::Ask, div().id("ask-box").flex_1().min_w_0().flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).min_h(px(32.0)).child(input_text(&self.ask, focused, "Ask about the selection…", &p, false, 1)), cx))
                    .child(if busy { icon_btn("ask-send", ic::STOP, "Stop the agent", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.stop_ai(); cx.notify(); })) }
                        else { icon_btn("ask-send", ic::SEND, "Send (Enter)", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { let q = this.ask.f.text.trim().to_string(); if !q.is_empty() { this.ask.f.clear(); this.start_ai(Job::Chat, q, cx); } cx.notify(); })) }))
                .child(div().id("ask-ctx").text_xs().text_color(p.accent).cursor_pointer().child("Use the current selection as context").on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = this.selection(); cx.notify(); }))))
    }

    fn finish_overlay(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let (nd, nai, unviewed, total) = self.finish_counts();
        let remote = self.m.mr.is_some();
        let gitlab = self.m.mr.as_ref().map(|m| m.kind == perusal_providers::Kind::GitLab).unwrap_or(false);
        let v = self.verdict.clone(); let focused = self.focus == Some(Fid::Summary);
        let vchip = |id: &'static str, label: &'static str, on: bool, color: gpui::Hsla| div().id(id).flex().items_center().gap_2().px_3().h(px(32.0)).rounded_md().border_1().border_color(if on { color } else { p.border }).bg(if on { color.opacity(0.14) } else { gpui::transparent_black() }).cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).hover(move |x| x.bg(p.hover)).child(label);
        let summary_box = self.winp(Fid::Summary, div().id("sum-box").flex().px_3().py(px(8.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).min_h(px(90.0)).child(input_text(&self.summary, focused, "Summary (optional): what should the author know?", &p, false, 4)), cx);
        let msg = self.submit_msg.clone().map(|(ok, m)| div().flex().items_start().gap_2().px_3().py_2().rounded_md().border_1().border_color(if ok { p.success } else { p.danger }).bg(if ok { p.success.opacity(0.1) } else { p.danger.opacity(0.1) }).text_sm().text_color(if ok { p.success } else { p.danger })
            .child(icon(if ok { ic::CHECK_CIRCLE } else { ic::ERROR_OUTLINE }, 17.0, if ok { p.success } else { p.danger })).child(div().flex_1().min_w_0().child(m)));
        let body = div().flex().flex_col().gap_4().px_5().py_4()
            .child(div().flex().flex_col().gap_1()
                .child(div().text_sm().child(format!("{nd} draft{} will be posted{}.", if nd == 1 { "" } else { "s" }, if nai > 0 { format!(" ({nai} written by AI)") } else { String::new() })))
                .when(unviewed > 0, |d| d.child(div().flex().items_center().gap_2().text_sm().text_color(p.warn).child(icon(ic::WARNING, 17.0, p.warn)).child(format!("{unviewed} of {total} files are not marked viewed yet. You can still finish now.")))))
            .child(summary_box)
            .when(remote, |d| d.child(div().flex().flex_col().gap_2().child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Verdict")).child(div().flex().gap_2()
                .child(vchip("vd-comment", "Comment", v == Verdict::Comment, p.accent).on_click(cx.listener(|this, _, _, cx| { this.verdict = Verdict::Comment; cx.notify(); })))
                .child(vchip("vd-approve", "Approve", v == Verdict::Approve, p.success).on_click(cx.listener(|this, _, _, cx| { this.verdict = Verdict::Approve; cx.notify(); })))
                .when(!gitlab, |d| d.child(vchip("vd-changes", "Request changes", v == Verdict::RequestChanges, p.danger).on_click(cx.listener(|this, _, _, cx| { this.verdict = Verdict::RequestChanges; cx.notify(); })))))))
            .when(!remote, |d| d.child(div().text_sm().text_color(p.muted).child("This is a local review, so there is no host to post to. Copy your drafts as Markdown or save them to a file.")))
            .children(msg);
        let footer = div().flex().items_center().justify_end().gap_2().px_5().py_3().border_t_1().border_color(p.border).bg(p.surface)
            .child(icon_text_btn("fin-cancel", ic::CLOSE, "Cancel", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.finish_open = false; this.focus = None; cx.notify(); })))
            .child(if remote {
                if self.submit_busy { div().flex().items_center().gap_2().px_3().child(loading_dots(&p)).child(div().text_sm().text_color(p.accent).child("Posting…")).into_any_element() }
                else { icon_text_btn("fin-submit", ic::SEND, "Post review", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.start_submit(cx); cx.notify(); })).into_any_element() }
            } else { icon_text_btn("fin-export", ic::CONTENT_COPY, "Copy and save Markdown", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.export_local(cx); cx.notify(); })).into_any_element() });
        let card = div().id("finish-card").flex().flex_col().w(px(560.0)).rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(div().flex().items_center().gap_2().px_5().py_3().border_b_1().border_color(p.border).child(icon(ic::RATE_REVIEW, 20.0, p.accent)).child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Finish review")))
            .child(body).child(footer);
        deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.55)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.finish_open = false; this.focus = None; cx.notify(); }))
            .child(card)).with_priority(20)
    }
}

type TextField2 = perusal_app::textfield::TextField;

/// Small helper: attach a tooltip to a button after the fact.
trait TooltipText { fn tooltip_text(self, t: String, p: &Pal) -> Self; }
impl TooltipText for gpui::Stateful<gpui::Div> { fn tooltip_text(self, t: String, p: &Pal) -> Self { tip(self, t, p) } }
