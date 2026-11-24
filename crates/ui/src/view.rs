use gpui::{
    div, prelude::*, px, relative, rgb, uniform_list, Context, Hsla, HighlightStyle, KeyDownEvent, SharedString, StyledText, Window,
};
use crate::model::Model;
use perusal_app::theme::{self, Theme};
use perusal_diff::text::{diff_lines, highlight, Class, Kind, Line};
use std::ops::Range;

#[derive(PartialEq, Clone, Copy)]
enum Mode { Normal, Filter, Comment }

struct RLine { line: Line, runs: Vec<(Range<usize>, HighlightStyle)> }

pub struct Back;
impl gpui::EventEmitter<Back> for View {}

pub struct View {
    m: Model, theme: Theme, mode: Mode, input: String, error: Option<String>, zoom: f32,
    shown_file: Option<usize>, rlines: Vec<RLine>, cursor_line: Option<u32>, status: String,
}

pub fn col(t: &std::collections::BTreeMap<String, String>, k: &str) -> Hsla {
    let h = t.get(k).map(|s| s.trim_start_matches('#')).unwrap_or("ff00ff");
    rgb(u32::from_str_radix(h, 16).unwrap_or(0xff00ff)).into()
}

impl View {
    pub fn new(m: Model, theme: Theme) -> Self { View { m, theme, mode: Mode::Normal, input: String::new(), error: None, zoom: 1.0, shown_file: None, rlines: vec![], cursor_line: None, status: String::new() } }
    fn c_ui(&self, k: &str) -> Hsla { col(&self.theme.ui, k) }
    fn current_file(&self) -> Option<usize> { self.m.vm.current_batch().first().copied() }

    fn load_diff(&mut self) {
        let Some(fi) = self.current_file() else { self.rlines.clear(); self.shown_file = None; return };
        if self.shown_file == Some(fi) { return; }
        self.shown_file = Some(fi); self.cursor_line = None;
        let f = &self.m.vm.files[fi];
        let old = perusal_gitops::show(&self.m.repo, &self.m.base_sha, &f.path).unwrap_or_default();
        let new = perusal_gitops::show(&self.m.repo, &self.m.branch, &f.path).unwrap_or_default();
        let lang = match f.ext() { "rs" => "rust", "py" => "python", _ => "" };
        let (lines, truncated) = if f.noisy() { (vec![], true) } else { (diff_lines(&old, &new, 3), false) };
        self.status = if truncated { format!("{} is collapsed (noisy). Press e to expand.", f.path) } else { String::new() };
        self.rlines = lines.into_iter().map(|line| {
            let mut runs: Vec<(Range<usize>, HighlightStyle)> = highlight(lang, &line.text).into_iter().filter(|(a, b, _)| *a < *b && *b <= line.text.len() && line.text.is_char_boundary(*a) && line.text.is_char_boundary(*b)).map(|(a, b, c)| {
                let key = match c { Class::Keyword => "keyword", Class::String => "string", Class::Comment => "comment", Class::Number => "number", Class::Type => "type", Class::Function => "function", Class::Other => "keyword" };
                (a..b, HighlightStyle { color: Some(col(&self.theme.syntax, key)), ..Default::default() })
            }).collect();
            runs.sort_by_key(|r| r.0.start);
            // drop overlaps (tree-sitter parents/children): keep non-overlapping, earliest first
            let mut end = 0; runs.retain(|r| { if r.0.start >= end { end = r.0.end; true } else { false } });
            // word-level highlight as background layered via separate pass is not composable here; mark with bold
            for w in &line.words { if w.0 < w.1 && w.1 <= line.text.len() && line.text.is_char_boundary(w.0) && line.text.is_char_boundary(w.1) {
                let bg = col(&self.theme.diff, if line.kind == Kind::Added { "added_word" } else { "removed_word" });
                runs.retain(|r| r.0.end <= w.0 || r.0.start >= w.1);
                runs.push((w.0..w.1, HighlightStyle { background_color: Some(bg), color: Some(self.c_ui("foreground")), ..Default::default() }));
                runs.sort_by_key(|r| r.0.start);
            } }
            RLine { line, runs }
        }).collect();
    }

