use crate::kit::{badge, button, col, field_key, field_text, input_box, kbd, markdown_view, pill, seg_box, seg_cell, switch, Btn, Fonts, Pal};
use crate::model::{self, Model};
use gpui::{deferred, div, prelude::*, px, relative, uniform_list, ClickEvent, Context, FontWeight, HighlightStyle, KeyDownEvent, MouseButton, SharedString, StyledText};
use perusal_agents::{chat_prompt, launch_for, on_path, parse_review, review_prompt, run_headless, safety::minimal_env, Selection, REVIEW_SKILL};
use perusal_app::markdown::{self, Block};
use perusal_app::settings::{DiffMode, MdMode, Settings, AGENTS};
use perusal_app::textfield::TextField;
use perusal_app::theme::{self, Theme};
use perusal_app::tree::{self, TreeRow};
use perusal_diff::text::{diff_lines, highlight, split_pairs, Class, Kind, Line};
use perusal_diff::Status;
use perusal_filter::Suggestion;
use std::collections::{BTreeSet, HashSet};
use std::ops::Range;

#[derive(PartialEq, Clone, Copy)]
enum Mode { Normal, Filter, Comment, Ask, Settings, SettingsCmd }
#[derive(PartialEq, Clone, Copy)]
enum RightTab { Review, Ai }
enum Job { Chat, Review }

#[derive(Clone)]
struct RLine { line: Line, runs: Vec<(Range<usize>, HighlightStyle)> }
#[derive(Clone)]
enum RowKind { FileHeader, Hunk(String), Uni(RLine), Split(Option<RLine>, Option<RLine>) }
struct Row { fi: usize, kind: RowKind }

pub struct Back;
impl gpui::EventEmitter<Back> for View {}

