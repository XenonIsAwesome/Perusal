//! Root: the Home screen (recent reviews, open local / URL / browse) and the active review.
use crate::icons as ic;
use crate::kit::{self, badge, bounds_probe, button, field_text, icon, icon_btn, new_bounds, pick_fonts, Btn, FieldBounds, Fonts, Pal};
use crate::model::{self, Model, OpenErr};
use crate::view::{Back, View};
use gpui::{div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, SharedString, Window, WindowBounds, WindowOptions};
use perusal_app::textfield::TextField;
use perusal_app::theme::{self, Theme};
use perusal_core::store::{RecentRow, Store};

pub enum Initial { Home, Local(String, String, Option<String>), Url(String) }

#[derive(Clone, Copy, PartialEq)]
enum Tab { Local, Url, Browse }

#[derive(Clone, Copy, PartialEq)]
enum Note { Info, Busy, Error, Ok }

pub struct Root {
    theme: Theme, pal: Pal, fonts: Fonts, focus: FocusHandle, review: Option<Entity<View>>,
    tab: Tab, f_repo: TextField, f_branch: TextField, f_url: TextField, f_browse: TextField, f_token: TextField, active: u8,
    fb: [FieldBounds; 5], drag: Option<u8>,
    recent: Vec<RecentRow>, mrs: Vec<(u64, String, String)>, msg: String, note: Note, busy: bool, token_host: Option<String>, retry_url: Option<String>,
}

type Opened = Result<Model, OpenErr>;

