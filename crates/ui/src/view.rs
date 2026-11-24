use crate::icons as ic;
use crate::kit::{self, badge, bounds_probe, col, field_key, field_text, icon, icon_btn, icon_text_btn, kbd, loading_dots, markdown_view, new_bounds, pill, seg_box, seg_cell, seg_icon, switch, tip, Btn, FieldBounds, Fonts, Pal};
use crate::model::{self, Model};
use gpui::{deferred, div, prelude::*, px, relative, uniform_list, ClickEvent, Context, FontWeight, HighlightStyle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, SharedString, StyledText};
use perusal_agents::run::{agent_env, run_streaming, AgentEvent, RunError};
use perusal_agents::{chat_prompt, launch_with, model_suggestions, on_path, parse_review, review_prompt, supports_effort, Options, Selection, REVIEW_SKILL};
use perusal_app::markdown::{self, Located};
use perusal_app::settings::{DiffMode, MdMode, Settings, AGENTS};
use perusal_app::textfield::TextField;
use perusal_app::theme::{self, Theme};
use perusal_app::tree::{self, TreeRow};
use perusal_diff::text::{diff_lines, highlight, split_pairs, Class, Kind, Line};
use perusal_diff::{GroupRule, Status};
use perusal_filter::Suggestion;
use perusal_providers::{InlineComment, Submission, Verdict};
use std::collections::{BTreeSet, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(PartialEq, Clone, Copy)]
enum Fid { Filter, Comment, Ask, Cmd, Model, Summary, RuleLabel, RulePat }
#[derive(PartialEq, Clone, Copy)]
enum Mode { Normal, Filter, Comment, Ask, Settings, SettingsEdit(Fid), Finish }
#[derive(PartialEq, Clone, Copy)]
enum RightTab { Review, Ai }
#[derive(PartialEq, Clone, Copy)]
enum Side { Left, Right }
#[derive(Clone, Copy)]
enum Job { Chat, Review }

#[derive(Clone)]
struct RLine { line: Line, runs: Vec<(Range<usize>, HighlightStyle)> }
#[derive(Clone)]
enum RowKind { FileHeader, Note(String), Hunk(String), Uni(RLine), Split(Option<RLine>, Option<RLine>) }
struct Row { fi: usize, kind: RowKind }

pub struct Back;
impl gpui::EventEmitter<Back> for View {}

pub struct View {
    m: Model, settings: Settings, theme: Theme, pal: Pal, fonts: Fonts, mode: Mode,
    filter_f: TextField, comment_f: TextField, ask_f: TextField, cmd_f: TextField, model_f: TextField, summary_f: TextField, label_f: TextField, pat_f: TextField,
    b_filter: FieldBounds, b_comment: FieldBounds, b_ask: FieldBounds, b_cmd: FieldBounds, b_model: FieldBounds, b_summary: FieldBounds, b_label: FieldBounds, b_pat: FieldBounds,
    drag_field: Option<Fid>, rule_err: Option<String>,
    error: Option<String>, zoom: f32, comment_global: bool,
    shown: Option<Vec<usize>>, nav_changed: bool, rows: Vec<Row>, md: Option<Vec<Located>>, md_src: String, md_rendered: bool, md_sel: Option<(usize, usize)>, diff_mode: DiffMode,
    sel: Option<(usize, usize)>, anchor: Option<usize>, dragging: bool, status: String,
    tree: bool, collapsed: HashSet<String>, trows: Vec<TreeRow>, multi: BTreeSet<usize>, last_clicked: Option<usize>,
    sug: Vec<Suggestion>, sug_replace: usize, sug_idx: Option<usize>,
    left_open: bool, right_open: bool, left_w: f32, right_w: f32, resizing: Option<Side>, commits_open: bool, drafts_open: bool,
    tab: RightTab, chat: Vec<(String, String)>, ai_ctx: Option<Selection>, ai_notice: Option<String>,
    ai_job: Option<Job>, ai_events: Arc<Mutex<Vec<AgentEvent>>>, ai_cancel: Arc<AtomicBool>, ai_started: Option<Instant>, ai_activity: Vec<String>, ai_partial: String,
    verdict: Verdict, submit_busy: bool, submit_msg: Option<(bool, String)>, banner: Option<String>,
}

fn status_style(s: Status, p: &Pal) -> (&'static str, gpui::Hsla) {
    match s { Status::Added => ("A", p.success), Status::Modified => ("M", p.warn), Status::Deleted => ("D", p.danger), Status::Renamed => ("R", p.accent) }
}

fn runs_for(lang: &str, line: &Line, th: &Theme, p: &Pal) -> Vec<(Range<usize>, HighlightStyle)> {
    let ok = |a: usize, b: usize| a < b && b <= line.text.len() && line.text.is_char_boundary(a) && line.text.is_char_boundary(b);
    let mut runs: Vec<(Range<usize>, HighlightStyle)> = highlight(lang, &line.text).into_iter().filter(|(a, b, _)| ok(*a, *b)).map(|(a, b, c)| {
        let key = match c { Class::Keyword => "keyword", Class::String => "string", Class::Comment => "comment", Class::Number => "number", Class::Type => "type", Class::Function => "function", Class::Other => "keyword" };
        (a..b, HighlightStyle { color: Some(col(&th.syntax, key)), ..Default::default() })
    }).collect();
    runs.sort_by_key(|r| r.0.start);
    let mut end = 0; runs.retain(|r| { if r.0.start >= end { end = r.0.end; true } else { false } });
    for w in &line.words {
        if ok(w.0, w.1) {
            runs.retain(|r| r.0.end <= w.0 || r.0.start >= w.1);
            runs.push((w.0..w.1, HighlightStyle { background_color: Some(if line.kind == Kind::Added { p.added_word } else { p.removed_word }), color: Some(p.fg), ..Default::default() }));
            runs.sort_by_key(|r| r.0.start);
        }
    }
    runs
}