    pub fn key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke; let key = ks.key.as_str();
        if ks.modifiers.control || ks.modifiers.platform {
            match key { "=" | "+" => self.zoom = theme::zoom(self.zoom, 1), "-" => self.zoom = theme::zoom(self.zoom, -1), "0" => self.zoom = theme::zoom(self.zoom, 0), _ => {} }
            cx.notify(); return;
        }
        match self.mode {
            Mode::Filter | Mode::Comment => match key {
                "escape" => { self.mode = Mode::Normal; self.input.clear(); self.error = None; }
                "backspace" => { self.input.pop(); }
                "enter" => {
                    if self.mode == Mode::Filter {
                        match self.m.vm.set_filter(&self.input) { Ok(()) => { self.error = None; self.mode = Mode::Normal; self.shown_file = None; } Err(e) => self.error = Some(e) }
                    } else if let Some(fi) = self.current_file() {
                        let path = self.m.vm.files[fi].path.clone(); let l = self.cursor_line;
                        let _ = self.m.store.add_draft(self.m.vm.review, Some(&path), l.map(|l| (l, l)), &self.input, false);
                        self.status = format!("draft added on {path}"); self.input.clear(); self.mode = Mode::Normal;
                    }
                }
                _ => { if let Some(c) = &ks.key_char { self.input.push_str(c); } }
            },
            Mode::Normal => {
                match key {
                    "escape" => { cx.emit(Back); return; }
                    "/" => { self.mode = Mode::Filter; self.input = self.m.vm.filter.clone(); }
                    "c" => { self.mode = Mode::Comment; self.input.clear(); }
                    "v" => if let Some(fi) = self.current_file() { let v = !self.m.vm.files[fi].viewed; for i in self.m.vm.current_batch().to_vec() { self.m.vm.set_viewed(&self.m.store, i, v); } },
                    "enter" => { self.m.vm.next(&self.m.store); self.shown_file = None; }
                    "backspace" => { self.m.vm.back(); self.shown_file = None; }
                    "n" => { let c = self.m.vm.cursor; if c + 1 < self.m.vm.batch_count() { self.m.vm.cursor = c + 1; self.shown_file = None; } }
                    "p" => { if self.m.vm.cursor > 0 { self.m.vm.cursor -= 1; self.shown_file = None; } }
                    "g" => { self.m.vm.grouping = !self.m.vm.grouping; let _ = self.m.vm.refresh(); self.shown_file = None; }
                    "x" => { self.m.graph.clear(); self.m.vm.commit_scope = None; let _ = self.m.vm.refresh(); self.shown_file = None; }
                    _ => {}
                }
            }
        }
        cx.notify();
    }
}

impl Render for View {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.load_diff();
        let z = self.zoom; let fs = px(13.0 * z);
        let (bg, fg, border, accent) = (self.c_ui("background"), self.c_ui("foreground"), self.c_ui("border"), self.c_ui("accent"));
        let (viewed, total) = self.m.vm.progress();
        let frac = if total == 0 { 0.0 } else { viewed as f32 / total as f32 };
        let n_files = self.m.vm.visible.len();
        let current = self.current_file();
        let ndrafts = self.m.store.drafts(self.m.vm.review).map(|d| d.len()).unwrap_or(0);
        let prompt: SharedString = match self.mode { Mode::Normal => format!("filter: {}", if self.m.vm.filter.is_empty() { "(press / to edit)".into() } else { self.m.vm.filter.clone() }), Mode::Filter => format!("filter> {}", self.input), Mode::Comment => format!("comment> {}", self.input) }.into();
        let (added_bg, removed_bg) = (col(&self.theme.diff, "added"), col(&self.theme.diff, "removed"));
        let (ci_viewed, ci_ai) = (col(&self.theme.status, "viewed"), col(&self.theme.status, "ai"));
        let _ = (window, ci_ai);

        let files_list = uniform_list("files", n_files, cx.processor(move |this, range: Range<usize>, _w, cx| {
            let fg = this.c_ui("foreground"); let accent = this.c_ui("accent"); let dim = col(&this.theme.status, "viewed");
            range.map(|row| {
                let fi = this.m.vm.visible[row]; let f = &this.m.vm.files[fi];
                let sel = this.m.vm.current_batch().contains(&fi);
                let label: SharedString = format!("{} {} {} +{} -{}{}", if f.viewed { "✓" } else { " " }, f.status.as_str()[..1].to_uppercase(), f.path, f.additions, f.deletions, if f.noisy() { " ~" } else { "" }).into();
                div().id(row).px_2().w_full().cursor_pointer().text_color(if f.viewed { dim } else { fg }).when(sel, |d| d.bg(accent.opacity(0.25)))
                    .on_click(cx.listener(move |this, _, _, cx| { this.m.vm.select_file(fi); this.shown_file = None; cx.notify(); }))
                    .child(label)
            }).collect()
        })).h_full();

