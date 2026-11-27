mod extra;
pub fn show_key_name(k: &str) -> String { extra::show_key(k) }
use crate::icons as ic;
use crate::kit::{self, badge, ci_badge, diff_squares, field_key, icon, icon_btn, icon_text_btn, input_text, kbd, loading_dots, markdown_view, new_bounds, seg_box, seg_icon, tip, wire_input, Btn, FieldBounds, Fonts, Input, Pal};
use crate::model::{self, Model};
use gpui::{deferred, div, list, point, prelude::*, px, relative, uniform_list, AnyElement, ClickEvent, Context, FontWeight, HighlightStyle, KeyDownEvent, ListAlignment, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, SharedString, StyledText};
use perusal_agents::run::{agent_env, run_streaming, AgentEvent, RunError};
use perusal_agents::{CtxDraft, CtxThread, ReviewContext, launch_with, on_path, parse_review, supports_effort, Options, Selection};
use perusal_app::events::{self, Snapshot};
use perusal_app::markdown::{self, Located};
use perusal_app::notify::Kind as NKind;
use perusal_app::panes::{drop_zone, Arrangement, BatchView, Corner, DividerId, Drop, Layout, Rect};
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
enum Fid { Filter, Sort, Comment, Ask, Summary, Preset, Rename, Gen, Find, Palette, Reviewers }
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
struct Comp { editing: Option<i64>, global: bool, preview: bool, host: Option<HostTarget> }

#[derive(Clone)]
struct RLine { line: Line, runs: Vec<(Range<usize>, HighlightStyle)> }
#[derive(Clone)]
struct CItem { key: String, author: String, body: String, draft: Option<i64>, ai: bool, resolved: bool, outdated: bool, replies: Vec<(String, String)>, /// the code a suggestion in the body replaces
    orig: Option<String>, /// new-file lines the comment is about
    range: Option<(u32, u32)>, /// set for threads that live on the host
    host: Option<HostThread> }
/// What the card needs to act on a host thread.
#[derive(Clone)]
struct HostThread { id: String, resolve_id: Option<String>, comment_ids: Vec<String> }
#[derive(Clone)]
enum HostTarget { Reply(String), Edit(String, String) }
/// A collapsed comment: drawn as an icon in the line-number gutter of the line it belongs to.
#[derive(Clone)]
struct Mark { key: String, draft: bool, resolved: bool, range: Option<(u32, u32)> }
#[derive(Clone)]
enum RowKind { FileHeader, /// one block of a rendered Markdown file shown inside a group; selectable and commentable like code lines
    MdBlock(Located), Collapsed(String, bool), /// old and new picture (either side may be missing) and the note shown when one cannot be displayed
    Image(Option<Arc<gpui::Image>>, Option<Arc<gpui::Image>>, Option<(u32, u32)>, String), Note(String), Hunk(String), Uni(RLine), Split(Option<RLine>, Option<RLine>), Comment(CItem) }
#[derive(Clone)]
struct Row { fi: usize, kind: RowKind, /// ordinal among selectable rows
    ord: Option<usize>, new_line: Option<u32>, marks: Vec<Mark> }

/// One scrolling diff list. In split-pane mode there is one per file of the batch; stacked, a single one holds every file.
struct Pane { key: Vec<usize>, range: Range<usize>, list: ListState, all: bool }
/// A pane being dragged by its grip to swap places with another pane.
#[derive(Clone)]
struct DraggedPane { from: usize, title: SharedString, pal: Pal }
struct DragPaneView(DraggedPane);
impl Render for DragPaneView {
    fn render(&mut self, _w: &mut gpui::Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.0.pal; div().px_3().py_1().rounded_md().bg(p.surface).border_1().border_color(p.accent).shadow_lg().text_sm().text_color(p.fg).child(self.0.title.clone())
    }
}
/// What the mouse is dragging in the pane area.
#[derive(Clone)]
enum PaneDrag { Divider(DividerId), Corner(Corner) }