impl View {
    pub fn new(mut m: Model, settings: Settings, fonts: Fonts) -> Self {
        let theme = theme::load(&settings.theme, Some(&model::themes_dir()));
        let filter = TextField::new(&m.vm.filter);
        m.vm.dir_order = settings.files_tree; m.vm.grouping = settings.group_related; m.vm.rules = settings.group_rules.clone();
        m.vm.sync_review_state(&m.store); let _ = m.vm.refresh();
        let chat = m.store.messages(m.vm.review).unwrap_or_default();
        View { pal: Pal::new(&theme), theme, fonts, mode: Mode::Normal, filter_f: filter, comment_f: TextField::default(), ask_f: TextField::default(), cmd_f: TextField::new(&settings.agent_command), model_f: TextField::new(&settings.model),
            summary_f: TextField::default(), label_f: TextField::default(), pat_f: TextField::default(),
            b_filter: new_bounds(), b_comment: new_bounds(), b_ask: new_bounds(), b_cmd: new_bounds(), b_model: new_bounds(), b_summary: new_bounds(), b_label: new_bounds(), b_pat: new_bounds(),
            drag_field: None, rule_err: None, error: None, zoom: 1.0, comment_global: false,
            shown: None, nav_changed: true, rows: vec![], md: None, md_src: String::new(), md_rendered: false, md_sel: None, diff_mode: settings.diff_mode,
            sel: None, anchor: None, dragging: false, status: String::new(),
            tree: settings.files_tree, collapsed: HashSet::new(), trows: vec![], multi: BTreeSet::new(), last_clicked: None, sug: vec![], sug_replace: 0, sug_idx: None,
            left_open: settings.left_open, right_open: settings.right_open, left_w: settings.left_width, right_w: settings.right_width, resizing: None, commits_open: true, drafts_open: true,
            tab: RightTab::Review, chat, ai_ctx: None, ai_notice: None, ai_job: None, ai_events: Arc::new(Mutex::new(vec![])), ai_cancel: Arc::new(AtomicBool::new(false)), ai_started: None, ai_activity: vec![], ai_partial: String::new(),
            verdict: Verdict::Comment, submit_busy: false, submit_msg: None, banner: None, settings, m }
    }
    fn current_file(&self) -> Option<usize> { self.m.vm.current_batch().first().copied() }
    fn save_settings(&self) { model::save_settings(&self.settings); }
    fn apply_theme(&mut self) { self.theme = theme::load(&self.settings.theme, Some(&model::themes_dir())); self.pal = Pal::new(&self.theme); self.shown = None; }
    fn resync(&mut self) { self.m.vm.sync_review_state(&self.m.store); let _ = self.m.vm.refresh(); }
    fn reload(&mut self) {
        self.shown = None; self.nav_changed = true; self.sel = None; self.anchor = None; self.md_sel = None;
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

    // ---------- text fields
    fn fld(&mut self, id: Fid) -> &mut TextField {
        match id { Fid::Filter => &mut self.filter_f, Fid::Comment => &mut self.comment_f, Fid::Ask => &mut self.ask_f, Fid::Cmd => &mut self.cmd_f, Fid::Model => &mut self.model_f, Fid::Summary => &mut self.summary_f, Fid::RuleLabel => &mut self.label_f, Fid::RulePat => &mut self.pat_f }
    }
    fn fbounds(&self, id: Fid) -> FieldBounds {
        match id { Fid::Filter => &self.b_filter, Fid::Comment => &self.b_comment, Fid::Ask => &self.b_ask, Fid::Cmd => &self.b_cmd, Fid::Model => &self.b_model, Fid::Summary => &self.b_summary, Fid::RuleLabel => &self.b_label, Fid::RulePat => &self.b_pat }.clone()
    }
    fn ffamily(&self, id: Fid) -> SharedString { if matches!(id, Fid::Filter | Fid::RulePat) { self.fonts.mono.clone() } else { self.fonts.ui.clone() } }
    fn field_down(&mut self, id: Fid, ev: &MouseDownEvent, window: &gpui::Window) {
        self.mode = match id { Fid::Filter => { self.update_suggestions(); Mode::Filter } Fid::Comment => Mode::Comment, Fid::Ask => { if self.ai_ctx.is_none() { self.ai_ctx = self.selection(); } Mode::Ask } Fid::Summary => Mode::Finish, other => Mode::SettingsEdit(other) };
        let (b, fam) = (self.fbounds(id).get(), self.ffamily(id));
        kit::field_mouse_down(self.fld(id), window, &b, &fam, px(14.0), ev); self.drag_field = Some(id);
        if id == Fid::Filter { self.update_suggestions(); }
    }
    fn field_drag(&mut self, id: Fid, x: gpui::Pixels, window: &gpui::Window) {
        let (b, fam) = (self.fbounds(id).get(), self.ffamily(id)); kit::field_mouse_drag(self.fld(id), window, &b, &fam, px(14.0), x);
    }
    /// A text field container wired for mouse caret placement and drag selection.
    fn wire(&self, id: Fid, el: gpui::Stateful<gpui::Div>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        el.cursor_text()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, w, cx| { this.field_down(id, ev, w); cx.stop_propagation(); cx.notify(); }))
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, w, cx| { if ev.pressed_button == Some(MouseButton::Left) && this.drag_field == Some(id) { this.field_drag(id, ev.position.x, w); cx.notify(); } }))
    }
    fn text_of(&self, id: Fid, focused: bool, placeholder: &str) -> gpui::Div {
        let f = match id { Fid::Filter => &self.filter_f, Fid::Comment => &self.comment_f, Fid::Ask => &self.ask_f, Fid::Cmd => &self.cmd_f, Fid::Model => &self.model_f, Fid::Summary => &self.summary_f, Fid::RuleLabel => &self.label_f, Fid::RulePat => &self.pat_f };
        div().relative().flex_1().min_w_0().cursor_text().text_size(px(14.0)).child(bounds_probe(&self.fbounds(id))).child(field_text(f, focused, placeholder, &self.pal))
    }

    // ---------- diff rows
    fn load_diff(&mut self) {
        let batch = self.m.vm.current_batch().to_vec();
        if batch.is_empty() { self.rows.clear(); self.md = None; self.shown = None; return; }
        if self.shown.as_ref() == Some(&batch) { return; }
        if self.nav_changed { self.md_rendered = self.settings.markdown == MdMode::Rendered && batch.len() == 1 && markdown::is_markdown(&self.m.vm.files[batch[0]].path); self.nav_changed = false; }
        self.shown = Some(batch.clone()); self.status.clear(); self.md = None;
        let (th, p) = (self.theme.clone(), self.pal);
        let mut rows = Vec::new();
        for &fi in &batch {
            let f = self.m.vm.files[fi].clone();
            let old = perusal_gitops::show(&self.m.repo, &self.m.base_sha, &f.path).unwrap_or_default();
            let new = perusal_gitops::show(&self.m.repo, &self.m.branch, &f.path).unwrap_or_default();
            if batch.len() == 1 && self.md_rendered && !f.symlink {
                let src = if new.is_empty() { old } else { new }; self.md = Some(markdown::parse_located(&src)); self.md_src = src; return;
            }
            if batch.len() > 1 { rows.push(Row { fi, kind: RowKind::FileHeader }); }
            if f.symlink {
                let t = |s: &str| s.trim().to_string();
                rows.push(Row { fi, kind: RowKind::Note(match (old.is_empty(), new.is_empty()) { (true, false) => format!("Symbolic link to {}", t(&new)), (false, true) => format!("Symbolic link to {} was removed", t(&old)), _ => format!("Symbolic link retargeted: {} → {}", t(&old), t(&new)) }) });
            }
            if f.noisy() && !f.symlink { self.status = format!("{} is collapsed because it is generated, binary or very large.", f.path); continue; }
            let lang = match f.ext() { "rs" => "rust", "py" => "python", _ => "" };
            let lines = diff_lines(&old, &new, 3);
            let mk = |l: &Line| RLine { line: l.clone(), runs: runs_for(lang, l, &th, &p) };
            let (mut last_old, mut last_new, mut first) = (0u32, 0u32, true);
            let mut hunk = |rows: &mut Vec<Row>, o: u32, n: u32| {
                let jump = (o > 0 && o > last_old + 1) || (n > 0 && n > last_new + 1) || (first && (o > 1 || n > 1));
                if jump { rows.push(Row { fi, kind: RowKind::Hunk(format!("@@ -{} +{} @@", o.max(1), n.max(1))) }); }
                first = false;
                if o > 0 { last_old = o; }
                if n > 0 { last_new = n; }
            };
            // added or deleted files have nothing on one side, so they always use a single column
            let split = self.diff_mode == DiffMode::Split && !matches!(f.status, Status::Added | Status::Deleted);
            if !split {
                for l in &lines { hunk(&mut rows, l.old_no.unwrap_or(0), l.new_no.unwrap_or(0)); rows.push(Row { fi, kind: RowKind::Uni(mk(l)) }); }
            } else {
                for (a, b) in split_pairs(&lines) {
                    hunk(&mut rows, a.and_then(|i| lines[i].old_no).unwrap_or(0), b.and_then(|i| lines[i].new_no).unwrap_or(0));
                    rows.push(Row { fi, kind: RowKind::Split(a.map(|i| mk(&lines[i])), b.map(|i| mk(&lines[i]))) });
                }
            }
        }
        self.rows = rows;
    }

    /// Selected lines (diff rows or rendered markdown blocks) as a code selection in one file.
    fn selection(&self) -> Option<Selection> {
        if let (Some(blocks), Some((a, b))) = (&self.md, self.md_sel) {
            let fi = self.current_file()?; let (lo, hi) = (a.min(b), a.max(b));
            let (first, last) = (blocks.get(lo)?.first_line, blocks.get(hi)?.last_line);
            let snippet: String = self.md_src.lines().skip(first.saturating_sub(1) as usize).take((last + 1 - first) as usize).collect::<Vec<_>>().join("\n");
            return Some(Selection { file: self.m.vm.files[fi].path.clone(), start: first, end: last, snippet });
        }
        let (a, b) = self.sel?; let fi = self.rows.get(a)?.fi;
        let (mut lo, mut hi, mut snippet) = (u32::MAX, 0u32, String::new());
        for r in &self.rows[a..=b.min(self.rows.len() - 1)] {
            if r.fi != fi { continue; }
            let lines: Vec<&Line> = match &r.kind { RowKind::Uni(l) => vec![&l.line], RowKind::Split(l, rr) => l.iter().chain(rr.iter()).map(|x| &x.line).collect(), _ => vec![] };
            for l in lines {
                if let Some(n) = l.new_no.or(l.old_no) { lo = lo.min(n); hi = hi.max(n); }
                snippet.push_str(match l.kind { Kind::Added => "+", Kind::Removed => "-", Kind::Context => " " }); snippet.push_str(&l.text); snippet.push('\n');
            }
        }
        (hi > 0).then(|| Selection { file: self.m.vm.files[fi].path.clone(), start: lo, end: hi, snippet })
    }
    fn row_hit(&mut self, i: usize, shift: bool) {
        if shift { if let Some(a) = self.anchor { self.sel = Some((a.min(i), a.max(i))); return; } }
        self.anchor = Some(i); self.sel = Some((i, i)); self.dragging = true;
    }
    fn clear_selection(&mut self) { self.sel = None; self.anchor = None; self.md_sel = None; }
    fn has_selection(&self) -> bool { self.sel.is_some() || self.md_sel.is_some() }

    // ---------- actions
    fn apply_filter(&mut self) {
        match self.m.vm.set_filter(self.filter_f.text.trim()) { Ok(()) => { self.error = None; self.mode = Mode::Normal; self.sug.clear(); self.reload(); } Err(e) => self.error = Some(e) }
    }
    fn update_suggestions(&mut self) { let (s, n) = perusal_filter::suggest(self.filter_f.before_caret()); self.sug = s; self.sug_replace = n; self.sug_idx = None; }
    fn accept_suggestion(&mut self, i: usize) { if let Some(s) = self.sug.get(i).cloned() { self.filter_f.replace_before_caret(self.sug_replace, &s.insert); self.update_suggestions(); } }
    fn add_comment(&mut self) {
        let text = self.comment_f.text.trim().to_string(); if text.is_empty() { return; }
        let sel = if self.comment_global { None } else { self.selection() };
        match (sel, self.current_file(), self.comment_global) {
            (Some(s), _, _) => { let _ = self.m.store.add_draft(self.m.vm.review, Some(&s.file), Some((s.start, s.end)), &text, false); self.status = format!("Draft added on {}:{}-{}", s.file, s.start, s.end); }
            (None, Some(fi), false) => { let path = self.m.vm.files[fi].path.clone(); let _ = self.m.store.add_draft(self.m.vm.review, Some(&path), None, &text, false); self.status = format!("Draft added on {path}"); }
            _ => { let _ = self.m.store.add_draft(self.m.vm.review, None, None, &text, false); self.status = "Draft added on the merge request".into(); }
        }
        self.comment_f.clear(); self.mode = Mode::Normal; self.comment_global = false; self.resync();
    }
    fn toggle_viewed(&mut self) { if let Some(fi) = self.current_file() { let v = !self.m.vm.files[fi].viewed; for i in self.m.vm.current_batch().to_vec() { self.m.vm.set_viewed(&self.m.store, i, v); } } }
    fn next(&mut self) { self.m.vm.next(&self.m.store); self.reload(); }
    fn back(&mut self) { self.m.vm.back(); self.reload(); }
    fn step(&mut self, d: i32) { let c = self.m.vm.cursor as i32 + d; if c >= 0 && (c as usize) < self.m.vm.batch_count() { self.m.vm.cursor = c as usize; self.reload(); } }
    fn clear_scope(&mut self) { self.m.graph.clear(); self.m.vm.commit_scope = None; let _ = self.m.vm.refresh(); self.reload(); }
    fn pick_commit(&mut self, i: usize) {
        if self.m.graph.selection == Some((i, i)) { self.clear_scope(); return; }
        self.m.graph.click(i); self.m.vm.commit_scope = self.m.graph.scope(&self.m.repo).ok().flatten(); let _ = self.m.vm.refresh(); self.reload();
    }
    fn set_diff_mode(&mut self, m: DiffMode) { if self.diff_mode != m { self.diff_mode = m; self.shown = None; self.sel = None; } }
    fn click_file(&mut self, fi: usize, mods: gpui::Modifiers) {
        if mods.control || mods.platform {
            if !self.multi.remove(&fi) { if self.multi.is_empty() { if let Some(c) = self.current_file() { if c != fi { self.multi.insert(c); } } } self.multi.insert(fi); }
            self.last_clicked = Some(fi);
        } else if mods.shift {
            let order = self.display_order(); let from = self.last_clicked.or_else(|| self.current_file()).unwrap_or(fi);
            if let (Some(a), Some(b)) = (order.iter().position(|x| *x == from), order.iter().position(|x| *x == fi)) { for x in &order[a.min(b)..=a.max(b)] { self.multi.insert(*x); } }
        } else { self.multi.clear(); self.last_clicked = Some(fi); self.m.vm.select_file(fi); self.reload(); }
    }
    fn group_multi(&mut self) {
        let order = self.display_order(); let mut members: Vec<usize> = order.into_iter().filter(|i| self.multi.contains(i)).collect();
        if members.len() < 2 { return; }
        members.dedup(); self.m.vm.group_files_manually(members); self.multi.clear(); self.reload();
        self.status = "Grouped into one batch. Viewing it shows every file together.".into();
    }
    fn set_rules(&mut self) { self.m.vm.rules = self.settings.group_rules.clone(); let _ = self.m.vm.refresh(); self.reload(); self.save_settings(); }
    fn add_rule(&mut self) {
        let pat = self.pat_f.text.trim().to_string(); let label = self.label_f.text.trim().to_string();
        if let Err(e) = perusal_diff::rules::validate_pattern(&pat) { self.rule_err = Some(e); return; }
        let id = format!("custom-{}", self.settings.group_rules.iter().filter(|r| !r.builtin).count() + 1);
        self.settings.group_rules.push(GroupRule { id, label: if label.is_empty() { pat.clone() } else { label }, pattern: pat, enabled: true, builtin: false });
        self.label_f.clear(); self.pat_f.clear(); self.rule_err = None; self.set_rules();
    }

    // ---------- AI
    fn ai_options(&self) -> Options { Options { model: self.settings.model.clone(), effort: self.settings.effort.clone() } }
    fn start_ai(&mut self, job: Job, question: String, cx: &mut Context<Self>) {
        if self.busy() { return; }
        let opts = self.ai_options();
        let Some(probe) = launch_with(&self.settings.agent, &self.settings.agent_command, "x", &opts) else { self.ai_notice = Some("No agent configured. Choose one in Settings.".into()); return };
        if !on_path(&probe.program) { self.ai_notice = Some(format!("`{}` was not found on your PATH. Install it, pick another agent in Settings, or set a custom command.", probe.program)); return; }
        self.ai_notice = None; self.ai_partial.clear(); self.ai_activity.clear();
        let sel = self.ai_ctx.clone();
        let (repo, rev, base, review) = (self.m.repo.clone(), self.m.branch.clone(), self.m.base_sha.clone(), self.m.vm.review);
        let (agent, custom) = (self.settings.agent.clone(), self.settings.agent_command.clone());
        let changed: Vec<String> = self.m.vm.files.iter().map(|f| f.path.clone()).collect();
        let prompt_chat = chat_prompt(sel.as_ref(), &self.chat, &question);
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
            let prompt = if is_review { push(AgentEvent::Activity("Collecting the patch".into())); let patch = perusal_gitops::patch(&repo, &base, &rev).map_err(RunError::Failed)?; review_prompt(REVIEW_SKILL, &changed, &patch) } else { prompt_chat };
            let l = launch_with(&agent, &custom, &prompt, &opts).ok_or_else(|| RunError::Failed("no agent configured".into()))?;
            push(AgentEvent::Activity(format!("Starting {}", l.program)));
            run_streaming(&l, &dest, &agent_env(), std::time::Duration::from_secs(900), &cancel, push)
        });
        cx.spawn(async move |this, cx| { let r: Result<String, RunError> = task.await; let _ = this.update(cx, |t, cx| t.finish_ai(job, r, cx)); }).detach();
        // poll for live events and the elapsed clock while the agent works
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
                }
                Err(e) => self.ai_notice = Some(format!("The agent's answer was not a usable review ({e}). First 300 characters: {}", text.chars().take(300).collect::<String>())),
            },
            (_, Err(RunError::Cancelled)) => self.ai_notice = Some("Stopped. Nothing was added.".into()),
            (_, Err(e)) => self.ai_notice = Some(e.to_string()),
        }
        cx.notify();
    }
    fn draft_from_answer(&mut self, text: &str) {
        let (f, l) = match &self.ai_ctx { Some(s) => (Some(s.file.clone()), Some((s.start, s.end))), None => (self.current_file().map(|i| self.m.vm.files[i].path.clone()), None) };
        let _ = self.m.store.add_draft(self.m.vm.review, f.as_deref(), l, text, true); self.status = "Answer saved as an AI draft.".into(); self.resync();
    }

    // ---------- finishing the review
    fn finish_counts(&self) -> (usize, usize, usize, usize) {
        let d = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let (v, t) = self.m.vm.progress();
        (d.len(), d.iter().filter(|x| x.ai).count(), t - v, t)
    }
    fn build_submission(&self) -> Submission {
        let drafts = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let mut summary = self.summary_f.text.trim().to_string(); let mut comments = vec![];
        for d in drafts {
            match (&d.file, d.start_line, d.end_line) {
                (Some(f), Some(a), Some(b)) => comments.push(InlineComment { path: f.clone(), start_line: a, end_line: b, body: d.body }),
                (Some(f), _, _) => { summary.push_str(&format!("\n\n**{f}**: {}", d.body)); }
                _ => { summary.push_str(&format!("\n\n{}", d.body)); }
            }
        }
        Submission { head_sha: self.m.branch.clone(), summary: summary.trim().to_string(), verdict: self.verdict.clone(), comments }
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
                self.summary_f.clear(); self.mode = Mode::Normal; self.banner = Some("Review posted to the host. Your drafts were cleared.".into());
            }
            Err(e) => self.submit_msg = Some((false, e)),
        }
        cx.notify();
    }
    fn export_local(&mut self, cx: &mut Context<Self>) {
        match self.m.store.export_markdown(self.m.vm.review).map_err(|e| e.to_string()).and_then(|md| { cx.write_to_clipboard(gpui::ClipboardItem::new_string(md.clone())); model::export_markdown_file(&self.m.title, &md) }) {
            Ok(p) => self.submit_msg = Some((true, format!("Copied to the clipboard and saved to {}", p.display()))),
            Err(e) => self.submit_msg = Some((false, e)),
        }
    }

    // ---------- keys
    pub fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke; let key = ks.key.as_str();
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        match self.mode {
            Mode::Settings => { if key == "escape" { self.mode = Mode::Normal; cx.notify(); } return; }
            Mode::SettingsEdit(id) => {
                if key == "escape" { self.mode = Mode::Settings; }
                else if key == "enter" {
                    match id { Fid::Cmd => { self.settings.agent_command = self.cmd_f.text.trim().to_string(); self.save_settings(); self.mode = Mode::Settings; }
                        Fid::Model => { self.settings.model = self.model_f.text.trim().to_string(); self.save_settings(); self.mode = Mode::Settings; }
                        _ => self.add_rule() }
                } else { field_key(self.fld(id), ks, cx); if id == Fid::RulePat { self.rule_err = None; } }
                cx.notify(); return;
            }
            Mode::Finish => {
                if key == "escape" { self.mode = Mode::Normal; } else if key == "enter" && ctrl { self.start_submit(cx); } else { field_key(&mut self.summary_f, ks, cx); }
                cx.notify(); return;
            }
            Mode::Filter => {
                match key {
                    "escape" => { if self.sug.is_empty() { self.mode = Mode::Normal; self.error = None; let applied = self.m.vm.filter.clone(); self.filter_f.set(&applied); } else { self.sug.clear(); } }
                    "enter" => { if let Some(i) = self.sug_idx { self.accept_suggestion(i); } else { self.apply_filter(); } }
                    "tab" => { if !self.sug.is_empty() { self.accept_suggestion(self.sug_idx.unwrap_or(0)); } }
                    "down" => { if !self.sug.is_empty() { self.sug_idx = Some(self.sug_idx.map(|i| (i + 1) % self.sug.len()).unwrap_or(0)); } }
                    "up" => { if !self.sug.is_empty() { self.sug_idx = Some(self.sug_idx.map(|i| (i + self.sug.len() - 1) % self.sug.len()).unwrap_or(self.sug.len() - 1)); } }
                    _ => { if field_key(&mut self.filter_f, ks, cx) { self.update_suggestions(); } }
                }
                cx.notify(); return;
            }
            Mode::Comment => {
                if key == "escape" { self.mode = Mode::Normal; self.comment_global = false; } else if key == "enter" { self.add_comment(); } else { field_key(&mut self.comment_f, ks, cx); }
                cx.notify(); return;
            }
            Mode::Ask => {
                if key == "escape" { self.mode = Mode::Normal; }
                else if key == "enter" { let q = self.ask_f.text.trim().to_string(); if !q.is_empty() { self.ask_f.clear(); self.start_ai(Job::Chat, q, cx); } }
                else { field_key(&mut self.ask_f, ks, cx); }
                cx.notify(); return;
            }
            Mode::Normal => {}
        }
        if ctrl { match key { "=" | "+" => self.zoom = theme::zoom(self.zoom, 1), "-" => self.zoom = theme::zoom(self.zoom, -1), "0" => self.zoom = theme::zoom(self.zoom, 0), _ => {} } cx.notify(); return; }
        match key {
            "escape" => { if self.has_selection() || !self.multi.is_empty() { self.clear_selection(); self.multi.clear(); } else if self.busy() { self.stop_ai(); } else { cx.emit(Back); return; } }
            "/" => { self.mode = Mode::Filter; self.update_suggestions(); }
            "c" => { self.comment_global = false; self.mode = Mode::Comment; }
            "a" => { self.ai_ctx = self.selection(); self.tab = RightTab::Ai; self.right_open = true; self.mode = Mode::Ask; }
            "s" => self.mode = Mode::Settings,
            "f" => { self.mode = Mode::Finish; self.submit_msg = None; }
            "m" => { if let Some(fi) = self.current_file() { if markdown::is_markdown(&self.m.vm.files[fi].path) { self.md_rendered = !self.md_rendered; self.shown = None; self.clear_selection(); } } }
            "u" => { let m = if self.diff_mode == DiffMode::Unified { DiffMode::Split } else { DiffMode::Unified }; self.set_diff_mode(m); }
            "v" => self.toggle_viewed(), "enter" => self.next(), "backspace" => self.back(),
            "t" => self.toggle_tree(), "n" => self.step(1), "p" => self.step(-1), "x" => self.clear_scope(),
            "g" => { self.settings.group_related = !self.settings.group_related; self.save_settings(); self.m.vm.grouping = self.settings.group_related; let _ = self.m.vm.refresh(); self.reload(); }
            _ => {}
        }
        cx.notify();
    }
}

