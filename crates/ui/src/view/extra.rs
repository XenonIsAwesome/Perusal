//! Keymap dispatch, command palette, shortcut help, quit support and head-move helpers for the review screen.
use super::*;
use perusal_app::keymap;
use std::collections::BTreeMap;

/// Loads `keymap.toml` from the config dir (with the vim preset when asked). A broken file falls back to the
/// defaults and the problem text is returned so it can be shown once.
pub(super) fn load_keys(vim: bool) -> (BTreeMap<String, String>, Option<String>) {
    let txt = std::fs::read_to_string(model::config_dir().join("keymap.toml")).unwrap_or_default();
    match keymap::load(&txt, vim) {
        Ok(m) => (m, None),
        Err(e) => (keymap::load("", vim).unwrap_or_else(|_| keymap::default_map()), Some(format!("keymap.toml ignored: {e}"))),
    }
}

/// Key text for display ("Ctrl+p" -> "Ctrl+P", "Enter" stays).
fn show_key(k: &str) -> String { k.replace("Ctrl+p", "Ctrl+P") }

impl View {
    pub fn draft_count(&self) -> usize { self.m.store.drafts(self.m.vm.review).map(|d| d.len()).unwrap_or(0) }
    pub fn is_remote(&self) -> bool { self.m.remote.is_some() }
    pub fn tab_title(&self) -> String { self.m.title.clone() }
    /// Quit prompt: Discard.
    pub fn discard_drafts(&mut self) { for d in self.m.store.drafts(self.m.vm.review).unwrap_or_default() { let _ = self.m.store.delete_draft(d.id); } }
    /// Quit prompt: Submit opens the finish dialog.
    pub fn open_finish(&mut self, cx: &mut Context<Self>) { self.finish_open = true; self.focus = Some(Fid::Summary); self.submit_msg = None; cx.notify(); }

    pub(super) fn run_action(&mut self, a: &str, cx: &mut Context<Self>) {
        match a {
            "next_file" => self.step(1), "prev_file" => self.step(-1), "toggle_viewed" => self.toggle_viewed(), "next_batch" => self.next(), "prev_batch" => self.back(),
            "comment" => { self.open_composer(false, None); }
            "ask_ai" => { self.ai_ctx = self.selection(); self.tab = RightTab::Ai; self.right_open = true; self.focus = Some(Fid::Ask); }
            "focus_filter" => { self.focus = Some(Fid::Filter); self.sug_for = Fid::Filter; self.update_suggestions(); }
            "toggle_grouping" => { self.settings.group_related = !self.settings.group_related; self.save_settings(); self.m.vm.grouping = self.settings.group_related; let _ = self.m.vm.refresh(); self.reload(); }
            "toggle_tree" => self.toggle_tree(),
            "toggle_split" => { let m = if self.diff_mode == DiffMode::Unified { DiffMode::Split } else { DiffMode::Unified }; self.set_diff_mode(m); }
            "toggle_markdown" => { if self.m.vm.current_batch().iter().any(|&i| markdown::is_markdown(&self.m.vm.files[i].path)) { self.md_rendered = !self.md_rendered; self.shown = None; self.clear_selection(); } }
            "clear_scope" => self.clear_scope(),
            "finish" => { self.finish_open = true; self.focus = Some(Fid::Summary); self.submit_msg = None; }
            "settings" => cx.emit(ViewEvent::OpenSettings),
            "palette" => { self.palette_open = true; self.pal_in.f.clear(); self.pal_idx = 0; self.focus = Some(Fid::Palette); }
            "help" => { self.help_open = true; }
            _ => {}
        }
    }

    /// Palette matches as (action, label), best first.
    fn palette_hits(&self) -> Vec<(&'static str, &'static str)> {
        let labels: Vec<&'static str> = keymap::ACTIONS.iter().filter(|(a, _, _)| *a != "palette").map(|(_, _, l)| *l).collect();
        keymap::palette(&labels, self.pal_in.f.text.trim()).into_iter().filter_map(|l| keymap::ACTIONS.iter().find(|(_, _, x)| *x == l).map(|(a, _, l)| (*a, *l))).collect()
    }
    pub(super) fn palette_key(&mut self, key: &str, ks: &gpui::Keystroke, cx: &mut Context<Self>) {
        let hits = self.palette_hits();
        match key {
            "escape" => { self.palette_open = false; self.focus = None; }
            "down" => { if !hits.is_empty() { self.pal_idx = (self.pal_idx + 1) % hits.len(); } }
            "up" => { if !hits.is_empty() { self.pal_idx = (self.pal_idx + hits.len() - 1) % hits.len(); } }
            "enter" => { let pick = hits.get(self.pal_idx).map(|h| h.0); self.palette_open = false; self.focus = None; if let Some(a) = pick { self.run_action(a, cx); } }
            _ => { if field_key(&mut self.pal_in.f, ks, cx) { self.pal_idx = 0; } }
        }
    }

