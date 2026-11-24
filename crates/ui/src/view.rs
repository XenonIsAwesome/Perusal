use crate::kit::{badge, button, col, field_key, field_text, kbd, pill, Btn, Fonts, Pal};
use crate::model::Model;
use gpui::{div, prelude::*, px, relative, uniform_list, Context, FontWeight, HighlightStyle, KeyDownEvent, SharedString, StyledText};
use perusal_app::textfield::TextField;
use perusal_app::theme::{self, Theme};
use perusal_diff::text::{diff_lines, highlight, Class, Kind, Line};
use perusal_diff::Status;
use std::ops::Range;

#[derive(PartialEq, Clone, Copy)]
enum Mode { Normal, Filter, Comment }

struct RLine { line: Line, runs: Vec<(Range<usize>, HighlightStyle)>, sep: Option<String> }

pub struct Back;
impl gpui::EventEmitter<Back> for View {}

pub struct View {
    m: Model, theme: Theme, pal: Pal, fonts: Fonts, mode: Mode, filter_f: TextField, comment_f: TextField, error: Option<String>, zoom: f32,
    shown_file: Option<usize>, rlines: Vec<RLine>, cursor_line: Option<u32>, status: String,
}

impl View {
    pub fn new(m: Model, theme: Theme, fonts: Fonts) -> Self {
        let filter = TextField::new(&m.vm.filter);
        View { pal: Pal::new(&theme), m, theme, fonts, mode: Mode::Normal, filter_f: filter, comment_f: TextField::default(), error: None, zoom: 1.0, shown_file: None, rlines: vec![], cursor_line: None, status: String::new() }
    }
    fn current_file(&self) -> Option<usize> { self.m.vm.current_batch().first().copied() }
    fn reload(&mut self) { self.shown_file = None; }

    fn load_diff(&mut self) {
        let Some(fi) = self.current_file() else { self.rlines.clear(); self.shown_file = None; return };
        if self.shown_file == Some(fi) { return; }
        self.shown_file = Some(fi); self.cursor_line = None;
        let f = &self.m.vm.files[fi];
        let old = perusal_gitops::show(&self.m.repo, &self.m.base_sha, &f.path).unwrap_or_default();
        let new = perusal_gitops::show(&self.m.repo, &self.m.branch, &f.path).unwrap_or_default();
        let lang = match f.ext() { "rs" => "rust", "py" => "python", _ => "" };
        let noisy = f.noisy();
        let lines = if noisy { vec![] } else { diff_lines(&old, &new, 3) };
        self.status = if noisy { format!("{} is collapsed because it is generated, binary or very large.", f.path) } else if f.binary { "Binary file".into() } else { String::new() };
        let p = self.pal;
        let (mut last_old, mut last_new, mut first) = (0u32, 0u32, true);
        let mut out = Vec::new();
        for line in lines {
            let (o, n) = (line.old_no.unwrap_or(0), line.new_no.unwrap_or(0));
            let jump = first || (o > 0 && o > last_old + 1) || (n > 0 && n > last_new + 1);
            if jump && !first || first && (o > 1 || n > 1) {
                out.push(RLine { line: Line { kind: Kind::Context, old_no: None, new_no: None, text: String::new(), words: vec![] }, runs: vec![], sep: Some(format!("@@ -{} +{} @@", o.max(1), n.max(1))) });
            }
            first = false;
            if o > 0 { last_old = o; }
            if n > 0 { last_new = n; }
            let mut runs: Vec<(Range<usize>, HighlightStyle)> = highlight(lang, &line.text).into_iter().filter(|(a, b, _)| a < b && *b <= line.text.len() && line.text.is_char_boundary(*a) && line.text.is_char_boundary(*b)).map(|(a, b, c)| {
                let key = match c { Class::Keyword => "keyword", Class::String => "string", Class::Comment => "comment", Class::Number => "number", Class::Type => "type", Class::Function => "function", Class::Other => "keyword" };
                (a..b, HighlightStyle { color: Some(col(&self.theme.syntax, key)), ..Default::default() })
            }).collect();
            runs.sort_by_key(|r| r.0.start);
            let mut end = 0; runs.retain(|r| { if r.0.start >= end { end = r.0.end; true } else { false } });
            for w in &line.words {
                if w.0 < w.1 && w.1 <= line.text.len() && line.text.is_char_boundary(w.0) && line.text.is_char_boundary(w.1) {
                    runs.retain(|r| r.0.end <= w.0 || r.0.start >= w.1);
                    runs.push((w.0..w.1, HighlightStyle { background_color: Some(if line.kind == Kind::Added { p.added_word } else { p.removed_word }), color: Some(p.fg), ..Default::default() }));
                    runs.sort_by_key(|r| r.0.start);
                }
            }
            out.push(RLine { line, runs, sep: None });
        }
        self.rlines = out;
    }

