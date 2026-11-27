//! Root: the tab strip (Home plus one tab per open review), the Home screen, shared Settings/setup, and notifications.
use crate::icons as ic;
use crate::kit::{self, badge, icon, icon_btn, icon_text_btn, input_text, loading_dots, wire_input, Btn, Fonts, Input, Pal};
use crate::model::{self, Model, OpenErr};
use crate::settings_view::{SettingsEvent, SettingsView};
use crate::view::{View, ViewEvent};
use gpui::{CursorStyle, ResizeEdge, TitlebarOptions, WindowDecorations, deferred, div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton, SharedString, Subscription, Window, WindowBounds, WindowOptions};
use perusal_app::mine::{self, Dim, Sel};
use perusal_app::notify::{Kind as NKind, Toasts};
use perusal_app::settings::Settings;
use perusal_app::theme;
use perusal_core::store::{RecentRow, Store};
use perusal_providers::{RemoteMr, Role, StateFilter};

pub enum Initial { Home, Local(String, String, Option<String>), Url(String) }

#[derive(Clone, Copy, PartialEq)]
enum CloseChoice { Cancel, Submit, Keep, Discard }
#[derive(Clone, Copy, PartialEq)]
enum HTab { Recent, Mine }
#[derive(Clone, Copy, PartialEq)]
enum OTab { Url, Local, Browse }
#[derive(Clone, Copy, PartialEq)]
enum Hf { Url, Repo, Branch, Base, Browse, Token, Rename, RSearch, MSearch, DSearch }
#[derive(Clone, Copy, PartialEq)]
enum Remote { All, Local, GitHub, GitLab }
#[derive(Clone, Copy, PartialEq)]
enum St { All, Open, Merged, Closed }

/// Payload of a dragged tab (its position and title, for the drag preview).
#[derive(Clone)]
struct DraggedTab { from: usize, title: SharedString, pal: Pal }
struct DragTab { t: DraggedTab }
impl gpui::Render for DragTab {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.t.pal;
        div().px_3().h(px(30.0)).flex().items_center().rounded_md().border_1().border_color(p.accent).bg(p.surface).text_sm().font_weight(FontWeight::MEDIUM).text_color(p.fg).shadow_lg().child(self.t.title.clone())
    }
}

/// A pending "leave this review?" question. `quit` means the window is closing and the next unfinished tab is asked after this one.
struct CloseReq { view: gpui::EntityId, quit: bool }

struct TabItem { key: String, title: String, ci: String, view: Entity<View>, _sub: Subscription }

pub struct Root {
    settings: Settings, decor: WindowDecorations, fs_applied: bool, pal: Pal, fonts: Fonts, focus: FocusHandle,
    tabs: Vec<TabItem>, active: Option<usize>, settings_view: Option<Entity<SettingsView>>, _ssub: Option<Subscription>, toasts: Toasts,
    htab: HTab, otab: OTab, hf: Option<Hf>,
    url: Input, repo: Input, branch: Input, base: Input, browse: Input, token: Input, rename: Input, renaming: Option<i64>, r_search: Input, m_search: Input,
    recent: Vec<RecentRow>, r_remote: Remote, r_state: St,
    mine: Vec<RemoteMr>, mine_errs: Vec<(String, String)>, mine_loading: bool, mine_role: Role, mine_state: StateFilter, mine_remote: Remote, mine_loaded: bool, close_req: Option<CloseReq>, mine_saved: perusal_app::mine::Saved, mine_sel: Sel, mine_me: std::collections::HashMap<String, String>, mine_dd: Option<Dim>, dd_search: Input,
    mrs: Vec<(u64, String, String)>, busy: bool, token_host: Option<String>, retry_url: Option<String>, msg: Option<(NKind, String)>,
}

type Opened = Result<Model, OpenErr>;