fn ago(i: Instant) -> String { let s = i.elapsed().as_secs(); if s < 60 { format!("{s}s") } else { format!("{}m {}s", s / 60, s % 60) } }

impl Render for View {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settings.follow_os {
            let dark = matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark);
            let want = self.settings.effective_theme(dark).to_string();
            if self.theme.name != theme::load(&want, Some(&model::themes_dir())).name { let s = self.settings.theme.clone(); self.settings.theme = want; self.apply_theme(); self.settings.theme = s; }
        }
        self.load_diff();
        if self.tree { self.trows = tree::build_rows(&self.m.vm.files, &self.m.vm.visible, &self.collapsed); }
        let p = self.pal; let z = self.zoom; let mono = self.fonts.mono.clone();
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
        let banner = self.banner.clone().map(|b| div().flex().items_center().gap_2().px_4().py(px(6.0)).bg(p.success.opacity(0.14)).border_b_1().border_color(p.success).text_sm().text_color(p.success)
            .child(icon(ic::CHECK_CIRCLE, 17.0, p.success)).child(div().flex_1().child(b)).child(tip(div().id("bn-x").cursor_pointer().child(icon(ic::CLOSE, 16.0, p.success)).on_click(cx.listener(|this, _, _, cx| { this.banner = None; cx.notify(); })), "Dismiss", &p)));
        let filter = self.filter_row(cx);

        let left = if self.left_open { self.files_panel(cx).into_any_element() } else { self.rail(Side::Left, cx).into_any_element() };
        let center = self.center(cx, &mono);
        let right = if self.right_open { self.right_panel(cx, &drafts, &mono).into_any_element() } else { self.rail(Side::Right, cx).into_any_element() };
        let (lw, rw) = (self.left_w * z, self.right_w * z);

        let footer = div().flex().items_center().gap_4().px_4().py(px(5.0)).bg(p.surface).border_t_1().border_color(p.border).text_xs().text_color(p.muted)
            .children([("n / p", "file"), ("v", "viewed"), ("Enter", "next"), ("/", "filter"), ("c", "comment"), ("a", "ask AI"), ("f", "finish"), ("u", "split"), ("m", "markdown"), ("t", "tree"), ("g", "group"), ("s", "settings"), ("Esc", "home")]
                .map(|(k, l)| div().flex().items_center().gap_1().child(kbd(k, &p)).child(l)));
        let overlay = match self.mode { Mode::Settings | Mode::SettingsEdit(_) => Some(self.settings_overlay(cx).into_any_element()), Mode::Finish => Some(self.finish_overlay(cx).into_any_element()), _ => None };

        let side = |w: f32, open: bool, s: Side, cx: &mut Context<Self>, el: gpui::AnyElement| {
            let col = div().h_full().flex().flex_col().flex_none().bg(p.surface).overflow_hidden().child(el);
            let col = if open { col.w(px(w)) } else { col.w(px(38.0)) };
            let handle = open.then(|| div().id(if s == Side::Left { "rz-l" } else { "rz-r" }).w(px(5.0)).h_full().flex_none().cursor_col_resize().bg(p.border).hover(move |x| x.bg(p.accent.opacity(0.6)))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| { this.resizing = Some(s); cx.stop_propagation(); cx.notify(); })));
            (col, handle)
        };
        let (lcol, lhandle) = side(lw, self.left_open, Side::Left, cx, left);
        let (rcol, rhandle) = side(rw, self.right_open, Side::Right, cx, right);

        div().relative().size_full().flex().flex_col().bg(p.bg).text_color(p.fg).text_size(px(14.0 * z))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.dragging = false; this.drag_field = None;
                if this.resizing.take().is_some() { this.settings.left_width = this.left_w; this.settings.right_width = this.right_w; this.save_settings(); }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, w, cx| {
                if ev.pressed_button != Some(MouseButton::Left) { return; }
                let zoom = this.zoom; let vw: f32 = w.viewport_size().width.into(); let x: f32 = ev.position.x.into();
                match this.resizing { Some(Side::Left) => { this.left_w = (x / zoom).clamp(200.0, 700.0); cx.notify(); } Some(Side::Right) => { this.right_w = ((vw - x) / zoom).clamp(220.0, 700.0); cx.notify(); } None => {} }
            }))
            .child(top).child(progress).children(banner).child(filter)
            .child(div().flex().flex_1().min_h_0().child(lcol).children(lhandle).child(center).children(rhandle).child(rcol))
            .child(footer)
            .children(overlay)
    }
}

