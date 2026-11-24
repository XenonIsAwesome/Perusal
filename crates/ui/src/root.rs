//! Root: the Home screen (recent reviews, open local, open from URL, browse remote) and the active review.
use crate::model::{self, Model, OpenErr};
use crate::view::{col, Back, View};
use gpui::{div, prelude::*, px, App, Bounds, Context, Entity, FocusHandle, Hsla, KeyDownEvent, SharedString, Window, WindowBounds, WindowOptions, size};
use perusal_app::theme::{self, Theme};
use perusal_core::store::{RecentRow, Store};

pub enum Initial { Home, Local(String, String, Option<String>), Url(String) }

const LABELS: [&str; 5] = ["Repository path", "Branch", "MR / PR URL", "Browse open MRs: host/owner/repo", "Access token"];

pub struct Root {
    theme: Theme, focus: FocusHandle, review: Option<Entity<View>>, fields: [String; 5], active: usize, recent: Vec<RecentRow>,
    mrs: Vec<(u64, String, String)>, msg: String, busy: bool, token_host: Option<String>, retry_url: Option<String>,
}

type Opened = Result<Model, OpenErr>;

impl Root {
    fn new(initial: Initial, cx: &mut Context<Self>) -> Self {
        let mut r = Root { theme: theme::builtin("dark").expect("built-in theme"), focus: cx.focus_handle(), review: None, fields: Default::default(), active: 2, recent: vec![], mrs: vec![], msg: String::new(), busy: false, token_host: None, retry_url: None };
        r.refresh_recent();
        match initial {
            Initial::Home => {}
            Initial::Local(repo, branch, base) => { let res = Model::open_local(std::path::Path::new(&repo), &branch, base.as_deref()); r.finish_open(res, cx); }
            Initial::Url(u) => r.open_url(u, cx),
        }
        r
    }
    fn refresh_recent(&mut self) {
        if let Ok(s) = Store::open(&model::db_path()) { self.recent = s.recent_rows().unwrap_or_default(); }
    }
    fn finish_open(&mut self, res: Opened, cx: &mut Context<Self>) {
        self.busy = false;
        match res {
            Ok(m) => {
                let v = cx.new(|_| View::new(m, self.theme.clone()));
                cx.subscribe(&v, |this, _, _: &Back, cx| { this.review = None; this.refresh_recent(); this.msg.clear(); cx.notify(); }).detach();
                self.review = Some(v); self.msg.clear(); self.token_host = None;
            }
            Err(OpenErr::NeedsToken { host, why }) => { self.msg = format!("{why}. Type the token below, then press Enter."); self.token_host = Some(host); self.active = 4; }
            Err(OpenErr::Msg(m)) => self.msg = m,
        }
        cx.notify();
    }
    /// Runs blocking open work on the background executor and delivers the result back to the UI.
    fn spawn_open(&mut self, label: String, work: impl FnOnce() -> Opened + Send + 'static, cx: &mut Context<Self>) {
        self.busy = true; self.msg = label; cx.notify();
        let task = cx.background_executor().spawn(async move { work() });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| t.finish_open(r, cx)); }).detach();
    }
    fn open_url(&mut self, url: String, cx: &mut Context<Self>) {
        self.retry_url = Some(url.clone());
        self.spawn_open(format!("Opening {url} (first open clones the repository, this can take a while)..."), move || Model::open_remote(&url), cx);
    }
    fn open_local(&mut self, cx: &mut Context<Self>) {
        let (repo, branch) = (self.fields[0].trim().to_string(), self.fields[1].trim().to_string());
        if repo.is_empty() || branch.is_empty() { self.msg = "enter a repository path and a branch".into(); cx.notify(); return; }
        let res = Model::open_local(std::path::Path::new(&repo), &branch, None); self.finish_open(res, cx);
    }
    fn open_key(&mut self, key: String, cx: &mut Context<Self>) { self.spawn_open("Opening review...".into(), move || Model::reopen(&key), cx); }
    fn browse(&mut self, cx: &mut Context<Self>) {
        let t = self.fields[3].clone(); self.busy = true; self.msg = format!("Listing open MRs of {t}..."); cx.notify();
        let task = cx.background_executor().spawn(async move { model::list_remote(&t) });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| {
            t.busy = false;
            match r { Ok(l) => { t.msg = format!("{} open merge/pull requests", l.len()); t.mrs = l; } Err(OpenErr::NeedsToken { host, why }) => { t.msg = format!("{why}. Type the token below, then press Enter."); t.token_host = Some(host); t.active = 4; } Err(OpenErr::Msg(m)) => t.msg = m }
            cx.notify(); }); }).detach();
    }
    fn save_token(&mut self, cx: &mut Context<Self>) {
        use perusal_providers::auth::TokenStore;
        let Some(host) = self.token_host.clone() else { self.msg = "no host is waiting for a token".into(); cx.notify(); return };
        let tok = self.fields[4].trim().to_string(); if tok.is_empty() { return; }
        match model::token_store().set(&host, &tok) { Ok(()) => { self.fields[4].clear(); self.token_host = None; self.msg = format!("token saved for {host}"); if let Some(u) = self.retry_url.clone() { self.open_url(u, cx); return; } } Err(e) => self.msg = format!("cannot save token: {e}") }
        cx.notify();
    }
    fn home_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.busy { return; }
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "tab" => { self.active = if ks.modifiers.shift { (self.active + 4) % 5 } else { (self.active + 1) % 5 }; }
            "backspace" => { self.fields[self.active].pop(); }
            "enter" => match self.active { 0 | 1 => self.open_local(cx), 2 => { let u = self.fields[2].trim().to_string(); if !u.is_empty() { self.open_url(u, cx); } } 3 => self.browse(cx), _ => self.save_token(cx) },
            _ => { if !ks.modifiers.control { if let Some(c) = &ks.key_char { self.fields[self.active].push_str(c); } } else if ks.key == "v" { /* paste is not wired; type or middle-click paste via the terminal */ } }
        }
        cx.notify();
    }
    fn field(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let (fg, border, accent) = (col(&self.theme.ui, "foreground"), col(&self.theme.ui, "border"), col(&self.theme.ui, "accent"));
        let focused = self.active == i;
        let shown = if i == 4 { "•".repeat(self.fields[i].chars().count()) } else { self.fields[i].clone() };
        let text: SharedString = format!("{}{}", shown, if focused { "▏" } else { "" }).into();
        div().id(("field", i)).flex().flex_col().gap_1().mb_2().cursor_pointer().on_click(cx.listener(move |t, _, _, cx| { t.active = i; cx.notify(); }))
            .child(div().text_color(col(&self.theme.status, "viewed")).child(LABELS[i]))
            .child(div().px_2().py_1().border_1().border_color(if focused { accent } else { border }).text_color(fg).min_h(px(26.0)).child(text))
    }
    fn button(&self, label: &'static str, id: &'static str, cx: &mut Context<Self>, f: impl Fn(&mut Self, &mut Context<Self>) + 'static) -> impl IntoElement {
        let accent: Hsla = col(&self.theme.ui, "accent");
        div().id(id).px_3().py_1().bg(accent).text_color(col(&self.theme.ui, "background")).cursor_pointer().child(label).on_click(cx.listener(move |t, _, _, cx| if !t.busy { f(t, cx) }))
    }
    fn render_home(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (fg, border, accent, dim) = (col(&self.theme.ui, "foreground"), col(&self.theme.ui, "border"), col(&self.theme.ui, "accent"), col(&self.theme.status, "viewed"));
        let rows: Vec<_> = self.recent.iter().enumerate().map(|(i, r)| {
            let key = r.key.clone(); let id = r.id; let pinned = r.pinned;
            let label: SharedString = format!("{}{}  ·  {}/{} viewed  ·  {} drafts  ·  {}", if r.pinned { "★ " } else { "" }, r.title, r.viewed, r.total, r.drafts, if r.key.starts_with("remote:") { r.key.trim_start_matches("remote:").to_string() } else { "local".to_string() }).into();
            div().id(("recent", i)).flex().justify_between().items_center().px_2().py_1().border_b_1().border_color(border).hover(|d| d.bg(accent.opacity(0.12)))
                .child(div().id(("open", i)).flex_1().cursor_pointer().child(label).on_click(cx.listener(move |t, _, _, cx| t.open_key(key.clone(), cx))))
                .child(div().flex().gap_3().text_color(dim)
                    .child(div().id(("pin", i)).cursor_pointer().child(if pinned { "unpin" } else { "pin" }).on_click(cx.listener(move |t, _, _, cx| { if let Ok(s) = Store::open(&model::db_path()) { let _ = s.set_pinned(id, !pinned); } t.refresh_recent(); cx.notify(); })))
                    .child(div().id(("del", i)).cursor_pointer().child("delete").on_click(cx.listener(move |t, _, _, cx| { if let Ok(s) = Store::open(&model::db_path()) { let _ = s.discard(id); } t.refresh_recent(); cx.notify(); }))))
        }).collect();
        let mrs: Vec<_> = self.mrs.iter().enumerate().map(|(i, (n, title, url))| {
            let url = url.clone(); let label: SharedString = format!("#{n}  {title}").into();
            div().id(("mr", i)).px_2().py_1().cursor_pointer().hover(|d| d.bg(accent.opacity(0.12))).child(label).on_click(cx.listener(move |t, _, _, cx| t.open_url(url.clone(), cx)))
        }).collect();
        let msg: SharedString = if self.msg.is_empty() { "Tab switches fields, Enter runs the focused one. Click works too.".into() } else { self.msg.clone().into() };
        let open_local = self.button("Open local branch", "b-local", cx, |t, cx| t.open_local(cx));
        let open_url = self.button("Open URL", "b-url", cx, |t, cx| { let u = t.fields[2].trim().to_string(); if !u.is_empty() { t.open_url(u, cx); } });
        let browse = self.button("List open MRs", "b-browse", cx, |t, cx| t.browse(cx));
        let f0 = self.field(0, cx); let f1 = self.field(1, cx); let f2 = self.field(2, cx); let f3 = self.field(3, cx);
        let tok = self.token_host.is_some().then(|| self.field(4, cx));
        div().size_full().flex().flex_col().p_4().gap_3().text_color(fg)
            .child(div().text_xl().child("Perusal"))
            .child(div().text_color(if self.busy { accent } else { dim }).child(msg))
            .child(div().flex().flex_1().min_h_0().gap_4()
                .child(div().flex().flex_col().flex_1().min_w_0().child(div().pb_1().child("Recent reviews")).child(div().id("recent-list").flex_1().overflow_y_scroll().border_1().border_color(border)
                    .children(if rows.is_empty() { vec![div().id("none").p_2().text_color(dim).child("No reviews yet. Open one on the right.")] } else { rows })))
                .child(div().flex().flex_col().w(px(520.0)).gap_1()
                    .child(div().pb_1().child("Open a local branch")).child(f0).child(f1).child(div().mb_3().child(open_local))
                    .child(div().pb_1().child("Open a merge / pull request")).child(f2).child(div().mb_3().child(open_url))
                    .child(div().pb_1().child("Browse remote")).child(f3).child(div().mb_2().child(browse))
                    .children(tok)
                    .child(div().id("mr-list").flex_1().min_h_0().overflow_y_scroll().border_1().border_color(border).children(mrs))))
    }
}

impl Render for Root {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = col(&self.theme.ui, "background");
        let body = if let Some(v) = &self.review { v.clone().into_any_element() } else { self.render_home(cx).into_any_element() };
        div().size_full().bg(bg).font_family("DejaVu Sans Mono").text_size(px(13.0)).track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| { if let Some(v) = this.review.clone() { v.update(cx, |v, cx| v.key(ev, cx)); } else { this.home_key(ev, cx); } }))
            .child(body)
    }
}

pub fn run(initial: Initial) {
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1400.0), px(850.0)), cx);
        cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() }, move |window, cx| {
            let root = cx.new(|cx| Root::new(initial, cx));
            window.focus(&root.read(cx).focus.clone(), cx);
            root
        }).unwrap();
        cx.activate(true);
    });
}