impl Root {
    fn new(initial: Initial, fonts: Fonts, cx: &mut Context<Self>) -> Self {
        let settings = model::load_settings();
        let theme = theme::load(&settings.theme, Some(&model::themes_dir()));
        let mut r = Root { fs_applied: false, decor: if settings.custom_titlebar { WindowDecorations::Client } else { WindowDecorations::Server }, pal: Pal::new(&theme), fonts, focus: cx.focus_handle(), tabs: vec![], active: None, settings_view: None, _ssub: None, toasts: Toasts::default(),
            htab: HTab::Recent, otab: OTab::Url, hf: Some(Hf::Url), url: Input::new(""), repo: Input::new(""), branch: Input::new(""), base: Input::new(""), browse: Input::new(""), token: Input::new(""), rename: Input::new(""), renaming: None, r_search: Input::new(""), m_search: Input::new(""),
            recent: vec![], r_remote: Remote::All, r_state: St::All, mine: vec![], mine_errs: vec![], mine_loading: false, mine_role: Role::Reviewable, mine_state: StateFilter::Open, mine_remote: Remote::All, mine_loaded: false, close_req: None, mine_saved: perusal_app::mine::Saved::default(), mine_sel: Sel::default(), mine_me: Default::default(), mine_dd: None, dd_search: Input::new(""),
            mrs: vec![], busy: false, token_host: None, retry_url: None, msg: None, settings };
        r.restore_mine();
        r.refresh_recent();
        if r.settings.home_tab.start(r.recent.len()) == perusal_app::settings::HomeTab::Mine { r.htab = HTab::Mine; r.load_mine(cx); }
        // repaint when a background avatar download finishes
        cx.spawn(async move |this, cx| { let mut seen = crate::avatars::version(); loop { cx.background_executor().timer(std::time::Duration::from_millis(400)).await; let v = crate::avatars::version(); if v != seen { seen = v; if this.update(cx, |_, cx| cx.notify()).is_err() { break; } } } }).detach();
        match initial {
            Initial::Home => {}
            Initial::Local(repo, branch, base) => { let res = Model::open_local(std::path::Path::new(&repo), &branch, base.as_deref()); r.finish_open(res, cx); }
            Initial::Url(u) => { r.url.f.set(&u); r.open_url(u, cx) }
        }
        if !r.settings.onboarded { r.open_settings(true, cx); }
        r.listen_for_links(cx);
        if perusal_app::desktop::appimage().is_some() && !perusal_app::desktop::entry_path(&std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default(), std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).as_deref()).exists() {
            r.toast(NKind::Info, "Perusal is running from an AppImage. Register it for perusal:// links in Settings > Appearance > Links.", cx);
        }
        r
    }
    /// Single instance: a second `perusal <link>` launch hands its link to this window over a Unix socket and exits.
    fn listen_for_links(&mut self, cx: &mut Context<Self>) {
        let sock = perusal_app::deeplink::socket_path(&model::data_dir());
        let Ok(l) = perusal_app::deeplink::listen(&sock) else { return };
        { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)); }
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || { while let Ok(m) = perusal_app::deeplink::accept_one(&l) { if tx.send(m).is_err() { break; } } });
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                while let Ok(m) = rx.try_recv() {
                    let alive = this.update(cx, |t, cx| {
                        match perusal_app::deeplink::url_from_arg(m.trim()) {
                            Ok(u) => { cx.activate(true); t.active = None; t.url.f.set(&u); t.open_url(u, cx); }
                            Err(e) => t.toast(NKind::Error, format!("Ignored a link: {e}"), cx),
                        }
                    });
                    if alive.is_err() { return; }
                }
            }
        }).detach();
    }
    /// Loads the remembered My reviews filters.
    fn restore_mine(&mut self) {
        let s = model::load_mine_state();
        self.mine_remote = match s.remote.as_str() { "github" => Remote::GitHub, "gitlab" => Remote::GitLab, _ => Remote::All };
        self.mine_state = match s.state.as_str() { "merged" => StateFilter::Merged, "closed" => StateFilter::Closed, "all" => StateFilter::All, _ => StateFilter::Open };
        self.mine_role = if s.reviewer { Role::ReviewRequested } else { Role::Reviewable };
        self.mine_sel = s.sel(); self.m_search.f.set(&s.query); self.mine_saved = s;
    }
    /// Writes the My reviews filters when they changed (called every frame; cheap compare).
    fn sync_mine_state(&mut self) {
        let mut s = perusal_app::mine::Saved {
            remote: match self.mine_remote { Remote::GitHub => "github", Remote::GitLab => "gitlab", _ => "all" }.into(),
            state: match self.mine_state { StateFilter::Merged => "merged", StateFilter::Closed => "closed", StateFilter::All => "all", StateFilter::Open => "open" }.into(),
            reviewer: self.mine_role == Role::ReviewRequested, query: self.m_search.f.text.clone(), ..Default::default() };
        s.set_sel(&self.mine_sel);
        if s != self.mine_saved { model::save_mine_state(&s); self.mine_saved = s; }
    }
    fn refresh_recent(&mut self) { if let Ok(s) = Store::open(&model::db_path()) { self.recent = s.recent_rows().unwrap_or_default(); } }
    fn toast(&mut self, kind: NKind, text: impl Into<String>, cx: &mut Context<Self>) {
        self.toasts.push(kind, text); cx.notify();
        let ttl = Toasts::ttl(kind) + std::time::Duration::from_millis(300);
        cx.spawn(async move |this, cx| { cx.background_executor().timer(ttl).await; let _ = this.update(cx, |t, cx| { if t.toasts.expire(std::time::Instant::now()) { cx.notify(); } }); }).detach();
    }
    fn say(&mut self, kind: NKind, m: impl Into<String>) { self.msg = Some((kind, m.into())); }

    // ---- settings and setup
    fn open_settings(&mut self, onboarding: bool, cx: &mut Context<Self>) {
        let (s, f) = (self.settings.clone(), self.fonts.clone());
        let v = cx.new(|cx| SettingsView::new(s, f, onboarding, cx));
        self._ssub = Some(cx.subscribe(&v, |this, v, ev: &SettingsEvent, cx| match ev {
            SettingsEvent::Changed => { let s = v.read(cx).settings.clone(); this.apply_settings(s, cx); }
            SettingsEvent::Close => { let s = v.read(cx).settings.clone(); this.apply_settings(s, cx); this.settings_view = None; this.mine_loaded = false; cx.notify(); }
        }));
        self.settings_view = Some(v); cx.notify();
    }
    fn apply_settings(&mut self, s: Settings, cx: &mut Context<Self>) {
        let theme = theme::load(&s.theme, Some(&model::themes_dir())); self.pal = Pal::new(&theme);
        if let Some(sv) = &self.settings_view { sv.update(cx, |v, _| v.set_pal(Pal::new(&theme))); }
        kit::REDUCED_MOTION.store(s.motion_reduced(), std::sync::atomic::Ordering::Relaxed);
        self.fonts = kit::pick_fonts_for(cx, &s); let fonts = self.fonts.clone();
        for t in &self.tabs { let (s2, f2) = (s.clone(), fonts.clone()); t.view.update(cx, |v, _| v.apply_settings(s2, f2)); }
        self.settings = s; cx.notify();
    }

    // ---- tabs
    fn finish_open(&mut self, res: Opened, cx: &mut Context<Self>) {
        self.busy = false;
        match res {
            Ok(m) => {
                let fonts = self.fonts.clone(); let settings = self.settings.clone();
                let title = m.title.clone();
                let v = cx.new(|_| View::new(m, settings, fonts));
                let key = v.read(cx).key_id();
                if let Some(i) = self.tabs.iter().position(|t| t.key == key) { self.active = Some(i); self.msg = None; cx.notify(); return; }
                let sub = Self::subscribe_view(&v, cx);
                v.update(cx, |v, cx| v.start_polling(cx));
                self.tabs.push(TabItem { key, title, ci: String::new(), view: v, _sub: sub }); self.active = Some(self.tabs.len() - 1); self.msg = None; self.token_host = None; self.refresh_recent();
            }
            Err(OpenErr::NeedsToken { host, why }) => { self.say(NKind::Error, format!("{why}. Paste an access token for {host} below, or sign in from Settings.")); self.token_host = Some(host); self.hf = Some(Hf::Token); }
            Err(OpenErr::Msg(m)) => self.say(NKind::Error, m),
        }
        cx.notify();
    }
    /// Moves one tab's review to the MR's current head (drafts re-anchored, changed files unmarked) and swaps in the new view.
    fn move_review_tab(&mut self, id: gpui::EntityId, url: String, old_head: String, cx: &mut Context<Self>) {
        self.toast(NKind::Info, "Fetching the latest commits…".to_string(), cx);
        let task = cx.background_executor().spawn(async move { Model::move_to_latest(&url, &old_head) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |t, cx| {
                match r {
                    Ok((m, msg)) => {
                        let Some(i) = t.tabs.iter().position(|x| x.view.entity_id() == id) else { return };
                        let (fonts, settings) = (t.fonts.clone(), t.settings.clone());
                        let v = cx.new(|_| View::new(m, settings, fonts)); let sub = Self::subscribe_view(&v, cx);
                        v.update(cx, |v, cx| v.start_polling(cx));
                        t.tabs[i].title = v.read(cx).tab_title(); t.tabs[i].view = v; t.tabs[i]._sub = sub;
                        t.toast(NKind::Success, msg, cx);
                    }
                    Err(OpenErr::NeedsToken { host, .. }) => t.toast(NKind::Error, format!("Sign in to {host} from Settings to move this review."), cx),
                    Err(OpenErr::Msg(m)) => t.toast(NKind::Error, m, cx),
                }
                cx.notify();
            });
        }).detach();
    }
    fn subscribe_view(v: &Entity<View>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(v, |this, v, ev: &ViewEvent, cx| match ev {
                    ViewEvent::MoveHead { url, old_head } => this.move_review_tab(v.entity_id(), url.clone(), old_head.clone(), cx),
                    ViewEvent::Fullscreen => cx.notify(),
                    ViewEvent::Home => { this.active = None; this.refresh_recent(); cx.notify(); }
                    ViewEvent::OpenSettings => this.open_settings(false, cx),
                    ViewEvent::Notice(k, t, key) => { this.toast(*k, t.clone(), cx); if key.map(|k| this.settings.desktop_event_on(k)).unwrap_or(false) { model::desktop_notify("Update", t); } }
                    ViewEvent::Renamed(t) => { let id = v.entity_id(); if let Some(tab) = this.tabs.iter_mut().find(|x| x.view.entity_id() == id) { tab.title = t.clone(); } this.refresh_recent(); cx.notify(); }
                    ViewEvent::Ci(c) => { let id = v.entity_id(); if let Some(tab) = this.tabs.iter_mut().find(|x| x.view.entity_id() == id) { tab.ci = c.clone(); } cx.notify(); }
        })
    }
    fn tab_index(&self, id: gpui::EntityId) -> Option<usize> { self.tabs.iter().position(|t| t.view.entity_id() == id) }
    /// Closes a tab, first asking what to do with the review when leaving would lose work.
    fn request_close_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(i) else { return };
        if t.view.read(cx).unfinished().is_some() { self.close_req = Some(CloseReq { view: t.view.entity_id(), quit: false }); self.active = Some(i); cx.notify(); }
        else { self.close_tab(i, cx); }
    }
    /// Called when the window is asked to close: true when it may close now, false while a question about an unfinished review is open.
    fn request_quit(&mut self, cx: &mut Context<Self>) -> bool {
        match self.tabs.iter().position(|t| t.view.read(cx).unfinished().is_some()) {
            None => true,
            Some(i) => { self.close_req = Some(CloseReq { view: self.tabs[i].view.entity_id(), quit: true }); self.active = Some(i); cx.notify(); false }
        }
    }
    /// Moves on after a choice: when quitting, ask about the next unfinished review, otherwise leave.
    fn after_close_choice(&mut self, quit: bool, cx: &mut Context<Self>) {
        if !quit { return; }
        if self.request_quit(cx) { cx.quit(); }
    }
    fn resolve_close(&mut self, choice: CloseChoice, cx: &mut Context<Self>) {
        let Some(req) = self.close_req.take() else { return };
        let Some(i) = self.tab_index(req.view) else { cx.notify(); return };
        match choice {
            CloseChoice::Cancel => {}
            CloseChoice::Submit => { self.active = Some(i); let v = self.tabs[i].view.clone(); v.update(cx, |v, cx| v.begin_finish(cx)); }
            CloseChoice::Keep => { self.close_tab(i, cx); self.after_close_choice(req.quit, cx); }
            CloseChoice::Discard => {
                let id = self.tabs[i].view.read(cx).review_id();
                if let Ok(s) = Store::open(&model::db_path()) { let _ = s.discard(id); }
                self.close_tab(i, cx); self.after_close_choice(req.quit, cx);
            }
        }
        cx.notify();
    }
    /// The "leave this review?" dialog.
    fn close_modal(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let req = self.close_req.as_ref()?; let i = self.tab_index(req.view)?; let p = self.pal;
        let v = self.tabs[i].view.read(cx); let (drafts, viewed, total) = v.unfinished()?; let remote = v.is_remote();
        let title = self.tabs[i].title.clone();
        let mut lines = vec![];
        if drafts > 0 { lines.push(format!("{drafts} comment draft{} not posted", if drafts == 1 { "" } else { "s" })); }
        if viewed < total { lines.push(format!("{viewed} of {total} files viewed")); }
        let hint = if self.close_req.as_ref().map(|r| r.quit).unwrap_or(false) { "Perusal is closing." } else { "" };
        Some(deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.55)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(div().flex().flex_col().gap_3().w(px(560.0)).p_5().rounded_lg().text_color(p.fg).font_family(self.fonts.ui.clone()).text_size(px(14.0)).border_1().border_color(p.border).bg(p.surface).shadow_lg()
                .child(div().flex().items_center().gap_2().child(icon(ic::WARNING, 22.0, p.warn)).child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("You have not finished this review")))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).truncate().child(title))
                .child(div().flex().flex_col().gap_1().text_sm().text_color(p.muted).children(lines.into_iter().map(|l| div().child(format!("• {l}")))).children((!hint.is_empty()).then(|| div().child(hint))))
                .child(div().text_xs().text_color(p.muted).child("Keep for later saves everything on this computer and lists the review under Recent reviews."))
                .child(div().flex().flex_wrap().justify_end().gap_2()
                    .child(icon_text_btn("cl-cancel", ic::CLOSE, "Cancel", Btn::Ghost, &p).on_click(cx.listener(|t, _, _, cx| t.resolve_close(CloseChoice::Cancel, cx))))
                    .child(icon_text_btn("cl-discard", ic::DELETE, "Discard", Btn::Secondary, &p).on_click(cx.listener(|t, _, _, cx| t.resolve_close(CloseChoice::Discard, cx))))
                    .child(icon_text_btn("cl-keep", ic::BOOKMARK_BORDER, "Keep for later", Btn::Secondary, &p).on_click(cx.listener(|t, _, _, cx| t.resolve_close(CloseChoice::Keep, cx))))
                    .child(icon_text_btn("cl-submit", ic::SEND, if remote { "Submit" } else { "Finish" }, Btn::Primary, &p).on_click(cx.listener(|t, _, _, cx| t.resolve_close(CloseChoice::Submit, cx)))))))
            .with_priority(60).into_any_element())
    }
    /// Drag-and-drop: moves tab `from` to position `to`.
    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() { return; }
        let t = self.tabs.remove(from); self.tabs.insert(to, t);
        self.active = perusal_app::tabs::reordered_active(self.active, from, to); cx.notify();
    }
    fn close_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if i >= self.tabs.len() { return; }
        self.tabs.remove(i);
        self.active = match self.active { Some(a) if a == i => if self.tabs.is_empty() { None } else { Some(i.min(self.tabs.len() - 1)) }, Some(a) if a > i => Some(a - 1), o => o };
        self.refresh_recent(); cx.notify();
    }
    fn cycle(&mut self, d: i32, cx: &mut Context<Self>) {
        let n = self.tabs.len() as i32 + 1; if n <= 1 { return; }
        let cur = self.active.map(|a| a as i32 + 1).unwrap_or(0); let next = (cur + d).rem_euclid(n);
        self.active = if next == 0 { None } else { Some(next as usize - 1) }; cx.notify();
    }

    // ---- opening
    fn spawn_open(&mut self, label: String, work: impl FnOnce() -> Opened + Send + 'static, cx: &mut Context<Self>) {
        self.busy = true; self.say(NKind::Info, label); cx.notify();
        let task = cx.background_executor().spawn(async move { work() });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| t.finish_open(r, cx)); }).detach();
    }
    fn open_url(&mut self, url: String, cx: &mut Context<Self>) {
        self.retry_url = Some(url.clone());
        self.spawn_open(format!("Opening {url}. The first open clones the repository, which can take a while for large projects."), move || Model::open_remote(&url), cx);
    }
    fn submit_url(&mut self, cx: &mut Context<Self>) {
        let u = model::normalize_url_input(&self.url.f.text);
        if u.is_empty() { self.say(NKind::Error, "Paste a pull request or merge request URL first."); cx.notify(); return; }
        self.open_url(u, cx);
    }
    fn open_local(&mut self, cx: &mut Context<Self>) {
        let (repo, branch, base) = (self.repo.f.text.trim().to_string(), self.branch.f.text.trim().to_string(), self.base.f.text.trim().to_string());
        if repo.is_empty() || branch.is_empty() { self.say(NKind::Error, "Enter a repository path and a branch name."); cx.notify(); return; }
        let res = Model::open_local(std::path::Path::new(&repo), &branch, if base.is_empty() { None } else { Some(&base) }); self.finish_open(res, cx);
    }
    fn open_key(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.iter().position(|t| t.key == key) { self.active = Some(i); cx.notify(); return; }
        self.spawn_open("Opening review...".into(), move || Model::reopen(&key), cx);
    }
    fn browse_list(&mut self, cx: &mut Context<Self>) {
        let t = self.browse.f.text.trim().to_string();
        if t.is_empty() { self.say(NKind::Error, "Enter host/owner/repo, for example github.com/rust-lang/rust."); cx.notify(); return; }
        self.busy = true; self.say(NKind::Info, format!("Listing open merge/pull requests of {t}...")); cx.notify();
        let task = cx.background_executor().spawn(async move { model::list_remote(&t) });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| {
            t.busy = false;
            match r {
                Ok(l) => { t.say(NKind::Success, format!("{} open merge/pull requests", l.len())); t.mrs = l; }
                Err(OpenErr::NeedsToken { host, why }) => { t.say(NKind::Error, format!("{why}. Paste an access token for {host} below, or sign in from Settings.")); t.token_host = Some(host); t.hf = Some(Hf::Token); }
                Err(OpenErr::Msg(m)) => t.say(NKind::Error, m),
            }
            cx.notify(); }); }).detach();
    }
    fn save_token(&mut self, cx: &mut Context<Self>) {
        use perusal_providers::auth::TokenStore;
        let Some(host) = self.token_host.clone() else { return };
        let tok = self.token.f.text.trim().to_string(); if tok.is_empty() { return; }
        match model::token_store().set(&host, &tok) {
            Ok(()) => { self.token.f.clear(); self.token_host = None; self.hf = Some(Hf::Url); self.say(NKind::Success, format!("Token saved for {host}."));
                if self.otab == OTab::Browse { self.browse_list(cx); return; }
                if let Some(u) = self.retry_url.clone() { self.open_url(u, cx); return; } }
            Err(e) => self.say(NKind::Error, format!("Cannot save the token: {e}")),
        }
        cx.notify();
    }
    fn load_mine(&mut self, cx: &mut Context<Self>) {
        if self.mine_loading { return; }
        let signed = self.settings.hosts.iter().any(|h| model::token_for(&h.host).is_some());
        self.mine_loaded = true; if !signed { self.mine.clear(); self.mine_errs.clear(); cx.notify(); return; }
        self.mine_loading = true; cx.notify();
        let (hosts, role, state) = (self.settings.hosts.clone(), self.mine_role, self.mine_state);
        let task = cx.background_executor().spawn(async move { model::fetch_my_reviews(&hosts, role, state) });
        cx.spawn(async move |this, cx| { let (l, e, me) = task.await; let _ = this.update(cx, |t, cx| { t.mine = l; t.mine_errs = e; t.mine_me = me; t.mine_loading = false; cx.notify(); }); }).detach();
    }

    // ---- input plumbing
    fn inp(&mut self, f: Hf) -> &mut Input { match f { Hf::Url => &mut self.url, Hf::Repo => &mut self.repo, Hf::Branch => &mut self.branch, Hf::Base => &mut self.base, Hf::Browse => &mut self.browse, Hf::Token => &mut self.token, Hf::Rename => &mut self.rename, Hf::RSearch => &mut self.r_search, Hf::MSearch => &mut self.m_search, Hf::DSearch => &mut self.dd_search } }
    fn field_ids(&self) -> Vec<Hf> {
        let mut v = match self.otab { OTab::Url => vec![Hf::Url], OTab::Local => vec![Hf::Repo, Hf::Branch, Hf::Base], OTab::Browse => vec![Hf::Browse] };
        if self.token_host.is_some() { v.push(Hf::Token); } v
    }
    fn enter(&mut self, cx: &mut Context<Self>) {
        match self.hf { Some(Hf::Url) => self.submit_url(cx), Some(Hf::Repo | Hf::Branch | Hf::Base) => self.open_local(cx), Some(Hf::Browse) => self.browse_list(cx), Some(Hf::Token) => self.save_token(cx), Some(Hf::Rename) => self.commit_rename(cx), Some(Hf::RSearch | Hf::MSearch | Hf::DSearch) => { self.hf = None; cx.notify(); } None => {} }
    }
    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.renaming.take() { let t = self.rename.f.text.trim().to_string(); if !t.is_empty() { if let Ok(s) = Store::open(&model::db_path()) { let _ = s.rename_review(id, &t); } } self.refresh_recent(); }
        self.hf = None; cx.notify();
    }
    fn home_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.busy { return; }
        let ks = &ev.keystroke; let Some(f) = self.hf else { return };
        match ks.key.as_str() {
            "tab" if f != Hf::Rename => { let v = self.field_ids(); let i = v.iter().position(|x| *x == f).unwrap_or(0); self.hf = Some(v[if ks.modifiers.shift { (i + v.len() - 1) % v.len() } else { (i + 1) % v.len() }]); }
            "enter" => { self.enter(cx); return; }
            "escape" => { if f == Hf::Rename { self.renaming = None; } self.hf = None; }
            _ => { let secret_free = self.inp(f); kit::field_key(&mut secret_free.f, ks, cx); }
        }
        cx.notify();
    }

    // ---- UI pieces
    fn finput(&mut self, id: &'static str, f: Hf, label: &'static str, ph: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal; let focused = self.hf == Some(f); let secret = f == Hf::Token; let fam = self.fonts.ui.clone();
        let inp = match f { Hf::Url => &self.url, Hf::Repo => &self.repo, Hf::Branch => &self.branch, Hf::Base => &self.base, Hf::Browse => &self.browse, Hf::Token => &self.token, Hf::Rename => &self.rename, Hf::RSearch => &self.r_search, Hf::MSearch => &self.m_search, Hf::DSearch => &self.dd_search };
        let el = div().id(id).flex().items_center().px_3().py(px(6.0)).min_h(px(32.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).text_color(p.fg).child(input_text(inp, focused, ph, &p, secret, 1));
        let el = match f {
            Hf::Url => wire_input(el, cx, |t| &mut t.url, |t, _| t.hf = Some(Hf::Url), fam, false), Hf::Repo => wire_input(el, cx, |t| &mut t.repo, |t, _| t.hf = Some(Hf::Repo), fam, false),
            Hf::Branch => wire_input(el, cx, |t| &mut t.branch, |t, _| t.hf = Some(Hf::Branch), fam, false), Hf::Base => wire_input(el, cx, |t| &mut t.base, |t, _| t.hf = Some(Hf::Base), fam, false),
            Hf::Browse => wire_input(el, cx, |t| &mut t.browse, |t, _| t.hf = Some(Hf::Browse), fam, false), Hf::Token => wire_input(el, cx, |t| &mut t.token, |t, _| t.hf = Some(Hf::Token), fam, true),
            Hf::Rename => wire_input(el, cx, |t| &mut t.rename, |t, _| t.hf = Some(Hf::Rename), fam, false),
            Hf::RSearch | Hf::MSearch | Hf::DSearch => el,
        };
        div().flex().flex_col().gap_1().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label)).child(el)
    }
    /// Search field shown above the Home lists: a magnifier, the text, and a clear button while there is text.
    fn search_box(&mut self, id: &'static str, f: Hf, ph: &'static str, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.pal; let focused = self.hf == Some(f); let fam = self.fonts.ui.clone();
        let (inp, has) = match f { Hf::RSearch => (&self.r_search, !self.r_search.f.text.is_empty()), _ => (&self.m_search, !self.m_search.f.text.is_empty()) };
        let el = div().id(id).flex().flex_1().min_w_0().items_center().gap_2().px_3().py(px(5.0)).min_h(px(32.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).text_color(p.fg)
            .child(icon(ic::SEARCH, 16.0, p.muted)).child(div().flex_1().min_w_0().child(input_text(inp, focused, ph, &p, false, 1)));
        let el = if f == Hf::RSearch { wire_input(el, cx, |t| &mut t.r_search, |t, _| t.hf = Some(Hf::RSearch), fam, false) } else { wire_input(el, cx, |t| &mut t.m_search, |t, _| t.hf = Some(Hf::MSearch), fam, false) };
        div().flex().items_center().gap_1().px_4().pt_2().child(el)
            .when(has, |d| d.child(icon_btn(SharedString::from(format!("{id}-clear")), ic::CLOSE, "Clear the search", Btn::Ghost, &p).on_click(cx.listener(move |t, _, _, cx| { if f == Hf::RSearch { t.r_search.f.clear(); } else { t.m_search.f.clear(); } cx.notify(); }))))
    }
    fn chip(&self, id: impl Into<gpui::ElementId>, label: String, on: bool) -> gpui::Stateful<gpui::Div> { self.chip_with(id, None, label, on) }
    /// A filter chip with an optional leading picture (used for people).
    fn chip_with(&self, id: impl Into<gpui::ElementId>, lead: Option<gpui::AnyElement>, label: String, on: bool) -> gpui::Stateful<gpui::Div> {
        let p = self.pal;
        div().id(id).flex().items_center().gap_1().px_3().when(lead.is_some(), |d| d.pl_1()).py(px(3.0)).rounded_full().border_1().border_color(if on { p.accent } else { p.border }).bg(if on { p.accent.opacity(0.14) } else { gpui::transparent_black() }).text_color(if on { p.fg } else { p.muted }).text_xs().font_weight(FontWeight::MEDIUM).cursor_pointer().hover(move |s| s.bg(p.hover)).children(lead).child(label)
    }
    fn state_dot(&self, s: &str) -> gpui::Hsla { let p = self.pal; match s { "merged" => p.ai, "closed" => p.danger, "open" => p.success, _ => p.accent } }

    fn tab_strip(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let home_on = self.active.is_none();
        let home = div().id("tab-home").flex().items_center().gap_2().px_3().h(px(36.0)).cursor_pointer().border_b_2().border_color(if home_on { p.accent } else { gpui::transparent_black() }).bg(if home_on { p.bg } else { gpui::transparent_black() }).hover(move |s| s.bg(p.hover))
            .child(icon(ic::HOME, 17.0, if home_on { p.accent } else { p.muted })).child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(if home_on { p.fg } else { p.muted }).child("Home")).on_click(cx.listener(|this, _, _, cx| { this.active = None; cx.notify(); }));
        let tabs: Vec<_> = self.tabs.iter().enumerate().map(|(i, t)| {
            let on = self.active == Some(i);
            let ci = kit::ci_badge(&t.ci, &p).map(|_| { let c = match t.ci.as_str() { "passed" => p.success, "failed" => p.danger, "running" => p.warn, _ => p.muted }; div().size(px(8.0)).rounded_full().bg(c) });
            div().id(("tab", i)).group("tab").flex().items_center().gap_2().pl_3().pr_1().h(px(36.0)).max_w(px(240.0)).min_w(px(110.0)).cursor_pointer().border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() }).bg(if on { p.bg } else { gpui::transparent_black() }).hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| { this.active = Some(i); cx.notify(); }))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| { this.request_close_tab(i, cx); cx.stop_propagation(); }))
                .on_drag(DraggedTab { from: i, title: t.title.clone().into(), pal: p }, |d, _, _, cx| cx.new(|_| DragTab { t: d.clone() }))
                .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(p.accent.opacity(0.22)))
                .on_drop(cx.listener(move |this, d: &DraggedTab, _, cx| this.move_tab(d.from, i, cx)))
                .children(ci).child(div().flex_1().min_w_0().truncate().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).child(t.title.clone()))
                .child(icon_btn(("tabx", i), ic::CLOSE, "Close this tab (Ctrl+W)", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.request_close_tab(i, cx); cx.stop_propagation(); })))
        }).collect();
        let custom = self.settings.custom_titlebar;
        let max = window.is_maximized();
        let ctl = |id: &'static str, i: ic::Icon, tip_t: &'static str, danger: bool, cx: &mut Context<Self>, f: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            let hv = if danger { gpui::rgb(0xe81123).into() } else { p.hover };
            kit::tip(div().id(id).flex().items_center().justify_center().w(px(46.0)).h(px(36.0)).cursor_pointer().hover(move |x| x.bg(hv)).child(icon(i, 16.0, p.fg))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, w, cx| { f(this, w, cx); cx.notify(); })), tip_t, &p)
        };
        // the empty part of the strip drags the window; double click maximizes
        let drag = div().id("strip-drag").flex_1().h(px(36.0)).when(custom, |d| d.on_mouse_down(MouseButton::Left, cx.listener(|_, ev: &gpui::MouseDownEvent, w, _| { if ev.click_count >= 2 { w.zoom_window(); } else { w.start_window_move(); } })));
        div().flex().items_end().w_full().bg(p.surface).border_b_1().border_color(p.border).child(home).children(tabs).child(drag)
            .when(custom, |d| d.child(ctl("win-min", ic::MINIMIZE, "Minimize", false, cx, |_, w, _| w.minimize_window()))
                .child(ctl("win-max", if max { ic::FILTER_NONE } else { ic::CROP_SQUARE }, if max { "Restore" } else { "Maximize" }, false, cx, |_, w, _| w.zoom_window()))
                .child(kit::tip(div().id("win-close").flex().items_center().justify_center().w(px(46.0)).h(px(36.0)).cursor_pointer().hover(|x| x.bg(gpui::rgb(0xe81123))).child(icon(ic::CLOSE, 16.0, p.fg))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, w, cx| { if this.request_quit(cx) { w.remove_window(); } cx.notify(); })), "Close", &p)))
    }

    fn recent_filtered(&self) -> Vec<RecentRow> {
        self.recent.iter().filter(|r| {
            let (is_remote, kind) = match r.key.strip_prefix("remote:") { Some(u) => (true, perusal_app::hosts::kind_of(&self.settings.hosts, u.trim_start_matches("https://").split('/').next().unwrap_or(""))), None => (false, "") };
            let ok_remote = match self.r_remote { Remote::All => true, Remote::Local => !is_remote, Remote::GitHub => is_remote && kind == "github", Remote::GitLab => is_remote && kind == "gitlab" };
            let ok_state = match self.r_state { St::All => true, St::Open => r.mr_state == "open", St::Merged => r.mr_state == "merged", St::Closed => r.mr_state == "closed" };
            let src = r.key.trim_start_matches("remote:").trim_start_matches("local:");
            ok_remote && ok_state && perusal_app::search::matches(&self.r_search.f.text, &[&r.title, src, &r.mr_state])
        }).cloned().collect()
    }

    fn render_recent(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.pal;
        let rows: Vec<_> = self.recent_filtered().into_iter().enumerate().map(|(i, r)| {
            let (key, id, pinned) = (r.key.clone(), r.id, r.pinned);
            let remote = r.key.strip_prefix("remote:").map(|s| s.trim_start_matches("https://").to_string());
            let src: SharedString = remote.clone().unwrap_or_else(|| r.key.trim_start_matches("local:").to_string()).into();
            let frac = if r.total == 0 { 0.0 } else { r.viewed as f32 / r.total as f32 };
            let dot = if remote.is_some() { self.state_dot(&r.mr_state) } else { p.accent };
            let renaming = self.renaming == Some(id);
            let title: gpui::AnyElement = if renaming {
                let fam = self.fonts.ui.clone();
                let el = div().id(("rn", i)).flex().items_center().px_2().py(px(3.0)).w(px(320.0)).rounded_md().border_1().border_color(p.accent).bg(p.bg).child(input_text(&self.rename, true, "Name this review", &p, false, 1));
                wire_input(el, cx, |t| &mut t.rename, |t, _| t.hf = Some(Hf::Rename), fam, false).into_any_element()
            } else {
                div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.fg).truncate().child(r.title.clone())).when(pinned, |d| d.child(badge("pinned", p.warn, p.warn.opacity(0.15))))
                    .when(!r.mr_state.is_empty() && remote.is_some(), |d| d.child(badge(match r.mr_state.as_str() { "merged" => "Merged", "closed" => "Closed", _ => "Open" }, dot, dot.opacity(0.15)))).into_any_element()
            };
            let title_text = r.title.clone();
            div().id(("recent", i)).flex().items_center().gap_3().px_4().py_3().border_b_1().border_color(p.border).cursor_pointer().hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| { if this.renaming.is_none() { this.open_key(key.clone(), cx); } }))
                .child(div().size(px(10.0)).rounded_full().bg(dot).flex_none())
                .child(div().flex().flex_col().flex_1().min_w_0().gap(px(2.0)).child(title).child(div().text_xs().text_color(p.muted).truncate().child(src)))
                .child(div().flex().flex_col().items_end().gap(px(4.0)).w(px(140.0)).flex_none()
                    .child(div().flex().items_center().gap_2().when(r.drafts > 0, |d| d.child(badge(format!("{} drafts", r.drafts), p.accent, p.accent.opacity(0.15)))).child(div().text_xs().text_color(p.muted).child(format!("{}/{} viewed", r.viewed, r.total))))
                    .child(div().w_full().h(px(4.0)).rounded_full().bg(p.border).child(div().h_full().rounded_full().bg(p.success).w(gpui::relative(frac)))))
                .child(div().flex().gap_1().flex_none()
                    .child(icon_btn(("ren", i), ic::EDIT, "Rename this review", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.renaming = Some(id); this.rename.f.set(&title_text); this.hf = Some(Hf::Rename); cx.stop_propagation(); cx.notify(); })))
                    .child(icon_btn(("pin", i), ic::PUSH_PIN, if pinned { "Unpin this review" } else { "Pin this review" }, if pinned { Btn::Secondary } else { Btn::Ghost }, &p).on_click(cx.listener(move |this, _, _, cx| { if let Ok(s) = Store::open(&model::db_path()) { let _ = s.set_pinned(id, !pinned); } this.refresh_recent(); cx.stop_propagation(); cx.notify(); })))
                    .child(icon_btn(("del", i), ic::DELETE, "Delete this review and its drafts", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { if let Ok(s) = Store::open(&model::db_path()) { let _ = s.discard(id); } this.refresh_recent(); cx.stop_propagation(); cx.notify(); }))))
        }).collect();
        let empty = rows.is_empty(); let any = !self.recent.is_empty();
        let sbox = self.search_box("recent-search", Hf::RSearch, "Search recent reviews by title, repository or branch", cx);
        let chips = |this: &Self| div().flex().flex_wrap().items_center().gap_2().px_4().py_2().border_b_1().border_color(p.border)
            .child(div().text_xs().text_color(p.muted).child("Remote"))
            .children([(Remote::All, "All"), (Remote::Local, "Local"), (Remote::GitHub, "GitHub"), (Remote::GitLab, "GitLab")].map(|(r, l)| this.chip(("rr", r as usize), l.into(), this.r_remote == r).on_click(cx.listener(move |t, _, _, cx| { t.r_remote = r; cx.notify(); }))))
            .child(div().w(px(10.0))).child(div().text_xs().text_color(p.muted).child("State"))
            .children([(St::All, "All"), (St::Open, "Open"), (St::Merged, "Merged"), (St::Closed, "Closed")].map(|(s, l)| this.chip(("rs", s as usize), l.into(), this.r_state == s).on_click(cx.listener(move |t, _, _, cx| { t.r_state = s; cx.notify(); }))));
        div().flex().flex_col().flex_1().min_h_0().child(sbox).child(chips(self))
            .child(div().id("recent-list").flex_1().min_h_0().overflow_y_scroll().bg(p.bg)
                .when(empty, |d| d.child(div().flex().flex_col().items_center().justify_center().gap_2().py(px(70.0)).text_color(p.muted)
                    .child(div().text_xl().child(if any { "No reviews match" } else { "Nothing here yet" })).child(div().text_sm().child(if any { "Clear a filter to see more." } else { "Reviews you open appear here, with progress and drafts saved." }))))
                .children(rows)).into_any_element()
    }

    /// A multi-select dropdown (author, assignee or label): choices are OR-ed, and only values still present after the other filters are offered.
    fn multi_dd(&self, d: Dim, name: &'static str, items: &[perusal_app::search::Item], cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.pal; let open = self.mine_dd == Some(d); let fam = self.fonts.ui.clone();
        let remote = match self.mine_remote { Remote::GitHub => Some("github"), Remote::GitLab => Some("gitlab"), _ => None };
        let people = d != Dim::Label; let q = self.dd_search.f.text.to_lowercase();
        let mut opts = mine::options(items, d, remote, &self.mine_sel, &self.m_search.f.text);
        if people { opts.insert(0, mine::ME.to_string()); }
        let opts: Vec<String> = opts.into_iter().filter(|o| q.is_empty() || (if o == mine::ME { "me".to_string() } else { o.to_lowercase() }).contains(&q)).collect();
        let sel = self.mine_sel.set(d).clone();
        let id = d as usize;
        let rows: Vec<_> = opts.iter().enumerate().map(|(i, o)| {
            let on = sel.contains(o); let v = o.clone();
            let label = if o == mine::ME { "Me".to_string() } else { o.clone() };
            div().id(("mdd-item", id * 1000 + i)).flex().items_center().gap_2().px_3().py(px(6.0)).cursor_pointer().hover(move |x| x.bg(p.hover)).text_sm()
                .child(icon(if on { ic::CHECK_BOX } else { ic::CHECK_BOX_OUTLINE_BLANK }, 18.0, if on { p.accent } else { p.muted }))
                .children(people.then(|| kit::avatar(&if o == mine::ME { self.mine_me.values().next().cloned().unwrap_or_else(|| "me".into()) } else { o.clone() }, 20.0)))
                .child(div().flex_1().min_w_0().truncate().child(label))
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .on_click(cx.listener(move |t, _, _, cx| { t.mine_sel.toggle(d, &v); cx.notify(); }))
        }).collect();
        let focused = self.hf == Some(Hf::DSearch);
        let search = wire_input(div().id(("mdd-search", id)).flex().items_center().gap_2().mx_2().my_1().px_2().py(px(4.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg)
            .child(icon(ic::SEARCH, 15.0, p.muted)).child(div().flex_1().min_w_0().child(input_text(&self.dd_search, focused, "Filter", &p, false, 1))), cx, |t| &mut t.dd_search, |t, _| t.hf = Some(Hf::DSearch), fam, false);
        let empty = rows.is_empty();
        div().relative().child(kit::dd_button(("mdd", id), mine::summary(name, &sel), open, &p).min_w(px(130.0))
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .on_click(cx.listener(move |t, _, _, cx| { if t.mine_dd == Some(d) { t.mine_dd = None; t.hf = None; } else { t.mine_dd = Some(d); t.dd_search.f.clear(); t.hf = Some(Hf::DSearch); } cx.notify(); })))
            .children(open.then(|| deferred(div().absolute().top(px(36.0)).left_0().w(px(280.0)).flex().flex_col().rounded_md().border_1().border_color(p.border).bg(p.surface).shadow_lg().py_1().occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(search).child(div().id(("mdd-list", id)).max_h(px(300.0)).overflow_y_scroll().children(rows)
                    .when(empty, |x| x.child(div().px_3().py_2().text_xs().text_color(p.muted).child("Nothing to choose from with the current filters")))))
                .with_priority(30))).into_any_element()
    }

    fn render_mine(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.pal;
        let signed = self.settings.hosts.iter().any(|h| model::token_for(&h.host).is_some());
        let sbox = self.search_box("mine-search", Hf::MSearch, "Search text, or author: assignee: label: is:draft", cx);
        let items: Vec<perusal_app::search::Item> = self.mine.iter().map(|m| mr_item(m, &self.mine_me)).collect();
        let remote = match self.mine_remote { Remote::GitHub => Some("github"), Remote::GitLab => Some("gitlab"), _ => None };
        let (sel, query) = (self.mine_sel.clone(), self.m_search.f.text.clone());
        let rows: Vec<_> = self.mine.iter().zip(&items).filter(|(_, it)| mine::passes(it, remote, &sel, &query, None)).map(|(m, _)| m.clone()).enumerate().map(|(i, m)| {
            let url = m.url.clone(); let dot = self.state_dot(&m.state);
            let when: String = m.updated.chars().take(10).collect();
            let assignees: Vec<_> = m.assignees.iter().take(3).map(|a| kit::avatar(&a.login, 20.0)).collect();
            div().id(("mine", i)).flex().items_center().gap_3().px_4().py_3().border_b_1().border_color(p.border).cursor_pointer().hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| { this.open_url(url.clone(), cx); }))
                .child(kit::avatar(&m.author, 28.0))
                .child(div().flex().flex_col().flex_1().min_w_0().gap(px(2.0))
                    .child(div().flex().items_center().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(m.title.clone()))
                        .when(m.draft, |d| d.child(badge("Draft", p.muted, p.hover)))
                        .child(badge(match m.state.as_str() { "merged" => "Merged", "closed" => "Closed", _ => "Open" }, dot, dot.opacity(0.15)))
                        .children(m.labels.iter().take(3).map(|l| badge(l.clone(), p.accent, p.accent.opacity(0.12)))))
                    .child(div().flex().items_center().gap_1().text_xs().text_color(p.muted).child(icon(if m.kind == perusal_providers::Kind::GitLab { ic::CALL_MERGE } else { ic::PUBLIC }, 13.0, p.muted))
                        .child(div().truncate().child(format!("{} · {}#{} · by {} · updated {when}", m.host, m.project, m.number, m.author)))))
                .when(!assignees.is_empty(), |d| d.child(div().flex().items_center().gap(px(-4.0)).children(assignees)))
                .child(icon(ic::CHEVRON_RIGHT, 18.0, p.muted))
        }).collect();
        let n = rows.len();
        let (dd_author, dd_assignee, dd_label) = (self.multi_dd(Dim::Author, "Author", &items, cx), self.multi_dd(Dim::Assignee, "Assignee", &items, cx), self.multi_dd(Dim::Label, "Label", &items, cx));
        let reviewer_on = self.mine_role == Role::ReviewRequested;
        let any_sel = !self.mine_sel.is_empty();
        let chips = |this: &Self| div().flex().flex_col().gap_2().px_4().py_2().border_b_1().border_color(p.border)
            .child(div().flex().flex_wrap().items_center().gap_2().child(dd_author).child(dd_assignee).child(dd_label)
                .child(this.chip_with("m-rev", Some(icon(if reviewer_on { ic::CHECK_BOX } else { ic::CHECK_BOX_OUTLINE_BLANK }, 16.0, if reviewer_on { p.accent } else { p.muted }).into_any_element()), "Assigned to me as reviewer".into(), reviewer_on)
                    .on_click(cx.listener(|t, _, _, cx| { t.mine_role = if t.mine_role == Role::ReviewRequested { Role::Reviewable } else { Role::ReviewRequested }; t.load_mine(cx); })))
                .children(any_sel.then(|| this.chip("m-clear", "Clear filters".into(), false).on_click(cx.listener(|t, _, _, cx| { t.mine_sel = Sel::default(); cx.notify(); }))))
                .child(div().flex_1()).child(icon_btn("mine-refresh", ic::REFRESH, "Refresh", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.mine_loading = false; this.load_mine(cx); }))))
            .child(div().flex().flex_wrap().items_center().gap_2()
                .child(div().text_xs().text_color(p.muted).child("Remote"))
                .children([(Remote::All, "All"), (Remote::GitHub, "GitHub"), (Remote::GitLab, "GitLab")].map(|(r, l)| this.chip(("mrr", r as usize), l.into(), this.mine_remote == r).on_click(cx.listener(move |t, _, _, cx| { t.mine_remote = r; cx.notify(); }))))
                .child(div().w(px(10.0))).child(div().text_xs().text_color(p.muted).child("State"))
                .children([(StateFilter::Open, "Open"), (StateFilter::Merged, "Merged"), (StateFilter::Closed, "Closed"), (StateFilter::All, "All")].map(|(s, l)| this.chip(("mrs", s as usize), l.into(), this.mine_state == s).on_click(cx.listener(move |t, _, _, cx| { t.mine_state = s; t.load_mine(cx); })))))
            .children(reviewer_on.then(|| div().text_xs().text_color(p.muted).child("GitHub has no dependable reviewer list, so GitHub still lists every pull request you can review.")));
        let errs: Vec<_> = self.mine_errs.iter().map(|(h, e)| div().flex().items_start().gap_2().px_4().py_2().text_sm().text_color(p.danger).child(icon(ic::ERROR_OUTLINE, 16.0, p.danger)).child(div().flex_1().min_w_0().child(format!("{h}: {e}"))).child(kit::copy_btn("mine-err-copy", format!("{h}: {e}"), &p))).collect();
        div().flex().flex_col().flex_1().min_h_0().child(sbox).child(chips(self)).children(errs)
            .child(div().id("mine-list").flex_1().min_h_0().overflow_y_scroll().bg(p.bg)
                .when(!signed, |d| d.child(div().flex().flex_col().items_center().justify_center().gap_3().py(px(60.0)).px_6().text_color(p.muted)
                    .child(icon(ic::LOGIN, 36.0, p.muted)).child(div().text_lg().text_color(p.fg).child("Sign in to see your reviews")).child(div().text_sm().child("Connect GitHub or GitLab and your pull and merge requests appear here, with the same filters those sites have."))
                    .child(icon_text_btn("mine-signin", ic::KEY, "Open Settings to sign in", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_settings(false, cx); })))))
                .when(signed && self.mine_loading, |d| d.child(div().flex().items_center().gap_3().px_4().py_4().text_color(p.accent).child(loading_dots(&p)).child("Loading your reviews…")))
                .when(signed && !self.mine_loading && n == 0 && self.mine_errs.is_empty(), |d| d.child(div().px_4().py(px(40.0)).text_color(p.muted).child("Nothing here with this search and these filters.")))
                .children(rows)).into_any_element()
    }

    fn render_open_card(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let tab_btn = |this: &Self, t: OTab, label: &'static str, id: &'static str, i: ic::Icon, cx: &mut Context<Self>| {
            let on = this.otab == t;
            div().id(id).flex().items_center().gap_2().px_3().py_2().cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() }).hover(move |s| s.text_color(p.fg))
                .child(icon(i, 17.0, if on { p.accent } else { p.muted })).child(label).on_click(cx.listener(move |t2, _, _, cx| { t2.otab = t; t2.hf = t2.field_ids().first().copied(); cx.notify(); }))
        };
        let tabs = div().flex().border_b_1().border_color(p.border).child(tab_btn(self, OTab::Url, "Pull / merge request", "t-url", ic::LINK, cx)).child(tab_btn(self, OTab::Local, "Local branch", "t-local", ic::FOLDER_OPEN, cx)).child(tab_btn(self, OTab::Browse, "Browse remote", "t-browse", ic::SEARCH, cx));
        let form = match self.otab {
            OTab::Url => div().flex().flex_col().gap_3().child(self.finput("f-url", Hf::Url, "URL", "https://github.com/owner/repo/pull/123", cx))
                .child(div().text_xs().text_color(p.muted).child("GitHub, GitHub Enterprise, GitLab and self-hosted GitLab. Public repositories open without signing in. Paste with Ctrl+V."))
                .child(icon_text_btn("go-url", ic::ROCKET_LAUNCH, "Open review", Btn::Primary, &p).on_click(cx.listener(|t, _, _, cx| { if !t.busy { t.submit_url(cx) } }))).into_any_element(),
            OTab::Local => div().flex().flex_col().gap_3().child(self.finput("f-repo", Hf::Repo, "Repository path", "/path/to/repo", cx)).child(self.finput("f-branch", Hf::Branch, "Branch to review", "my-feature-branch", cx))
                .child(self.finput("f-base", Hf::Base, "Base branch or commit (optional)", "auto: merge-base with the default branch", cx))
                .child(div().text_xs().text_color(p.muted).child("Reviews the branch against its base. Leave the base empty to use the merge-base with the default branch. Comments stay local."))
                .child(icon_text_btn("go-local", ic::ROCKET_LAUNCH, "Open review", Btn::Primary, &p).on_click(cx.listener(|t, _, _, cx| { if !t.busy { t.open_local(cx) } }))).into_any_element(),
            OTab::Browse => {
                let mrs: Vec<_> = self.mrs.iter().enumerate().map(|(i, (n, title, url))| {
                    let url = url.clone();
                    div().id(("mr", i)).flex().items_center().gap_2().px_3().py_2().border_b_1().border_color(p.border).cursor_pointer().hover(move |s| s.bg(p.hover))
                        .child(div().size(px(8.0)).rounded_full().bg(p.success).flex_none()).child(div().text_color(p.muted).flex_none().child(format!("#{n}"))).child(div().flex_1().min_w_0().truncate().child(title.clone()))
                        .on_click(cx.listener(move |t, _, _, cx| { t.url.f.set(&url); t.open_url(url.clone(), cx) }))
                }).collect();
                div().flex().flex_col().gap_3().child(self.finput("f-browse", Hf::Browse, "Repository", "github.com/owner/repo", cx))
                    .child(icon_text_btn("go-browse", ic::SEARCH, "List open requests", Btn::Secondary, &p).on_click(cx.listener(|t, _, _, cx| { if !t.busy { t.browse_list(cx) } })))
                    .child(div().id("mr-list").max_h(px(260.0)).overflow_y_scroll().rounded_md().border_1().border_color(p.border).children(mrs)).into_any_element()
            }
        };
        let token = self.token_host.clone().map(|h| div().flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(p.warn).bg(p.warn.opacity(0.08))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("Access token for {h}")))
            .child(self.finput("f-token", Hf::Token, "Personal access token", "paste token, stored locally with 0600 permissions", cx))
            .child(div().flex().gap_2().child(icon_text_btn("go-token", ic::CHECK, "Save and retry", Btn::Primary, &p).on_click(cx.listener(|t, _, _, cx| t.save_token(cx))))
                .child(icon_text_btn("go-signin", ic::KEY, "More sign-in options", Btn::Secondary, &p).on_click(cx.listener(|t, _, _, cx| t.open_settings(false, cx))))));
        let msg = self.msg.clone().map(|(k, m)| { let (c, i) = match k { NKind::Error => (p.danger, ic::ERROR_OUTLINE), NKind::Success => (p.success, ic::CHECK_CIRCLE), _ => (p.accent, ic::INFO) }; (c, i, m, k == NKind::Error) });
        let msg_el = msg.map(|(c, i, m, is_err)| div().flex().items_start().gap_2().px_3().py_2().rounded_md().border_1().border_color(c).bg(c.opacity(0.1)).text_sm().text_color(c).child(icon(i, 17.0, c)).child(div().flex_1().min_w_0().child(m.clone())).children(is_err.then(|| kit::copy_btn("home-msg-copy", m, &p))));
        div().flex().flex_col().w(px(480.0)).flex_none().rounded_lg().border_1().border_color(p.border).bg(p.surface).overflow_hidden()
            .child(div().px_4().py_3().border_b_1().border_color(p.border).font_weight(FontWeight::SEMIBOLD).child("Open a review")).child(div().px_2().child(tabs))
            .child(div().flex().flex_col().gap_4().p_4().child(form).children(token).children(msg_el))
    }

    fn render_home(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let left_tabs = div().flex().items_center().border_b_1().border_color(p.border).bg(p.surface).px_2()
            .child({ let on = self.htab == HTab::Recent; div().id("h-recent").flex().items_center().gap_2().px_3().py_3().cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() })
                .child(icon(ic::HISTORY, 17.0, if on { p.accent } else { p.muted })).child("Recent reviews").child(badge(format!("{}", self.recent.len()), p.muted, p.hover)).on_click(cx.listener(|t, _, _, cx| { t.htab = HTab::Recent; cx.notify(); })) })
            .child({ let on = self.htab == HTab::Mine; div().id("h-mine").flex().items_center().gap_2().px_3().py_3().cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() })
                .child(icon(ic::FORUM, 17.0, if on { p.accent } else { p.muted })).child("My reviews").on_click(cx.listener(|t, _, _, cx| { t.htab = HTab::Mine; if !t.mine_loaded { t.load_mine(cx); } cx.notify(); })) });
        let list = if self.htab == HTab::Recent { self.render_recent(cx) } else { self.render_mine(cx) };
        let open = self.render_open_card(cx);
        div().flex().flex_col().flex_1().min_h_0().bg(p.bg).text_color(p.fg)
            .child(div().flex().items_center().gap_3().px_6().h(px(52.0)).border_b_1().border_color(p.border).bg(p.surface)
                .child(kit::logo(32.0))
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Perusal")).child(div().flex_1().text_sm().text_color(p.muted).child("Review pull requests and merge requests, one batch at a time"))
                .child(icon_btn("home-settings", ic::SETTINGS, "Settings", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.open_settings(false, cx); }))))
            .child(div().flex().flex_1().min_h_0().justify_center().p_6().child(div().flex().flex_1().min_h_0().max_w(px(1240.0)).gap_6()
                .child(div().flex().flex_col().flex_1().min_w_0().min_h_0().rounded_lg().border_1().border_color(p.border).bg(p.surface).overflow_hidden().child(left_tabs).child(list))
                .child(open)))
    }

    fn toast_stack(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.toasts.is_empty() { return None; }
        let p = self.pal;
        let items: Vec<_> = self.toasts.items().iter().map(|t| {
            let (c, i) = match t.kind { NKind::Success => (p.success, ic::CHECK_CIRCLE), NKind::Error => (p.danger, ic::ERROR_OUTLINE), NKind::Warning => (p.warn, ic::WARNING), NKind::Info => (p.accent, ic::NOTIFICATIONS) };
            let id = t.id;
            div().id(("toast", id as usize)).text_color(p.fg).flex().items_start().gap_3().w(px(380.0)).px_4().py_3().rounded_lg().bg(p.surface).border_1().border_color(p.border).border_l_4().border_color(c).shadow_lg()
                .child(icon(i, 19.0, c)).child(div().flex_1().min_w_0().text_sm().child(t.text.clone()))
                .children((t.kind == NKind::Error).then(|| kit::copy_btn(("toast-copy", id as usize), t.text.clone(), &p)))
                .child(icon_btn(("toast-x", id as usize), ic::CLOSE, "Dismiss", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.toasts.dismiss(id); cx.notify(); })))
        }).collect();
        Some(deferred(div().absolute().bottom(px(44.0)).right(px(16.0)).flex().flex_col().gap_2().occlude().children(items)).with_priority(50))
    }
}