    pub fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke; let key = ks.key.as_str();
        if (ks.modifiers.control || ks.modifiers.platform) && self.mode == Mode::Normal {
            match key { "=" | "+" => self.zoom = theme::zoom(self.zoom, 1), "-" => self.zoom = theme::zoom(self.zoom, -1), "0" => self.zoom = theme::zoom(self.zoom, 0), _ => {} }
            cx.notify(); return;
        }
        match self.mode {
            Mode::Filter | Mode::Comment => {
                if key == "escape" { self.mode = Mode::Normal; self.error = None; cx.notify(); return; }
                if key == "enter" {
                    if self.mode == Mode::Filter { self.apply_filter(); }
                    else { self.add_comment(); }
                } else if self.mode == Mode::Filter { field_key(&mut self.filter_f, ks, cx); } else { field_key(&mut self.comment_f, ks, cx); }
            }
            Mode::Normal => match key {
                "escape" => { cx.emit(Back); return; }
                "/" => self.mode = Mode::Filter, "c" => self.mode = Mode::Comment,
                "v" => self.toggle_viewed(), "enter" => self.next(), "backspace" => self.back(),
                "n" => self.step(1), "p" => self.step(-1), "g" => self.toggle_group(), "x" => self.clear_scope(),
                _ => {}
            },
        }
        cx.notify();
    }
    fn apply_filter(&mut self) {
        match self.m.vm.set_filter(self.filter_f.text.trim()) { Ok(()) => { self.error = None; self.mode = Mode::Normal; self.reload(); } Err(e) => self.error = Some(e) }
    }
    fn add_comment(&mut self) {
        let text = self.comment_f.text.trim().to_string(); if text.is_empty() { return; }
        if let Some(fi) = self.current_file() {
            let path = self.m.vm.files[fi].path.clone(); let l = self.cursor_line;
            let _ = self.m.store.add_draft(self.m.vm.review, Some(&path), l.map(|l| (l, l)), &text, false);
            self.status = format!("Draft added on {path}{}", l.map(|l| format!(":{l}")).unwrap_or_default());
        } else { let _ = self.m.store.add_draft(self.m.vm.review, None, None, &text, false); }
        self.comment_f.clear(); self.mode = Mode::Normal;
    }
    fn toggle_viewed(&mut self) { if self.current_file().is_some() { let v = !self.current_file().map(|i| self.m.vm.files[i].viewed).unwrap_or(false); for i in self.m.vm.current_batch().to_vec() { self.m.vm.set_viewed(&self.m.store, i, v); } } }
    fn next(&mut self) { self.m.vm.next(&self.m.store); self.reload(); }
    fn back(&mut self) { self.m.vm.back(); self.reload(); }
    fn step(&mut self, d: i32) { let c = self.m.vm.cursor as i32 + d; if c >= 0 && (c as usize) < self.m.vm.batch_count() { self.m.vm.cursor = c as usize; self.reload(); } }
    fn toggle_group(&mut self) { self.m.vm.grouping = !self.m.vm.grouping; let _ = self.m.vm.refresh(); self.reload(); }
    fn clear_scope(&mut self) { self.m.graph.clear(); self.m.vm.commit_scope = None; let _ = self.m.vm.refresh(); self.reload(); }
}