    fn modal(&self, id: &'static str, w: f32, body: gpui::Div, cx: &mut Context<Self>, on_close: fn(&mut Self)) -> AnyElement {
        let p = self.pal;
        let card = div().id(id).flex().flex_col().w(px(w)).max_h(px(640.0)).rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation())).child(body);
        deferred(div().absolute().top_0().left_0().size_full().flex().items_start().justify_center().pt(px(90.0)).bg(gpui::black().opacity(0.45)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| { on_close(this); cx.notify(); })).child(card)).with_priority(25).into_any_element()
    }

    pub(super) fn palette_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.pal; let hits = self.palette_hits(); let sel = self.pal_idx.min(hits.len().saturating_sub(1));
        let input = self.winp(Fid::Palette, div().id("palette-in").flex().items_center().gap_2().px_4().py_3().border_b_1().border_color(p.border)
            .child(icon(ic::SEARCH, 18.0, p.muted)).child(div().flex_1().min_w_0().child(input_text(&self.pal_in, true, "Type a command…", &p, false, 1))), cx);
        let rows: Vec<_> = hits.iter().enumerate().map(|(i, (a, l))| {
            let key = self.keys.get(*a).cloned().unwrap_or_default(); let act = *a;
            div().id(("pal-row", i)).flex().items_center().gap_2().px_4().py(px(7.0)).cursor_pointer().text_sm().when(i == sel, |d| d.bg(p.accent.opacity(0.16))).hover(move |x| x.bg(p.hover))
                .child(div().flex_1().child(*l)).when(!key.is_empty(), |d| d.child(kbd_s(&show_key(&key), &p)))
                .on_click(cx.listener(move |this, _, _, cx| { this.palette_open = false; this.focus = None; this.run_action(act, cx); cx.notify(); }))
        }).collect();
        let body = div().flex().flex_col().child(input).child(div().flex().flex_col().max_h(px(420.0)).overflow_hidden().py_1().children(rows)
            .when(hits.is_empty(), |d| d.child(div().px_4().py_3().text_sm().text_color(p.muted).child("No matching command"))));
        self.modal("palette-card", 560.0, body, cx, |t| { t.palette_open = false; t.focus = None; })
    }

    pub(super) fn help_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.pal;
        let mut rows: Vec<(String, String)> = keymap::ACTIONS.iter().map(|(a, _, l)| (show_key(self.keys.get(*a).map(String::as_str).unwrap_or("")), l.to_string())).collect();
        rows.extend([("Esc".to_string(), "Back to Home / clear selection / stop the agent".to_string()), ("Ctrl + / Ctrl -".into(), "Zoom in / out (Ctrl+0 resets)".into()), ("Ctrl+W".into(), "Close this tab".into())]);
        let list = div().flex().flex_col().px_4().py_2().gap(px(2.0)).children(rows.into_iter().map(|(k, l)| div().flex().items_center().gap_3().py(px(3.0)).child(div().w(px(110.0)).flex_none().child(kbd_s(&k, &p))).child(div().text_sm().child(l))));
        let head = div().flex().items_center().gap_2().px_4().py_3().border_b_1().border_color(p.border).child(icon(ic::KEYBOARD, 20.0, p.accent)).child(div().text_lg().font_weight(FontWeight::SEMIBOLD).flex_1().child("Keyboard shortcuts"))
            .child(div().text_xs().text_color(p.muted).child("Rebind in keymap.toml"));
        self.modal("help-card", 560.0, div().flex().flex_col().child(head).child(list), cx, |t| t.help_open = false)
    }
}

fn kbd_s(text: &str, p: &Pal) -> gpui::Div { div().px(px(6.0)).py(px(1.0)).rounded_sm().border_1().border_color(p.border).bg(p.bg).text_xs().text_color(p.fg).child(text.to_string()) }