impl Render for Root {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let decor: WindowDecorations = if self.settings.custom_titlebar { WindowDecorations::Client } else { WindowDecorations::Server };
        if self.decor != decor { self.decor = decor; w.request_decorations(decor); }
        self.sync_mine_state();
        let resizable = self.settings.custom_titlebar && !w.is_maximized() && !w.is_fullscreen();
        // a review in full screen takes the whole window: no tab strip, and the window itself goes full screen
        let fs = self.active.and_then(|i| self.tabs.get(i)).map(|t| t.view.read(cx).is_fullscreen()).unwrap_or(false);
        if fs != self.fs_applied { self.fs_applied = fs; if fs != w.is_fullscreen() { w.toggle_fullscreen(); } }
        let strip = (!fs).then(|| self.tab_strip(w, cx));
        let body = match self.active.and_then(|i| self.tabs.get(i)) { Some(t) => div().flex().flex_col().flex_1().min_h_0().child(t.view.clone()).into_any_element(), None => self.render_home(cx).into_any_element() };
        let toasts = self.toast_stack(cx);
        let settings = self.settings_view.clone();
        let close_modal = self.close_modal(cx);
        div().relative().size_full().flex().flex_col().bg(p.bg).font_family(self.fonts.ui.clone()).text_size(px(14.0)).track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { if this.mine_dd.is_some() { this.mine_dd = None; if this.hf == Some(Hf::DSearch) { this.hf = None; } cx.notify(); }
                if this.hf.is_some() { if this.renaming.is_some() { this.commit_rename(cx); } this.hf = None; cx.notify(); } }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                let ks = &ev.keystroke; let ctrl = ks.modifiers.control || ks.modifiers.platform;
                if this.close_req.is_some() { if ks.key == "escape" { this.resolve_close(CloseChoice::Cancel, cx); } return; }
                if ctrl && ks.key == "q" && this.settings_view.is_none() { if this.request_quit(cx) { _w.remove_window(); } return; }
                if let Some(sv) = this.settings_view.clone() { sv.update(cx, |v, cx| v.key(ev, cx)); return; }
                if ctrl && ks.key == "tab" { this.cycle(if ks.modifiers.shift { -1 } else { 1 }, cx); return; }
                if ctrl && ks.key == "w" { if let Some(a) = this.active { this.request_close_tab(a, cx); } return; }
                if let Some(t) = this.active.and_then(|i| this.tabs.get(i)) { let v = t.view.clone(); v.update(cx, |v, cx| v.key(ev, cx)); } else { this.home_key(ev, cx); }
            }))
            .children(strip).child(body).children(settings).children(close_modal).children(toasts)
            .when(resizable, |d| {
                let edge = |id: &'static str, e: ResizeEdge, c: CursorStyle| div().id(id).absolute().cursor(c).on_mouse_down(MouseButton::Left, move |_, w, cx| { w.start_window_resize(e); cx.stop_propagation(); });
                let t = px(5.0); let k = px(12.0);
                d.child(edge("rz-t", ResizeEdge::Top, CursorStyle::ResizeUpDown).top_0().left(k).right(k).h(t))
                    .child(edge("rz-b", ResizeEdge::Bottom, CursorStyle::ResizeUpDown).bottom_0().left(k).right(k).h(t))
                    .child(edge("rz-l", ResizeEdge::Left, CursorStyle::ResizeLeftRight).left_0().top(k).bottom(k).w(t))
                    .child(edge("rz-r", ResizeEdge::Right, CursorStyle::ResizeLeftRight).right_0().top(k).bottom(k).w(t))
                    .child(edge("rz-tl", ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight).top_0().left_0().size(k))
                    .child(edge("rz-tr", ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft).top_0().right_0().size(k))
                    .child(edge("rz-bl", ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft).bottom_0().left_0().size(k))
                    .child(edge("rz-br", ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight).bottom_0().right_0().size(k))
            })
    }
}