fn status_style(s: Status, p: &Pal) -> (&'static str, gpui::Hsla) {
    match s { Status::Added => ("A", p.success), Status::Modified => ("M", p.warn), Status::Deleted => ("D", p.danger), Status::Renamed => ("R", p.accent) }
}

impl Render for View {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.load_diff();
        let p = self.pal; let z = self.zoom;
        let (viewed, total) = self.m.vm.progress();
        let frac = if total == 0 { 0.0 } else { viewed as f32 / total as f32 };
        let n_files = self.m.vm.visible.len();
        let current = self.current_file();
        let drafts = self.m.store.drafts(self.m.vm.review).unwrap_or_default();
        let mono = self.fonts.mono.clone();
        let cur = current.map(|i| self.m.vm.files[i].clone());

        // ---- sidebar: files
        let files_list = uniform_list("files", n_files, cx.processor(move |this, range: Range<usize>, _w, cx| {
            let p = this.pal;
            range.map(|row| {
                let fi = this.m.vm.visible[row]; let f = &this.m.vm.files[fi];
                let sel = this.m.vm.current_batch().contains(&fi);
                let (letter, c) = status_style(f.status, &p);
                let (dir, name) = match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) };
                div().id(row).flex().items_center().gap_2().px_3().h(px(30.0 * this.zoom)).w_full().cursor_pointer()
                    .border_l_2().border_color(if sel { p.accent } else { gpui::transparent_black() }).when(sel, |d| d.bg(p.accent.opacity(0.12))).hover(move |s| s.bg(p.hover))
                    .on_click(cx.listener(move |this, _, _, cx| { this.m.vm.select_file(fi); this.reload(); cx.notify(); }))
                    .child(div().size(px(18.0)).rounded_sm().flex().items_center().justify_center().text_xs().font_weight(FontWeight::BOLD).text_color(c).bg(c.opacity(0.15)).flex_none().child(letter))
                    .child(div().flex().flex_1().min_w_0().truncate().child(div().text_color(p.muted).child(dir)).child(div().text_color(if f.viewed { p.muted } else { p.fg }).font_weight(FontWeight::MEDIUM).child(name)))
                    .when(f.noisy(), |d| d.child(div().text_xs().text_color(p.muted).child("collapsed")))
                    .child(div().text_xs().flex_none().flex().gap_1().child(div().text_color(p.success).child(format!("+{}", f.additions))).child(div().text_color(p.danger).child(format!("−{}", f.deletions))))
                    .child(div().size(px(16.0)).flex_none().flex().items_center().justify_center().when(f.viewed, |d| d.rounded_full().bg(p.success).text_color(gpui::white()).text_xs().child("✓")))
            }).collect()
        })).flex_1().min_h_0().w_full();

        // ---- center: diff
        let diff = uniform_list("diff", self.rlines.len(), { let mono = mono.clone(); cx.processor(move |this, range: Range<usize>, _w, cx| {
            let p = this.pal; let h = px(20.0 * this.zoom);
            range.map(|i| {
                let r = &this.rlines[i];
                if let Some(sep) = &r.sep {
                    return div().id(i).flex().items_center().h(h).w_full().bg(p.accent.opacity(0.10)).text_color(p.muted).font_family(mono.clone()).text_xs().child(div().w(px(110.0)).flex_none()).child(sep.clone());
                }
                let l = &r.line;
                let (bg, sign, gut) = match l.kind { Kind::Added => (p.added, "+", p.added_word.opacity(0.55)), Kind::Removed => (p.removed, "−", p.removed_word.opacity(0.55)), Kind::Context => (gpui::transparent_black(), " ", gpui::transparent_black()) };
                let selected = l.new_no.is_some() && l.new_no == this.cursor_line;
                let nn = l.new_no;
                let text = StyledText::new(SharedString::from(l.text.clone())).with_highlights(r.runs.clone());
                div().id(i).flex().items_center().h(h).w_full().bg(bg).font_family(mono.clone()).text_size(px(13.0 * this.zoom)).text_color(p.fg).cursor_pointer()
                    .when(selected, |d| d.border_l_2().border_color(p.accent)).hover(move |s| s.bg(bg.opacity(0.9)))
                    .on_click(cx.listener(move |this, _, _, cx| { this.cursor_line = nn; cx.notify(); }))
                    .child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.old_no.map(|n| n.to_string()).unwrap_or_default()))
                    .child(div().w(px(52.0)).flex_none().px_2().text_right().text_color(p.muted).bg(gut).child(l.new_no.map(|n| n.to_string()).unwrap_or_default()))
                    .child(div().w(px(22.0)).flex_none().text_center().text_color(match l.kind { Kind::Added => p.success, Kind::Removed => p.danger, _ => p.muted }).child(sign))
                    .child(div().flex_1().min_w_0().whitespace_nowrap().child(text))
            }).collect()
        }) }).flex_1().min_h_0().w_full();

        // ---- right: commits + drafts
        let commit_rows: Vec<_> = self.m.graph.rows.iter().enumerate().map(|(i, r)| {
            let sel = self.m.graph.selection.map(|(a, b)| i >= a && i <= b).unwrap_or(false);
            let last = i + 1 == self.m.graph.rows.len();
            div().id(("c", i)).flex().gap_2().px_3().cursor_pointer().when(sel, |d| d.bg(p.accent.opacity(0.14))).hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.m.graph.click(i); this.m.vm.commit_scope = this.m.graph.scope(&this.m.repo).ok().flatten();
                    let _ = this.m.vm.refresh(); this.reload(); cx.notify();
                }))
                .child(div().flex().flex_col().items_center().w(px(12.0)).flex_none()
                    .child(div().h(px(8.0)).w(px(2.0)).bg(if i == 0 { gpui::transparent_black() } else { p.border }))
                    .child(div().size(px(10.0)).rounded_full().when(r.in_mr, |d| d.bg(p.accent)).when(!r.in_mr, |d| d.border_2().border_color(p.muted)))
                    .child(div().flex_1().w(px(2.0)).bg(if last { gpui::transparent_black() } else { p.border })))
                .child(div().flex().flex_col().min_w_0().flex_1().py_1().child(div().truncate().text_sm().text_color(if r.in_mr { p.fg } else { p.muted }).child(r.subject.clone()))
                    .child(div().font_family(mono.clone()).text_xs().text_color(p.muted).child(r.sha[..7.min(r.sha.len())].to_string())))
        }).collect();
        let draft_rows: Vec<_> = drafts.iter().enumerate().map(|(i, d)| {
            let id = d.id;
            let loc = match (&d.file, d.start_line) { (Some(f), Some(l)) => format!("{}:{l}", f.rsplit('/').next().unwrap_or(f)), (Some(f), None) => f.rsplit('/').next().unwrap_or(f).to_string(), _ => "General".into() };
            div().id(("d", i)).flex().flex_col().gap_1().px_3().py_2().border_b_1().border_color(p.border)
                .child(div().flex().items_center().justify_between().child(div().flex().items_center().gap_2().child(div().text_xs().font_family(mono.clone()).text_color(p.muted).child(loc)).when(d.ai, |x| x.child(badge("AI", p.ai, p.ai.opacity(0.15)))))
                    .child(div().id(("dd", i)).text_xs().text_color(p.muted).cursor_pointer().hover(move |s| s.text_color(p.danger)).child("Remove").on_click(cx.listener(move |this, _, _, cx| { let _ = this.m.store.delete_draft(id); cx.notify(); }))))
                .child(div().text_sm().child(d.body.clone()))
        }).collect();
        let ndrafts = drafts.len();
        let section = |title: &'static str, count: Option<usize>| div().flex().items_center().justify_between().px_3().py_2().border_b_1().border_color(p.border).bg(p.surface)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title)).children(count.map(|c| badge(format!("{c}"), p.muted, p.hover)));

        // ---- header pieces
        let title: SharedString = self.m.title.clone().into();
        let filter_focused = self.mode == Mode::Filter;
        let filter_box = div().id("filter").flex().items_center().gap_2().px_3().py(px(5.0)).w(px(460.0)).rounded_md().border_1().border_color(if filter_focused { p.accent } else if self.error.is_some() { p.danger } else { p.border }).bg(p.bg).cursor_text()
            .on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Filter; cx.notify(); }))
            .child(div().text_color(p.muted).child("⌕")).child(div().flex_1().min_w_0().overflow_hidden().text_sm().font_family(mono.clone()).child(field_text(&self.filter_f, filter_focused, "Filter files: additions > 50 && !viewed", &p)));
        let batch_label = format!("Batch {} of {}", self.m.vm.cursor + 1, self.m.vm.batch_count().max(1));
        let status_text: SharedString = self.error.clone().map(|e| format!("Filter error: {e}")).unwrap_or_else(|| self.status.clone()).into();
        let status_color = if self.error.is_some() { p.danger } else { p.muted };

        let file_card_header = cur.map(|f| {
            let (dir, name) = match f.path.rsplit_once('/') { Some((d, n)) => (format!("{d}/"), n.to_string()), None => (String::new(), f.path.clone()) };
            let total_lines = (f.additions + f.deletions).max(1) as f32;
            let green = ((f.additions as f32 / total_lines) * 5.0).round() as usize;
            let squares: Vec<_> = (0..5).map(|i| div().size(px(10.0)).rounded_sm().bg(if i < green { p.success } else if f.additions + f.deletions == 0 { p.border } else { p.danger })).collect();
            let is_viewed = f.viewed;
            div().flex().items_center().gap_3().px_4().py_2().border_b_1().border_color(p.border).bg(p.surface)
                .child(div().flex().flex_1().min_w_0().truncate().font_family(mono.clone()).text_sm().child(div().text_color(p.muted).child(dir)).child(div().font_weight(FontWeight::SEMIBOLD).child(name)))
                .child(div().flex().items_center().gap_2().text_xs().font_family(mono.clone()).child(div().text_color(p.success).child(format!("+{}", f.additions))).child(div().text_color(p.danger).child(format!("−{}", f.deletions))).child(div().flex().gap(px(2.0)).children(squares)))
                .child(button("comment", "Comment", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Comment; cx.notify(); })))
                .child(div().id("viewed").flex().items_center().gap_2().px_3().py(px(5.0)).rounded_md().border_1().border_color(if is_viewed { p.success } else { p.border }).cursor_pointer().hover(move |s| s.bg(p.hover)).text_sm().font_weight(FontWeight::MEDIUM)
                    .on_click(cx.listener(|this, _, _, cx| { this.toggle_viewed(); cx.notify(); }))
                    .child(div().size(px(16.0)).rounded_sm().border_1().border_color(if is_viewed { p.success } else { p.muted }).flex().items_center().justify_center().when(is_viewed, |d| d.bg(p.success).text_color(gpui::white()).text_xs().child("✓")))
                    .child("Viewed"))
        });
        let comment_target: SharedString = match (current.map(|i| &self.m.vm.files[i].path), self.cursor_line) { (Some(f), Some(l)) => format!("Comment on {f}:{l}"), (Some(f), None) => format!("Comment on {f} (click a line to anchor it)"), _ => "General comment".into() }.into();
        let composing = self.mode == Mode::Comment;

        div().size_full().flex().flex_col().bg(p.bg).text_color(p.fg).text_size(px(14.0 * z))
            // top bar
            .child(div().flex().items_center().gap_3().px_4().h(px(52.0)).bg(p.surface).border_b_1().border_color(p.border)
                .child(button("home", "‹  Home", Btn::Ghost, &p).on_click(cx.listener(|_, _, _, cx| cx.emit(Back))))
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).truncate().min_w_0().child(title))
                .children(self.m.remote.clone().map(|u| pill(u.trim_start_matches("https://").to_string(), &p)))
                .child(div().flex_1())
                .when(ndrafts > 0, |d| d.child(badge(format!("{ndrafts} draft{}", if ndrafts == 1 { "" } else { "s" }), p.accent, p.accent.opacity(0.15))))
                .child(button("grp", if self.m.vm.grouping { "Grouped ✓" } else { "Group related" }, Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.toggle_group(); cx.notify(); })))
                .child(button("prev", "‹ Back", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.back(); cx.notify(); })))
                .child(button("next", "Next ›", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.next(); cx.notify(); }))))
            // filter + progress
            .child(div().flex().items_center().gap_4().px_4().py_2().bg(p.bg).border_b_1().border_color(p.border)
                .child(filter_box)
                .child(div().text_xs().text_color(status_color).flex_1().min_w_0().truncate().child(status_text))
                .child(div().text_sm().text_color(p.muted).child(batch_label))
                .child(div().flex().items_center().gap_2().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!("{viewed} / {total} files viewed")))
                    .child(div().w(px(180.0)).h(px(8.0)).rounded_full().bg(p.border).child(div().h_full().rounded_full().bg(p.success).w(relative(frac))))
                    .child(div().text_sm().text_color(p.muted).child(format!("{}%", (frac * 100.0).round() as u32)))))
            // columns
            .child(div().flex().flex_1().min_h_0()
                .child(div().w(px(380.0 * z)).h_full().flex().flex_col().flex_none().bg(p.surface).border_r_1().border_color(p.border).child(section("Files", Some(n_files))).child(files_list))
                .child(div().flex().flex_col().flex_1().min_w_0().h_full()
                    .children(file_card_header).child(diff)
                    .when(composing, |d| d.child(div().flex().flex_col().gap_2().p_3().border_t_1().border_color(p.accent).bg(p.surface)
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(comment_target.clone()))
                        .child(div().px_3().py(px(6.0)).rounded_md().border_1().border_color(p.accent).bg(p.bg).child(field_text(&self.comment_f, true, "Write a comment. Press Enter to save as a draft, Esc to cancel.", &p)))
                        .child(div().flex().gap_2().child(button("save-c", "Add draft", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.add_comment(); cx.notify(); }))).child(button("cancel-c", "Cancel", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Normal; cx.notify(); })))))))
                .child(div().w(px(320.0 * z)).h_full().flex().flex_col().flex_none().bg(p.surface).border_l_1().border_color(p.border)
                    .child(section("Commits", Some(self.m.graph.rows.len()))).child(div().id("commits").max_h(px(300.0)).overflow_y_scroll().children(commit_rows))
                    .child(div().id("clear-scope").px_3().py_1().text_xs().text_color(p.accent).cursor_pointer().child("Show all commits").on_click(cx.listener(|this, _, _, cx| { this.clear_scope(); cx.notify(); })))
                    .child(section("Drafts", Some(ndrafts))).child(div().id("drafts").flex_1().min_h_0().overflow_y_scroll()
                        .when(ndrafts == 0, |d| d.child(div().p_4().text_sm().text_color(p.muted).child("No drafts yet. Select a line and press c.")))
                        .children(draft_rows))))
            // footer
            .child(div().flex().items_center().gap_4().px_4().py(px(6.0)).bg(p.surface).border_t_1().border_color(p.border).text_xs().text_color(p.muted)
                .children([("n / p", "file"), ("v", "viewed"), ("Enter", "next batch"), ("Backspace", "back"), ("/", "filter"), ("c", "comment"), ("g", "group"), ("Esc", "home"), ("Ctrl + / −", "zoom")]
                    .map(|(k, l)| div().flex().items_center().gap_1().child(kbd(k, &p)).child(l))))
    }
}