impl View {
    fn rail(&mut self, s: Side, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let (i, t) = if s == Side::Left { (ic::CHEVRON_RIGHT, "Show the files panel") } else { (ic::CHEVRON_LEFT, "Show the review panel") };
        div().flex().flex_col().items_center().pt_2().size_full().bg(p.surface).child(icon_btn(if s == Side::Left { "open-l" } else { "open-r" }, i, t, Btn::Ghost, &p)
            .on_click(cx.listener(move |this, _, _, cx| { if s == Side::Left { this.left_open = true; this.settings.left_open = true; } else { this.right_open = true; this.settings.right_open = true; } this.save_settings(); cx.notify(); })))
    }

    fn top_bar(&mut self, cx: &mut Context<Self>, ndrafts: usize) -> impl IntoElement {
        let p = self.pal;
        let reviewing = matches!(self.ai_job, Some(Job::Review));
        div().flex().items_center().gap_3().px_4().h(px(52.0)).bg(p.surface).border_b_1().border_color(p.border)
            .child(icon_btn("home", ic::HOME, "Back to Home (Esc)", Btn::Ghost, &p).on_click(cx.listener(|_, _, _, cx| cx.emit(Back))))
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).truncate().min_w_0().child(SharedString::from(self.m.title.clone())))
            .children(self.m.remote.clone().map(|u| pill(u.trim_start_matches("https://").to_string(), &p)))
            .child(div().flex_1())
            .when(ndrafts > 0, |d| d.child(badge(format!("{ndrafts} draft{}", if ndrafts == 1 { "" } else { "s" }), p.accent, p.accent.opacity(0.15))))
            .child(if reviewing {
                icon_text_btn("ai-review", ic::STOP, "Stop AI review", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.stop_ai(); cx.notify(); })).into_any_element()
            } else {
                icon_text_btn("ai-review", ic::AUTO_AWESOME, "AI review", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.tab = RightTab::Ai; this.right_open = true; this.start_ai(Job::Review, String::new(), cx); cx.notify(); })).into_any_element()
            })
            .child(icon_btn("settings", ic::SETTINGS, "Settings (s)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Settings; cx.notify(); })))
    }

    fn filter_row(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let mono = self.fonts.mono.clone();
        let focused = self.mode == Mode::Filter;
        let status_text: SharedString = self.error.clone().map(|e| format!("Filter error: {e}")).unwrap_or_else(|| self.status.clone()).into();
        let bad = self.error.is_some();
        let dropdown = (focused && !self.sug.is_empty()).then(|| {
            let items: Vec<_> = self.sug.iter().enumerate().map(|(i, s)| {
                let on = self.sug_idx == Some(i);
                div().id(("sug", i)).flex().items_center().gap_3().px_3().py(px(5.0)).cursor_pointer().when(on, |d| d.bg(p.accent.opacity(0.18))).hover(move |x| x.bg(p.hover))
                    .on_click(cx.listener(move |this, _, _, cx| { this.accept_suggestion(i); cx.notify(); }))
                    .child(div().font_family(mono.clone()).text_sm().font_weight(FontWeight::MEDIUM).child(s.label.clone()))
                    .child(div().flex_1().text_xs().text_color(p.muted).truncate().child(s.detail))
                    .child(badge(s.kind, p.muted, p.hover))
            }).collect();
            deferred(div().absolute().top(px(42.0)).left_0().w_full().rounded_md().border_1().border_color(p.border).bg(p.surface).shadow_lg().py_1().occlude().children(items)
                .child(div().px_3().py_1().text_xs().text_color(p.muted).child("Up/Down to choose, Tab to accept, Enter to run the filter"))).with_priority(10)
        });
        let boxed = div().id("filter").flex().items_center().gap_2().px_3().py(px(6.0)).w_full().rounded_md().border_1().border_color(if focused { p.accent } else if bad { p.danger } else { p.border }).bg(p.surface)
            .child(icon(ic::SEARCH, 18.0, p.muted)).child(self.text_of(Fid::Filter, focused, "Filter files: additions > 50 && !viewed   ·   try ai, comments > 0, status == \"added\"").font_family(mono.clone()))
            .when(!self.filter_f.is_empty(), |d| d.child(tip(div().id("clr").cursor_pointer().child(icon(ic::CLOSE, 16.0, p.muted)).on_click(cx.listener(|this, _, _, cx| { this.filter_f.clear(); this.apply_filter(); cx.stop_propagation(); cx.notify(); })), "Clear filter", &p)));
        let boxed = self.wire(Fid::Filter, boxed, cx);
        div().flex().flex_col().w_full().px_4().py_2().bg(p.bg).border_b_1().border_color(p.border).gap_1()
            .child(div().relative().w_full().child(boxed).children(dropdown))
            .when(bad || !self.status.is_empty(), |d| d.child(div().text_xs().text_color(if bad { p.danger } else { p.muted }).truncate().child(status_text)))
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
                            .child(div().text_xs().flex_none().flex().gap_1().child(div().text_color(p.success).child(format!("+{additions}"))).child(div().text_color(p.danger).child(format!("−{deletions}"))))
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
                    .child(div().text_xs().flex_none().flex().gap_1().child(div().text_color(p.success).child(format!("+{}", f.additions))).child(div().text_color(p.danger).child(format!("−{}", f.deletions))))
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
            .child(icon_text_btn("grp-multi", ic::LINK, "Review together", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.group_multi(); cx.notify(); })))
            .child(icon_btn("clr-multi", ic::CLOSE, "Clear selection", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.multi.clear(); cx.notify(); }))));
        let hint = (n == 0 && n_files > 1 && self.m.vm.commit_scope.is_none()).then(|| div().px_3().py(px(4.0)).text_xs().text_color(p.muted).border_b_1().border_color(p.border).child("Ctrl+click or Shift+click files to review them together"));
        let scope = self.m.graph.selection.filter(|_| self.m.vm.commit_scope.is_some()).and_then(|(a, _)| self.m.graph.rows.get(a).cloned()).map(|r| div().flex().items_center().gap_2().px_3().py(px(6.0)).bg(p.accent.opacity(0.12)).border_b_1().border_color(p.border)
            .child(icon(ic::COMMIT, 17.0, p.accent)).child(div().flex_1().min_w_0().truncate().text_sm().child(format!("Only commit {} · {} file{}", &r.sha[..7.min(r.sha.len())], n_files, if n_files == 1 { "" } else { "s" })))
            .child(icon_btn("scope-x", ic::CLOSE, "Show all commits (x)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.clear_scope(); cx.notify(); }))));
        div().flex().flex_col().size_full().child(header).children(scope).children(bar).children(hint).child(list)
    }

    fn center(&mut self, cx: &mut Context<Self>, mono: &SharedString) -> impl IntoElement {
        let p = self.pal;
        let batch = self.m.vm.current_batch().to_vec();
        let first = batch.first().map(|&i| self.m.vm.files[i].clone());
        let is_md = first.as_ref().map(|f| batch.len() == 1 && markdown::is_markdown(&f.path) && !f.symlink).unwrap_or(false);
        let viewed_all = !batch.is_empty() && batch.iter().all(|&i| self.m.vm.files[i].viewed);
        let manual = batch.first().map(|&i| self.m.vm.is_manually_grouped(i)).unwrap_or(false);
        let (adds, dels): (u32, u32) = batch.iter().fold((0, 0), |a, &i| (a.0 + self.m.vm.files[i].additions, a.1 + self.m.vm.files[i].deletions));
        let md_on = self.md_rendered; let mode = self.diff_mode;

        let title = if batch.len() > 1 { div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Batch of {} files", batch.len()))).child(div().text_xs().text_color(p.muted).child(if manual { "grouped by you" } else { "related files" })) }
            else if let Some(f) = &first { let (dir, name) = match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) };
                div().flex().items_center().gap_2().min_w_0().truncate().when(f.symlink, |d| d.child(icon(ic::LINK, 17.0, p.accent))).child(div().flex().font_family(mono.clone()).text_sm().child(div().text_color(p.muted).child(dir)).child(div().font_weight(FontWeight::SEMIBOLD).child(name))) } else { div().child("No files match") };
        let header = (!batch.is_empty()).then(|| {
            let mut h = div().flex().items_center().gap_3().px_4().py_2().border_b_1().border_color(p.border).bg(p.surface)
                .child(div().flex_1().min_w_0().child(title))
                .child(div().flex().items_center().gap_2().text_xs().font_family(mono.clone()).child(div().text_color(p.success).child(format!("+{adds}"))).child(div().text_color(p.danger).child(format!("−{dels}"))));
            if manual { h = h.child(icon_btn("ungroup", ic::LINK_OFF, "Ungroup these files", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { if let Some(fi) = this.current_file() { this.m.vm.ungroup(fi); this.reload(); } cx.notify(); }))); }
            h.child(icon_btn("comment", ic::COMMENT, "Comment on the selection (c)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.comment_global = false; this.mode = Mode::Comment; cx.notify(); })))
                .child(div().id("viewed").flex().items_center().gap_2().px_3().h(px(30.0)).rounded_md().border_1().border_color(if viewed_all { p.success } else { p.border }).cursor_pointer().hover(move |s| s.bg(p.hover)).text_sm().font_weight(FontWeight::MEDIUM)
                    .on_click(cx.listener(|this, _, _, cx| { this.toggle_viewed(); cx.notify(); }))
                    .child(div().size(px(16.0)).rounded_sm().border_1().border_color(if viewed_all { p.success } else { p.muted }).flex().items_center().justify_center().when(viewed_all, |d| d.bg(p.success).child(icon(ic::CHECK, 12.0, gpui::white()))))
                    .child("Viewed"))
        });

        let drafts = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let body = if let Some(blocks) = &self.md {
            let cur_path = first.as_ref().map(|f| f.path.clone()).unwrap_or_default();
            let items: Vec<_> = blocks.iter().enumerate().map(|(i, b)| {
                let on = self.md_sel.map(|(a, c)| i >= a.min(c) && i <= a.max(c)).unwrap_or(false);
                let n = drafts.iter().filter(|d| d.file.as_deref() == Some(cur_path.as_str()) && d.start_line.map(|s| s <= b.last_line && d.end_line.unwrap_or(s) >= b.first_line).unwrap_or(false)).count();
                div().id(("mdb", i)).relative().px_3().py(px(2.0)).rounded_md().cursor_pointer().border_l_2().border_color(if on { p.accent } else { gpui::transparent_black() }).when(on, |d| d.bg(p.accent.opacity(0.12))).hover(move |s| s.bg(p.hover.opacity(0.6)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.md_sel = match (ev.modifiers().shift, this.md_sel) { (true, Some((a, _))) => Some((a, i)), _ => Some((i, i)) }; cx.notify();
                    }))
                    .child(markdown_view(std::slice::from_ref(&b.block), &p, mono))
                    .when(n > 0, |d| d.child(div().absolute().top(px(2.0)).right(px(4.0)).flex().items_center().gap_1().text_xs().text_color(p.accent).child(icon(ic::COMMENT, 14.0, p.accent)).child(format!("{n}"))))
            }).collect();
            div().id("md").flex_1().min_h_0().overflow_y_scroll().p_6().pb(px(90.0)).child(div().max_w(px(880.0)).flex().flex_col().gap_2()
                .child(div().text_xs().text_color(p.muted).pb_2().child("Rendered preview. Click a block to comment on it; Shift+click extends the selection.")).children(items)).into_any_element()
        } else if self.rows.is_empty() {
            div().flex_1().flex().items_center().justify_center().text_color(p.muted).child(if self.status.is_empty() { "Nothing to show for this selection.".to_string() } else { self.status.clone() }).into_any_element()
        } else {
            let mono = mono.clone();
            uniform_list("diff", self.rows.len(), cx.processor(move |this, range: Range<usize>, _w, cx| {
                let p = this.pal; let h = px(20.0 * this.zoom); let fsz = px(13.0 * this.zoom);
                range.map(|i| {
                    let r = &this.rows[i]; let fi = r.fi; let st = this.m.vm.files[fi].status;
                    let selected = this.sel.map(|(a, b)| i >= a && i <= b).unwrap_or(false) && this.rows.get(this.sel.unwrap().0).map(|x| x.fi) == Some(fi);
                    let base = div().id(i).flex().items_center().h(h).w_full().font_family(mono.clone()).text_size(fsz).text_color(p.fg);
                    let cell = |side: Option<&RLine>, left: bool| -> gpui::Div {
                        let Some(rl) = side else { return div().flex_1().h_full().bg(if selected { p.accent.opacity(0.22) } else { p.hover.opacity(0.4) }) };
                        let l = &rl.line;
                        let (bg, sign, gut) = match l.kind { Kind::Added => (p.added, "+", p.added_word.opacity(0.55)), Kind::Removed => (p.removed, "−", p.removed_word.opacity(0.55)), Kind::Context => (gpui::transparent_black(), " ", gpui::transparent_black()) };
                        let no = if left { l.old_no } else { l.new_no };
                        div().flex().items_center().h_full().flex_1().min_w_0().overflow_hidden().bg(if selected { p.accent.opacity(0.28) } else { bg })
                            .child(div().w(px(46.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(no.map(|n| n.to_string()).unwrap_or_default()))
                            .child(div().w(px(20.0)).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(sign))
                            .child(div().flex_1().min_w_0().whitespace_nowrap().child(StyledText::new(SharedString::from(l.text.clone())).with_highlights(rl.runs.clone())))
                    };
                    let inner = match &r.kind {
                        RowKind::FileHeader => base.bg(p.surface).border_t_1().border_color(p.border).px_3().gap_2().font_weight(FontWeight::SEMIBOLD).child(this.m.vm.files[fi].path.clone()),
                        RowKind::Note(t) => base.bg(p.accent.opacity(0.10)).px_3().gap_2().text_color(p.fg).child(icon(ic::LINK, 15.0, p.accent)).child(t.clone()),
                        RowKind::Hunk(t) => base.bg(p.accent.opacity(0.10)).text_color(p.muted).text_xs().child(div().w(px(70.0)).flex_none()).child(t.clone()),
                        RowKind::Uni(rl) => {
                            let l = &rl.line;
                            let bg = match l.kind { Kind::Added => p.added, Kind::Removed => p.removed, Kind::Context => gpui::transparent_black() };
                            let gut = match l.kind { Kind::Added => p.added_word.opacity(0.55), Kind::Removed => p.removed_word.opacity(0.55), Kind::Context => gpui::transparent_black() };
                            let (show_old, show_new) = (st != Status::Added, st != Status::Deleted);
                            base.bg(if selected { p.accent.opacity(0.28) } else { bg })
                                .when(show_old, |d| d.child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.old_no.map(|n| n.to_string()).unwrap_or_default())))
                                .when(show_new, |d| d.child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.new_no.map(|n| n.to_string()).unwrap_or_default())))
                                .child(div().w(px(22.0)).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(match l.kind { Kind::Added => "+", Kind::Removed => "−", _ => " " }))
                                .child(div().flex_1().min_w_0().whitespace_nowrap().child(StyledText::new(SharedString::from(l.text.clone())).with_highlights(rl.runs.clone())))
                        }
                        RowKind::Split(a, b) => base.child(cell(a.as_ref(), true)).child(div().w(px(1.0)).h_full().bg(p.border)).child(cell(b.as_ref(), false)),
                    };
                    let selectable = !matches!(r.kind, RowKind::FileHeader | RowKind::Hunk(_) | RowKind::Note(_));
                    inner.when(selected, |d| d.border_l_2().border_color(p.accent)).when(selectable, |d| d.cursor_pointer()
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| { this.row_hit(i, ev.modifiers.shift); cx.notify(); }))
                        .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                            if this.dragging && ev.pressed_button == Some(MouseButton::Left) { if let Some(a) = this.anchor { let s = Some((a.min(i), a.max(i))); if s != this.sel { this.sel = s; cx.notify(); } } }
                        })))
                }).collect()
            })).flex_1().min_h_0().w_full().pb(px(70.0)).into_any_element()
        };

        let sel = self.selection();
        let sel_bar = sel.clone().filter(|_| self.mode == Mode::Normal).map(|s| div().flex().items_center().gap_2().px_4().py(px(6.0)).bg(p.accent.opacity(0.12)).border_t_1().border_color(p.accent)
            .child(div().text_sm().flex_1().truncate().child(format!("{}  lines {}–{}", s.file, s.start, s.end)))
            .child(icon_text_btn("sel-comment", ic::ADD_COMMENT, "Comment", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.comment_global = false; this.mode = Mode::Comment; cx.notify(); })))
            .child(icon_text_btn("sel-ai", ic::AUTO_AWESOME, "Ask AI", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = this.selection(); this.tab = RightTab::Ai; this.right_open = true; this.mode = Mode::Ask; cx.notify(); })))
            .child(icon_btn("sel-clear", ic::CLOSE, "Clear selection (Esc)", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.clear_selection(); cx.notify(); }))));
        let target: SharedString = if self.comment_global { "Comment on the whole merge request".into() } else { match (&sel, first.as_ref()) { (Some(s), _) => format!("Comment on {}:{}–{}", s.file, s.start, s.end), (None, Some(f)) => format!("Comment on {} (select lines first to anchor it)", f.path), _ => "General comment".into() }.into() };
        let composing = self.mode == Mode::Comment;
        let composer = composing.then(|| div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.accent).bg(p.surface)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(target))
            .child(self.wire(Fid::Comment, div().id("cbox").flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(p.accent).bg(p.bg).child(self.text_of(Fid::Comment, true, "Write a comment. Enter saves a draft, Esc cancels.")), cx))
            .child(div().flex().gap_2().child(icon_text_btn("save-c", ic::CHECK, "Add draft", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.add_comment(); cx.notify(); }))).child(icon_text_btn("cancel-c", ic::CLOSE, "Cancel", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; this.comment_global = false; cx.notify(); })))));

        // floating action bar
        let sep = || div().w(px(1.0)).h(px(22.0)).bg(p.border).flex_none();
        let has_sel_bar = sel_bar.is_some();
        let bar = div().absolute().bottom(px(if composing { 230.0 } else if has_sel_bar { 66.0 } else { 18.0 })).left_0().right_0().flex().justify_center().child(
            div().flex().items_center().gap_2().px_3().py_2().rounded_lg().bg(p.surface).border_1().border_color(p.border).shadow_lg()
                .child(icon_btn("prev", ic::CHEVRON_LEFT, "Previous batch (Backspace)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.back(); cx.notify(); })))
                .child(icon_btn("next", ic::CHEVRON_RIGHT, "Mark viewed and go to the next batch (Enter)", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.next(); cx.notify(); })))
                .child(sep())
                .child(icon_btn("gcomment", ic::ADD_COMMENT, "Comment on the whole merge request", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.comment_global = true; this.mode = Mode::Comment; cx.notify(); })))
                .child(sep())
                .when(!md_on, |d| d.child(seg_box(&p).child(seg_icon("df-uni", ic::VIEW_AGENDA, "Unified diff (u)", mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Unified); cx.notify(); })))
                    .child(seg_icon("df-split", ic::VERTICAL_SPLIT, "Side-by-side diff (u)", mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Split); cx.notify(); })))))
                .when(is_md, |d| d.child(seg_box(&p).child(seg_icon("md-diff", ic::CODE, "Show the markdown source diff (m)", !md_on, &p).on_click(cx.listener(|this, _, _, cx| { if this.md_rendered { this.md_rendered = false; this.shown = None; this.clear_selection(); } cx.notify(); })))
                    .child(seg_icon("md-prev", ic::VISIBILITY, "Show the rendered markdown (m)", md_on, &p).on_click(cx.listener(|this, _, _, cx| { if !this.md_rendered { this.md_rendered = true; this.shown = None; this.clear_selection(); } cx.notify(); })))))
                .child(sep())
                .child(icon_text_btn("finish", ic::RATE_REVIEW, "Finish review", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Finish; this.submit_msg = None; cx.notify(); }))));

        div().relative().flex().flex_col().flex_1().min_w_0().h_full().children(header).child(body).children(sel_bar).children(composer).child(bar)
    }

    fn section_header(&self, id: &'static str, title: &'static str, count: Option<usize>, open: bool, cx: &mut Context<Self>, toggle: fn(&mut View)) -> gpui::Stateful<gpui::Div> {
        let p = self.pal;
        div().id(id).flex().items_center().gap_2().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface).cursor_pointer().hover(move |s| s.bg(p.hover))
            .child(icon(if open { ic::EXPAND_MORE } else { ic::CHEVRON_RIGHT }, 18.0, p.muted)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child(title)).children(count.map(|c| badge(format!("{c}"), p.muted, p.hover)))
            .on_click(cx.listener(move |this, _, _, cx| { toggle(this); cx.notify(); }))
    }

    fn right_panel(&mut self, cx: &mut Context<Self>, drafts: &[perusal_core::store::Draft], mono: &SharedString) -> impl IntoElement {
        let p = self.pal;
        let tabs = div().flex().items_center().border_b_1().border_color(p.border).px_2()
            .child(self.tab_btn("tab-review", "Review", ic::COMMIT, RightTab::Review, cx)).child(self.tab_btn("tab-ai", "Ask AI", ic::AUTO_AWESOME, RightTab::Ai, cx)).child(div().flex_1())
            .child(icon_btn("close-r", ic::CHEVRON_RIGHT, "Hide this panel", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.right_open = false; this.settings.right_open = false; this.save_settings(); cx.notify(); })));
        let body = if self.tab == RightTab::Review { self.review_tab(cx, drafts, mono).into_any_element() } else { self.ai_tab(cx).into_any_element() };
        div().flex().flex_col().size_full().child(tabs).child(body)
    }
    fn tab_btn(&self, id: &'static str, label: &'static str, i: ic::Icon, t: RightTab, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let on = self.tab == t;
        div().id(id).flex().items_center().gap_1().px_3().py_2().cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() })
            .child(icon(i, 16.0, if on { p.accent } else { p.muted })).child(label).on_click(cx.listener(move |this, _, _, cx| { this.tab = t; cx.notify(); }))
    }

    fn review_tab(&mut self, cx: &mut Context<Self>, drafts: &[perusal_core::store::Draft], mono: &SharedString) -> impl IntoElement {
        let p = self.pal;
        let commit_rows: Vec<_> = self.m.graph.rows.iter().enumerate().map(|(i, r)| {
            let sel = self.m.graph.selection.map(|(a, b)| i >= a && i <= b).unwrap_or(false) && self.m.vm.commit_scope.is_some(); let last = i + 1 == self.m.graph.rows.len();
            div().id(("c", i)).flex().gap_2().px_3().cursor_pointer().when(sel, |d| d.bg(p.accent.opacity(0.14))).hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| { if this.m.graph.rows.get(i).map(|r| r.in_mr).unwrap_or(false) { this.pick_commit(i); } cx.notify(); }))
                .child(div().flex().flex_col().items_center().w(px(12.0)).flex_none()
                    .child(div().h(px(8.0)).w(px(2.0)).bg(if i == 0 { gpui::transparent_black() } else { p.border }))
                    .child(div().size(px(10.0)).rounded_full().when(r.in_mr, |d| d.bg(p.accent)).when(!r.in_mr, |d| d.border_2().border_color(p.muted)))
                    .child(div().flex_1().w(px(2.0)).bg(if last { gpui::transparent_black() } else { p.border })))
                .child(div().flex().flex_col().min_w_0().flex_1().py_1().child(div().truncate().text_sm().text_color(if r.in_mr { p.fg } else { p.muted }).child(r.subject.clone()))
                    .child(div().font_family(mono.clone()).text_xs().text_color(p.muted).child(r.sha[..7.min(r.sha.len())].to_string())))
                .when(sel, |d| d.child(div().flex().items_center().child(icon(ic::CHECK, 15.0, p.accent))))
        }).collect();
        let draft_rows: Vec<_> = drafts.iter().enumerate().map(|(i, d)| {
            let id = d.id;
            let loc = match (&d.file, d.start_line, d.end_line) { (Some(f), Some(a), Some(b)) if a != b => format!("{}:{a}–{b}", f.rsplit('/').next().unwrap_or(f)), (Some(f), Some(a), _) => format!("{}:{a}", f.rsplit('/').next().unwrap_or(f)), (Some(f), None, _) => f.rsplit('/').next().unwrap_or(f).to_string(), _ => "Merge request".into() };
            div().id(("d", i)).flex().flex_col().gap_1().px_3().py_2().border_b_1().border_color(p.border)
                .child(div().flex().items_center().justify_between().child(div().flex().items_center().gap_2().child(div().text_xs().font_family(mono.clone()).text_color(p.muted).child(loc)).when(d.ai, |x| x.child(badge("AI", p.ai, p.ai.opacity(0.15)))))
                    .child(icon_btn(("dd", i), ic::DELETE, "Remove this draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(id); this.resync(); cx.notify(); }))))
                .child(div().text_sm().child(d.body.clone()))
        }).collect();
        let n = drafts.len();
        let (co, dro) = (self.commits_open, self.drafts_open);
        div().flex().flex_col().flex_1().min_h_0()
            .child(self.section_header("sec-commits", "Commits", Some(self.m.graph.rows.len()), co, cx, |v| v.commits_open = !v.commits_open))
            .when(co, |d| d.child(div().id("commits").max_h(px(260.0)).overflow_y_scroll().children(commit_rows)))
            .child(self.section_header("sec-drafts", "Drafts", Some(n), dro, cx, |v| v.drafts_open = !v.drafts_open))
            .when(dro, |d| d.child(div().id("drafts").flex_1().min_h_0().overflow_y_scroll()
                .when(n == 0, |d| d.child(div().p_4().text_sm().text_color(p.muted).child("No drafts yet. Select lines (click, drag, or Shift+click) and press c.")))
                .children(draft_rows)))
    }

    fn ai_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let ctx: Option<SharedString> = self.ai_ctx.as_ref().map(|s| format!("{}:{}–{}", s.file, s.start, s.end).into());
        let msgs: Vec<_> = self.chat.iter().enumerate().map(|(i, (role, text))| {
            let user = role == "user"; let t = text.clone();
            div().flex().flex_col().gap_1().px_3().py_2().border_b_1().border_color(p.border).when(user, |d| d.bg(p.hover.opacity(0.5)))
                .child(div().flex().items_center().justify_between().child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(if user { p.muted } else { p.ai }).child(if user { "You" } else { "Agent" }))
                    .when(!user, |d| d.child(icon_btn(("todraft", i), ic::ADD_COMMENT, "Save this answer as an AI draft", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.draft_from_answer(&t); cx.notify(); })))))
                .child(div().text_sm().child(text.clone()))
        }).collect();
        let focused = self.mode == Mode::Ask; let busy = self.busy();
        let agent = self.settings.agent_label();
        let mut cfg = vec![agent.to_string()];
        if !self.settings.model.is_empty() { cfg.push(self.settings.model.clone()); }
        if !self.settings.effort.is_empty() && supports_effort(&self.settings.agent) { cfg.push(self.settings.effort.clone()); }
        let working = self.ai_job.map(|j| {
            let label = match j { Job::Review => "Reviewing the merge request", Job::Chat => "Thinking" };
            let el = self.ai_started.map(ago).unwrap_or_default();
            let acts: Vec<_> = self.ai_activity.iter().rev().take(6).rev().enumerate().map(|(i, a)| div().flex().items_center().gap_2().text_xs().text_color(if i == 5 || i + 1 == self.ai_activity.len().min(6) { p.fg } else { p.muted }).child(icon(ic::CHEVRON_RIGHT, 13.0, p.muted)).child(div().truncate().child(a.clone()))).collect();
            div().flex().flex_col().gap_2().px_3().py_3().bg(p.accent.opacity(0.08)).border_b_1().border_color(p.accent)
                .child(div().flex().items_center().gap_3().child(loading_dots(&p)).child(div().flex_1().text_sm().font_weight(FontWeight::MEDIUM).text_color(p.accent).child(format!("{label}… {el}")))
                    .child(icon_btn("ai-stop", ic::STOP, "Stop the agent (Esc)", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.stop_ai(); cx.notify(); }))))
                .children(acts)
                .when(!self.ai_partial.trim().is_empty(), |d| d.child(div().text_sm().text_color(p.fg).max_h(px(160.0)).overflow_hidden().child(self.ai_partial.trim().to_string())))
        });
        div().flex().flex_col().flex_1().min_h_0()
            .child(div().flex().items_center().justify_between().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface)
                .child(div().flex().flex_col().min_w_0().child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("Ask {agent}"))).child(div().text_xs().text_color(p.muted).truncate().child(cfg.join(" · "))))
                .child(icon_btn("ai-settings", ic::TUNE, "Agent, model and effort settings", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Settings; cx.notify(); }))))
            .children(self.ai_notice.clone().map(|n| div().flex().items_start().gap_2().px_3().py_2().text_sm().text_color(p.danger).bg(p.danger.opacity(0.1)).border_b_1().border_color(p.danger).child(icon(ic::ERROR_OUTLINE, 17.0, p.danger)).child(div().flex_1().min_w_0().child(n))))
            .children(working)
            .child(div().id("chat").flex_1().min_h_0().overflow_y_scroll()
                .when(self.chat.is_empty() && !busy, |d| d.child(div().p_4().flex().flex_col().gap_2().text_sm().text_color(p.muted)
                    .child("Select lines in the diff and press a to ask about them, or ask a general question about this merge request.")
                    .child("Answers stay private to you. Nothing is posted to the host. Save one as a draft to keep it.")))
                .children(msgs))
            .child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border).bg(p.surface)
                .children(ctx.map(|c| div().flex().items_center().gap_2().child(badge("context", p.accent, p.accent.opacity(0.15))).child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).truncate().child(c))))
                .child(div().flex().items_center().gap_2()
                    .child(self.wire(Fid::Ask, div().id("ask-box").flex_1().min_w_0().flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).min_h(px(32.0)).child(self.text_of(Fid::Ask, focused, "Ask about the selection…")), cx))
                    .child(if busy { icon_btn("ask-send", ic::STOP, "Stop the agent", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.stop_ai(); cx.notify(); })) }
                        else { icon_btn("ask-send", ic::SEND, "Send (Enter)", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { let q = this.ask_f.text.trim().to_string(); if !q.is_empty() { this.ask_f.clear(); this.start_ai(Job::Chat, q, cx); } cx.notify(); })) }))
                .child(div().id("ask-ctx").text_xs().text_color(p.accent).cursor_pointer().child("Use the current selection as context").on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = this.selection(); cx.notify(); }))))
    }

    fn settings_overlay(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let row = |label: &'static str, hint: &'static str, ctl: gpui::AnyElement| div().flex().items_center().justify_between().gap_4().py_2()
            .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label)).child(div().text_xs().text_color(p.muted).child(hint))).child(ctl);
        let section = |t: &'static str| div().pt_4().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.muted).border_b_1().border_color(p.border).child(t.to_uppercase());
        let s = self.settings.clone();
        let themes = theme::available(Some(&model::themes_dir()));
        let chip = |id: (&'static str, usize), label: String, on: bool| div().id(id).px_3().py(px(4.0)).rounded_md().border_1().border_color(if on { p.accent } else { p.border }).bg(if on { p.accent.opacity(0.12) } else { gpui::transparent_black() }).cursor_pointer().text_sm().hover(move |x| x.bg(p.hover)).child(label);
        let theme_chips: Vec<_> = themes.iter().enumerate().map(|(i, n)| { let nn = n.clone(); chip(("th", i), n.clone(), &s.theme == n).on_click(cx.listener(move |this, _, _, cx| { this.settings.theme = nn.clone(); this.save_settings(); this.apply_theme(); cx.notify(); })) }).collect();
        let agent_chips: Vec<_> = AGENTS.iter().enumerate().map(|(i, (k, l))| { let kk = k.to_string(); chip(("ag", i), l.to_string(), s.agent == *k).on_click(cx.listener(move |this, _, _, cx| { this.settings.agent = kk.clone(); this.save_settings(); this.ai_notice = None; cx.notify(); })) }).collect();
        let mut model_opts: Vec<String> = vec![String::new()]; model_opts.extend(model_suggestions(&s.agent).iter().map(|m| m.to_string()));
        let model_chips: Vec<_> = model_opts.iter().enumerate().map(|(i, m)| { let mm = m.clone(); chip(("md", i), if m.is_empty() { "Default".into() } else { m.clone() }, s.model == *m).on_click(cx.listener(move |this, _, _, cx| { this.settings.model = mm.clone(); this.model_f.set(&mm); this.save_settings(); cx.notify(); })) }).collect();
        let effort_ok = supports_effort(&s.agent);
        let effort_chips: Vec<_> = [("", "Default"), ("low", "Low"), ("medium", "Medium"), ("high", "High")].iter().enumerate().map(|(i, (v, l))| { let vv = v.to_string(); chip(("ef", i), l.to_string(), s.effort == *v).on_click(cx.listener(move |this, _, _, cx| { this.settings.effort = vv.clone(); this.save_settings(); cx.notify(); })) }).collect();
        let pick = |id: &'static str, cur: &str, options: &[String], cx: &mut Context<Self>, set: fn(&mut Settings, String)| -> gpui::AnyElement {
            seg_box(&p).children(options.iter().enumerate().map(|(i, o)| { let on = cur == o; let oo = o.clone(); seg_cell((id, i), o.clone(), on, &p).on_click(cx.listener(move |this, _, _, cx| { set(&mut this.settings, oo.clone()); this.save_settings(); this.apply_theme(); cx.notify(); })) }).collect::<Vec<_>>()).into_any_element()
        };
        let editing = |id: Fid| self.mode == Mode::SettingsEdit(id);
        let ibox = |this: &Self, id: Fid, ph: &str, cx: &mut Context<Self>| this.wire(id, div().id(match id { Fid::Cmd => "i-cmd", Fid::Model => "i-model", Fid::RuleLabel => "i-label", _ => "i-pat" }).flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if editing(id) { p.accent } else { p.border }).bg(p.bg).child(this.text_of(id, editing(id), ph)), cx);
        let rules: Vec<_> = s.group_rules.iter().enumerate().map(|(i, r)| {
            let id = r.id.clone(); let id2 = r.id.clone();
            div().flex().items_center().gap_3().py(px(6.0)).border_b_1().border_color(p.border)
                .child(div().id(("rule-sw", i)).cursor_pointer().child(switch(r.enabled, &p)).on_click(cx.listener(move |this, _, _, cx| { if let Some(r) = this.settings.group_rules.iter_mut().find(|r| r.id == id) { r.enabled = !r.enabled; } this.set_rules(); cx.notify(); })))
                .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().child(r.label.clone())).child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).truncate().child(r.pattern.clone())))
                .when(!r.builtin, |d| d.child(icon_btn(("rule-del", i), ic::DELETE, "Delete this rule", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.settings.group_rules.retain(|r| r.id != id2); this.set_rules(); cx.notify(); }))))
        }).collect();
        let themes_dir = model::themes_dir();
        let example = "Read AGENTS.md in this folder, then create a warmer dark theme called warm.toml and validate it with `perusal theme check warm.toml`.";

        let scroll = div().id("settings-scroll").flex_1().min_h_0().overflow_y_scroll().px_5().pb_4().flex().flex_col()
            .child(section("Appearance"))
            .child(row("Theme", "Built-in themes plus valid files in your themes folder", div().flex().flex_wrap().gap_2().max_w(px(380.0)).justify_end().children(theme_chips).into_any_element()))
            .child(row("Follow system appearance", "Switch between a light and a dark theme with the OS", div().id("sw-os").cursor_pointer().child(switch(s.follow_os, &p)).on_click(cx.listener(|this, _, _, cx| { this.settings.follow_os = !this.settings.follow_os; this.save_settings(); cx.notify(); })).into_any_element()))
            .when(s.follow_os, |d| d
                .child(row("Light theme", "Used when the OS is light", pick("pl", &s.theme_light, &themes, cx, |st, v| st.theme_light = v)))
                .child(row("Dark theme", "Used when the OS is dark", pick("pd", &s.theme_dark, &themes, cx, |st, v| st.theme_dark = v))))
            .child(div().flex().flex_col().gap_2().p_3().mt_2().rounded_md().border_1().border_color(p.border).bg(p.bg)
                .child(div().flex().items_center().gap_2().child(icon(ic::PALETTE, 18.0, p.accent)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child("Custom themes"))
                    .child(icon_text_btn("open-themes", ic::FOLDER_OPEN, "Open themes folder", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { let d = model::ensure_theme_docs(); if let Err(e) = model::open_dir(&d) { this.ai_notice = Some(e); } cx.notify(); }))))
                .child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).child(themes_dir.display().to_string()))
                .child(div().text_sm().text_color(p.muted).child("Ask your AI agent to make one. Open an agent in the themes folder (for example run `claude` there). The folder already contains an AGENTS.md that explains the theme format, the colour rules and how to validate a file. Then ask, for example:"))
                .child(div().flex().items_center().gap_2().px_3().py_2().rounded_md().border_1().border_color(p.border).bg(p.surface)
                    .child(div().flex_1().min_w_0().text_sm().child(format!("\"{example}\"")))
                    .child(icon_btn("copy-prompt", ic::CONTENT_COPY, "Copy this prompt", Btn::Ghost, &p).on_click(cx.listener(move |_, _, _, cx| { cx.write_to_clipboard(gpui::ClipboardItem::new_string(example.to_string())); }))))
                .child(div().text_xs().text_color(p.muted).child("New valid theme files appear in the list above the next time you open Settings.")))
            .child(section("Review"))
            .child(row("Group related files", "Batch files that belong together using the rules below", div().id("sw-grp").cursor_pointer().child(switch(s.group_related, &p)).on_click(cx.listener(|this, _, _, cx| { this.settings.group_related = !this.settings.group_related; this.save_settings(); this.m.vm.grouping = this.settings.group_related; let _ = this.m.vm.refresh(); this.reload(); cx.notify(); })).into_any_element()))
            .child(div().flex().flex_col().p_3().rounded_md().border_1().border_color(p.border).bg(p.bg)
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Grouping rules"))
                .child(div().text_xs().text_color(p.muted).pb_2().child("Files that match the same rule with the same name (across folders) are reviewed together. The first matching rule wins; names like main, lib, index are ignored."))
                .children(rules)
                .child(div().flex().flex_col().gap_2().pt_3()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Add a rule"))
                    .child(ibox(self, Fid::RuleLabel, "Name, for example Docs with images", cx))
                    .child(ibox(self, Fid::RulePat, r"Pattern with a stem group, e.g. ^docs/(?P<stem>[^/.]+)\.(?:md|png)$", cx))
                    .children(self.rule_err.clone().map(|e| div().text_xs().text_color(p.danger).child(e)))
                    .child(div().flex().child(icon_text_btn("add-rule", ic::ADD, "Add rule", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.add_rule(); cx.notify(); }))))))
            .child(row("Files panel", "Flat list or a collapsible directory tree", seg_box(&p).child(seg_icon("fp-list", ic::VIEW_LIST, "Flat list", !s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { if this.tree { this.toggle_tree(); } cx.notify(); })))
                .child(seg_icon("fp-tree", ic::ACCOUNT_TREE, "Directory tree", s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { if !this.tree { this.toggle_tree(); } cx.notify(); }))).into_any_element()))
            .child(section("Diff"))
            .child(row("Default diff view", "How diffs open; switch per file with the floating bar", seg_box(&p).child(seg_icon("dm-uni", ic::VIEW_AGENDA, "Unified", s.diff_mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Unified; this.save_settings(); this.set_diff_mode(DiffMode::Unified); cx.notify(); })))
                .child(seg_icon("dm-split", ic::VERTICAL_SPLIT, "Side by side", s.diff_mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Split; this.save_settings(); this.set_diff_mode(DiffMode::Split); cx.notify(); }))).into_any_element()))
            .child(row("Markdown files", "Open .md files as a source diff or as rendered text", seg_box(&p).child(seg_icon("md-src", ic::CODE, "Source diff", s.markdown == MdMode::Source, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Source; this.save_settings(); cx.notify(); })))
                .child(seg_icon("md-rnd", ic::VISIBILITY, "Rendered", s.markdown == MdMode::Rendered, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Rendered; this.save_settings(); cx.notify(); }))).into_any_element()))
            .child(section("AI"))
            .child(row("Agent", "Runs the CLI you already have installed; no accounts or keys are stored here", div().flex().flex_wrap().gap_2().max_w(px(380.0)).justify_end().children(agent_chips).into_any_element()))
            .child(row("Model", "One-click choices, or type any model name the agent accepts", div().flex().flex_col().items_end().gap_2().max_w(px(380.0)).child(div().flex().flex_wrap().gap_2().justify_end().children(model_chips)).child(div().w(px(260.0)).child(ibox(self, Fid::Model, "Custom model, then Enter", cx))).into_any_element()))
            .child(row("Effort / variant", if effort_ok { "How hard the model thinks (Claude: thinking budget, Codex: reasoning effort, opencode: variant)" } else { "Not supported by this agent" }, div().flex().gap_2().when(!effort_ok, |d| d.opacity(0.4)).children(effort_chips).into_any_element()))
            .child(row("Custom command", "Optional. Program and arguments; the prompt is added as the last argument", div().w(px(300.0)).child(ibox(self, Fid::Cmd, "e.g. my-agent --read-only", cx)).into_any_element()));

        let card = div().id("settings-card").flex().flex_col().w(px(720.0)).h(px(780.0)).max_h(relative(0.92)).rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
            .child(div().flex().items_center().px_5().py_3().border_b_1().border_color(p.border).child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Settings")))
            .child(scroll)
            .child(div().flex().items_center().justify_end().gap_2().px_5().py_3().border_t_1().border_color(p.border).bg(p.surface)
                .child(div().flex_1().text_xs().text_color(p.muted).child("Changes are saved as you make them."))
                .child(icon_text_btn("set-done", ic::CHECK, "Done", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); }))));
        deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.55)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); }))
            .child(card)).with_priority(20)
    }

    fn finish_overlay(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let (nd, nai, unviewed, total) = self.finish_counts();
        let remote = self.m.mr.is_some();
        let gitlab = self.m.mr.as_ref().map(|m| m.kind == perusal_providers::Kind::GitLab).unwrap_or(false);
        let v = self.verdict.clone();
        let vchip = |id: &'static str, label: &'static str, on: bool, color: gpui::Hsla| div().id(id).flex().items_center().gap_2().px_3().h(px(32.0)).rounded_md().border_1().border_color(if on { color } else { p.border }).bg(if on { color.opacity(0.14) } else { gpui::transparent_black() }).cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).hover(move |x| x.bg(p.hover)).child(label);
        let summary_box = self.wire(Fid::Summary, div().id("sum-box").flex().items_center().px_3().py(px(8.0)).rounded_md().border_1().border_color(if self.mode == Mode::Finish { p.accent } else { p.border }).bg(p.bg).min_h(px(40.0)).child(self.text_of(Fid::Summary, self.mode == Mode::Finish, "Summary (optional): what should the author know?")), cx);
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
            .child(icon_text_btn("fin-cancel", ic::CLOSE, "Cancel", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); })))
            .child(if remote {
                if self.submit_busy { div().flex().items_center().gap_2().px_3().child(loading_dots(&p)).child(div().text_sm().text_color(p.accent).child("Posting…")).into_any_element() }
                else { icon_text_btn("fin-submit", ic::SEND, "Post review", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.start_submit(cx); cx.notify(); })).into_any_element() }
            } else { icon_text_btn("fin-export", ic::CONTENT_COPY, "Copy and save Markdown", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.export_local(cx); cx.notify(); })).into_any_element() });
        let card = div().id("finish-card").flex().flex_col().w(px(560.0)).rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
            .child(div().flex().items_center().gap_2().px_5().py_3().border_b_1().border_color(p.border).child(icon(ic::RATE_REVIEW, 20.0, p.accent)).child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Finish review")))
            .child(body).child(footer);
        deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.55)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); }))
            .child(card)).with_priority(20)
    }
}