pub fn run(initial: Initial) {
    std::thread::spawn(|| { let _ = perusal_agents::search_dirs(); }); // warm the login-shell PATH lookup off the UI thread
    gpui_platform::application().run(move |cx: &mut App| {
        let _ = cx.text_system().add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!("../assets/MaterialIcons-Regular.ttf").as_slice())]);
        let st0 = model::load_settings(); kit::REDUCED_MOTION.store(st0.motion_reduced(), std::sync::atomic::Ordering::Relaxed);
        let fonts = kit::pick_fonts_for(cx, &st0);
        let bounds = Bounds::centered(None, size(px(1480.0), px(900.0)), cx);
        let custom = model::load_settings().custom_titlebar;
        cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), window_min_size: Some(size(px(900.0), px(560.0))),
            window_decorations: Some(if custom { WindowDecorations::Client } else { WindowDecorations::Server }),
            icon: kit::window_icon(), titlebar: Some(TitlebarOptions { title: Some("Perusal".into()), appears_transparent: true, ..Default::default() }), ..Default::default() }, move |window, cx| {
            let root = cx.new(|cx| Root::new(initial, fonts, cx));
            let r2 = root.clone();
            window.on_window_should_close(cx, move |_, cx| r2.update(cx, |r, cx| r.request_quit(cx)));
            window.focus(&root.read(cx).focus.clone(), cx);
            root
        }).unwrap();
        cx.activate(true);
    });
}

fn mr_item<'a>(m: &'a RemoteMr, mes: &'a std::collections::HashMap<String, String>) -> perusal_app::search::Item<'a> {
    perusal_app::search::Item { title: &m.title, host: &m.host, project: &m.project, number: m.number, author: &m.author, state: &m.state, kind: if m.kind == perusal_providers::Kind::GitLab { "gitlab" } else { "github" },
        assignees: m.assignees.iter().map(|p| p.login.as_str()).collect(), reviewers: m.reviewers.iter().map(|p| p.login.as_str()).collect(), labels: m.labels.iter().map(String::as_str).collect(), description: &m.description, draft: m.draft,
        me: mes.get(&m.host).map(String::as_str) }
}