pub enum ViewEvent { Fullscreen, Home, OpenSettings, MoveHead { url: String, old_head: String }, Notice(NKind, String, Option<&'static str>), Renamed(String), Ci(String), State }
impl gpui::EventEmitter<ViewEvent> for View {}

pub struct View {
    /// Comment ids you have 👍-ed on the host, and the ids that set was fetched for.
    liked: HashSet<String>, likes_for: Vec<String>,
    new_head: Option<String>, head_dismissed: bool, rev_open: bool, rev_in: Input, me: Option<String>, keys: std::collections::BTreeMap<String, String>, palette_open: bool, pal_files: bool, pal_in: Input, pal_idx: usize, help_open: bool,
    pub m: Model, settings: Settings, theme: Theme, pal: Pal, fonts: Fonts,
    refreshing: bool, md_srcs: HashMap<usize, String>, md_olds: HashMap<usize, String>, find_open: bool, find_in: Input, find_hits: Vec<Hit>, find_cur: usize, find_page: Page, ov_scroll: gpui::ScrollHandle,
    focus: Option<Fid>, gen: Input, sort_open: bool, gen_for: Option<perusal_agents::GenKind>, gen_busy: bool, gen_cancel: Arc<AtomicBool>, gen_msg: Option<String>, forced: HashSet<usize>, collapsed_files: HashSet<usize>, sym_path: Option<String>, filter: Input, sort: Input, comment: Input, ask: Input, summary: Input, preset_name: Input, rename: Input,
    b_body: FieldBounds, b_composer: FieldBounds, b_panes: FieldBounds,
    /// Split panes: per-pane lists, the tiling, whether the batch is shown in panes, and the drag in progress.
    panes: Vec<Pane>, layout: Layout, batch_panes: bool, arrangement: Arrangement, pane_drag: Option<PaneDrag>, pane_hover: Option<(usize, Drop)>, pane_from: usize, pane_snapped: bool, fullscreen: bool, wheel_acc: f32,
    error: Option<String>, zoom: f32, composer: Option<Comp>, finish_open: bool, presets_open: bool,
    shown: Option<Vec<usize>>, nav_changed: bool, base: Vec<Row>, rows: Vec<Row>, md_rendered: bool, md_was: bool, md_jump: bool, diff_mode: DiffMode, sym_target: Option<usize>,
    collapsed_c: HashSet<String>, raw_c: HashSet<String>,
    sel: Option<(usize, usize)>, anchor: Option<usize>, dragging: bool, row_y: HashMap<usize, f32>, sel_x: f32, sb_drag: Option<(usize, f32)>, hxs: Vec<f32>, hb_drag: Option<(usize, f32)>, hx_texts: Vec<std::rc::Rc<std::cell::Cell<f32>>>, gutter: HashMap<usize, (usize, bool, bool)>,
    tree: bool, collapsed: HashSet<String>, trows: Vec<TreeRow>, multi: BTreeSet<usize>, last_clicked: Option<usize>,
    sug: Vec<Suggestion>, sug_replace: usize, sug_idx: Option<usize>, sug_for: Fid,
    left_open: bool, right_open: bool, left_w: f32, right_w: f32, resizing: Option<Side>, commits_h: Option<f32>, resizing_cd: Option<(f32, f32)>, commits_open: bool, drafts_open: bool, page: Page, info: Option<perusal_providers::MrInfo>, hover_c: Option<(usize, u32, u32)>, auto_c: HashSet<String>,
    tab: RightTab, chat: Vec<(String, String)>, ai_ctx: Option<Selection>, ai_notice: Option<String>,
    ai_job: Option<Job>, ai_events: Arc<Mutex<Vec<AgentEvent>>>, ai_cancel: Arc<AtomicBool>, ai_started: Option<Instant>, ai_activity: Vec<String>, ai_partial: String,
    verdict: Verdict, submit_busy: bool, submit_msg: Option<(bool, String)>,
    threads: Vec<Thread>, ci: String, snap: Snapshot, local_mr: Option<String>, notices: Vec<(NKind, String, Option<&'static str>)>, presets: Presets, project: String,
}

fn image_format(k: perusal_diff::image::ImageKind) -> gpui::ImageFormat {
    use perusal_diff::image::ImageKind as K;
    match k { K::Png => gpui::ImageFormat::Png, K::Jpeg => gpui::ImageFormat::Jpeg, K::Gif => gpui::ImageFormat::Gif, K::Webp => gpui::ImageFormat::Webp, K::Bmp => gpui::ImageFormat::Bmp, K::Tiff => gpui::ImageFormat::Tiff, K::Ico => gpui::ImageFormat::Ico, K::Svg => gpui::ImageFormat::Svg }
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
        Some(row) if matches!(row.kind, RowKind::Uni(_) | RowKind::Split(..) | RowKind::MdBlock(_) | RowKind::FileHeader) => { row.marks.push(Mark { key: c.key.clone(), draft: c.draft.is_some(), resolved: c.resolved, range: c.range }); true }
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
        m.vm.sync_review_state(&m.store); if let Ok(g) = m.store.load_groups(m.vm.review) { m.vm.set_manual_paths(g); } let _ = m.vm.refresh();
        // back where the reviewer left off: the same commit range and file (each only if it still exists)
        if let Ok(Some((file, commits))) = m.store.position(m.vm.review) {
            if let Some((n, o)) = commits { if m.graph.restore(&n, &o) { m.vm.commit_scope = m.graph.scope(&m.repo).ok().flatten(); let _ = m.vm.refresh(); } }
            if let Some(f) = file { m.vm.select_path(&f); }
        }
        let chat = m.store.messages(m.vm.review).unwrap_or_default();
        let (title, mr_state) = (m.title.clone(), m.mr_state.clone());
        let (keys, key_err) = extra::load_keys(settings.vim_keys);
        let mut v = View { liked: HashSet::new(), likes_for: vec![], new_head: None, head_dismissed: false, rev_open: false, rev_in: Input::new(""), me: None, keys, palette_open: false, pal_files: false, pal_in: Input::new(""), pal_idx: 0, help_open: false, refreshing: false, md_srcs: HashMap::new(), md_olds: HashMap::new(), find_open: false, find_in: Input::new(""), find_hits: vec![], find_cur: 0, find_page: Page::Code, ov_scroll: gpui::ScrollHandle::new(), pal: Pal::new(&theme), theme, fonts, focus: None, gen: Input::new(""), sort_open: false, gen_for: None, gen_busy: false, gen_cancel: Arc::new(AtomicBool::new(false)), gen_msg: None, forced: HashSet::new(), collapsed_files: HashSet::new(), sym_path: None, filter: Input::new(&f0), sort: Input::new(&s0), comment: Input::multi(""), ask: Input::new(""), summary: Input::multi(""), preset_name: Input::new(""), rename: Input::new(&title),
            b_body: new_bounds(), b_composer: new_bounds(), b_panes: new_bounds(), panes: vec![], layout: Layout::new(1, settings.pane_arrangement), batch_panes: settings.batch_view == BatchView::Panes, arrangement: settings.pane_arrangement, pane_drag: None, pane_hover: None, pane_from: 0, pane_snapped: false, fullscreen: false, wheel_acc: 0.0, error: None, zoom: 1.0, composer: None, finish_open: false, presets_open: false,
            shown: None, nav_changed: true, base: vec![], rows: vec![], md_rendered: false, md_was: false, md_jump: false, diff_mode: settings.diff_mode, sym_target: None,
            collapsed_c: HashSet::new(), raw_c: HashSet::new(), sel: None, anchor: None, dragging: false, row_y: HashMap::new(), sel_x: 0.0, sb_drag: None, hxs: vec![], hb_drag: None, hx_texts: vec![], gutter: HashMap::new(),
            tree: settings.files_tree, collapsed: HashSet::new(), trows: vec![], multi: BTreeSet::new(), last_clicked: None, sug: vec![], sug_replace: 0, sug_idx: None, sug_for: Fid::Filter,
            left_open: settings.left_open, right_open: settings.right_open, left_w: settings.left_width, right_w: settings.right_width, resizing: None, commits_h: settings.commits_height, resizing_cd: None, commits_open: true, drafts_open: true, page: if m.info.is_some() && settings.first_view == perusal_app::settings::FirstView::Overview { Page::Overview } else { Page::Code }, info: m.info.clone(), hover_c: None, auto_c: HashSet::new(),
            tab: RightTab::Review, chat, ai_ctx: None, ai_notice: None, ai_job: None, ai_events: Arc::new(Mutex::new(vec![])), ai_cancel: Arc::new(AtomicBool::new(false)), ai_started: None, ai_activity: vec![], ai_partial: String::new(),
            verdict: Verdict::Comment, submit_busy: false, submit_msg: None, threads: vec![], ci: String::new(), snap: Snapshot { state: mr_state, ..Default::default() }, local_mr: None, notices: vec![], presets, project, settings, m };
        if let Some(e) = key_err { v.note(NKind::Warning, e, None); }
        v.report_moved(); v
    }
    pub fn key_id(&self) -> String { self.m.remote.clone().map(|u| format!("remote:{u}")).unwrap_or_else(|| format!("local:{}#{}", self.m.repo.display(), self.m.branch)) }
    fn current_file(&self) -> Option<usize> { self.m.vm.current_batch().first().copied() }
    fn save_groups(&self) { let _ = self.m.store.save_groups(self.m.vm.review, &self.m.vm.manual_paths()); }
    fn save_settings(&self) { model::save_settings(&self.settings); }
    /// Puts a file's path on the clipboard (the copy button in the file's top bar).
    fn copy_path(&mut self, path: &str, cx: &mut Context<Self>) { cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.to_string())); self.note(NKind::Success, format!("Copied {path}"), Some("copied-path")); self.flush(cx); }
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
    pub fn apply_settings(&mut self, s: Settings, fonts: Fonts) {
        self.fonts = fonts; self.keys = extra::load_keys(s.vim_keys).0;
        let regroup = s.group_related != self.settings.group_related || s.group_rules != self.settings.group_rules || s.files_tree != self.settings.files_tree;
        let diff_changed = s.diff_mode != self.settings.diff_mode;
        let panes_changed = (s.batch_view != self.settings.batch_view, s.pane_arrangement != self.settings.pane_arrangement);
        self.theme = theme::load(&s.theme, Some(&model::themes_dir())); self.pal = Pal::new(&self.theme);
        self.tree = s.files_tree; self.m.vm.dir_order = s.files_tree; self.m.vm.grouping = s.group_related; self.m.vm.rules = s.group_rules.clone();
        if !s.ai_on() { if self.tab == RightTab::Ai { self.tab = RightTab::Review; } self.gen_for = None; }
        if diff_changed { self.diff_mode = s.diff_mode; }
        self.settings.left_width = self.left_w; self.settings.right_width = self.right_w;
        let ws = s.hide_whitespace != self.settings.hide_whitespace;
        self.presets = model::load_presets();
        if panes_changed.0 { self.batch_panes = s.batch_view == BatchView::Panes; }
        // a changed preset replaces a hand-made layout, otherwise the setting would seem to do nothing
        if panes_changed.1 { self.arrangement = s.pane_arrangement; self.layout = Layout::new(self.panes.len(), s.pane_arrangement); }
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
        if batch.is_empty() { self.base.clear(); self.rows.clear(); self.shown = None; self.panes.clear(); return; }
        if self.shown.as_ref() == Some(&batch) { return; }
        self.reset_hx();
        let new_batch = self.nav_changed;
        if self.nav_changed { self.md_rendered = self.settings.markdown == MdMode::Rendered && batch.iter().any(|&i| markdown::is_markdown(&self.m.vm.files[i].path) && !self.m.vm.files[i].symlink); self.nav_changed = false; }
        // opening a batch rendered, or switching to the rendered view, starts at the first change instead of the top of the file
        if self.md_rendered && (new_batch || !self.md_was) { self.md_jump = true; }
        self.md_was = self.md_rendered;
        self.shown = Some(batch.clone()); self.sym_target = None; self.sym_path = None;
        self.remember_position();
        let p = self.pal;
        self.md_srcs.clear(); self.md_olds.clear();
        let mut rows: Vec<Row> = Vec::new();
        let mk = |fi: usize, kind: RowKind| Row { fi, kind, ord: None, new_line: None, marks: vec![] };
        for &fi in &batch {
            let f = self.m.vm.files[fi].clone();
            // with a commit or range selected in the graph the diff shows only that range's changes
            let (old_rev, new_rev) = self.diff_revs();
            let old = perusal_gitops::show(&self.m.repo, &old_rev, &f.path).unwrap_or_default();
            let new = perusal_gitops::show(&self.m.repo, &new_rev, &f.path).unwrap_or_default();
            if batch.len() > 1 { rows.push(mk(fi, RowKind::FileHeader)); if self.file_collapsed(fi) { continue; } }
            if f.symlink {
                let t = |s: &str| s.trim().to_string();
                rows.push(mk(fi, RowKind::Note(match (old.is_empty(), new.is_empty()) { (true, false) => format!("Symbolic link to {}", t(&new)), (false, true) => format!("Symbolic link to {} was removed", t(&old)), _ => format!("Symbolic link retargeted: {} → {}", t(&old), t(&new)) })));
                if batch.len() == 1 { let tgt = resolve_link(&f.path, if new.is_empty() { &old } else { &new }); self.sym_path = Some(tgt.clone()); self.sym_target = self.m.vm.files.iter().position(|x| x.path == tgt && !x.symlink); }
            }
            if self.md_rendered && !f.symlink && markdown::is_markdown(&f.path) && (!f.noisy() || self.forced.contains(&fi)) {
                // rendered like GitHub's rich diff: additions green, deletions red and struck through, edited words marked.
                // Each block (a paragraph, list item, table row or code line) is its own selectable row, so any line can be commented on.
                let lbs = if old.is_empty() || new.is_empty() { markdown::parse_located(if new.is_empty() { &old } else { &new }) } else { markdown::parse_diff(&old, &new) };
                for lb in lbs { let (last, gone) = (lb.last_line, lb.change == markdown::Change::Removed); rows.push(Row { fi, kind: RowKind::MdBlock(lb), ord: None, new_line: (!gone).then_some(last), marks: vec![] }); }
                self.md_srcs.insert(fi, if new.is_empty() { old.clone() } else { new.clone() }); self.md_olds.insert(fi, old.clone()); continue;
            }
            let why_binary = format!("{} is a binary file, so there is no text to show.", f.path);
            if !f.symlink && perusal_diff::image::looks_like_image(&f.path) {
                // pictures are shown (old next to new, whatever the diff mode) instead of a text diff; anything unreadable keeps the usual note
                let load = |rev: &str| perusal_gitops::show_bytes(&self.m.repo, rev, &f.path).filter(|b| !b.is_empty());
                let (ob, nb) = (load(&old_rev), load(&new_rev));
                let pic = |b: &Option<Vec<u8>>| b.as_ref().map(|b| perusal_diff::image::sniff(&f.path, b).map(|k| (Arc::new(gpui::Image::from_bytes(image_format(k), b.clone())), perusal_diff::image::dimensions(b))));
                let (oi, ni) = (pic(&ob), pic(&nb));
                if (ob.is_some() || nb.is_some()) && oi.iter().chain(ni.iter()).all(|x| x.is_some()) {
                    let size = ni.as_ref().or(oi.as_ref()).and_then(|x| x.as_ref()).and_then(|x| x.1);
                    rows.push(mk(fi, RowKind::Image(oi.and_then(|x| x).map(|x| x.0), ni.and_then(|x| x).map(|x| x.0), size, why_binary))); continue;
                }
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
            // files with nothing on one side (added, deleted, or only additions / only deletions) always use a single column
            let split = self.diff_mode == DiffMode::Split && !f.one_sided();
            // line-number columns: as wide as the file's largest number needs, and only the sides that have numbers in the rows shown
            self.gutter.insert(fi, (perusal_diff::digits(old.lines().count().max(new.lines().count()) as u32).max(3), lines.iter().any(|l| l.old_no.is_some()), lines.iter().any(|l| l.new_no.is_some())));
            if !split {
                for l in &lines { hunk(&mut rows, l.old_no.unwrap_or(0), l.new_no.unwrap_or(0)); rows.push(Row { fi, kind: RowKind::Uni(rl(l)), ord: None, new_line: l.new_no, marks: vec![] }); }
            } else {
                for (a, b) in split_pairs(&lines) {
                    hunk(&mut rows, a.and_then(|i| lines[i].old_no).unwrap_or(0), b.and_then(|i| lines[i].new_no).unwrap_or(0));
                    rows.push(Row { fi, kind: RowKind::Split(a.map(|i| rl(&lines[i])), b.map(|i| rl(&lines[i]))), ord: None, new_line: b.and_then(|i| lines[i].new_no).or_else(|| a.and_then(|i| lines[i].new_no)), marks: vec![] });
                }
            }
        }
        let mut ord = 0; for r in &mut rows { if matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..) | RowKind::MdBlock(_)) { r.ord = Some(ord); ord += 1; } }
        self.base = rows; self.rebuild_rows();
        if std::mem::take(&mut self.md_jump) {
            let first = self.rows.iter().position(|r| matches!(&r.kind, RowKind::MdBlock(lb) if lb.change != markdown::Change::None));
            if let Some(row) = first { if let Some(pn) = self.panes.get(self.pane_of(row)) { pn.list.scroll_to(gpui::ListOffset { item_ix: row.saturating_sub(pn.range.start).saturating_sub(1), offset_in_item: px(0.0) }); } }
        }
    }

    /// Comments for a file: (end line it belongs to, card). Remote threads (anyone's) and your own drafts.
    fn comments_for(&self, fi: usize) -> Vec<(Option<u32>, CItem)> {
        let path = &self.m.vm.files[fi].path; let mut out = vec![];
        for t in self.threads.iter().filter(|t| t.path.as_deref() == Some(path.as_str())) {
            let (author, body) = t.comments.first().cloned().unwrap_or_default();
            out.push((t.line, CItem { key: format!("t:{}", t.id), author, body, draft: None, ai: false, resolved: t.resolved, outdated: false, replies: t.comments.iter().skip(1).cloned().collect(), orig: None, range: t.line.map(|l| (l, l)),
                host: Some(HostThread { id: t.id.clone(), resolve_id: self.m.mr.as_ref().and_then(|r| t.resolve_id(r.kind).map(String::from)), comment_ids: t.comment_ids.clone() }) }));
        }
        for d in self.m.store.drafts(self.m.vm.review).unwrap_or_default().into_iter().filter(|d| d.file.as_deref() == Some(path.as_str())) {
            let orig = (d.body.contains("```suggestion")).then(|| d.start_line.zip(d.end_line).and_then(|(a, b)| {
                let t = perusal_gitops::show(&self.m.repo, &self.m.branch, path)?;
                let v: Vec<&str> = t.lines().skip(a.saturating_sub(1) as usize).take((b + 1).saturating_sub(a) as usize).collect(); (!v.is_empty()).then(|| v.join("\n"))
            })).flatten();
            out.push((d.end_line.or(d.start_line), CItem { key: format!("d:{}", d.id), author: "You".into(), body: d.body, draft: Some(d.id), ai: d.ai, resolved: false, outdated: false, replies: vec![], orig, range: d.end_line.or(d.start_line).map(|e| (d.start_line.unwrap_or(e).min(e), e)), host: None }));
        }
        out
    }
    /// Inserts comment cards into the diff rows next to the lines they belong to.
    fn rebuild_rows(&mut self) {
        let tops: Vec<(Vec<usize>, gpui::ListOffset)> = self.panes.iter().map(|p| (p.key.clone(), p.list.logical_scroll_top())).collect();
        let mut out: Vec<Row> = Vec::with_capacity(self.base.len() + 8);
        let files: Vec<usize> = { let mut f: Vec<usize> = vec![]; for r in &self.base { if !f.contains(&r.fi) { f.push(r.fi); } } f };
        for fi in files {
            let mut pending = self.comments_for(fi);
            // resolved threads start collapsed (once; expanding one by hand sticks)
            for (_, c) in &pending { if c.resolved && self.auto_c.insert(c.key.clone()) { self.collapsed_c.insert(c.key.clone()); } }
            let group: Vec<&Row> = self.base.iter().filter(|r| r.fi == fi).collect();
            if self.file_collapsed(fi) { out.extend(group.into_iter().cloned()); continue; }
            let mut placed_top = false; let mut anchor = out.len();
            let card = |c: CItem| Row { fi, kind: RowKind::Comment(c), ord: None, new_line: None, marks: vec![] };
            for r in group {
                out.push(r.clone());
                if matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..) | RowKind::MdBlock(_) | RowKind::FileHeader) { anchor = out.len() - 1; }
                let header_like = matches!(r.kind, RowKind::FileHeader | RowKind::Note(_) | RowKind::Collapsed(..));
                if !placed_top && !header_like { placed_top = true; }
                if header_like && !placed_top {
                    // file-level comments (no line) sit right under the file header
                    let (file_level, rest): (Vec<_>, Vec<_>) = pending.into_iter().partition(|(l, _)| l.is_none()); pending = rest;
                    for (_, c) in file_level {
                        if self.collapsed_c.contains(&c.key) && add_mark(&mut out, anchor, &c) { continue; }
                        out.push(card(c));
                    }
                }
                if let (Some(n), true) = (r.new_line, matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..) | RowKind::MdBlock(_))) {
                    // a comment on any line of a rendered block shows under that block
                    let span = if let RowKind::MdBlock(lb) = &r.kind { (lb.first_line, lb.last_line) } else { (n, n) };
                    let (here, rest): (Vec<_>, Vec<_>) = pending.into_iter().partition(|(l, _)| l.map(|x| x >= span.0 && x <= span.1).unwrap_or(false)); pending = rest;
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
        self.build_panes(tops);
    }
    /// True when the shown batch has several files and the user wants them in split panes.
    fn paned(&self) -> bool { self.batch_panes && self.shown.as_ref().map(|b| b.len() > 1).unwrap_or(false) }
    /// Splits the rows into one list per file (split panes) or a single list (stacked), keeping scroll positions.
    fn build_panes(&mut self, tops: Vec<(Vec<usize>, gpui::ListOffset)>) {
        let mut spans: Vec<(Vec<usize>, Range<usize>)> = vec![];
        if self.paned() {
            for (i, r) in self.rows.iter().enumerate() { match spans.last_mut() { Some((k, rg)) if k[0] == r.fi => rg.end = i + 1, _ => spans.push((vec![r.fi], i..i + 1)) } }
        } else { spans.push((self.shown.clone().unwrap_or_default(), 0..self.rows.len())); }
        let old = std::mem::take(&mut self.panes);
        for (key, range) in spans {
            let len = range.len();
            // measure every row up front so the scroll range is exact (otherwise it keeps growing as unseen rows are measured); huge diffs stay lazy
            let all = len <= 4000;
            let list = match old.iter().find(|o| o.key == key && o.all == all) {
                Some(o) => { o.list.reset(len); o.list.clone() }
                None => { let l = ListState::new(len, ListAlignment::Top, px(600.0)); if all { l.clone().measure_all() } else { l } }
            };
            if let Some((_, t)) = tops.iter().find(|(k, _)| *k == key) { list.scroll_to(*t); }
            self.panes.push(Pane { key, range, list, all });
        }
        let n = self.panes.len();
        self.hxs.resize(n, 0.0); self.hx_texts.truncate(n); while self.hx_texts.len() < n { self.hx_texts.push(Default::default()); }
        if self.paned() && !self.layout.fits(n, self.arrangement) { self.layout = Layout::new(n, self.arrangement); }
    }
    /// A file folded away by its header. Split panes always show their whole file (they scroll instead), so folding only exists stacked.
    fn file_collapsed(&self, fi: usize) -> bool { !self.paned() && self.collapsed_files.contains(&fi) }
    fn reset_hx(&mut self) { self.hxs.fill(0.0); for c in &self.hx_texts { c.set(0.0); } }
    /// The pane a row belongs to.
    fn pane_of(&self, row: usize) -> usize { self.panes.iter().position(|p| p.range.contains(&row)).unwrap_or(0) }
    /// Pane `k`'s rectangle in unit coordinates of the pane area.
    fn pane_rect(&self, k: usize) -> Rect { if self.paned() { self.layout.rects().get(k).map(|r| r.1).unwrap_or_default() } else { Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 } } }
    fn set_batch_panes(&mut self, on: bool) { if self.batch_panes != on { self.batch_panes = on; self.clear_selection(); self.shown = None; } }
    fn set_arrangement(&mut self, a: Arrangement) { self.arrangement = a; self.layout = Layout::new(self.panes.len(), a); self.clear_selection(); self.rebuild_rows(); }
    pub fn set_fullscreen(&mut self, on: bool, cx: &mut Context<Self>) { if self.fullscreen != on { self.fullscreen = on; cx.emit(ViewEvent::Fullscreen); } }
    pub fn is_fullscreen(&self) -> bool { self.fullscreen }
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
        let (a, b) = self.sel?; let fi = self.rows.iter().find(|r| r.ord == Some(a))?.fi;
        let (mut lo, mut hi, mut snippet) = (u32::MAX, 0u32, String::new());
        let (mut olo, mut ohi) = (u32::MAX, 0u32);
        // a rendered Markdown file has only block rows: anchor and quote them from one side (see `markdown::selection_of`)
        let blocks: Vec<&Located> = self.rows.iter().filter(|r| r.fi == fi && r.ord.map(|o| o >= a && o <= b).unwrap_or(false)).filter_map(|r| if let RowKind::MdBlock(lb) = &r.kind { Some(lb) } else { None }).collect();
        if !blocks.is_empty() {
            let (old, new) = (self.md_olds.get(&fi).map(String::as_str).unwrap_or(""), self.md_srcs.get(&fi).map(String::as_str).unwrap_or(""));
            return markdown::selection_of(&blocks, old, new).map(|m| Selection { file: self.m.vm.files[fi].path.clone(), start: m.start, end: m.end, snippet: m.snippet, old_side: m.old_side });
        }
        for r in self.rows.iter().filter(|r| r.fi == fi && r.ord.map(|o| o >= a && o <= b).unwrap_or(false)) {
            let lines: Vec<&Line> = match &r.kind { RowKind::Uni(l) => vec![&l.line], RowKind::Split(l, rr) => l.iter().chain(rr.iter()).map(|x| &x.line).collect(), _ => vec![] };
            for l in lines {
                if let Some(n) = l.new_no { lo = lo.min(n); hi = hi.max(n); } else if let Some(n) = l.old_no { olo = olo.min(n); ohi = ohi.max(n); }
                snippet.push_str(match l.kind { Kind::Added => "+", Kind::Removed => "-", Kind::Context => " " }); snippet.push_str(&l.text); snippet.push('\n');
            }
        }
        // only removed lines selected: the comment belongs on the old side; mixed selections anchor on their new lines
        if hi == 0 && ohi > 0 { return Some(Selection { file: self.m.vm.files[fi].path.clone(), start: olo, end: ohi, snippet, old_side: true }); }
        (hi > 0).then(|| Selection { file: self.m.vm.files[fi].path.clone(), start: lo, end: hi, snippet, old_side: false })
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
    fn clear_selection(&mut self) { self.sel = None; self.anchor = None; self.row_y.clear(); }
    fn has_selection(&self) -> bool { self.sel.is_some() }

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
        if self.gen_busy || !self.settings.ai_on() { return; }
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
            .children(self.gen_msg.clone().map(|m| div().flex().items_start().gap_1().text_xs().text_color(p.danger).child(div().flex_1().min_w_0().child(m.clone())).child(kit::copy_btn("gen-err-copy", m, &p))))
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
    /// Saves the open file and the commit range selected in the graph, so reopening the review lands on the same spot.
    fn remember_position(&self) {
        let file = self.current_file().and_then(|fi| self.m.vm.files.get(fi)).map(|f| f.path.as_str());
        let commits = self.m.graph.selected_shas().filter(|_| self.m.vm.commit_scope.is_some());
        let _ = self.m.store.set_position(self.m.vm.review, file, commits.as_ref().map(|(n, o)| (n.as_str(), o.as_str())));
    }
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
        self.composer = Some(Comp { editing, global, preview: false, host: None }); self.focus = Some(Fid::Comment);
    }
    fn save_composer(&mut self, cx: &mut Context<Self>) { self.save_composer_with(false, cx); }
    /// Saves the comment as a draft; with `publish` it is also posted to the host right away (remote reviews).
    fn save_composer_with(&mut self, publish: bool, cx: &mut Context<Self>) {
        let text = self.comment.f.text.trim().to_string(); if text.is_empty() { return; }
        let Some(c) = self.composer.take() else { return };
        if let Some(h) = c.host {
            self.comment.f.clear(); self.focus = None;
            let act = match h { HostTarget::Reply(thread) => model::HostAct::Reply { thread, body: text }, HostTarget::Edit(thread, comment) => model::HostAct::Edit { thread, comment, body: text } };
            self.host_act(act, cx); return;
        }
        let mut added: Option<i64> = None;
        if let Some(id) = c.editing { let _ = self.m.store.edit_draft(id, &text); self.note(NKind::Success, "Comment updated.", None); if publish { added = Some(id); } }
        else {
            let mut sel = if c.global { None } else { self.selection() };
            // while scoped to commits, line numbers belong to that commit's version: map them onto the review head or refuse
            if let Some(s) = sel.as_mut() {
                if self.m.vm.commit_scope.is_some() { if let Some((_, new_rev)) = self.m.graph.revs() {
                    let mapped = if s.old_side { None } else {
                        let (a, b) = (perusal_gitops::show(&self.m.repo, &new_rev, &s.file).unwrap_or_default(), perusal_gitops::show(&self.m.repo, &self.m.branch, &s.file).unwrap_or_default());
                        perusal_diff::text::reanchor(&a, &b, (s.start, s.end))
                    };
                    match mapped {
                        Some((a, b)) => { s.start = a; s.end = b; }
                        None => {
                            self.note(NKind::Warning, "Those lines were changed by a later commit, so the comment cannot be placed on the review head. Clear the commit scope (x) and comment there.", None);
                            self.open_composer(false, None); self.comment.f.set(&text); return;
                        }
                    }
                } }
            }
            match (sel, self.current_file(), c.global) {
                (Some(s), _, _) => { added = self.m.store.add_draft_side(self.m.vm.review, Some(&s.file), Some((s.start, s.end)), &text, false, s.old_side).ok(); self.note(NKind::Success, format!("Draft added on {}:{}-{}{}", s.file, s.start, s.end, if s.old_side { " (removed lines)" } else { "" }), None); }
                (None, Some(fi), false) => { let path = self.m.vm.files[fi].path.clone(); added = self.m.store.add_draft(self.m.vm.review, Some(&path), None, &text, false).ok(); self.note(NKind::Success, format!("Draft added on {path}"), None); }
                _ => { added = self.m.store.add_draft(self.m.vm.review, None, None, &text, false).ok(); self.note(NKind::Success, "Draft added on the merge request.", None); }
            }
        }
        self.comment.f.clear(); self.focus = None; self.clear_selection(); self.resync();
        // Saved locally first so nothing is lost if the host is unreachable; then posted right away.
        if let (Some(id), true) = (added, self.instant() || (publish && self.m.mr.is_some())) { self.publish_draft(id, cx); }
    }
    fn toggle_viewed(&mut self) { if let Some(fi) = self.current_file() { let v = !self.m.vm.files[fi].viewed; for i in self.m.vm.current_batch().to_vec() { self.m.vm.set_viewed(&self.m.store, i, v); } } }
    fn next(&mut self) { self.m.vm.next(&self.m.store); self.reload(); }
    fn back(&mut self) { self.m.vm.back(); self.reload(); }
    fn step(&mut self, d: i32) { let n = self.m.vm.batch_count() as i32; if n > 0 { self.m.vm.cursor = (self.m.vm.cursor as i32 + d).rem_euclid(n) as usize; self.reload(); } }
    /// (old, new) revisions the diff compares: the whole review, or the commit range selected in the graph.
    fn diff_revs(&self) -> (String, String) {
        match self.m.vm.commit_scope.as_ref().and(self.m.graph.revs()) { Some(r) => r, None => (self.m.base_sha.clone(), self.m.branch.clone()) }
    }
    fn clear_scope(&mut self) { self.m.graph.clear(); self.m.vm.commit_scope = None; let _ = self.m.vm.refresh(); self.reload(); }
    fn pick_commit(&mut self, i: usize, shift: bool) { self.page = Page::Code;
        if self.m.graph.selection == Some((i, i)) && !shift { self.clear_scope(); return; }
        match (shift, self.m.graph.selection) { (true, Some((a, _))) => self.m.graph.select_range(a, i), _ => self.m.graph.click(i) } self.m.vm.commit_scope = self.m.graph.scope(&self.m.repo).ok().flatten(); let _ = self.m.vm.refresh(); self.reload();
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
        members.dedup(); self.m.vm.group_files_manually(members); self.save_groups(); self.multi.clear(); self.reload();
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
            Some(Fid::Palette) => { self.palette_open = false; }
            Some(Fid::Reviewers) => { self.rev_open = false; }
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
    /// Re-fetches the request (new commits, state, CI, threads) and reloads the review on the latest head, keeping drafts, viewed marks, filter and sort.
    /// Local branch reviews re-read the branch instead.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing { return; }
        self.refreshing = true; cx.notify();
        let (remote, repo, branch, base, mr) = (self.m.remote.clone(), self.m.repo.clone(), self.m.branch.clone(), self.m.base_sha.clone(), self.m.mr.clone());
        let task = cx.background_executor().spawn(async move {
            let m = match &remote { Some(u) => Model::open_remote(u), None => Model::open_local(&repo, &branch, Some(&base)) };
            let st = mr.as_ref().map(model::fetch_remote_state);
            (m, st)
        });
        cx.spawn(async move |this, cx| { let (m, st) = task.await; let _ = this.update(cx, |t, cx| t.finish_refresh(m, st, cx)); }).detach();
    }
    fn finish_refresh(&mut self, m: Result<Model, model::OpenErr>, st: Option<Result<model::RemoteState, String>>, cx: &mut Context<Self>) {
        self.refreshing = false;
        match m { Ok(new) => self.adopt_model(new), Err(e) => self.note(NKind::Error, format!("Could not refresh: {e}"), None) }
        match st { Some(Ok(s)) => self.apply_remote(s, cx), Some(Err(e)) => self.note(NKind::Error, format!("Could not refresh the request from the host: {e}"), None), None => {} }
        self.flush(cx); cx.notify();
    }
    /// Tells the user what following a newer head changed (viewed marks dropped, drafts that could not follow their lines).
    fn report_moved(&mut self) {
        let Some(mv) = self.m.moved.take() else { return };
        if !mv.unmarked.is_empty() {
            let (n, one) = (mv.unmarked.len(), mv.unmarked.len() == 1);
            let names = mv.unmarked.iter().take(5).cloned().collect::<Vec<_>>().join(", ") + if n > 5 { ", …" } else { "" };
            self.note(NKind::Info, format!("{n} viewed file{} changed since you reviewed {}, so {} unmarked: {names}", if one { "" } else { "s" }, if one { "it" } else { "them" }, if one { "it was" } else { "they were" }), None);
        }
        if mv.lost > 0 { self.note(NKind::Warning, format!("{} draft{} could not follow {} lines to the new commits and keep the old line numbers.", mv.lost, if mv.lost == 1 { "" } else { "s" }, if mv.lost == 1 { "its" } else { "their" }), None); }
    }
    /// Swaps in a freshly opened model of the same review, carrying over the view settings, filter and sort; drafts follow their lines to the new head.
    fn adopt_model(&mut self, mut new: Model) {
        let tip = |m: &Model| m.graph.rows.iter().find(|r| r.in_mr).map(|r| r.sha.clone()).unwrap_or_default();
        let (old_tip, old_n) = (tip(&self.m), self.m.graph.rows.iter().filter(|r| r.in_mr).count());
        let (old_head, new_head) = (self.m.branch.clone(), new.branch.clone());
        new.vm.dir_order = self.m.vm.dir_order; new.vm.grouping = self.m.vm.grouping; new.vm.rules = self.m.vm.rules.clone(); new.vm.mark_on_next = self.m.vm.mark_on_next; new.vm.hide_whitespace = self.m.vm.hide_whitespace;
        let (f, s) = (self.m.vm.filter.clone(), self.m.vm.sort.clone()); let _ = new.vm.set_filter(&f); let _ = new.vm.set_sort(&s);
        // a remote model already followed the new head when it was opened (viewed marks of changed files dropped, drafts moved); a local branch is handled here
        let moved = new.moved.take();
        if new.mr.is_none() {
            let new_tip_now = tip(&new);
            let (o_head, n_head) = (if old_tip.is_empty() { old_head.clone() } else { old_tip.clone() }, if new_tip_now.is_empty() { new_head.clone() } else { new_tip_now });
            let mut unmarked: Vec<String> = vec![];
            if o_head != n_head || self.m.base_sha != new.base_sha {
                for f in self.m.vm.files.iter().filter(|f| f.viewed) {
                    if perusal_gitops::file_changed_between(&new.repo, &self.m.base_sha, &o_head, &new.base_sha, &n_head, &f.path) { unmarked.push(f.path.clone()); }
                }
            }
            for path in &unmarked { if let Some(i) = new.vm.files.iter().position(|f| &f.path == path) { new.vm.set_viewed(&new.store, i, false); } }
            new.moved = Some(model::Moved { unmarked, lost: 0 });
        } else { new.moved = moved; }
        new.vm.sync_review_state(&new.store); let _ = new.vm.refresh();
        let (new_tip, new_n) = (tip(&new), new.graph.rows.iter().filter(|r| r.in_mr).count());
        self.m = new; self.snap.head = self.m.branch.clone();
        self.collapsed_files.clear(); self.forced.clear(); self.multi.clear(); self.clear_selection(); self.reload();
        self.report_moved();
        if old_tip != new_tip {
            self.note(NKind::Success, format!("Refreshed: now {new_n} commit{} (was {old_n}). Drafts were kept.", if new_n == 1 { "" } else { "s" }), None);
        } else { self.note(NKind::Info, "Already up to date.", None); }
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
        let nh = (self.m.mr.is_some() && !now.head.is_empty() && now.head != self.m.branch).then(|| now.head.clone());
        if nh != self.new_head { self.head_dismissed = false; self.new_head = nh; }
        if self.m.mr_state != now.state { cx.emit(ViewEvent::State); } // the Recent page lists this state too
        self.m.mr_state = now.state.clone(); let _ = self.m.store.set_mr_state(self.m.vm.review, &now.state);
        let ci = now.ci.clone().unwrap_or_default();
        if ci != self.ci { self.ci = ci.clone(); cx.emit(ViewEvent::Ci(ci)); }
        self.snap = now; self.info = Some(s.info.clone());
        if s.me.is_some() { self.me = s.me.clone(); }
        if s.threads != self.threads { self.threads = s.threads; self.rebuild_rows(); } else { self.threads = s.threads; }
        self.refresh_likes(false, cx);
    }
    fn ci_url(&self) -> Option<String> { let w = self.m.web_url.clone()?; Some(perusal_providers::ci_page(&self.m.mr.as_ref()?.kind, &w)) }

    // ---------- AI
    fn ai_options(&self) -> Options { Options { model: self.settings.model.clone(), effort: self.settings.effort.clone(), path: self.settings.agent_path.clone() } }
    fn start_ai(&mut self, job: Job, question: String, cx: &mut Context<Self>) {
        if self.busy() || !self.settings.ai_on() { return; }
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
            RowKind::Image(..) => vec![],
            RowKind::MdBlock(lb) => markdown::block_texts(std::slice::from_ref(&lb.block)),
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
        if self.page == Page::Code { if let Some(pn) = self.panes.get(self.pane_of(h.row)) { pn.list.scroll_to(gpui::ListOffset { item_ix: h.row.saturating_sub(pn.range.start).saturating_sub(3), offset_in_item: px(0.0) }); } } else { self.ov_scroll.scroll_to_item(h.row); }
    }
    /// The active search text while looking at the diff (empty otherwise); rendered Markdown blocks highlight with it.
    fn find_query(&self) -> String { if self.find_open && self.page == Page::Code { self.find_in.f.text.clone() } else { String::new() } }
    /// Ordinal of the current hit among the hits of one rendered Markdown block row.
    fn md_block_current(&self, row: usize) -> Option<usize> { self.hits_in_row(row).iter().position(|h| h.2) }
    /// Hits inside one row as (part, range, is the current hit).
    fn hits_in_row(&self, row: usize) -> Vec<(usize, Range<usize>, bool)> {
        if !self.find_open || self.find_hits.is_empty() || self.page != Page::Code { return vec![]; }
        let start = self.find_hits.partition_point(|h| h.row < row);
        self.find_hits[start..].iter().enumerate().take_while(|(_, h)| h.row == row).map(|(i, h)| (h.part, h.range.clone(), start + i == self.find_cur)).collect()
    }
    /// Background for rows whose text is not drawn as a plain code line (headers, comments, hunks), so hits there are still visible.
    fn row_tint(&self, ix: usize) -> Option<gpui::Hsla> {
        let kind_plain = self.rows.get(ix).map(|r| matches!(r.kind, RowKind::Uni(_) | RowKind::Split(..) | RowKind::MdBlock(_))).unwrap_or(true);
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
        self.submission_of(drafts, self.summary.f.text.trim().to_string(), self.verdict)
    }
    fn submission_of(&self, drafts: Vec<perusal_core::store::Draft>, mut summary: String, verdict: Verdict) -> Submission {
        let (mut comments, mut general) = (vec![], vec![]);
        for d in drafts {
            match (&d.file, d.start_line, d.end_line) {
                (Some(f), Some(a), Some(b)) if d.old_side => comments.push(InlineComment { path: f.clone(), start_line: a, end_line: b, body: d.body, start_old: a, end_old: b, old_side: true }),
                (Some(f), Some(a), Some(b)) => {
                    let (old, new) = (perusal_gitops::show(&self.m.repo, &self.m.base_sha, f).unwrap_or_default(), perusal_gitops::show(&self.m.repo, &self.m.branch, f).unwrap_or_default());
                    let (start_old, end_old) = (perusal_diff::text::old_line_for_new(&old, &new, a), perusal_diff::text::old_line_for_new(&old, &new, b));
                    // hosts reject inline comments on lines outside the diff ("Line could not be resolved"), so those go into the summary instead
                    let shown: HashSet<u32> = diff_lines(&old, &new, 3).iter().filter_map(|l| l.new_no).collect();
                    if self.m.mr.is_some() && !(shown.contains(&a) && shown.contains(&b)) {
                        let at = if a == b { format!("{f}:{a}") } else { format!("{f}:{a}-{b}") };
                        summary.push_str(&format!("\n\n**{at}**: {}", d.body));
                    } else {
                        comments.push(InlineComment { path: f.clone(), start_line: a, end_line: b, body: d.body, start_old, end_old, old_side: false })
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
        if sub.comments.is_empty() && sub.general.is_empty() && sub.summary.is_empty() && sub.verdict == Verdict::Comment { self.submit_msg = Some((false, "Add a summary or at least one comment, or choose Approve or Request changes.".into())); return; }
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
        if self.help_open { self.help_open = false; cx.notify(); return; }
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
                Fid::Palette => self.palette_key(key, ks, cx),
                Fid::Reviewers => self.reviewers_key(key, ks, cx),
            }
            self.flush(cx); cx.notify(); return;
        }
        if ctrl && matches!(key, "=" | "+" | "-" | "0") { match key { "=" | "+" => self.zoom = theme::zoom(self.zoom, 1), "-" => self.zoom = theme::zoom(self.zoom, -1), _ => self.zoom = theme::zoom(self.zoom, 0) } cx.notify(); return; }
        if key == "f5" { self.refresh(cx); }
        else if key == "escape" {
            if self.has_selection() || !self.multi.is_empty() { self.clear_selection(); self.multi.clear(); } else if self.composer.is_some() { self.composer = None; } else if self.busy() { self.stop_ai(); } else if self.fullscreen { self.set_fullscreen(false, cx); } else { cx.emit(ViewEvent::Home); return; }
        } else {
            let name = perusal_app::keymap::press_name(ctrl, ks.modifiers.alt, ks.modifiers.shift, key, ks.key_char.as_deref());
            if let Some(a) = perusal_app::keymap::action_for(&self.keys, &name).map(str::to_string) {
                // pages: any code-navigation key from the overview goes to the code first
                if self.page == Page::Overview && matches!(a.as_str(), "next_file" | "prev_file" | "next_batch" | "prev_batch" | "toggle_viewed" | "toggle_split" | "toggle_markdown" | "clear_scope") { self.page = Page::Code; }
                self.run_action(&a, cx);
            }
            else if ctrl { return; }
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
        let p = self.pal; let z = self.zoom; let fs = self.fullscreen;
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
            .children([("n / p", "file"), ("v", "viewed"), ("Enter", "next"), ("/", "filter"), ("c", "comment"), ("a", "ask AI")].into_iter().filter(|(k, _)| *k != "a" || self.settings.ai_on()).chain([ ("f", "finish"), ("u", "split"), ("m", "markdown"), ("t", "tree"), ("g", "group"), ("o", "overview"), ("s", "settings"), ("Esc", "home")])
                .map(|(k, l)| div().flex().items_center().gap_1().child(kbd(k, &p)).child(l)));
        let overlay = if self.rev_open { Some(self.reviewers_overlay(cx)) } else if self.help_open { Some(self.help_overlay(cx)) } else if self.palette_open { Some(self.palette_overlay(cx)) } else { self.finish_open.then(|| self.finish_overlay(cx).into_any_element()) };

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
                this.dragging = false; for i in [&mut this.filter, &mut this.sort, &mut this.comment, &mut this.ask, &mut this.summary, &mut this.preset_name, &mut this.rename, &mut this.gen, &mut this.pal_in, &mut this.rev_in] { i.dragging = false; }
                if let Some((k, _)) = this.sb_drag.take() { if let Some(pn) = this.panes.get(k) { pn.list.scrollbar_drag_ended(); } }
                this.hb_drag = None; this.pane_drag = None; this.pane_hover = None; this.pane_snapped = false;
                if this.resizing.take().is_some() { this.settings.left_width = this.left_w; this.settings.right_width = this.right_w; this.save_settings(); }
                if this.resizing_cd.take().is_some() { this.settings.commits_height = this.commits_h; this.save_settings(); }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, w, cx| {
                if ev.pressed_button != Some(MouseButton::Left) { return; }
                let zoom = this.zoom; let vw: f32 = w.viewport_size().width.into(); let x: f32 = ev.position.x.into(); let y: f32 = ev.position.y.into();
                if let Some((y0, h0)) = this.resizing_cd { this.commits_h = Some((h0 + (y - y0)).clamp(60.0, 700.0)); cx.notify(); }
                match this.resizing { Some(Side::Left) => { this.left_w = (x / zoom).clamp(200.0, 700.0); cx.notify(); } Some(Side::Right) => { this.right_w = ((vw - x) / zoom).clamp(220.0, 700.0); cx.notify(); } None => {} }
                if let Some((k, grab)) = this.hb_drag { if let Some((ox, tw, max)) = this.hbar_geom(k) {
                    let thumb = (tw * tw / (tw + max)).max(28.0); let travel = (tw - thumb).max(1.0);
                    if let Some(h) = this.hxs.get_mut(k) { *h = ((x - ox - grab) / travel).clamp(0.0, 1.0) * max; } cx.notify();
                } }
                if let Some((k, grab)) = this.sb_drag { if let Some(pn) = this.panes.get(k) {
                    let vp = pn.list.viewport_bounds(); let vh: f32 = vp.size.height.into(); let oy: f32 = vp.origin.y.into(); let max: f32 = pn.list.max_offset_for_scrollbar().y.into();
                    let thumb = (vh * vh / (vh + max)).max(28.0); let travel = (vh - thumb).max(1.0);
                    let frac = ((y - oy - grab) / travel).clamp(0.0, 1.0); pn.list.set_offset_from_scrollbar(point(px(0.0), px(-frac * max))); cx.notify();
                } }
                if let Some(d) = this.pane_drag.clone() { this.drag_panes(d, x, y, zoom); cx.notify(); }
            }))
            .when(!fs, |d| d.child(top).children(self.head_banner(cx)).child(progress).child(filter))
            .child(if fs { div().flex().flex_1().min_h_0().child(center) } else { div().flex().flex_1().min_h_0().child(lcol).children(lhandle).child(center).children(rhandle).child(rcol) })
            .when(!fs, |d| d.child(footer))
            .children(overlay)
            .child(self.wheel_zoom(cx))
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
            Fid::Palette => wire_input(el, cx, |t| &mut t.pal_in, |t, _| { t.focus = Some(Fid::Palette); }, ui, false),
            Fid::Reviewers => wire_input(el, cx, |t| &mut t.rev_in, |t, _| { t.focus = Some(Fid::Reviewers); }, ui, false),
        }
    }
    fn input_of(&self, id: Fid) -> &Input { match id { Fid::Filter => &self.filter, Fid::Sort => &self.sort, Fid::Comment => &self.comment, Fid::Ask => &self.ask, Fid::Summary => &self.summary, Fid::Preset => &self.preset_name, Fid::Rename => &self.rename, Fid::Gen => &self.gen, Fid::Find => &self.find_in, Fid::Palette => &self.pal_in, Fid::Reviewers => &self.rev_in } }
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
                .when(!self.settings.hidden("rename"), |d| d.child(icon_btn("rename", ic::EDIT, "Rename this review", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { let t = this.m.title.clone(); this.rename.f.set(&t); this.focus = Some(Fid::Rename); cx.stop_propagation(); cx.notify(); })))).into_any_element()
        };
        let ci = ci_badge(&self.ci, &p).filter(|_| !self.settings.hidden("ci")).map(|b| b.on_click(cx.listener(|this, _, _, _| { if let Some(u) = this.ci_url() { let _ = model::open_url(&u); } })));
        let local_mr = self.local_mr.clone().map(|u| icon_text_btn("open-mr", ic::OPEN_IN_NEW, "Open the PR", Btn::Secondary, &p).on_click(cx.listener(move |_, _, _, _| { let _ = model::open_url(&u); })));
        div().relative().flex().items_center().gap_3().px_4().h(px(52.0)).bg(p.surface).border_b_1().border_color(p.border)
            .child(icon_btn("home", ic::HOME, "Back to Home (Esc)", Btn::Ghost, &p).on_click(cx.listener(|_, _, _, cx| cx.emit(ViewEvent::Home))))
            .child(title)
            .children(self.m.remote.clone().map(|u| { let label = u.trim_start_matches("https://").to_string(); let hv = p.hover; let t = tip(div().id("mr-link").flex().items_center().gap_1().px_2().py(px(1.0)).rounded_full().text_xs().border_1().border_color(p.border).text_color(p.accent).cursor_pointer().hover(move |x| x.bg(hv)).child(label).child(icon(ic::OPEN_IN_NEW, 12.0, p.accent)), "Open on the host in your browser", &p); t.on_click(cx.listener(move |_, _, _, _| { let _ = model::open_url(&u); })) }))
            .children(Some(self.m.mr_state.clone()).filter(|s| !s.is_empty()).map(|s| { let (c, l) = match s.as_str() { "merged" => (p.ai, "Merged"), "closed" => (p.danger, "Closed"), _ => (p.success, "Open") }; badge(l, c, c.opacity(0.15)) }))
            .children(ci).children(local_mr)
            .child(div().flex_1())
            .when(ndrafts > 0, |d| d.child(badge(format!("{ndrafts} draft{}", if ndrafts == 1 { "" } else { "s" }), p.accent, p.accent.opacity(0.15))))
            .when(!self.settings.hidden("refresh"), |d| d.child(if self.refreshing {
                tip(div().id("refresh-mr").size(px(30.0)).flex().items_center().justify_center().flex_none().rounded_md().border_1().border_color(p.accent).bg(p.surface).child(kit::spinner(&p, 18.0)), "Refreshing…", &p)
            } else {
                icon_btn("refresh-mr", ic::REFRESH, if self.m.mr.is_some() { "Refresh: fetch new commits, CI and comments (R)" } else { "Refresh: re-read the branch (R)" }, Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.refresh(cx); }))
            }))
            .children(self.refreshing.then(|| kit::indeterminate_bar(&p)))
            .when(reviewing || !self.settings.hidden("ai_review"), |d| d.child(if reviewing { icon_text_btn("ai-review", ic::STOP, "Stop AI review", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.stop_ai(); cx.notify(); })).into_any_element() }
                else { icon_text_btn("ai-review", ic::AUTO_AWESOME, "AI review", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.tab = RightTab::Ai; this.right_open = true; this.start_ai(Job::Review, String::new(), cx); cx.notify(); })).into_any_element() }))
            .when(self.m.mr.is_some() && !self.settings.hidden("request_review"), |d| d.child(icon_btn("req-review", ic::PERSON_ADD, "Request a review from someone", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.rev_open = true; this.rev_in.f.clear(); this.focus = Some(Fid::Reviewers); cx.stop_propagation(); cx.notify(); }))))
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
                .child(div().id("tier-rows").max_h(px(220.0)).overflow_y_scroll().flex().flex_col().py_1().when(n == 0, |d| d.child(div().px_3().py_2().text_sm().text_color(p.muted).child("No sorting. Add a tier below."))).children(rows))
                .child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border)
                    .child(div().text_xs().text_color(p.muted).child("Add a tier"))
                    .child(div().flex().flex_wrap().gap_1().children(chips))
                    .child(div().text_xs().text_color(p.muted).child("Or write the whole sort as an expression"))
                    .child(self.winp(Fid::Sort, div().id("sort-expr").flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if on { p.accent } else { p.border }).bg(p.bg).font_family(self.fonts.mono.clone()).child(input_text(&self.sort, on, "additions + deletions desc, name asc", &p, false, 1)), cx))
                    .children(sug_items))
                .when(self.settings.ai_on(), |d| d.child(self.gen_box(perusal_agents::GenKind::Sort, "e.g. biggest files first, then alphabetical", cx)))).with_priority(15)
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
                .child(div().id("preset-list").max_h(px(280.0)).overflow_y_scroll().flex().flex_col().when(empty, |d| d.child(div().px_3().py_3().text_sm().text_color(p.muted).child("No presets yet. Save the current filter and sort below."))).children(items))
                .child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border)
                    .child(self.winp(Fid::Preset, div().id("pr-name").flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if self.focus == Some(Fid::Preset) { p.accent } else { p.border }).bg(p.bg).child(input_text(&self.preset_name, self.focus == Some(Fid::Preset), "Name this preset", &p, false, 1)), cx))
                    .child(div().flex().gap_2().child(icon_text_btn("pr-save-proj", ic::BOOKMARK, "Save for this project", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_preset(true); cx.stop_propagation(); cx.notify(); })))
                        .child(icon_text_btn("pr-save-glob", ic::BOOKMARK_BORDER, "Save globally", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_preset(false); cx.stop_propagation(); cx.notify(); })))))).with_priority(15)
        });
        let err = self.error.clone();
        div().flex().flex_col().w_full().px_4().py_2().bg(p.bg).border_b_1().border_color(p.border).gap_1()
            .child(div().relative().w_full().flex().items_center().gap_2().child(fbox).when(!self.settings.hidden("gen_filter"), |d| d.child(ai_filter)).when(!self.settings.hidden("whitespace"), |d| d.child(ws_btn)).when(!self.settings.hidden("sort"), |d| d.child(sort_btn))
                .when(!self.settings.hidden("presets"), |d| d.child(icon_btn("presets", ic::BOOKMARK, "Filter and sort presets", if self.presets_open { Btn::Secondary } else { Btn::Ghost }, &p).on_click(cx.listener(|this, _, _, cx| { this.presets_open = !this.presets_open; this.sug.clear(); cx.stop_propagation(); cx.notify(); }))))
                .children(dropdown).children(presets_menu).children(sort_pop).children(filter_pop))
            .children((self.tree && !self.m.vm.sort.is_empty()).then(|| div().flex().items_center().gap_2().text_xs().text_color(p.warn).child(icon(ic::WARNING, 14.0, p.warn)).child("Sorting does not apply while files are shown as a directory tree. Switch to the flat list to use it.")))
            .children(err.map(|e| div().flex().items_center().gap_1().text_xs().text_color(p.danger).child(div().min_w_0().truncate().child(e.clone())).child(kit::copy_btn("filter-err-copy", e, &p))))
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
            .when(tree_on && !self.settings.hidden("fold_all"), |d| d
                .child(icon_btn("exp-all", ic::UNFOLD_MORE, "Expand all folders", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.collapsed.clear(); cx.notify(); })))
                .child(icon_btn("col-all", ic::UNFOLD_LESS, "Collapse all folders", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.collapsed = tree::all_dirs(&this.m.vm.files, &this.m.vm.visible).into_iter().collect(); cx.notify(); }))))
            .when(!self.settings.hidden("files_view"), |d| d.child(seg_box(&p).child(seg_icon("seg-list", ic::VIEW_LIST, "Flat list", !tree_on, &p).on_click(cx.listener(|this, _, _, cx| { if this.tree { this.toggle_tree(); } cx.notify(); })))
                .child(seg_icon("seg-tree", ic::ACCOUNT_TREE, "Directory tree (t)", tree_on, &p).on_click(cx.listener(|this, _, _, cx| { if !this.tree { this.toggle_tree(); } cx.notify(); })))))
            .child(icon_btn("close-l", ic::CHEVRON_LEFT, "Hide the files panel", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.left_open = false; this.settings.left_open = false; this.save_settings(); cx.notify(); })));
        let n = self.multi.len();
        let bar = (n > 0).then(|| div().flex().items_center().gap_2().px_3().py(px(6.0)).bg(p.warn.opacity(0.12)).border_b_1().border_color(p.border)
            .child(div().text_sm().flex_1().child(format!("{n} file{} selected", if n == 1 { "" } else { "s" })))
            .child(if n >= 2 { icon_text_btn("grp-multi", ic::LINK, "Review together", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.group_multi(); cx.notify(); })) }
                else { tip(icon_text_btn("grp-multi", ic::LINK, "Review together", Btn::Primary, &p).opacity(0.4).cursor_default(), "Select at least two files to review them together", &p) })
            .child(icon_btn("clr-multi", ic::CLOSE, "Clear selection", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.multi.clear(); cx.notify(); }))));
        let hint = (n == 0 && n_files > 1 && self.m.vm.commit_scope.is_none()).then(|| div().px_3().py(px(4.0)).text_xs().text_color(p.muted).border_b_1().border_color(p.border).child("Ctrl+click or Shift+click files to review them together"));
        let scope = self.m.graph.selection.filter(|_| self.m.vm.commit_scope.is_some()).and_then(|(a, b)| self.m.graph.rows.get(a).cloned().map(|r| (r, b - a + 1))).map(|(r, span)| div().flex().items_center().gap_2().px_3().py(px(6.0)).bg(p.accent.opacity(0.12)).border_b_1().border_color(p.border)
            .child(icon(ic::COMMIT, 17.0, p.accent)).child(div().flex_1().min_w_0().truncate().text_sm().child(format!("{} {} · {} file{}", if span > 1 { format!("Only {span} commits up to") } else { "Only commit".to_string() }, &r.sha[..7.min(r.sha.len())], n_files, if n_files == 1 { "" } else { "s" })))
            .child(icon_btn("scope-x", ic::CLOSE, "Show all commits (x)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.clear_scope(); cx.notify(); }))));
        div().flex().flex_col().size_full().child(header).children(scope).children(bar).children(hint).child(list)
    }

    /// One row of the diff list (a code line, a hunk marker, a note or a comment card).
    fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(r) = self.rows.get(ix).cloned() else { return div().into_any_element() };
        let p = self.pal; let h = px(20.0 * self.zoom * self.settings.line_height); let fsz_f = 13.0 * self.zoom; let fsz = px(fsz_f); let mono = self.fonts.mono.clone();
        let fi = r.fi;
        // line-number column: wide enough for the file's largest line number at the current zoom, never wrapping
        let (gdigits, has_old, has_new) = self.gutter.get(&fi).copied().unwrap_or((3, true, true));
        let gw = px((gdigits as f32 * 0.62 * fsz_f).ceil() + 16.0 * self.zoom);
        let sw = px(20.0 * self.zoom);
        let selected = r.ord.map(|o| self.in_sel(o)).unwrap_or(false);
        let base = div().id(ix).flex().items_center().h(h).w_full().font_family(mono.clone()).when(!self.settings.ligatures, |d| d.font_features(gpui::FontFeatures::disable_ligatures())).text_size(fsz).text_color(p.fg);
        let pk = self.pane_of(ix); let hx = self.hxs.get(pk).copied().unwrap_or(0.0); let hxt = self.hx_texts.get(pk).cloned().unwrap_or_default();
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
                .child(div().w(gw).flex_none().px_2().text_right().whitespace_nowrap().overflow_hidden().text_color(p.muted).bg(gut).child(no.map(|n| n.to_string()).unwrap_or_default()))
                .child(div().w(sw).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(sign))
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
                let paned = self.paned(); let closed = self.file_collapsed(fi); let f = &self.m.vm.files[fi];
                return base.bg(p.surface).border_t_1().border_color(p.border).px_2().gap_2().font_weight(FontWeight::SEMIBOLD).when(!paned, |d| d.cursor_pointer().hover(move |x| x.bg(p.hover))).when(paned && self.panes.len() > 1, |d| d.pl(px(36.0)))
                    .when(!paned, |d| d.child(icon(if closed { ic::CHEVRON_RIGHT } else { ic::EXPAND_MORE }, 17.0, p.muted))).child(div().min_w_0().truncate().child(f.path.clone()))
                    .child({ let path = f.path.clone(); icon_btn(("copy-path", ix), ic::CONTENT_COPY, "Copy the file path", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.copy_path(&path, cx); cx.stop_propagation(); cx.notify(); })) })
                    .child(div().flex_1())
                    .children(marks)
                    .child(div().flex().gap_2().text_xs().child(div().text_color(p.success).child(format!("+{}", f.additions))).child(div().text_color(p.danger).child(format!("−{}", f.deletions))))
                    .when(!paned, |d| d.on_click(cx.listener(move |this, _, _, cx| { if !this.collapsed_files.remove(&fi) { this.collapsed_files.insert(fi); } this.shown = None; this.load_diff(); cx.notify(); }))).into_any_element();
            }
            RowKind::Collapsed(t, expandable) => return base.bg(p.accent.opacity(0.10)).px_3().gap_2().text_color(p.fg).child(icon(ic::UNFOLD_MORE, 15.0, p.accent)).child(div().flex_1().min_w_0().truncate().child(t.clone()))
                .when(*expandable, |d| d.child(div().id(("expand", ix)).px_2().rounded_md().border_1().border_color(p.border).cursor_pointer().hover(move |x| x.bg(p.hover)).text_sm().child("Show anyway")
                    .on_click(cx.listener(move |this, _, _, cx| { this.forced.insert(fi); this.shown = None; this.load_diff(); cx.notify(); })))).into_any_element(),
            RowKind::Image(old, new, size, note) => {
                // old and new always side by side; each picture at its natural size, shrunk (never enlarged) to fit its half and a maximum height
                let maxh = 420.0 * self.zoom; let muted = p.muted; let sides = old.is_some() && new.is_some();
                let avail = { let b = self.b_panes.get(); let w: f32 = b.size.width.into(); (self.pane_rect(pk).w * w - 40.0).max(120.0) / if sides { 2.0 } else { 1.0 } - 16.0 };
                let (bw, bh) = match size { Some((w, h)) if *w > 0 && *h > 0 => { let k = (avail / *w as f32).min(maxh / *h as f32).min(1.0); (*w as f32 * k, *h as f32 * k) } _ => (avail.min(maxh), maxh.min(avail)) };
                let tile = |label: &str, im: &Option<Arc<gpui::Image>>, tint: gpui::Hsla| im.clone().map(|im| { let note = note.clone();
                    div().flex_1().min_w_0().flex().flex_col().items_center().gap_1().child(div().text_xs().text_color(tint).font_weight(FontWeight::SEMIBOLD).child(label.to_string()))
                        .child(div().p_1().border_1().border_color(p.border).rounded_sm().bg(p.bg).child(gpui::img(im).w(px(bw)).h(px(bh)).object_fit(gpui::ObjectFit::Contain)
                            .with_fallback(move || div().p_3().text_sm().text_color(muted).child(note.clone()).into_any_element()))).into_any_element() });
                return div().id(ix).w_full().flex().items_start().gap_4().px_4().py_3().children(tile("Before", old, p.danger)).children(tile("After", new, p.success)).into_any_element();
            }
            RowKind::Note(t) => return base.bg(p.accent.opacity(0.10)).px_3().gap_2().text_color(p.fg).child(icon(ic::LINK, 15.0, p.accent)).child(t.clone()).into_any_element(),
            RowKind::Hunk(t) => return base.bg(p.accent.opacity(0.10)).text_color(p.muted).text_xs().child(div().w(gw + sw).flex_none()).child(t.clone()).into_any_element(),
            RowKind::Comment(c) => return self.comment_card(fi, &c.clone(), cx).into_any_element(),
            RowKind::MdBlock(lb) => {
                use markdown::{Block, Change};
                let (bg, bar) = match lb.change { Change::Added => (p.added, p.success), Change::Removed => (p.removed, p.danger), Change::Modified => (gpui::transparent_black(), p.warn), Change::None => (gpui::transparent_black(), gpui::transparent_black()) };
                // table rows and code lines stack into one box, so they carry no padding or row tint of their own
                let tight = matches!(lb.block, Block::CodeLine { .. } | Block::Table { .. });
                div().id(ix).w_full().px_6().py(px(if tight { 0.0 } else { 3.0 })).border_l_4().border_color(bar).font_family(self.fonts.ui.clone()).text_size(px(14.0))
                    .bg(if selected { p.accent.opacity(0.14) } else if tight { gpui::transparent_black() } else { bg })
                    .child(kit::markdown_view_find(std::slice::from_ref(&lb.block), &p, &mono, &self.find_query(), self.md_block_current(ix)))
            }
            RowKind::Uni(rl) => {
                let l = &rl.line;
                let bg = match l.kind { Kind::Added => p.added, Kind::Removed => p.removed, Kind::Context => gpui::transparent_black() };
                let gut = match l.kind { Kind::Added => p.added_word.opacity(0.55), Kind::Removed => p.removed_word.opacity(0.55), Kind::Context => gpui::transparent_black() };
                let (show_old, show_new) = (has_old, has_new);
                base.bg(if selected { p.accent.opacity(0.28) } else { bg })
                    .when(show_old, |d| d.child(div().w(gw).flex_none().px_2().text_right().whitespace_nowrap().overflow_hidden().text_color(p.muted).bg(gut).child(l.old_no.map(|n| n.to_string()).unwrap_or_default())))
                    .when(show_new, |d| d.child(div().w(gw).flex_none().px_2().text_right().whitespace_nowrap().overflow_hidden().text_color(p.muted).bg(gut).child(l.new_no.map(|n| n.to_string()).unwrap_or_default())))
                    .child(div().w(sw).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(match l.kind { Kind::Added => "+", Kind::Removed => "−", _ => " " }))
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
        let host = c.host.clone(); let mine = self.me.as_deref().map(|m| m == c.author).unwrap_or(false);
        let kind_of = self.m.mr.as_ref().map(|r| r.kind).unwrap_or(perusal_providers::Kind::GitHub);
        let me = self.me.clone();
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
                    .children(host.clone().map(|h| self.thread_buttons(&c.key, &h, c.resolved, mine, kind_of, cx)))
                    .children(draft.map(|d| icon_btn(SharedString::from(format!("ed-{d}")), ic::EDIT, "Edit this comment", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.open_composer(false, Some(d)); cx.stop_propagation(); cx.notify(); }))))
                    .children(draft.filter(|_| instant).map(|d| icon_btn(SharedString::from(format!("pub-{d}")), ic::SEND, "Publish this draft now", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.publish_draft(d, cx); cx.stop_propagation(); cx.notify(); }))))
                    .children(draft.map(|d| icon_btn(SharedString::from(format!("rm-{d}")), ic::DELETE, "Delete this draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(d); this.resync(); cx.stop_propagation(); cx.notify(); })))))
                .when(!collapsed, |d| d.child(div().flex().flex_col().gap_2().px_3().py_2().child(body(&c.body))
                    .children(c.replies.iter().enumerate().map(|(i, (a, t))| {
                        let own = me.as_deref() == Some(a.as_str()); let ids = host.clone().map(|h| (h.id, h.comment_ids.get(i + 1).cloned()));
                        let btns = if own { ids.map(|(tid, cid)| self.reply_buttons(&c.key, i, tid, cid, t, cx)) } else { None };
                        div().flex().flex_col().gap_1().pt_2().border_t_1().border_color(p.border).child(div().flex().items_center().gap_2().child(kit::avatar(a, 20.0)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child(a.clone())).children(btns)).child(body(t))
                    }).collect::<Vec<_>>())))
        ).id(id)
    }

    /// Horizontal scrollbar geometry of pane `k`: (track left x, track width, max offset); None when nothing overflows.
    fn hbar_geom(&self, k: usize) -> Option<(f32, f32, f32)> {
        let b = self.b_panes.get(); let r = self.pane_rect(k); let bw: f32 = b.size.width.into();
        let (ox, w): (f32, f32) = (f32::from(b.origin.x) + r.x * bw, r.w * bw);
        let cols = if self.diff_mode == DiffMode::Split { 2.0 } else { 1.0 };
        let max = (self.hx_texts.get(k).map(|c| c.get()).unwrap_or(0.0) - (w - 130.0) / cols).max(0.0);
        (max > 1.0 && w > 60.0).then_some((ox, w - 12.0, max))
    }

    fn hscrollbar(&mut self, k: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.pal; let (ox, tw, max) = self.hbar_geom(k)?;
        let hx = self.hxs.get(k).copied().unwrap_or(0.0);
        let thumb = (tw * tw / (tw + max)).max(28.0); let travel = (tw - thumb).max(1.0); let left = (hx / max).clamp(0.0, 1.0) * travel;
        let dragging = self.hb_drag.map(|d| d.0) == Some(k);
        Some(div().id(("hb-track", k)).absolute().bottom_0().left_0().w(px(tw)).h(px(12.0)).cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                let x: f32 = ev.position.x.into(); let rel = x - ox;
                let grab = if rel >= left && rel <= left + thumb { rel - left } else { thumb / 2.0 };
                this.hb_drag = Some((k, grab)); if let Some(h) = this.hxs.get_mut(k) { *h = ((rel - grab) / travel).clamp(0.0, 1.0) * max; } cx.stop_propagation(); cx.notify();
            }))
            .child(div().absolute().left(px(left)).bottom(px(2.0)).h(px(8.0)).w(px(thumb)).rounded_full().bg(if dragging { p.muted } else { p.muted.opacity(0.5) }).hover(move |s| s.bg(p.muted))).into_any_element())
    }

    fn scrollbar(&mut self, k: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.pal; let list = self.panes.get(k)?.list.clone();
        let vp = list.viewport_bounds(); let vh: f32 = vp.size.height.into(); let max: f32 = list.max_offset_for_scrollbar().y.into();
        if max <= 1.0 || vh <= 1.0 { return None; }
        let off: f32 = (-list.scroll_px_offset_for_scrollbar().y).into();
        let thumb = (vh * vh / (vh + max)).max(28.0); let travel = (vh - thumb).max(1.0); let top = (off / max).clamp(0.0, 1.0) * travel;
        let oy: f32 = vp.origin.y.into(); let dragging = self.sb_drag.map(|d| d.0) == Some(k);
        Some(div().id(("sb-track", k)).absolute().top_0().right_0().w(px(12.0)).h(px(vh)).cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                let y: f32 = ev.position.y.into(); let rel = y - oy;
                let grab = if rel >= top && rel <= top + thumb { rel - top } else { thumb / 2.0 };
                let Some(pn) = this.panes.get(k) else { return };
                this.sb_drag = Some((k, grab)); pn.list.scrollbar_drag_started();
                let frac = ((rel - grab) / travel).clamp(0.0, 1.0); pn.list.set_offset_from_scrollbar(point(px(0.0), px(-frac * max))); cx.stop_propagation(); cx.notify();
            }))
            .child(div().absolute().top(px(top)).right(px(2.0)).w(px(8.0)).h(px(thumb)).rounded_full().bg(if dragging { p.muted } else { p.muted.opacity(0.5) }).hover(move |s| s.bg(p.muted))).into_any_element())
    }

    /// Ctrl + mouse wheel zooms like Ctrl + / Ctrl -. Registered for the capture phase so the list underneath does not scroll as well.
    fn wheel_zoom(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let me = cx.entity().downgrade();
        gpui::canvas(|b, _, _| b, move |b, _, window, _| {
            let me = me.clone();
            window.on_mouse_event(move |ev: &gpui::ScrollWheelEvent, phase, _w, cx| {
                if phase != gpui::DispatchPhase::Capture || !(ev.modifiers.control || ev.modifiers.platform) || !b.contains(&ev.position) { return; }
                let dy: f32 = ev.delta.pixel_delta(px(20.0)).y.into();
                let _ = me.update(cx, |this, cx| {
                    this.wheel_acc += dy;
                    let steps = (this.wheel_acc / 20.0).trunc();
                    if steps != 0.0 { this.wheel_acc -= steps * 20.0; for _ in 0..steps.abs() as i32 { this.zoom = theme::zoom(this.zoom, if steps > 0.0 { 1 } else { -1 }); } }
                    cx.notify();
                });
                cx.stop_propagation();
            });
        }).absolute().size_full()
    }

    /// Moves the dragged divider or corner to the mouse (window coordinates), snapping to halves, thirds, quarters and aligned dividers.
    fn drag_panes(&mut self, d: PaneDrag, x: f32, y: f32, zoom: f32) {
        let b = self.b_panes.get(); let (ox, oy): (f32, f32) = (b.origin.x.into(), b.origin.y.into()); let (w, h): (f32, f32) = (b.size.width.into(), b.size.height.into());
        if w < 1.0 || h < 1.0 { return; }
        // `b_panes` is measured in window pixels, which the zoom does not scale (it only scales fonts and row heights), so a minimum pane size is plain pixels too
        let (tx, ty) = ((x - ox) / w, (y - oy) / h); let min = 140.0 * zoom.min(1.5); let snap = 12.0;
        self.pane_snapped = match d {
            PaneDrag::Divider(id) => {
                let vertical = self.layout.dividers().iter().find(|d| d.id == id).map(|d| d.vertical).unwrap_or(true);
                let (t, ext) = if vertical { (tx, w) } else { (ty, h) };
                self.layout.drag(&id, t, ext, min, snap).is_some()
            }
            PaneDrag::Corner(c) => {
                self.layout.drag_corner(&c, tx, ty, (w, h), min, snap);
                let near = |v: f32| perusal_app::panes::SNAP_FRACTIONS.iter().any(|f| (v - f).abs() < 1e-3);
                self.layout.corners().iter().any(|k| k.v == c.v && k.h == c.h && near(k.x) && near(k.y))
            }
        };
    }

    /// The diff itself: one list, or one list per file in a tiling of resizable panes with draggable dividers and corners.
    fn diff_panes(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.pal; let paned = self.paned(); let multi = self.panes.len() > 1;
        let mut area = div().id("diff-wrap").absolute().top_0().left_0().right_0().bottom(px(70.0)).child(kit::bounds_probe(&self.b_panes));
        for k in 0..self.panes.len() {
            let r = self.pane_rect(k); let (start, list_state) = (self.panes[k].range.start, self.panes[k].list.clone());
            let (sb, hsb) = (self.scrollbar(k, cx), self.hscrollbar(k, cx));
            let title: SharedString = self.panes[k].key.first().and_then(|&fi| self.m.vm.files.get(fi)).map(|f| f.path.clone()).unwrap_or_default().into();
            let pane = div().id(("pane", k)).absolute().left(relative(r.x)).top(relative(r.y)).w(relative(r.w)).h(relative(r.h)).overflow_hidden()
                // horizontal wheel (and Shift + wheel, which GPUI turns into a horizontal delta) pans the long lines of this pane
                .on_scroll_wheel(cx.listener(move |this, ev: &gpui::ScrollWheelEvent, _, cx| {
                    let dx: f32 = ev.delta.pixel_delta(px(20.0)).x.into();
                    if dx == 0.0 || ev.modifiers.control || ev.modifiers.platform { return; }
                    let Some((_, _, max)) = this.hbar_geom(k) else { return };
                    if let Some(h) = this.hxs.get_mut(k) { let n = (*h - dx).clamp(0.0, max); if n != *h { *h = n; cx.notify(); } }
                }))
                .when(multi, |d| d.border_1().border_color(p.border)
                    .on_drag_move(cx.listener(move |this, ev: &gpui::DragMoveEvent<DraggedPane>, _, cx| {
                        let (b, pos) = (ev.bounds, ev.event.position);
                        let hover = (b.contains(&pos) && ev.drag(cx).from != k).then(|| {
                            let (w, h): (f32, f32) = (b.size.width.into(), b.size.height.into());
                            drop_zone((f32::from(pos.x) - f32::from(b.origin.x)) / w.max(1.0), (f32::from(pos.y) - f32::from(b.origin.y)) / h.max(1.0))
                        }).map(|z| (k, z));
                        this.pane_from = ev.drag(cx).from;
                        if (hover.is_some() || this.pane_hover.map(|h| h.0) == Some(k)) && this.pane_hover != hover { this.pane_hover = hover; cx.notify(); }
                    }))
                    .on_drop(cx.listener(move |this, d: &DraggedPane, _, cx| {
                        let zone = this.pane_hover.filter(|h| h.0 == k).map(|h| h.1).unwrap_or(Drop::Swap); this.pane_hover = None;
                        let done = match zone { Drop::Swap => this.layout.swap(d.from, k), Drop::Split(e) => this.layout.move_pane(d.from, k, e) };
                        if done { this.clear_selection(); } cx.notify();
                    })))
                .child(list(list_state, cx.processor(move |this, ix: usize, _w, cx| { let g = start + ix; let el = this.render_row(g, cx); match this.row_tint(g) { Some(c) => div().w_full().bg(c).child(el).into_any_element(), None => el } })).size_full())
                .children(sb).children(hsb)
                .when(paned && multi, |d| d.child(div().id(("grip", k)).absolute().top(px(3.0)).left(px(4.0)).w(px(28.0)).h(px(20.0)).flex().items_center().justify_center().rounded_md().cursor_grab().text_color(p.muted).hover(move |s| s.bg(p.hover))
                    .on_drag(DraggedPane { from: k, title: title.clone(), pal: p }, |d, _, _, cx| cx.new(|_| DragPaneView(d.clone())))
                    .tooltip_text("Drag onto another pane: the middle swaps them, a side splits that pane".to_string(), &p).child(div().flex().flex_col().gap(px(3.0)).children((0..2).map(|_| div().flex().gap(px(3.0)).children((0..3).map(|_| div().size(px(3.0)).rounded_full().bg(p.muted))))))));
            area = area.child(pane);
        }
        // where the dragged pane would end up: the layout after the drop, outlined in blue
        if let Some((to, z)) = self.pane_hover.filter(|_| paned) {
            let mut after = self.layout.clone(); let from = self.pane_from;
            let ok = match z { Drop::Swap => after.swap(from, to), Drop::Split(e) => after.move_pane(from, to, e) };
            if let Some(r) = after.rects().get(from).map(|x| x.1).filter(|_| ok) {
                area = area.child(deferred(div().absolute().left(relative(r.x)).top(relative(r.y)).w(relative(r.w)).h(relative(r.h)).bg(p.accent.opacity(0.30)).border_4().border_color(p.accent).rounded_sm()).with_priority(100));
            }
        }
        if paned {
            let active = |d: &Option<PaneDrag>, id: &DividerId| matches!(d, Some(PaneDrag::Divider(x)) if x == id);
            for (i, dv) in self.layout.dividers().into_iter().enumerate() {
                let on = active(&self.pane_drag, &dv.id); let id = dv.id.clone();
                let line = if on && self.pane_snapped { p.success } else if on { p.accent } else { p.border };
                let base = div().id(("dv", i)).absolute().flex().items_center().justify_center().hover(move |s| s.bg(p.accent.opacity(0.25)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        if ev.click_count >= 2 { this.layout.reset(&id); } else { this.pane_drag = Some(PaneDrag::Divider(id.clone())); }
                        cx.stop_propagation(); cx.notify();
                    }));
                area = area.child(if dv.vertical {
                    base.left(relative(dv.pos)).ml(px(-4.0)).w(px(8.0)).top(relative(dv.span.0)).h(relative(dv.span.1 - dv.span.0)).cursor_col_resize().child(div().w(px(if on { 3.0 } else { 2.0 })).h_full().bg(line))
                } else {
                    base.top(relative(dv.pos)).mt(px(-4.0)).h(px(8.0)).left(relative(dv.span.0)).w(relative(dv.span.1 - dv.span.0)).cursor_row_resize().child(div().h(px(if on { 3.0 } else { 2.0 })).w_full().bg(line))
                });
            }
            for (i, c) in self.layout.corners().into_iter().enumerate() {
                let on = matches!(&self.pane_drag, Some(PaneDrag::Corner(x)) if x.v == c.v && x.h == c.h); let cc = c.clone();
                area = area.child(div().id(("cn", i)).absolute().left(relative(c.x)).top(relative(c.y)).ml(px(-8.0)).mt(px(-8.0)).size(px(16.0)).rounded_full().cursor(gpui::CursorStyle::ResizeUpLeftDownRight)
                    .bg(if on { if self.pane_snapped { p.success } else { p.accent } } else { p.border }).border_2().border_color(p.surface).hover(move |s| s.bg(p.accent))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| { this.pane_drag = Some(PaneDrag::Corner(cc.clone())); cx.stop_propagation(); cx.notify(); })));
            }
        }
        div().relative().flex_1().min_h_0().w_full().child(area).into_any_element()
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
        // Ctrl+F on this page: hits are counted per drawn text run, per section (0 title, 2 description, then each comment), like the highlighting does
        let bodies: Vec<String> = global_threads.iter().flat_map(|t| t.comments.iter().map(|c| c.1.clone())).chain(global_drafts.iter().map(|d| d.body.clone())).collect();
        let first_list = 4 + usize::from(bodies.is_empty());
        let fq = if self.find_open { self.find_in.f.text.clone() } else { String::new() };
        if self.find_open {
            let mut hits = vec![];
            if !fq.is_empty() {
                let mut sections: Vec<(usize, Vec<String>)> = vec![(0, vec![info.title.clone()]), (2, markdown::block_texts(&markdown::parse(&info.description)))];
                for (i, b) in bodies.iter().enumerate() { sections.push((first_list + i, markdown::block_texts(&markdown::parse(b)))); }
                for (ix, texts) in sections { for (part, t) in texts.iter().enumerate() { for range in perusal_app::find::find_all(t, &fq) { hits.push(Hit { row: ix, part, range }); } } }
            }
            self.find_page = Page::Overview; self.find_hits = hits; if self.find_cur >= self.find_hits.len() { self.find_cur = 0; }
        }
        let cur_ord = |sec: usize| -> Option<usize> { let h = self.find_hits.get(self.find_cur)?; (h.row == sec).then(|| self.find_hits[..self.find_cur].iter().filter(|x| x.row == sec).count()) };
        let title_runs: Vec<(Range<usize>, HighlightStyle)> = perusal_app::find::find_all(&info.title, &fq).into_iter().enumerate()
            .map(|(k, r)| (r, HighlightStyle { background_color: Some(if cur_ord(0) == Some(k) { p.accent.opacity(0.8) } else { p.warn.opacity(0.5) }), color: Some(p.fg), ..Default::default() })).collect();
        let me = self.who("You");
        let card = |who: String, body: &str, sec_ix: usize, tag: Option<AnyElement>| div().w_full().max_w(px(900.0)).flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(p.border).bg(p.surface)
            .child(div().flex().items_center().gap_2().child(kit::avatar(if who == "You" { &me } else { &who }, 24.0)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(who)).children(tag)).child(kit::markdown_view_find(&markdown::parse(body), &p, &mono, &fq, cur_ord(sec_ix)).text_sm());
        let mut list: Vec<AnyElement> = vec![];
        for t in &global_threads {
            for (i, (a, b)) in t.comments.iter().enumerate() { list.push(card(a.clone(), b, first_list + list.len(), (i == 0 && t.resolved).then(|| badge("Resolved", p.success, p.success.opacity(0.15)).into_any_element())).into_any_element()); }
        }
        for d in &global_drafts {
            let id = d.id;
            list.push(card("You".into(), &d.body, first_list + list.len(), Some(badge("Draft", p.warn, p.warn.opacity(0.15)).into_any_element()))
                .child(div().flex().justify_end().child(icon_btn(("ovde", id as usize), ic::EDIT, "Edit this comment", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.open_composer(false, Some(id)); cx.notify(); })))
                    .children(instant.then(|| icon_btn(("ovdp", id as usize), ic::SEND, "Publish this draft now", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.publish_draft(id, cx); cx.notify(); }))))
                    .child(icon_btn(("ovdd", id as usize), ic::DELETE, "Delete this draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(id); this.resync(); cx.notify(); })))).into_any_element());
        }
        let state = model::norm_state(&info.state); let dot = match state.as_str() { "merged" => p.ai, "closed" => p.danger, _ => p.success };
        let sec = |el: gpui::Div| el.w_full().max_w(px(900.0));
        let page = div().id("overview").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.ov_scroll).flex().flex_col().items_center().gap_4().px_6().pt_6().pb(px(100.0))
            .child(sec(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child(if title_runs.is_empty() { div().child(info.title.clone()) } else { div().child(StyledText::new(SharedString::from(info.title.clone())).with_highlights(title_runs)) })))
            .child(sec(div().flex().flex_wrap().items_center().gap_3().text_sm().text_color(p.muted)
                .child(badge(match state.as_str() { "merged" => "Merged", "closed" => "Closed", _ => "Open" }, dot, dot.opacity(0.15))).child(div().flex().items_center().gap_2().child(kit::avatar(&info.author, 22.0)).child(format!("by {}", info.author)))
                .children((!info.assignees.is_empty()).then(|| div().flex().items_center().gap_1().child(div().text_xs().child("Assigned")).children(info.assignees.iter().map(|a| div().flex().items_center().gap_1().child(kit::avatar(&a.login, 20.0)).child(a.login.clone())))))
                .children((!info.reviewers.is_empty()).then(|| div().flex().items_center().gap_1().child(div().text_xs().child("Reviewers")).children(info.reviewers.iter().map(|a| div().flex().items_center().gap_1().child(kit::avatar(&a.login, 20.0)).child(a.login.clone())))))
                .when(!info.source_ref.is_empty(), |d| d.child(div().flex().items_center().gap_1().font_family(mono.clone()).child(info.source_ref.clone()).child(icon(ic::ARROW_FORWARD, 14.0, p.muted)).child(info.base_ref.clone())))))
            .child(sec(div().flex().flex_col().gap_2().p_4().rounded_md().border_1().border_color(p.border).bg(p.surface)
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child("Description"))
                .child(if info.description.trim().is_empty() { div().text_color(p.muted).child("No description.").into_any_element() } else { kit::markdown_view_find(&markdown::parse(&info.description), &p, &mono, &fq, cur_ord(2)).into_any_element() })))
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
        let one_sided = !batch.is_empty() && batch.iter().all(|&i| self.m.vm.files[i].one_sided() || self.m.vm.files[i].symlink);
        let ncomments = self.comment_count();
        let (panes_on, fs) = (self.batch_panes, self.fullscreen);
        let custom = self.layout.custom;
        let (arr_icon, arr_tip) = if custom { (ic::DASHBOARD_CUSTOMIZE, "Pane layout: custom (you dragged panes). Click to pick a preset") } else { match self.arrangement { Arrangement::Auto => (ic::DASHBOARD, "Pane layout: automatic. Click to change"), Arrangement::Columns => (ic::VIEW_COLUMN, "Pane layout: side by side. Click to change"), Arrangement::Rows => (ic::HORIZONTAL_SPLIT, "Pane layout: stacked rows. Click to change"), Arrangement::Grid => (ic::GRID_VIEW, "Pane layout: grid. Click to change") } };

        let title = if batch.len() > 1 { div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Batch of {} files", batch.len()))).child(div().text_xs().text_color(p.muted).child(if manual { "grouped by you" } else { "related files" })) }
            else if let Some(f) = &first { let (dir, name) = match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) };
                div().flex().items_center().gap_2().min_w_0().truncate().when(f.symlink, |d| d.child(icon(ic::LINK, 17.0, p.accent))).child(div().flex().font_family(mono.clone()).text_sm().child(div().text_color(p.muted).child(dir)).child(div().font_weight(FontWeight::SEMIBOLD).child(name))) } else { div().child("No files match") };
        let sym = self.sym_path.clone().filter(|_| batch.len() == 1).map(|path| { let known = self.sym_target.is_some();
            icon_text_btn("sym-go", ic::ARROW_FORWARD, "Go to the original", Btn::Secondary, &p).on_click(cx.listener(move |this, _, _, cx| { this.goto_symlink_target(cx); cx.notify(); }))
                .tooltip_text(if known { format!("Open {path}") } else { format!("{path} is not changed in this merge request") }, &p) });
        let copy = match (batch.len(), &first) { (1, Some(f)) => { let path = f.path.clone(); Some(icon_btn("copy-path", ic::CONTENT_COPY, "Copy the file path", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.copy_path(&path, cx); cx.stop_propagation(); cx.notify(); }))) } _ => None };
        let header = (!batch.is_empty()).then(|| {
            let mut h = div().flex().items_center().gap_3().px_4().py_2().border_b_1().border_color(p.border).bg(p.surface).child(div().flex_1().min_w_0().flex().items_center().gap_1().child(title).children(copy))
                .child(div().flex().items_center().gap_2().text_xs().font_family(mono.clone()).child(div().text_color(p.success).child(format!("+{adds}"))).child(div().text_color(p.danger).child(format!("−{dels}"))).child(diff_squares(adds, dels, &p, 10.0)));
            if let Some(s) = sym { h = h.child(s); }
            if ncomments > 0 { h = h.child(icon_btn("collapse-c", ic::UNFOLD_LESS, "Collapse or expand all comments in this file", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.collapse_all_comments(); cx.notify(); }))); }
            if manual { h = h.child(icon_btn("ungroup", ic::LINK_OFF, "Ungroup these files", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { if let Some(fi) = this.current_file() { this.m.vm.ungroup(fi); this.save_groups(); this.reload(); } cx.notify(); }))); }
            h
        });

        let body: AnyElement = if self.rows.is_empty() {
            div().flex_1().flex().items_center().justify_center().text_color(p.muted).child("Nothing to show for this selection.").into_any_element()
        } else {
            self.diff_panes(cx)
        };

        // toolbar that floats above the selected lines
        let sel = self.selection();
        let toolbar = sel.clone().filter(|_| self.composer.is_none()).and_then(|s| {
            let range = self.sel;
            let (lo, hi) = range?; let miny = self.row_y.iter().filter(|(o, _)| **o >= lo && **o <= hi).map(|(_, y)| *y).fold(f32::MAX, f32::min);
            if miny == f32::MAX { return None; }
            let b = self.b_body.get(); let (ox, oy): (f32, f32) = (b.origin.x.into(), b.origin.y.into()); let w: f32 = b.size.width.into();
            let top = (miny - oy - 50.0).max(4.0); let left = (self.sel_x - ox - 30.0).clamp(8.0, (w - 330.0).max(8.0));
            Some(div().absolute().top(px(top)).left(px(left)).flex().items_center().gap_1().px_2().py_1().rounded_lg().bg(p.surface).border_1().border_color(p.accent).shadow_lg().occlude()
                .child(div().text_xs().text_color(p.muted).px_1().child(format!("{}:{}–{}", s.file.rsplit('/').next().unwrap_or(&s.file), s.start, s.end)))
                .when(!self.settings.hidden("sel_comment"), |d| d.child(icon_text_btn("sel-comment", ic::ADD_COMMENT, "Comment", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_composer(false, None); cx.stop_propagation(); cx.notify(); }))))
                .when(!self.settings.hidden("sel_ask_ai"), |d| d.child(icon_text_btn("sel-ai", ic::AUTO_AWESOME, "Ask AI", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = this.selection(); this.tab = RightTab::Ai; this.right_open = true; this.focus = Some(Fid::Ask); cx.stop_propagation(); cx.notify(); }))))
                .child(icon_btn("sel-clear", ic::CLOSE, "Clear the selection (Esc)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.clear_selection(); cx.stop_propagation(); cx.notify(); }))))
        });

        // floating action bar
        let sep = || div().w(px(1.0)).h(px(22.0)).bg(p.border).flex_none();
        // Separators go only between groups that have something visible, so hiding buttons never leaves a stray one.
        let (h, show_finish) = (|id: &str| self.settings.hidden(id), !self.instant() && !self.settings.hidden("finish"));
        let g1 = !h("nav") || !h("viewed") || !h("global_comment");
        let g2 = (!md_on && !one_sided && !h("diff_mode")) || (is_md && !h("md_mode")) || (batch.len() > 1 && !h("batch_view")) || !h("fullscreen");
        let bar = div().absolute().bottom(px(16.0)).left_0().right_0().flex().justify_center().child(
            div().flex().items_center().gap_2().px_3().py_2().rounded_lg().bg(p.surface).border_1().border_color(p.border).shadow_lg()
                .when(!self.settings.hidden("nav"), |d| d.child(icon_btn("prev", ic::CHEVRON_LEFT, "Previous batch (Backspace)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.back(); cx.notify(); }))))
                .when(!self.settings.hidden("nav"), |d| d.child(icon_btn("next", ic::CHEVRON_RIGHT, "Next batch (Enter)", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.next(); cx.notify(); }))))
                .when(!self.settings.hidden("viewed"), |d| d.child(tip(div().id("viewed-box").size(px(30.0)).flex().items_center().justify_center().flex_none().rounded_md().border_1().border_color(if viewed_all { p.success } else { p.border }).bg(if viewed_all { p.success } else { p.surface }).cursor_pointer().hover(move |s| s.bg(if viewed_all { p.success } else { p.hover }))
                    .on_click(cx.listener(|this, _, _, cx| { this.toggle_viewed(); cx.notify(); })).child(icon(if viewed_all { ic::VISIBILITY } else { ic::VISIBILITY_OFF }, 18.0, if viewed_all { gpui::white() } else { p.muted })), if viewed_all { "Viewed. Click to mark as not viewed (v)" } else { "Mark this file as viewed (v)" }, &p)))
                .when(!self.settings.hidden("global_comment"), |d| d.child(icon_btn("gcomment", ic::ADD_COMMENT, "Comment on the whole merge request", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_composer(true, None); cx.notify(); }))))
                .when(g1 && (g2 || show_finish), |d| d.child(sep()))
                .when(!md_on && !one_sided && !self.settings.hidden("diff_mode"), |d| d.child(seg_box(&p).child(seg_icon("df-uni", ic::VIEW_AGENDA, "Unified diff (u)", mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Unified); cx.notify(); })))
                    .child(seg_icon("df-split", ic::VERTICAL_SPLIT, "Side-by-side diff (u)", mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Split); cx.notify(); })))))
                .when(is_md && !self.settings.hidden("md_mode"), |d| d.child(seg_box(&p).child(seg_icon("md-diff", ic::CODE, "Show the markdown source diff (m)", !md_on, &p).on_click(cx.listener(|this, _, _, cx| { if this.md_rendered { this.md_rendered = false; this.shown = None; this.clear_selection(); } cx.notify(); })))
                    .child(seg_icon("md-prev", ic::VISIBILITY, "Show the rendered markdown (m)", md_on, &p).on_click(cx.listener(|this, _, _, cx| { if !this.md_rendered { this.md_rendered = true; this.shown = None; this.clear_selection(); } cx.notify(); })))))
                .when(batch.len() > 1 && !self.settings.hidden("batch_view"), |d| d.child(seg_box(&p)
                    .child(seg_icon("bv-panes", ic::VIEW_QUILT, "Show the files of this batch in split panes (w)", panes_on, &p).on_click(cx.listener(|this, _, _, cx| { this.set_batch_panes(true); cx.notify(); })))
                    .child(seg_icon("bv-stack", ic::VIEW_STREAM, "Show the files of this batch one after the other (w)", !panes_on, &p).on_click(cx.listener(|this, _, _, cx| { this.set_batch_panes(false); cx.notify(); }))))
                    .when(panes_on, |d| d.child(icon_btn("bv-arr", arr_icon, arr_tip, Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| {
                        let all = Arrangement::ALL; let i = all.iter().position(|a| *a == this.arrangement).unwrap_or(0); let next = if this.layout.custom { this.arrangement } else { all[(i + 1) % all.len()] }; this.set_arrangement(next); cx.notify();
                    }))).child(icon_btn("bv-reset", ic::RESET_LAYOUT, "Reset the pane sizes (or double-click a divider)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.layout = Layout::new(this.panes.len(), this.arrangement); cx.notify(); })))))
                .when(!self.settings.hidden("fullscreen"), |d| d.child(icon_btn("fullscreen", if fs { ic::FULLSCREEN_EXIT } else { ic::FULLSCREEN }, if fs { "Leave full screen (F11 or Esc)" } else { "Diff viewer full screen (F11)" }, Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { let on = !this.fullscreen; this.set_fullscreen(on, cx); cx.notify(); }))))
                .when(g2 && show_finish, |d| d.child(sep()))
                .children(show_finish.then(|| icon_text_btn("finish", ic::RATE_REVIEW, "Finish review", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.finish_open = true; this.focus = Some(Fid::Summary); this.submit_msg = None; cx.notify(); })))));

        let composer = self.composer.is_some().then(|| self.composer_panel(cx));
        let wrap = div().relative().flex().flex_col().flex_1().min_h_0().w_full().child(div().absolute().size_full().child(kit::bounds_probe(&self.b_body))).child(body).children(toolbar).child(bar);
        div().relative().flex().flex_col().flex_1().min_w_0().min_h_0().children(header).child(wrap).children(composer)
    }

    /// The rich comment editor: Markdown toolbar, Write and Preview tabs, suggestions.
    fn composer_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let mono = self.fonts.mono.clone();
        let c = self.composer.as_ref().map(|c| (c.editing, c.global, c.preview)).unwrap_or((None, false, false));
        let (editing, global, preview) = c;
        let sel = if global || editing.is_some() { None } else { self.selection() };
        let host_label = self.composer.as_ref().and_then(|c| c.host.as_ref()).map(|h| match h { HostTarget::Reply(_) => ("Reply to this thread (posted immediately)", "Reply"), HostTarget::Edit(..) => ("Edit your posted comment (updated on the host immediately)", "Save edit") });
        let target: SharedString = if let Some((t, _)) = host_label { t.into() } else if editing.is_some() { "Edit comment".into() } else if global { "Comment on the whole merge request".into() } else { match (&sel, self.current_file()) { (Some(s), _) => format!("Comment on {}:{}–{}", s.file, s.start, s.end), (None, Some(fi)) => format!("Comment on {} (select lines first to anchor it)", self.m.vm.files[fi].path), _ => "General comment".into() }.into() };
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
            .child(div().flex().items_center().gap_2().px_3().py_2().child(div().flex_1().min_w_0().truncate().text_xs().text_color(p.muted).child("Ctrl+Enter saves a draft. Markdown and ```suggestion blocks are supported."))
                .child(icon_text_btn("cmp-cancel", ic::CLOSE, "Cancel", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.composer = None; this.focus = None; cx.stop_propagation(); cx.notify(); })))
                .children((self.m.mr.is_some() && !self.instant() && host_label.is_none()).then(|| icon_text_btn("cmp-publish", ic::SEND, "Publish now", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_composer_with(true, cx); this.flush(cx); cx.stop_propagation(); cx.notify(); }))))
                .child(icon_text_btn("cmp-save", ic::CHECK, if let Some((_, b)) = host_label { b } else if editing.is_some() { "Save changes" } else { "Add draft" }, Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_composer(cx); this.flush(cx); cx.stop_propagation(); cx.notify(); }))))
    }

    fn right_panel(&mut self, cx: &mut Context<Self>, drafts: &[perusal_core::store::Draft]) -> impl IntoElement {
        let p = self.pal;
        let tabs = div().flex().items_center().border_b_1().border_color(p.border).px_2()
            .child(self.tab_btn("tab-review", "Review", ic::COMMIT, RightTab::Review, cx)).when(self.settings.ai_on() && (self.tab == RightTab::Ai || !self.settings.hidden("ask_ai_tab")), |d| d.child(self.tab_btn("tab-ai", "Ask AI", ic::AUTO_AWESOME, RightTab::Ai, cx))).child(div().flex_1())
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
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| { if this.m.graph.rows.get(i).map(|r| r.in_mr).unwrap_or(false) { this.pick_commit(i, ev.modifiers().shift); } cx.notify(); }))
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
            .when(co, |d| d.child(div().id("commits").when_some(self.commits_h, |d, h| d.h(px(h))).when(self.commits_h.is_none(), |d| d.max_h(px(260.0))).overflow_y_scroll().children(commit_rows)))
            // drag this bar to give Commits more or less room (Drafts takes the rest)
            .child(div().id("cd-split").h(px(6.0)).w_full().flex_none().cursor(gpui::CursorStyle::ResizeUpDown).bg(if self.resizing_cd.is_some() { p.accent.opacity(0.5) } else { p.border.opacity(0.6) }).hover(move |s| s.bg(p.accent.opacity(0.5)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| { let y: f32 = ev.position.y.into(); let auto = (this.m.graph.rows.len() as f32 * 50.0).clamp(60.0, 260.0); // about what the list shows until first resized
                    this.resizing_cd = Some((y, this.commits_h.unwrap_or(auto))); cx.stop_propagation(); cx.notify(); })))
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
            .children(self.ai_notice.clone().map(|n| div().flex().items_start().gap_2().px_3().py_2().text_sm().text_color(p.danger).bg(p.danger.opacity(0.1)).border_b_1().border_color(p.danger).child(icon(ic::ERROR_OUTLINE, 17.0, p.danger)).child(div().flex_1().min_w_0().child(n.clone())).child(kit::copy_btn("ai-err-copy", n, &p))
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
        let v = self.verdict; let focused = self.focus == Some(Fid::Summary);
        let vchip = |id: &'static str, label: &'static str, on: bool, color: gpui::Hsla| div().id(id).flex().items_center().gap_2().px_3().h(px(32.0)).rounded_md().border_1().border_color(if on { color } else { p.border }).bg(if on { color.opacity(0.14) } else { gpui::transparent_black() }).cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).hover(move |x| x.bg(p.hover)).child(label);
        let summary_box = self.winp(Fid::Summary, div().id("sum-box").flex().px_3().py(px(8.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).min_h(px(90.0)).child(input_text(&self.summary, focused, "Summary (optional): what should the author know?", &p, false, 4)), cx);
        let msg = self.submit_msg.clone().map(|(ok, m)| div().flex().items_start().gap_2().px_3().py_2().rounded_md().border_1().border_color(if ok { p.success } else { p.danger }).bg(if ok { p.success.opacity(0.1) } else { p.danger.opacity(0.1) }).text_sm().text_color(if ok { p.success } else { p.danger })
            .child(icon(if ok { ic::CHECK_CIRCLE } else { ic::ERROR_OUTLINE }, 17.0, if ok { p.success } else { p.danger })).child(div().flex_1().min_w_0().child(m.clone())).children((!ok).then(|| kit::copy_btn("submit-err-copy", m, &p))));
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