        let diff_len = self.rlines.len();
        let diff = uniform_list("diff", diff_len, cx.processor(move |this, range: Range<usize>, _w, cx| {
            let fg = this.c_ui("foreground"); let dim = col(&this.theme.status, "viewed"); let accent = this.c_ui("accent");
            let (a_bg, r_bg) = (col(&this.theme.diff, "added"), col(&this.theme.diff, "removed"));
            range.map(|i| {
                let r = &this.rlines[i]; let l = &r.line;
                let (bg, sign) = match l.kind { Kind::Added => (Some(a_bg), "+"), Kind::Removed => (Some(r_bg), "-"), Kind::Context => (None, " ") };
                let nums: SharedString = format!("{:>5} {:>5} {sign} ", l.old_no.map(|n| n.to_string()).unwrap_or_default(), l.new_no.map(|n| n.to_string()).unwrap_or_default()).into();
                let selected = l.new_no.is_some() && l.new_no == this.cursor_line;
                let text = StyledText::new(SharedString::from(l.text.clone())).with_highlights(r.runs.clone());
                let nn = l.new_no;
                div().id(i).flex().w_full().when_some(bg, |d, c| d.bg(c)).when(selected, |d| d.border_l_2().border_color(accent)).text_color(fg)
                    .on_click(cx.listener(move |this, _, _, cx| { this.cursor_line = nn; cx.notify(); }))
                    .child(div().text_color(dim).child(nums)).child(text)
            }).collect()
        })).h_full().w_full();

        let graph_rows: Vec<_> = self.m.graph.rows.iter().enumerate().map(|(i, r)| {
            let sel = self.m.graph.selection.map(|(a, b)| i >= a && i <= b).unwrap_or(false);
            let label: SharedString = format!("{} {} {}", if r.in_mr { "●" } else { "○" }, &r.sha[..7.min(r.sha.len())], r.subject).into();
            let dim = ci_viewed;
            div().id(("c", i)).px_2().cursor_pointer().text_color(if r.in_mr { fg } else { dim }).when(sel, |d| d.bg(accent.opacity(0.25))).child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.m.graph.click(i);
                    this.m.vm.commit_scope = this.m.graph.scope(&this.m.repo).ok().flatten();
                    let _ = this.m.vm.refresh(); this.shown_file = None; cx.notify();
                }))
        }).collect();

        let top: SharedString = format!("{}{}  ·  {}/{} viewed  ·  {} drafts  ·  batch {}/{}  ·  {}", self.m.title, self.m.remote.as_ref().map(|u| format!("  {u}")).unwrap_or_default(), viewed, total, ndrafts, self.m.vm.cursor + 1, self.m.vm.batch_count().max(1), if self.m.vm.grouping { "grouped" } else { "single files" }).into();
        let help: SharedString = "Esc home  n/p file  v viewed  Enter next  Backspace back  / filter  c comment  g group  x clear commit  Ctrl +/-/0 zoom".into();
        let status: SharedString = self.error.clone().map(|e| format!("error: {e}")).unwrap_or_else(|| self.status.clone()).into();
        let cur_path: SharedString = current.map(|i| self.m.vm.files[i].path.clone()).unwrap_or_else(|| "no files match".into()).into();
        let _ = (added_bg, removed_bg);

        div().size_full().flex().flex_col().bg(bg).text_color(fg).text_size(fs).font_family("DejaVu Sans Mono")
            .child(div().px_2().py_1().border_b_1().border_color(border).child(top))
            .child(div().h(px(4.0)).w_full().bg(border).child(div().h_full().w(relative(frac)).bg(accent)))
            .child(div().px_2().py_1().border_b_1().border_color(border).child(prompt))
            .child(div().flex().flex_1().min_h_0()
                .child(div().w(px(380.0 * z)).h_full().border_r_1().border_color(border).child(files_list))
                .child(div().flex().flex_col().flex_1().min_w_0().h_full().child(div().px_2().py_1().border_b_1().border_color(border).child(cur_path)).child(div().flex_1().min_h_0().child(diff)))
                .child(div().w(px(300.0 * z)).h_full().border_l_1().border_color(border).flex().flex_col().child(div().px_2().py_1().border_b_1().border_color(border).child("commits")).children(graph_rows)))
            .child(div().px_2().py_1().border_t_1().border_color(border).flex().justify_between().child(status).child(help))
    }
}