// field ids: 0 repo, 1 branch, 2 url, 3 browse, 4 token
impl Root {
    fn new(initial: Initial, fonts: Fonts, cx: &mut Context<Self>) -> Self {
        let theme = theme::load(&model::load_settings().theme, Some(&model::themes_dir()));
        let mut r = Root { pal: Pal::new(&theme), theme, fonts, focus: cx.focus_handle(), review: None, tab: Tab::Url, f_repo: Default::default(), f_branch: Default::default(), f_url: Default::default(),
            f_browse: Default::default(), f_token: Default::default(), active: 2, fb: [new_bounds(), new_bounds(), new_bounds(), new_bounds(), new_bounds()], drag: None, recent: vec![], mrs: vec![], msg: String::new(), note: Note::Info, busy: false, token_host: None, retry_url: None };
        r.refresh_recent();
        match initial {
            Initial::Home => {}
            Initial::Local(repo, branch, base) => { let res = Model::open_local(std::path::Path::new(&repo), &branch, base.as_deref()); r.finish_open(res, cx); }
            Initial::Url(u) => { r.f_url.set(&u); r.open_url(u, cx) }
        }
        r
    }
    fn field_mut(&mut self, id: u8) -> &mut TextField { match id { 0 => &mut self.f_repo, 1 => &mut self.f_branch, 2 => &mut self.f_url, 3 => &mut self.f_browse, _ => &mut self.f_token } }
    fn visible_fields(&self) -> Vec<u8> {
        let mut v = match self.tab { Tab::Local => vec![0, 1], Tab::Url => vec![2], Tab::Browse => vec![3] };
        if self.token_host.is_some() { v.push(4); } v
    }
    fn refresh_recent(&mut self) { if let Ok(s) = Store::open(&model::db_path()) { self.recent = s.recent_rows().unwrap_or_default(); } }
    fn say(&mut self, note: Note, m: impl Into<String>) { self.note = note; self.msg = m.into(); }
    fn finish_open(&mut self, res: Opened, cx: &mut Context<Self>) {
        self.busy = false;
        match res {
            Ok(m) => {
                let fonts = self.fonts.clone();
                let v = cx.new(|_| View::new(m, model::load_settings(), fonts));
                cx.subscribe(&v, |this, _, _: &Back, cx| { this.theme = theme::load(&model::load_settings().theme, Some(&model::themes_dir())); this.pal = Pal::new(&this.theme); this.review = None; this.refresh_recent(); this.msg.clear(); cx.notify(); }).detach();
                self.review = Some(v); self.msg.clear(); self.token_host = None;
            }
            Err(OpenErr::NeedsToken { host, why }) => { self.say(Note::Error, format!("{why}. Paste an access token for {host} below and press Enter.")); self.token_host = Some(host); self.active = 4; }
            Err(OpenErr::Msg(m)) => self.say(Note::Error, m),
        }
        cx.notify();
    }
    fn spawn_open(&mut self, label: String, work: impl FnOnce() -> Opened + Send + 'static, cx: &mut Context<Self>) {
        self.busy = true; self.say(Note::Busy, label); cx.notify();
        let task = cx.background_executor().spawn(async move { work() });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| t.finish_open(r, cx)); }).detach();
    }
    fn open_url(&mut self, url: String, cx: &mut Context<Self>) {
        self.retry_url = Some(url.clone());
        self.spawn_open(format!("Opening {url}. The first open clones the repository, which can take a while for large projects."), move || Model::open_remote(&url), cx);
    }
    fn submit_url(&mut self, cx: &mut Context<Self>) {
        let u = model::normalize_url_input(&self.f_url.text);
        if u.is_empty() { self.say(Note::Error, "Paste a pull request or merge request URL first."); cx.notify(); return; }
        self.open_url(u, cx);
    }
    fn open_local(&mut self, cx: &mut Context<Self>) {
        let (repo, branch) = (self.f_repo.text.trim().to_string(), self.f_branch.text.trim().to_string());
        if repo.is_empty() || branch.is_empty() { self.say(Note::Error, "Enter a repository path and a branch name."); cx.notify(); return; }
        let res = Model::open_local(std::path::Path::new(&repo), &branch, None); self.finish_open(res, cx);
    }
    fn open_key(&mut self, key: String, cx: &mut Context<Self>) { self.spawn_open("Opening review...".into(), move || Model::reopen(&key), cx); }
    fn browse(&mut self, cx: &mut Context<Self>) {
        let t = self.f_browse.text.trim().to_string();
        if t.is_empty() { self.say(Note::Error, "Enter host/owner/repo, for example github.com/rust-lang/rust."); cx.notify(); return; }
        self.busy = true; self.say(Note::Busy, format!("Listing open merge/pull requests of {t}...")); cx.notify();
        let task = cx.background_executor().spawn(async move { model::list_remote(&t) });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| {
            t.busy = false;
            match r {
                Ok(l) => { t.say(Note::Ok, format!("{} open merge/pull requests", l.len())); t.mrs = l; }
                Err(OpenErr::NeedsToken { host, why }) => { t.say(Note::Error, format!("{why}. Paste an access token for {host} below and press Enter.")); t.token_host = Some(host); t.active = 4; }
                Err(OpenErr::Msg(m)) => t.say(Note::Error, m),
            }
            cx.notify(); }); }).detach();
    }
    fn save_token(&mut self, cx: &mut Context<Self>) {
        use perusal_providers::auth::TokenStore;
        let Some(host) = self.token_host.clone() else { return };
        let tok = self.f_token.text.trim().to_string(); if tok.is_empty() { return; }
        match model::token_store().set(&host, &tok) {
            Ok(()) => { self.f_token.clear(); self.token_host = None; self.active = if self.tab == Tab::Browse { 3 } else { 2 }; self.say(Note::Ok, format!("Token saved for {host}."));
                if self.tab == Tab::Browse { self.browse(cx); return; }
                if let Some(u) = self.retry_url.clone() { self.open_url(u, cx); return; } }
            Err(e) => self.say(Note::Error, format!("Cannot save token: {e}")),
        }
        cx.notify();
    }
    fn enter(&mut self, cx: &mut Context<Self>) {
        match self.active { 0 | 1 => self.open_local(cx), 2 => self.submit_url(cx), 3 => self.browse(cx), _ => self.save_token(cx) }
    }
    fn set_tab(&mut self, t: Tab, cx: &mut Context<Self>) { self.tab = t; self.active = self.visible_fields()[0]; cx.notify(); }
    fn home_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.busy { return; }
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "tab" => { let v = self.visible_fields(); let i = v.iter().position(|x| *x == self.active).unwrap_or(0); self.active = v[if ks.modifiers.shift { (i + v.len() - 1) % v.len() } else { (i + 1) % v.len() }]; }
            "enter" => { self.enter(cx); return; }
            _ => { let id = self.active; let f = self.field_mut(id); let mut f2 = std::mem::take(f); kit::field_key(&mut f2, ks, cx); *self.field_mut(id) = f2; }
        }
        cx.notify();
    }

    fn field_view(&self, id: u8, label: &'static str, placeholder: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let f = match id { 0 => &self.f_repo, 1 => &self.f_branch, 2 => &self.f_url, 3 => &self.f_browse, _ => &self.f_token };
        let focused = self.active == id; let secret = id == 4;
        let shown = if secret { let mut t = f.clone(); t.text = "•".repeat(f.text.chars().count()); t } else { f.clone() };
        let fam = self.fonts.ui.clone();
        let bounds = self.fb[id as usize].clone();
        let (b1, b2, fam1, fam2) = (bounds.clone(), bounds.clone(), fam.clone(), fam.clone());
        div().flex().flex_col().gap_1()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(p.fg).child(label))
            .child(div().id(("field", id as usize)).flex().items_center().px_3().py(px(6.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).text_color(p.fg).min_h(px(32.0)).cursor_text()
                .on_mouse_down(MouseButton::Left, cx.listener(move |t, ev: &MouseDownEvent, w, cx| {
                    t.active = id; t.drag = Some(id);
                    let secret_text = id == 4; let f = t.field_mut(id);
                    if secret_text { let mut m = f.clone(); m.text = "•".repeat(f.text.chars().count()); kit::field_mouse_down(&mut m, w, &b1.get(), &fam1, px(14.0), ev); f.cursor = m.cursor; f.anchor = m.anchor; }
                    else { kit::field_mouse_down(f, w, &b1.get(), &fam1, px(14.0), ev); }
                    cx.notify();
                }))
                .on_mouse_move(cx.listener(move |t, ev: &MouseMoveEvent, w, cx| {
                    if ev.pressed_button == Some(MouseButton::Left) && t.drag == Some(id) {
                        let secret_text = id == 4; let f = t.field_mut(id);
                        if secret_text { let mut m = f.clone(); m.text = "•".repeat(f.text.chars().count()); kit::field_mouse_drag(&mut m, w, &b2.get(), &fam2, px(14.0), ev.position.x); f.cursor = m.cursor; f.anchor = m.anchor; }
                        else { kit::field_mouse_drag(f, w, &b2.get(), &fam2, px(14.0), ev.position.x); }
                        cx.notify();
                    }
                }))
                .child(div().relative().flex_1().min_w_0().cursor_text().text_size(px(14.0)).child(bounds_probe(&bounds)).child(field_text(&shown, focused, placeholder, &p))))
    }

    fn render_recent(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let rows: Vec<_> = self.recent.iter().enumerate().map(|(i, r)| {
            let (key, id, pinned) = (r.key.clone(), r.id, r.pinned);
            let remote = r.key.strip_prefix("remote:").map(|s| s.trim_start_matches("https://").to_string());
            let src: SharedString = remote.clone().unwrap_or_else(|| r.key.trim_start_matches("local:").to_string()).into();
            let frac = if r.total == 0 { 0.0 } else { r.viewed as f32 / r.total as f32 };
            let dot = if r.state == "merged" { p.ai } else if remote.is_some() { p.success } else { p.accent };
            div().id(("recent", i)).group("row").flex().items_center().gap_3().px_4().py_3().border_b_1().border_color(p.border).cursor_pointer().hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |t, _, _, cx| t.open_key(key.clone(), cx)))
                .child(div().size(px(10.0)).rounded_full().bg(dot).flex_none())
                .child(div().flex().flex_col().flex_1().min_w_0().gap(px(2.0))
                    .child(div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.fg).truncate().child(r.title.clone())).when(pinned, |d| d.child(badge("pinned", p.warn, p.warn.opacity(0.15)))))
                    .child(div().text_xs().text_color(p.muted).truncate().child(src)))
                .child(div().flex().flex_col().items_end().gap(px(4.0)).w(px(150.0)).flex_none()
                    .child(div().flex().items_center().gap_2().when(r.drafts > 0, |d| d.child(badge(format!("{} drafts", r.drafts), p.accent, p.accent.opacity(0.15)))).child(div().text_xs().text_color(p.muted).child(format!("{}/{} viewed", r.viewed, r.total))))
                    .child(div().w_full().h(px(4.0)).rounded_full().bg(p.border).child(div().h_full().rounded_full().bg(p.success).w(gpui::relative(frac)))))
                .child(div().flex().gap_1().flex_none()
                    .child(icon_btn(("pin", i), ic::PUSH_PIN, if pinned { "Unpin this review" } else { "Pin this review" }, if pinned { Btn::Secondary } else { Btn::Ghost }, &p).on_click(cx.listener(move |t, _, _, cx| { cx.stop_propagation(); if let Ok(s) = Store::open(&model::db_path()) { let _ = s.set_pinned(id, !pinned); } t.refresh_recent(); cx.notify(); })))
                    .child(icon_btn(("del", i), ic::DELETE, "Delete this review and its drafts", Btn::Ghost, &p).on_click(cx.listener(move |t, _, _, cx| { cx.stop_propagation(); if let Ok(s) = Store::open(&model::db_path()) { let _ = s.discard(id); } t.refresh_recent(); cx.notify(); }))))
        }).collect();
        let empty = rows.is_empty();
        div().flex().flex_col().flex_1().min_w_0().min_h_0().rounded_lg().border_1().border_color(p.border).bg(p.surface).overflow_hidden()
            .child(div().flex().items_center().justify_between().px_4().py_3().border_b_1().border_color(p.border).bg(p.surface)
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Recent reviews")).child(badge(format!("{}", self.recent.len()), p.muted, p.hover)))
            .child(div().id("recent-list").flex_1().min_h_0().overflow_y_scroll().bg(p.bg)
                .when(empty, |d| d.child(div().flex().flex_col().items_center().justify_center().gap_2().py(px(80.0)).text_color(p.muted)
                    .child(div().text_xl().child("Nothing here yet")).child(div().text_sm().child("Reviews you open appear here, with progress and drafts saved."))))
                .children(rows))
    }

    fn render_open(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let tab_btn = |t: Tab, label: &'static str, id: &'static str, i: ic::Icon, cx: &mut Context<Self>| {
            let on = self.tab == t;
            div().id(id).flex().items_center().gap_2().px_3().py_2().cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted })
                .border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() }).hover(move |s| s.text_color(p.fg)).child(icon(i, 17.0, if on { p.accent } else { p.muted })).child(label)
                .on_click(cx.listener(move |t2, _, _, cx| t2.set_tab(t, cx)))
        };
        let tabs = div().flex().border_b_1().border_color(p.border).child(tab_btn(Tab::Url, "Pull / merge request", "t-url", ic::LINK, cx)).child(tab_btn(Tab::Local, "Local branch", "t-local", ic::FOLDER_OPEN, cx)).child(tab_btn(Tab::Browse, "Browse remote", "t-browse", ic::SEARCH, cx));
        let form = match self.tab {
            Tab::Url => div().flex().flex_col().gap_3()
                .child(self.field_view(2, "URL", "https://github.com/owner/repo/pull/123", cx))
                .child(div().text_xs().text_color(p.muted).child("GitHub, GitHub Enterprise, GitLab and self-hosted GitLab URLs. Paste with Ctrl+V."))
                .child(button("go-url", "Open review", Btn::Primary, &p).on_click(cx.listener(|t, _, _, cx| { if !t.busy { t.submit_url(cx) } }))),
            Tab::Local => div().flex().flex_col().gap_3()
                .child(self.field_view(0, "Repository path", "/path/to/repo", cx)).child(self.field_view(1, "Branch", "my-feature-branch", cx))
                .child(div().text_xs().text_color(p.muted).child("Reviews the branch against its merge-base with the default branch. Comments stay local."))
                .child(button("go-local", "Open review", Btn::Primary, &p).on_click(cx.listener(|t, _, _, cx| { if !t.busy { t.open_local(cx) } }))),
            Tab::Browse => {
                let mrs: Vec<_> = self.mrs.iter().enumerate().map(|(i, (n, title, url))| {
                    let url = url.clone();
                    div().id(("mr", i)).flex().items_center().gap_2().px_3().py_2().border_b_1().border_color(p.border).cursor_pointer().hover(move |s| s.bg(p.hover))
                        .child(div().size(px(8.0)).rounded_full().bg(p.success).flex_none()).child(div().text_color(p.muted).flex_none().child(format!("#{n}"))).child(div().flex_1().min_w_0().truncate().child(title.clone()))
                        .on_click(cx.listener(move |t, _, _, cx| { t.f_url.set(&url); t.open_url(url.clone(), cx) }))
                }).collect();
                div().flex().flex_col().gap_3()
                    .child(self.field_view(3, "Repository", "github.com/owner/repo", cx))
                    .child(button("go-browse", "List open requests", Btn::Secondary, &p).on_click(cx.listener(|t, _, _, cx| { if !t.busy { t.browse(cx) } })))
                    .child(div().id("mr-list").max_h(px(260.0)).overflow_y_scroll().rounded_md().border_1().border_color(p.border).children(mrs))
            }
        };
        let token = self.token_host.clone().map(|h| div().flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(p.warn).bg(p.warn.opacity(0.08))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("Access token for {h}")))
            .child(self.field_view(4, "Personal access token", "paste token, stored locally with 0600 permissions", cx))
            .child(button("go-token", "Save token and retry", Btn::Primary, &p).on_click(cx.listener(|t, _, _, cx| t.save_token(cx)))));
        div().flex().flex_col().w(px(480.0)).flex_none().rounded_lg().border_1().border_color(p.border).bg(p.surface).overflow_hidden()
            .child(div().px_4().py_3().border_b_1().border_color(p.border).font_weight(FontWeight::SEMIBOLD).child("Open a review"))
            .child(div().px_2().child(tabs))
            .child(div().flex().flex_col().gap_4().p_4().child(form).children(token))
    }

    fn render_home(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let (bar_fg, bar_bg, bar_border) = match self.note { Note::Info => (p.muted, p.surface, p.border), Note::Busy => (p.accent, p.accent.opacity(0.1), p.accent), Note::Error => (p.danger, p.danger.opacity(0.1), p.danger), Note::Ok => (p.success, p.success.opacity(0.1), p.success) };
        let msg: SharedString = self.msg.clone().into();
        let show_note = !self.msg.is_empty();
        let recent = self.render_recent(cx); let open = self.render_open(cx);
        div().size_full().flex().flex_col().bg(p.bg).text_color(p.fg)
            .child(div().flex().items_center().gap_3().px_6().h(px(56.0)).border_b_1().border_color(p.border).bg(p.surface)
                .child(div().size(px(28.0)).rounded_md().bg(p.accent).flex().items_center().justify_center().text_color(gpui::white()).font_weight(FontWeight::BOLD).child("P"))
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Perusal"))
                .child(div().text_sm().text_color(p.muted).child("Review pull requests and merge requests, one batch at a time")))
            .child(div().flex().flex_col().flex_1().min_h_0().items_center().p_6().gap_4()
                .child(div().flex().flex_col().w_full().max_w(px(1240.0)).flex_1().min_h_0().gap_4()
                    .when(show_note, |d| d.child(div().px_4().py_2().rounded_md().border_1().border_color(bar_border).bg(bar_bg).text_color(bar_fg).text_sm().child(msg)))
                    .child(div().flex().flex_1().min_h_0().gap_6().child(recent).child(open))))
    }
}

impl Render for Root {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if let Some(v) = &self.review { v.clone().into_any_element() } else { self.render_home(cx).into_any_element() };
        div().size_full().bg(self.pal.bg).font_family(self.fonts.ui.clone()).text_size(px(14.0)).track_focus(&self.focus)
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| { this.drag = None; }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| { if let Some(v) = this.review.clone() { v.update(cx, |v, cx| v.key(ev, cx)); } else { this.home_key(ev, cx); } }))
            .child(body)
    }
}

pub fn run(initial: Initial) {
    gpui_platform::application().run(move |cx: &mut App| {
        let _ = cx.text_system().add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!("../assets/MaterialIcons-Regular.ttf").as_slice())]);
        let fonts = pick_fonts(cx);
        let bounds = Bounds::centered(None, size(px(1480.0), px(900.0)), cx);
        cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() }, move |window, cx| {
            let root = cx.new(|cx| Root::new(initial, fonts, cx));
            window.focus(&root.read(cx).focus.clone(), cx);
            root
        }).unwrap();
        cx.activate(true);
    });
}