pub struct View {
    m: Model, settings: Settings, theme: Theme, pal: Pal, fonts: Fonts, mode: Mode,
    filter_f: TextField, comment_f: TextField, ask_f: TextField, cmd_f: TextField, error: Option<String>, zoom: f32,
    shown: Option<Vec<usize>>, nav_changed: bool, rows: Vec<Row>, md: Option<Vec<Block>>, md_rendered: bool, diff_mode: DiffMode,
    sel: Option<(usize, usize)>, anchor: Option<usize>, dragging: bool, status: String,
    tree: bool, collapsed: HashSet<String>, trows: Vec<TreeRow>, multi: BTreeSet<usize>, last_clicked: Option<usize>,
    sug: Vec<Suggestion>, sug_replace: usize, sug_idx: Option<usize>,
    tab: RightTab, chat: Vec<(String, String)>, ai_ctx: Option<Selection>, ai_busy: Option<String>, ai_notice: Option<String>,
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
        m.vm.dir_order = settings.files_tree; m.vm.grouping = settings.group_related; let _ = m.vm.refresh();
        let chat = m.store.messages(m.vm.review).unwrap_or_default();
        View { pal: Pal::new(&theme), theme, fonts, mode: Mode::Normal, filter_f: filter, comment_f: TextField::default(), ask_f: TextField::default(), cmd_f: TextField::new(&settings.agent_command), error: None, zoom: 1.0,
            shown: None, nav_changed: true, rows: vec![], md: None, md_rendered: false, diff_mode: settings.diff_mode, sel: None, anchor: None, dragging: false, status: String::new(),
            tree: settings.files_tree, collapsed: HashSet::new(), trows: vec![], multi: BTreeSet::new(), last_clicked: None, sug: vec![], sug_replace: 0, sug_idx: None,
            tab: RightTab::Review, chat, ai_ctx: None, ai_busy: None, ai_notice: None, settings, m }
    }
    fn current_file(&self) -> Option<usize> { self.m.vm.current_batch().first().copied() }
    fn save_settings(&self) { model::save_settings(&self.settings); }
    fn apply_theme(&mut self) { self.theme = theme::load(&self.settings.theme, Some(&model::themes_dir())); self.pal = Pal::new(&self.theme); self.shown = None; }
    fn reload(&mut self) {
        self.shown = None; self.nav_changed = true; self.sel = None; self.anchor = None;
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

    // ---------- diff rows
    fn load_diff(&mut self) {
        let batch = self.m.vm.current_batch().to_vec();
        if batch.is_empty() { self.rows.clear(); self.md = None; self.shown = None; return; }
        if self.shown.as_ref() == Some(&batch) { return; }
        if self.nav_changed { self.md_rendered = self.settings.markdown == MdMode::Rendered && batch.len() == 1 && markdown::is_markdown(&self.m.vm.files[batch[0]].path); self.nav_changed = false; }
        self.shown = Some(batch.clone());
        self.status.clear(); self.md = None;
        let (th, p) = (self.theme.clone(), self.pal);
        let mut rows = Vec::new();
        for &fi in &batch {
            let f = self.m.vm.files[fi].clone();
            let old = perusal_gitops::show(&self.m.repo, &self.m.base_sha, &f.path).unwrap_or_default();
            let new = perusal_gitops::show(&self.m.repo, &self.m.branch, &f.path).unwrap_or_default();
            if batch.len() == 1 && self.md_rendered { self.md = Some(markdown::parse(if new.is_empty() { &old } else { &new })); return; }
            if batch.len() > 1 { rows.push(Row { fi, kind: RowKind::FileHeader }); }
            if f.noisy() { self.status = format!("{} is collapsed because it is generated, binary or very large.", f.path); continue; }
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
            if self.diff_mode == DiffMode::Unified {
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

    /// Selected rows as a code selection (single file), for comments and AI.
    fn selection(&self) -> Option<Selection> {
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

    // ---------- actions
    fn apply_filter(&mut self) {
        match self.m.vm.set_filter(self.filter_f.text.trim()) { Ok(()) => { self.error = None; self.mode = Mode::Normal; self.sug.clear(); self.reload(); } Err(e) => self.error = Some(e) }
    }
    fn update_suggestions(&mut self) { let (s, n) = perusal_filter::suggest(self.filter_f.before_caret()); self.sug = s; self.sug_replace = n; self.sug_idx = None; }
    fn accept_suggestion(&mut self, i: usize) {
        if let Some(s) = self.sug.get(i).cloned() { self.filter_f.replace_before_caret(self.sug_replace, &s.insert); self.update_suggestions(); }
    }
    fn add_comment(&mut self) {
        let text = self.comment_f.text.trim().to_string(); if text.is_empty() { return; }
        let sel = self.selection();
        match (sel, self.current_file()) {
            (Some(s), _) => { let _ = self.m.store.add_draft(self.m.vm.review, Some(&s.file), Some((s.start, s.end)), &text, false); self.status = format!("Draft added on {}:{}-{}", s.file, s.start, s.end); }
            (None, Some(fi)) => { let path = self.m.vm.files[fi].path.clone(); let _ = self.m.store.add_draft(self.m.vm.review, Some(&path), None, &text, false); self.status = format!("Draft added on {path}"); }
            _ => { let _ = self.m.store.add_draft(self.m.vm.review, None, None, &text, false); }
        }
        self.comment_f.clear(); self.mode = Mode::Normal;
    }
    fn toggle_viewed(&mut self) { if let Some(fi) = self.current_file() { let v = !self.m.vm.files[fi].viewed; for i in self.m.vm.current_batch().to_vec() { self.m.vm.set_viewed(&self.m.store, i, v); } } }
    fn next(&mut self) { self.m.vm.next(&self.m.store); self.reload(); }
    fn back(&mut self) { self.m.vm.back(); self.reload(); }
    fn step(&mut self, d: i32) { let c = self.m.vm.cursor as i32 + d; if c >= 0 && (c as usize) < self.m.vm.batch_count() { self.m.vm.cursor = c as usize; self.reload(); } }
    fn clear_scope(&mut self) { self.m.graph.clear(); self.m.vm.commit_scope = None; let _ = self.m.vm.refresh(); self.reload(); }
    fn set_diff_mode(&mut self, m: DiffMode) { if self.diff_mode != m { self.diff_mode = m; self.shown = None; self.sel = None; } }
    fn click_file(&mut self, fi: usize, mods: gpui::Modifiers) {
        if mods.control || mods.platform {
            if !self.multi.remove(&fi) { if self.multi.is_empty() { if let Some(c) = self.current_file() { if c != fi { self.multi.insert(c); } } } self.multi.insert(fi); }
            self.last_clicked = Some(fi);
        } else if mods.shift {
            let order = self.display_order();
            let from = self.last_clicked.or_else(|| self.current_file()).unwrap_or(fi);
            if let (Some(a), Some(b)) = (order.iter().position(|x| *x == from), order.iter().position(|x| *x == fi)) { for x in &order[a.min(b)..=a.max(b)] { self.multi.insert(*x); } }
        } else { self.multi.clear(); self.last_clicked = Some(fi); self.m.vm.select_file(fi); self.reload(); }
    }
    fn group_multi(&mut self) {
        let order = self.display_order(); let mut members: Vec<usize> = order.into_iter().filter(|i| self.multi.contains(i)).collect();
        if members.len() < 2 { return; }
        members.dedup(); self.m.vm.group_files_manually(members); self.multi.clear(); self.reload();
        self.status = "Grouped into one batch. Viewing it shows every file together.".into();
    }

    // ---------- AI
    fn start_ai(&mut self, job: Job, question: String, cx: &mut Context<Self>) {
        if self.ai_busy.is_some() { return; }
        let probe = launch_for(&self.settings.agent, &self.settings.agent_command, "x");
        let Some(probe) = probe else { self.ai_notice = Some("No agent configured. Choose one in Settings.".into()); return };
        if !on_path(&probe.program) { self.ai_notice = Some(format!("`{}` was not found on your PATH. Install it, pick another agent in Settings, or set a custom command.", probe.program)); return; }
        self.ai_notice = None;
        let sel = self.ai_ctx.clone();
        let (repo, rev, base, review) = (self.m.repo.clone(), self.m.branch.clone(), self.m.base_sha.clone(), self.m.vm.review);
        let (agent, custom) = (self.settings.agent.clone(), self.settings.agent_command.clone());
        let changed: Vec<String> = self.m.vm.files.iter().map(|f| f.path.clone()).collect();
        let prompt_chat = chat_prompt(sel.as_ref(), &self.chat, &question);
        if let Job::Chat = job { self.chat.push(("user".into(), question.clone())); let _ = self.m.store.add_message(review, "user", &question); }
        self.ai_busy = Some(match job { Job::Chat => "Asking the agent...".into(), Job::Review => "Running an AI review of this merge request. This can take a few minutes...".into() });
        let is_review = matches!(job, Job::Review);
        let task = cx.background_executor().spawn(async move {
            let dest = model::snapshot_dir(review, &rev);
            if !dest.join(".perusal-snapshot").exists() { perusal_gitops::export_tree(&repo, &rev, &dest)?; let _ = std::fs::write(dest.join(".perusal-snapshot"), "1"); }
            let prompt = if is_review { let patch = perusal_gitops::patch(&repo, &base, &rev)?; review_prompt(REVIEW_SKILL, &changed, &patch) } else { prompt_chat };
            let l = launch_for(&agent, &custom, &prompt).ok_or("no agent configured")?;
            let env = minimal_env(&std::env::vars().collect());
            run_headless(&l, &dest, &env, std::time::Duration::from_secs(900)).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| { let r: Result<String, String> = task.await; let _ = this.update(cx, |t, cx| t.finish_ai(job, r, cx)); }).detach();
        cx.notify();
    }
    fn finish_ai(&mut self, job: Job, r: Result<String, String>, cx: &mut Context<Self>) {
        self.ai_busy = None;
        let review = self.m.vm.review;
        match (job, r) {
            (Job::Chat, Ok(text)) => { let t = text.trim().to_string(); self.chat.push(("agent".into(), t.clone())); let _ = self.m.store.add_message(review, "agent", &t); }
            (Job::Review, Ok(text)) => match parse_review(&text) {
                Ok(cs) => {
                    for c in &cs { let _ = self.m.store.add_draft(review, Some(&c.file), Some((c.start_line, c.end_line)), &perusal_agents::draft_body(c), true); }
                    let msg = if cs.is_empty() { "AI review finished with nothing to flag.".to_string() } else { format!("AI review added {} draft comment{}, tagged AI. Review, edit or remove them in Drafts.", cs.len(), if cs.len() == 1 { "" } else { "s" }) };
                    self.chat.push(("agent".into(), msg.clone())); let _ = self.m.store.add_message(review, "agent", &msg); self.tab = RightTab::Review;
                }
                Err(e) => self.ai_notice = Some(format!("The agent's answer was not a usable review ({e}). First 300 characters: {}", text.chars().take(300).collect::<String>())),
            },
            (_, Err(e)) => self.ai_notice = Some(e),
        }
        cx.notify();
    }
    fn draft_from_answer(&mut self, text: &str) {
        let (f, l) = match &self.ai_ctx { Some(s) => (Some(s.file.clone()), Some((s.start, s.end))), None => (self.current_file().map(|i| self.m.vm.files[i].path.clone()), None) };
        let _ = self.m.store.add_draft(self.m.vm.review, f.as_deref(), l, text, true); self.status = "Answer saved as an AI draft.".into();
    }

    // ---------- keys
    pub fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke; let key = ks.key.as_str();
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        match self.mode {
            Mode::Settings => { if key == "escape" { self.mode = Mode::Normal; cx.notify(); } return; }
            Mode::SettingsCmd => {
                if key == "escape" || key == "enter" { self.settings.agent_command = self.cmd_f.text.trim().to_string(); self.save_settings(); self.mode = Mode::Settings; }
                else { field_key(&mut self.cmd_f, ks, cx); }
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
                if key == "escape" { self.mode = Mode::Normal; } else if key == "enter" { self.add_comment(); } else { field_key(&mut self.comment_f, ks, cx); }
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
            "escape" => { if self.sel.is_some() || !self.multi.is_empty() { self.sel = None; self.anchor = None; self.multi.clear(); } else { cx.emit(Back); return; } }
            "/" => { self.mode = Mode::Filter; self.update_suggestions(); }
            "c" => self.mode = Mode::Comment,
            "a" => { self.ai_ctx = self.selection(); self.tab = RightTab::Ai; self.mode = Mode::Ask; }
            "s" => self.mode = Mode::Settings,
            "m" => { if let Some(fi) = self.current_file() { if markdown::is_markdown(&self.m.vm.files[fi].path) { self.md_rendered = !self.md_rendered; self.shown = None; } } }
            "u" => { let m = if self.diff_mode == DiffMode::Unified { DiffMode::Split } else { DiffMode::Unified }; self.set_diff_mode(m); }
            "v" => self.toggle_viewed(), "enter" => self.next(), "backspace" => self.back(),
            "t" => self.toggle_tree(), "n" => self.step(1), "p" => self.step(-1), "x" => self.clear_scope(),
            "g" => { self.settings.group_related = !self.settings.group_related; self.save_settings(); self.m.vm.grouping = self.settings.group_related; let _ = self.m.vm.refresh(); self.reload(); }
            _ => {}
        }
        cx.notify();
    }
}

impl Render for View {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settings.follow_os {
            let dark = matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark);
            let want = self.settings.effective_theme(dark).to_string();
            if self.theme.name != theme::load(&want, Some(&model::themes_dir())).name { let s = self.settings.theme.clone(); self.settings.theme = want; self.apply_theme(); self.settings.theme = s; }
        }
        self.load_diff();
        if self.tree { self.trows = tree::build_rows(&self.m.vm.files, &self.m.vm.visible, &self.collapsed); }
        let p = self.pal; let z = self.zoom;
        let mono = self.fonts.mono.clone();
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
        let files = self.files_panel(cx);
        let center = self.center(cx, &mono);
        let right = self.right_panel(cx, &drafts, &mono);

        let footer = div().flex().items_center().gap_4().px_4().py(px(6.0)).bg(p.surface).border_t_1().border_color(p.border).text_xs().text_color(p.muted)
            .children([("n / p", "file"), ("v", "viewed"), ("Enter", "next"), ("/", "filter"), ("c", "comment"), ("a", "ask AI"), ("u", "split"), ("m", "markdown"), ("t", "tree"), ("g", "group"), ("s", "settings"), ("Esc", "home")]
                .map(|(k, l)| div().flex().items_center().gap_1().child(kbd(k, &p)).child(l)));
        let overlay = (self.mode == Mode::Settings || self.mode == Mode::SettingsCmd).then(|| self.settings_overlay(cx));

        div().relative().size_full().flex().flex_col().bg(p.bg).text_color(p.fg).text_size(px(14.0 * z))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| { this.dragging = false; }))
            .child(top).child(progress).child(filter)
            .child(div().flex().flex_1().min_h_0()
                .child(div().w(px(380.0 * z)).h_full().flex().flex_col().flex_none().bg(p.surface).border_r_1().border_color(p.border).child(files))
                .child(center)
                .child(div().w(px(340.0 * z)).h_full().flex().flex_col().flex_none().bg(p.surface).border_l_1().border_color(p.border).child(right)))
            .child(footer)
            .children(overlay)
    }
}

impl View {
    fn top_bar(&mut self, cx: &mut Context<Self>, ndrafts: usize) -> impl IntoElement {
        let p = self.pal;
        div().flex().items_center().gap_3().px_4().h(px(52.0)).bg(p.surface).border_b_1().border_color(p.border)
            .child(button("home", "‹  Home", Btn::Ghost, &p).on_click(cx.listener(|_, _, _, cx| cx.emit(Back))))
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).truncate().min_w_0().child(SharedString::from(self.m.title.clone())))
            .children(self.m.remote.clone().map(|u| pill(u.trim_start_matches("https://").to_string(), &p)))
            .child(div().flex_1())
            .when(ndrafts > 0, |d| d.child(badge(format!("{ndrafts} draft{}", if ndrafts == 1 { "" } else { "s" }), p.accent, p.accent.opacity(0.15))))
            .child(button("ai-review", "AI review", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.tab = RightTab::Ai; this.start_ai(Job::Review, String::new(), cx); cx.notify(); })))
            .child(button("settings", "Settings", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Settings; cx.notify(); })))
            .child(button("prev", "‹ Back", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.back(); cx.notify(); })))
            .child(button("next", "Next ›", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.next(); cx.notify(); })))
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
        div().flex().flex_col().w_full().px_4().py_2().bg(p.bg).border_b_1().border_color(p.border).gap_1()
            .child(div().relative().w_full()
                .child(div().id("filter").flex().items_center().gap_2().px_3().py(px(6.0)).w_full().rounded_md().border_1().border_color(if focused { p.accent } else if bad { p.danger } else { p.border }).bg(p.surface).cursor_text()
                    .on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Filter; this.update_suggestions(); cx.notify(); }))
                    .child(div().text_color(p.muted).child("⌕"))
                    .child(div().flex_1().min_w_0().overflow_hidden().text_sm().font_family(mono.clone()).child(field_text(&self.filter_f, focused, "Filter files: additions > 50 && !viewed   (press / to focus)", &p)))
                    .when(!self.filter_f.is_empty(), |d| d.child(div().id("clr").text_xs().text_color(p.muted).cursor_pointer().child("Clear").on_click(cx.listener(|this, _, _, cx| { this.filter_f.clear(); this.apply_filter(); cx.notify(); })))))
                .children(dropdown))
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
                            .child(div().w(px(12.0)).flex_none().text_xs().text_color(p.muted).child(if collapsed { "▸" } else { "▾" }))
                            .child(div().min_w_0().flex_1().truncate().font_weight(FontWeight::SEMIBOLD).child(format!("{name}/")))
                            .child(div().text_xs().flex_none().flex().gap_1().child(div().text_color(p.success).child(format!("+{additions}"))).child(div().text_color(p.danger).child(format!("−{deletions}"))))
                            .child(div().flex_none().px_2().rounded_full().text_xs().font_weight(FontWeight::MEDIUM).text_color(if done { p.success } else { p.muted }).bg(if done { p.success.opacity(0.15) } else { p.hover }).child(format!("{viewed}/{total}")));
                    }
                }
                let (fi, depth) = if this.tree { match this.trows[row] { TreeRow::File { fi, depth } => (fi, depth), _ => unreachable!() } } else { (this.m.vm.visible[row], 0) };
                let f = &this.m.vm.files[fi];
                let sel = this.m.vm.current_batch().contains(&fi); let picked = this.multi.contains(&fi); let grouped = this.m.vm.is_manually_grouped(fi);
                let (letter, c) = status_style(f.status, &p);
                let (dir, name) = if this.tree { (String::new(), f.name().to_string()) } else { match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) } };
                div().id(row).flex().items_center().gap_2().pl(px(12.0 + if this.tree { 14.0 * depth as f32 + 14.0 } else { 0.0 })).pr_3().h(rh).w_full().cursor_pointer()
                    .border_l_2().border_color(if sel { p.accent } else if picked { p.warn } else { gpui::transparent_black() })
                    .when(sel, |d| d.bg(p.accent.opacity(0.12))).when(picked, |d| d.bg(p.warn.opacity(0.14))).hover(move |s| s.bg(p.hover))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| { this.click_file(fi, ev.modifiers()); cx.notify(); }))
                    .child(div().size(px(18.0)).rounded_sm().flex().items_center().justify_center().text_xs().font_weight(FontWeight::BOLD).text_color(c).bg(c.opacity(0.15)).flex_none().child(letter))
                    .child(div().flex().flex_1().min_w_0().truncate().child(div().text_color(p.muted).child(dir)).child(div().text_color(if f.viewed { p.muted } else { p.fg }).font_weight(FontWeight::MEDIUM).child(name)))
                    .when(grouped, |d| d.child(badge("batch", p.accent, p.accent.opacity(0.15))))
                    .when(f.noisy(), |d| d.child(div().text_xs().text_color(p.muted).child("collapsed")))
                    .child(div().text_xs().flex_none().flex().gap_1().child(div().text_color(p.success).child(format!("+{}", f.additions))).child(div().text_color(p.danger).child(format!("−{}", f.deletions))))
                    .child(div().size(px(16.0)).flex_none().flex().items_center().justify_center().when(f.viewed, |d| d.rounded_full().bg(p.success).text_color(gpui::white()).text_xs().child("✓")))
            }).collect()
        })).flex_1().min_h_0().w_full();
        let tree_on = self.tree;
        let header = div().flex().items_center().gap_2().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Files")).child(badge(format!("{n_files}"), p.muted, p.hover)).child(div().flex_1())
            .when(tree_on, |d| d
                .child(div().id("exp-all").text_xs().text_color(p.accent).cursor_pointer().child("Expand all").on_click(cx.listener(|this, _, _, cx| { this.collapsed.clear(); cx.notify(); })))
                .child(div().id("col-all").text_xs().text_color(p.accent).cursor_pointer().child("Collapse all").on_click(cx.listener(|this, _, _, cx| { this.collapsed = tree::all_dirs(&this.m.vm.files, &this.m.vm.visible).into_iter().collect(); cx.notify(); }))))
            .child(seg_box(&p).child(seg_cell("seg-list", "List", !tree_on, &p).on_click(cx.listener(|this, _, _, cx| { if this.tree { this.toggle_tree(); } cx.notify(); })))
                .child(seg_cell("seg-tree", "Tree", tree_on, &p).on_click(cx.listener(|this, _, _, cx| { if !this.tree { this.toggle_tree(); } cx.notify(); }))));
        let n = self.multi.len();
        let bar = (n > 0).then(|| div().flex().items_center().gap_2().px_3().py(px(6.0)).bg(p.warn.opacity(0.12)).border_b_1().border_color(p.border)
            .child(div().text_sm().flex_1().child(format!("{n} file{} selected", if n == 1 { "" } else { "s" })))
            .child(button("grp-multi", "Review together", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.group_multi(); cx.notify(); })))
            .child(button("clr-multi", "Clear", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.multi.clear(); cx.notify(); }))));
        let hint = (n == 0 && n_files > 1).then(|| div().px_3().py(px(4.0)).text_xs().text_color(p.muted).border_b_1().border_color(p.border).child("Ctrl+click or Shift+click files to review them together"));
        div().flex().flex_col().size_full().child(header).children(bar).children(hint).child(list)
    }

    fn center(&mut self, cx: &mut Context<Self>, mono: &SharedString) -> impl IntoElement {
        let p = self.pal; let z = self.zoom;
        let batch = self.m.vm.current_batch().to_vec();
        let first = batch.first().map(|&i| self.m.vm.files[i].clone());
        let is_md = first.as_ref().map(|f| batch.len() == 1 && markdown::is_markdown(&f.path)).unwrap_or(false);
        let viewed_all = !batch.is_empty() && batch.iter().all(|&i| self.m.vm.files[i].viewed);
        let manual = batch.first().map(|&i| self.m.vm.is_manually_grouped(i)).unwrap_or(false);
        let (adds, dels): (u32, u32) = batch.iter().fold((0, 0), |a, &i| (a.0 + self.m.vm.files[i].additions, a.1 + self.m.vm.files[i].deletions));
        let md_on = self.md_rendered; let mode = self.diff_mode;

        let title = if batch.len() > 1 { div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Batch of {} files", batch.len()))).child(div().text_xs().text_color(p.muted).child(if manual { "grouped by you" } else { "related files" })) }
            else if let Some(f) = &first { let (dir, name) = match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) };
                div().flex().min_w_0().truncate().font_family(mono.clone()).text_sm().child(div().text_color(p.muted).child(dir)).child(div().font_weight(FontWeight::SEMIBOLD).child(name)) } else { div().child("No files match") };
        let header = (!batch.is_empty()).then(|| {
            let mut h = div().flex().items_center().gap_3().px_4().py_2().border_b_1().border_color(p.border).bg(p.surface)
                .child(div().flex_1().min_w_0().child(title))
                .child(div().flex().items_center().gap_2().text_xs().font_family(mono.clone()).child(div().text_color(p.success).child(format!("+{adds}"))).child(div().text_color(p.danger).child(format!("−{dels}"))));
            if is_md { h = h.child(seg_box(&p).child(seg_cell("md-diff", "Diff", !md_on, &p).on_click(cx.listener(|this, _, _, cx| { if this.md_rendered { this.md_rendered = false; this.shown = None; } cx.notify(); })))
                .child(seg_cell("md-prev", "Preview", md_on, &p).on_click(cx.listener(|this, _, _, cx| { if !this.md_rendered { this.md_rendered = true; this.shown = None; } cx.notify(); })))); }
            if !md_on { h = h.child(seg_box(&p).child(seg_cell("df-uni", "Unified", mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Unified); cx.notify(); })))
                .child(seg_cell("df-split", "Split", mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.set_diff_mode(DiffMode::Split); cx.notify(); })))); }
            if manual { h = h.child(button("ungroup", "Ungroup", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { if let Some(fi) = this.current_file() { this.m.vm.ungroup(fi); this.reload(); } cx.notify(); }))); }
            h.child(button("comment", "Comment", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Comment; cx.notify(); })))
                .child(div().id("viewed").flex().items_center().gap_2().px_3().py(px(5.0)).rounded_md().border_1().border_color(if viewed_all { p.success } else { p.border }).cursor_pointer().hover(move |s| s.bg(p.hover)).text_sm().font_weight(FontWeight::MEDIUM)
                    .on_click(cx.listener(|this, _, _, cx| { this.toggle_viewed(); cx.notify(); }))
                    .child(div().size(px(16.0)).rounded_sm().border_1().border_color(if viewed_all { p.success } else { p.muted }).flex().items_center().justify_center().when(viewed_all, |d| d.bg(p.success).text_color(gpui::white()).text_xs().child("✓")))
                    .child("Viewed"))
        });

        let body = if let Some(blocks) = &self.md {
            div().id("md").flex_1().min_h_0().overflow_y_scroll().p_6().child(div().max_w(px(880.0)).child(markdown_view(blocks, &p, mono))).into_any_element()
        } else if self.rows.is_empty() {
            div().flex_1().flex().items_center().justify_center().text_color(p.muted).child(if self.status.is_empty() { "Nothing to show for this selection.".to_string() } else { self.status.clone() }).into_any_element()
        } else {
            let mono = mono.clone();
            uniform_list("diff", self.rows.len(), cx.processor(move |this, range: Range<usize>, _w, cx| {
                let p = this.pal; let h = px(20.0 * this.zoom); let fsz = px(13.0 * this.zoom);
                range.map(|i| {
                    let r = &this.rows[i]; let fi = r.fi;
                    let selected = this.sel.map(|(a, b)| i >= a && i <= b).unwrap_or(false) && this.rows.get(this.sel.unwrap().0).map(|x| x.fi) == Some(fi);
                    let base = div().id(i).flex().items_center().h(h).w_full().font_family(mono.clone()).text_size(fsz).text_color(p.fg);
                    let cell = |side: Option<&RLine>, left: bool, _flex: bool| -> gpui::Div {
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
                        RowKind::FileHeader => base.bg(p.surface).border_t_1().border_color(p.border).px_3().font_weight(FontWeight::SEMIBOLD).child(this.m.vm.files[fi].path.clone()),
                        RowKind::Hunk(t) => base.bg(p.accent.opacity(0.10)).text_color(p.muted).text_xs().child(div().w(px(70.0)).flex_none()).child(t.clone()),
                        RowKind::Uni(rl) => {
                            let l = &rl.line;
                            let bg = match l.kind { Kind::Added => p.added, Kind::Removed => p.removed, Kind::Context => gpui::transparent_black() };
                            let gut = match l.kind { Kind::Added => p.added_word.opacity(0.55), Kind::Removed => p.removed_word.opacity(0.55), Kind::Context => gpui::transparent_black() };
                            base.bg(if selected { p.accent.opacity(0.28) } else { bg })
                                .child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.old_no.map(|n| n.to_string()).unwrap_or_default()))
                                .child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.new_no.map(|n| n.to_string()).unwrap_or_default()))
                                .child(div().w(px(22.0)).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(match l.kind { Kind::Added => "+", Kind::Removed => "−", _ => " " }))
                                .child(div().flex_1().min_w_0().whitespace_nowrap().child(StyledText::new(SharedString::from(l.text.clone())).with_highlights(rl.runs.clone())))
                        }
                        RowKind::Split(a, b) => base.child(cell(a.as_ref(), true, true)).child(div().w(px(1.0)).h_full().bg(p.border)).child(cell(b.as_ref(), false, true)),
                    };
                    let selectable = !matches!(r.kind, RowKind::FileHeader | RowKind::Hunk(_));
                    inner.when(selected, |d| d.border_l_2().border_color(p.accent)).when(selectable, |d| d.cursor_pointer()
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| { this.row_hit(i, ev.modifiers.shift); cx.notify(); }))
                        .on_mouse_move(cx.listener(move |this, ev: &gpui::MouseMoveEvent, _, cx| {
                            if this.dragging && ev.pressed_button == Some(MouseButton::Left) { if let Some(a) = this.anchor { let s = Some((a.min(i), a.max(i))); if s != this.sel { this.sel = s; cx.notify(); } } }
                        })))
                }).collect()
            })).flex_1().min_h_0().w_full().into_any_element()
        };

        // selection bar and composer
        let sel = self.selection();
        let sel_bar = sel.clone().filter(|_| self.mode == Mode::Normal).map(|s| div().flex().items_center().gap_2().px_4().py(px(6.0)).bg(p.accent.opacity(0.12)).border_t_1().border_color(p.accent)
            .child(div().text_sm().flex_1().truncate().child(format!("{}  lines {}–{}", s.file, s.start, s.end)))
            .child(button("sel-comment", "Comment", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Comment; cx.notify(); })))
            .child(button("sel-ai", "Ask AI", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = this.selection(); this.tab = RightTab::Ai; this.mode = Mode::Ask; cx.notify(); })))
            .child(button("sel-clear", "Clear", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.sel = None; this.anchor = None; cx.notify(); }))));
        let target: SharedString = match (&sel, first.as_ref()) { (Some(s), _) => format!("Comment on {}:{}–{}", s.file, s.start, s.end), (None, Some(f)) => format!("Comment on {} (select lines by clicking or dragging first)", f.path), _ => "General comment".into() }.into();
        let composer = (self.mode == Mode::Comment).then(|| div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.accent).bg(p.surface)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(target))
            .child(div().px_3().py(px(6.0)).rounded_md().border_1().border_color(p.accent).bg(p.bg).child(field_text(&self.comment_f, true, "Write a comment. Enter saves a draft, Esc cancels.", &p)))
            .child(div().flex().gap_2().child(button("save-c", "Add draft", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.add_comment(); cx.notify(); }))).child(button("cancel-c", "Cancel", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); })))));
        let _ = z;
        div().flex().flex_col().flex_1().min_w_0().h_full().children(header).child(body).children(sel_bar).children(composer)
    }

    fn right_panel(&mut self, cx: &mut Context<Self>, drafts: &[perusal_core::store::Draft], mono: &SharedString) -> impl IntoElement {
        let p = self.pal;
        let tabs = div().flex().border_b_1().border_color(p.border).px_2()
            .child(self.tab_btn("tab-review", "Review", RightTab::Review, cx)).child(self.tab_btn("tab-ai", "Ask AI", RightTab::Ai, cx));
        let body = if self.tab == RightTab::Review { self.review_tab(cx, drafts, mono).into_any_element() } else { self.ai_tab(cx).into_any_element() };
        div().flex().flex_col().size_full().child(tabs).child(body)
    }
    fn tab_btn(&self, id: &'static str, label: &'static str, t: RightTab, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let on = self.tab == t;
        div().id(id).px_3().py_2().cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() })
            .child(label).on_click(cx.listener(move |this, _, _, cx| { this.tab = t; cx.notify(); }))
    }

    fn review_tab(&mut self, cx: &mut Context<Self>, drafts: &[perusal_core::store::Draft], mono: &SharedString) -> impl IntoElement {
        let p = self.pal;
        let commit_rows: Vec<_> = self.m.graph.rows.iter().enumerate().map(|(i, r)| {
            let sel = self.m.graph.selection.map(|(a, b)| i >= a && i <= b).unwrap_or(false); let last = i + 1 == self.m.graph.rows.len();
            div().id(("c", i)).flex().gap_2().px_3().cursor_pointer().when(sel, |d| d.bg(p.accent.opacity(0.14))).hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| { this.m.graph.click(i); this.m.vm.commit_scope = this.m.graph.scope(&this.m.repo).ok().flatten(); let _ = this.m.vm.refresh(); this.reload(); cx.notify(); }))
                .child(div().flex().flex_col().items_center().w(px(12.0)).flex_none()
                    .child(div().h(px(8.0)).w(px(2.0)).bg(if i == 0 { gpui::transparent_black() } else { p.border }))
                    .child(div().size(px(10.0)).rounded_full().when(r.in_mr, |d| d.bg(p.accent)).when(!r.in_mr, |d| d.border_2().border_color(p.muted)))
                    .child(div().flex_1().w(px(2.0)).bg(if last { gpui::transparent_black() } else { p.border })))
                .child(div().flex().flex_col().min_w_0().flex_1().py_1().child(div().truncate().text_sm().text_color(if r.in_mr { p.fg } else { p.muted }).child(r.subject.clone()))
                    .child(div().font_family(mono.clone()).text_xs().text_color(p.muted).child(r.sha[..7.min(r.sha.len())].to_string())))
        }).collect();
        let draft_rows: Vec<_> = drafts.iter().enumerate().map(|(i, d)| {
            let id = d.id;
            let loc = match (&d.file, d.start_line, d.end_line) { (Some(f), Some(a), Some(b)) if a != b => format!("{}:{a}–{b}", f.rsplit('/').next().unwrap_or(f)), (Some(f), Some(a), _) => format!("{}:{a}", f.rsplit('/').next().unwrap_or(f)), (Some(f), None, _) => f.rsplit('/').next().unwrap_or(f).to_string(), _ => "General".into() };
            div().id(("d", i)).flex().flex_col().gap_1().px_3().py_2().border_b_1().border_color(p.border)
                .child(div().flex().items_center().justify_between().child(div().flex().items_center().gap_2().child(div().text_xs().font_family(mono.clone()).text_color(p.muted).child(loc)).when(d.ai, |x| x.child(badge("AI", p.ai, p.ai.opacity(0.15)))))
                    .child(div().id(("dd", i)).text_xs().text_color(p.muted).cursor_pointer().hover(move |s| s.text_color(p.danger)).child("Remove").on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(id); cx.notify(); }))))
                .child(div().text_sm().child(d.body.clone()))
        }).collect();
        let n = drafts.len();
        let sec = |t: &'static str, c: usize| div().flex().items_center().justify_between().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(t)).child(badge(format!("{c}"), p.muted, p.hover));
        div().flex().flex_col().flex_1().min_h_0()
            .child(sec("Commits", self.m.graph.rows.len())).child(div().id("commits").max_h(px(260.0)).overflow_y_scroll().children(commit_rows))
            .child(div().id("clear-scope").px_3().py_1().text_xs().text_color(p.accent).cursor_pointer().child("Show all commits").on_click(cx.listener(|this, _, _, cx| { this.clear_scope(); cx.notify(); })))
            .child(sec("Drafts", n)).child(div().id("drafts").flex_1().min_h_0().overflow_y_scroll()
                .when(n == 0, |d| d.child(div().p_4().text_sm().text_color(p.muted).child("No drafts yet. Select lines (click, drag, or Shift+click) and press c.")))
                .children(draft_rows))
    }

    fn ai_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let ctx: Option<SharedString> = self.ai_ctx.as_ref().map(|s| format!("{}:{}–{}", s.file, s.start, s.end).into());
        let msgs: Vec<_> = self.chat.iter().enumerate().map(|(i, (role, text))| {
            let user = role == "user"; let t = text.clone();
            div().flex().flex_col().gap_1().px_3().py_2().border_b_1().border_color(p.border).when(user, |d| d.bg(p.hover.opacity(0.5)))
                .child(div().flex().items_center().justify_between().child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(if user { p.muted } else { p.ai }).child(if user { "You" } else { "Agent" }))
                    .when(!user, |d| d.child(div().id(("todraft", i)).text_xs().text_color(p.accent).cursor_pointer().child("Add as draft").on_click(cx.listener(move |this, _, _, cx| { this.draft_from_answer(&t); cx.notify(); })))))
                .child(div().text_sm().child(text.clone()))
        }).collect();
        let focused = self.mode == Mode::Ask;
        let agent = self.settings.agent_label();
        let busy = self.ai_busy.clone();
        div().flex().flex_col().flex_1().min_h_0()
            .child(div().flex().items_center().justify_between().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface)
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("Ask {agent}")))
                .child(div().id("ai-settings").text_xs().text_color(p.accent).cursor_pointer().child("Change agent").on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Settings; cx.notify(); }))))
            .children(self.ai_notice.clone().map(|n| div().px_3().py_2().text_sm().text_color(p.danger).bg(p.danger.opacity(0.1)).border_b_1().border_color(p.danger).child(n)))
            .children(busy.map(|b| div().px_3().py_2().text_sm().text_color(p.accent).bg(p.accent.opacity(0.1)).child(b)))
            .child(div().id("chat").flex_1().min_h_0().overflow_y_scroll()
                .when(self.chat.is_empty() && self.ai_busy.is_none(), |d| d.child(div().p_4().flex().flex_col().gap_2().text_sm().text_color(p.muted)
                    .child("Select lines in the diff and press a to ask about them, or ask a general question about this merge request.")
                    .child("Answers stay private to you. Nothing is posted to the host. Use Add as draft to keep one.")))
                .children(msgs))
            .child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.border).bg(p.surface)
                .children(ctx.map(|c| div().flex().items_center().gap_2().child(badge("context", p.accent, p.accent.opacity(0.15))).child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).truncate().child(c))))
                .child(div().id("ask-box").px_3().py(px(6.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).cursor_text().min_h(px(32.0))
                    .on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Ask; if this.ai_ctx.is_none() { this.ai_ctx = this.selection(); } cx.notify(); }))
                    .child(field_text(&self.ask_f, focused, "Ask about the selection…", &p)))
                .child(div().flex().gap_2()
                    .child(button("ask-send", "Send", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { let q = this.ask_f.text.trim().to_string(); if !q.is_empty() { this.ask_f.clear(); this.start_ai(Job::Chat, q, cx); } cx.notify(); })))
                    .child(button("ask-ctx", "Use current selection", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.ai_ctx = this.selection(); cx.notify(); })))))
    }

    fn settings_overlay(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let row = |label: &'static str, hint: &'static str, ctl: gpui::AnyElement| div().flex().items_center().justify_between().gap_4().py_2()
            .child(div().flex().flex_col().min_w_0().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label)).child(div().text_xs().text_color(p.muted).child(hint))).child(ctl);
        let section = |t: &'static str| div().pt_3().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.muted).border_b_1().border_color(p.border).child(t.to_uppercase());
        let s = self.settings.clone();
        let themes = theme::available(Some(&model::themes_dir()));
        let theme_chips: Vec<_> = themes.iter().enumerate().map(|(i, n)| {
            let on = &s.theme == n; let nn = n.clone();
            div().id(("th", i)).px_3().py(px(4.0)).rounded_md().border_1().border_color(if on { p.accent } else { p.border }).bg(if on { p.accent.opacity(0.12) } else { gpui::transparent_black() }).cursor_pointer().text_sm().hover(move |x| x.bg(p.hover)).child(n.clone())
                .on_click(cx.listener(move |this, _, _, cx| { this.settings.theme = nn.clone(); this.save_settings(); this.apply_theme(); cx.notify(); }))
        }).collect();
        let agent_chips: Vec<_> = AGENTS.iter().enumerate().map(|(i, (k, l))| {
            let on = s.agent == *k; let kk = k.to_string();
            div().id(("ag", i)).px_3().py(px(4.0)).rounded_md().border_1().border_color(if on { p.accent } else { p.border }).bg(if on { p.accent.opacity(0.12) } else { gpui::transparent_black() }).cursor_pointer().text_sm().hover(move |x| x.bg(p.hover)).child(*l)
                .on_click(cx.listener(move |this, _, _, cx| { this.settings.agent = kk.clone(); this.save_settings(); this.ai_notice = None; cx.notify(); }))
        }).collect();
        let pick = |id: &'static str, cur: &str, options: &[String], cx: &mut Context<Self>, set: fn(&mut Settings, String)| -> gpui::AnyElement {
            seg_box(&p).children(options.iter().enumerate().map(|(i, o)| { let on = cur == o; let oo = o.clone(); seg_cell((id, i), o.clone(), on, &p).on_click(cx.listener(move |this, _, _, cx| { set(&mut this.settings, oo.clone()); this.save_settings(); this.apply_theme(); cx.notify(); })) }).collect::<Vec<_>>()).into_any_element()
        };
        let cmd_focus = self.mode == Mode::SettingsCmd;
        let card = div().id("settings-card").w(px(680.0)).max_h(px(760.0)).overflow_y_scroll().rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude()
            .child(div().flex().items_center().justify_between().px_5().py_3().border_b_1().border_color(p.border)
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Settings")).child(button("set-done", "Done", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); }))))
            .child(div().px_5().pb_4().flex().flex_col()
                .child(section("Appearance"))
                .child(row("Theme", "Built-in themes plus valid files in the themes folder", div().flex().flex_wrap().gap_2().max_w(px(400.0)).justify_end().children(theme_chips).into_any_element()))
                .child(row("Follow system appearance", "Switch between a light and a dark theme with the OS", div().id("sw-os").cursor_pointer().child(switch(s.follow_os, &p)).on_click(cx.listener(|this, _, _, cx| { this.settings.follow_os = !this.settings.follow_os; this.save_settings(); cx.notify(); })).into_any_element()))
                .when(s.follow_os, |d| d
                    .child(row("Light theme", "Used when the OS is light", pick("pl", &s.theme_light, &themes, cx, |st, v| st.theme_light = v)))
                    .child(row("Dark theme", "Used when the OS is dark", pick("pd", &s.theme_dark, &themes, cx, |st, v| st.theme_dark = v))))
                .child(section("Review"))
                .child(row("Group related files", "Batch foo.h with foo.cpp, foo.ts with foo.test.ts and so on", div().id("sw-grp").cursor_pointer().child(switch(s.group_related, &p)).on_click(cx.listener(|this, _, _, cx| { this.settings.group_related = !this.settings.group_related; this.save_settings(); this.m.vm.grouping = this.settings.group_related; let _ = this.m.vm.refresh(); this.reload(); cx.notify(); })).into_any_element()))
                .child(row("Files panel", "Flat list or a collapsible directory tree", seg_box(&p).child(seg_cell("fp-list", "List", !s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { if this.tree { this.toggle_tree(); } cx.notify(); })))
                    .child(seg_cell("fp-tree", "Tree", s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { if !this.tree { this.toggle_tree(); } cx.notify(); }))).into_any_element()))
                .child(section("Diff"))
                .child(row("Default diff view", "How diffs open; you can switch per file in the file header", seg_box(&p).child(seg_cell("dm-uni", "Unified", s.diff_mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Unified; this.save_settings(); this.set_diff_mode(DiffMode::Unified); cx.notify(); })))
                    .child(seg_cell("dm-split", "Split", s.diff_mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Split; this.save_settings(); this.set_diff_mode(DiffMode::Split); cx.notify(); }))).into_any_element()))
                .child(row("Markdown files", "Open .md files as rendered text or as a source diff", seg_box(&p).child(seg_cell("md-src", "Source diff", s.markdown == MdMode::Source, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Source; this.save_settings(); cx.notify(); })))
                    .child(seg_cell("md-rnd", "Rendered", s.markdown == MdMode::Rendered, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Rendered; this.save_settings(); cx.notify(); }))).into_any_element()))
                .child(section("AI"))
                .child(row("Agent", "Runs the CLI you already have installed; no accounts or keys are stored here", div().flex().flex_wrap().gap_2().max_w(px(400.0)).justify_end().children(agent_chips).into_any_element()))
                .child(row("Custom command", "Optional. Program and arguments; the prompt is added as the last argument", div().id("cmd-box").w(px(300.0)).cursor_text().on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::SettingsCmd; cx.notify(); }))
                    .child(input_box(&self.cmd_f, cmd_focus, "e.g. my-agent --read-only", &p, false)).into_any_element())));
        deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.55)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); }))
            .child(card)).with_priority(20)
    }
}
