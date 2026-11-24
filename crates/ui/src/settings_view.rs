//! The Settings panel (and the first-run setup), shared by the Home screen and every review.
use crate::icons as ic;
use crate::kit::{self, badge, dd_button, icon, icon_btn, icon_text_btn, input_text, seg_box, seg_cell, seg_icon, switch, wire_input, Btn, Fonts, Input, Pal};
use crate::model;
use gpui::{deferred, div, prelude::*, px, relative, Context, EventEmitter, FontWeight, MouseButton, SharedString};
use perusal_agents::{model_suggestions, on_path, supports_effort};
use perusal_app::hosts;
use perusal_app::settings::{DiffMode, HostCfg, MdMode, Settings, AGENTS};
use perusal_app::theme;
use perusal_diff::GroupRule;
use perusal_providers::auth::{DeviceCode, TokenStore};

pub enum SettingsEvent { Changed, Close }
impl EventEmitter<SettingsEvent> for SettingsView {}

#[derive(Clone, Copy, PartialEq)]
enum F { Cmd, Model, Label, Pat, Host, Token, Client }

#[derive(Clone)]
enum Acc { Unknown, Out, In(String), Bad(String) }

pub struct SettingsView {
    pub settings: Settings, pal: Pal, fonts: Fonts,
    cmd: Input, model: Input, label: Input, pat: Input, host_in: Input, token: Input, client: Input,
    focus: Option<F>, rules_open: bool, dd: Option<u8>, rule_err: Option<String>, new_kind: &'static str,
    accounts: Vec<(String, String, Acc)>, token_host: Option<String>, client_host: Option<String>, device: Option<(String, DeviceCode)>, msg: Option<(bool, String)>, busy: bool,
    /// First-run setup: the step shown (0 welcome, 1 accounts, 2 agent, 3 done). `None` is the normal Settings panel.
    pub onboarding: Option<u8>,
}

impl SettingsView {
    pub fn new(settings: Settings, fonts: Fonts, onboarding: bool, cx: &mut Context<Self>) -> Self {
        let theme = theme::load(&settings.theme, Some(&model::themes_dir()));
        let mut v = SettingsView { pal: Pal::new(&theme), fonts, cmd: Input::new(&settings.agent_command), model: Input::new(&settings.model), label: Input::new(""), pat: Input::new(""), host_in: Input::new(""), token: Input::new(""), client: Input::new(""),
            focus: None, rules_open: false, dd: None, rule_err: None, new_kind: "github", accounts: vec![], token_host: None, client_host: None, device: None, msg: None, busy: false, onboarding: onboarding.then_some(0), settings };
        v.refresh_accounts(cx); v
    }
    pub fn set_pal(&mut self, p: Pal) { self.pal = p; }
    fn changed(&mut self, cx: &mut Context<Self>) { model::save_settings(&self.settings); cx.emit(SettingsEvent::Changed); cx.notify(); }
    fn say(&mut self, ok: bool, m: impl Into<String>) { self.msg = Some((ok, m.into())); }

    // ---- accounts
    fn refresh_accounts(&mut self, cx: &mut Context<Self>) {
        self.accounts = self.settings.hosts.iter().map(|h| (h.host.clone(), h.kind.clone(), if model::token_for(&h.host).is_some() { Acc::Unknown } else { Acc::Out })).collect();
        let hosts: Vec<(String, String)> = self.accounts.iter().filter(|a| matches!(a.2, Acc::Unknown)).map(|a| (a.0.clone(), a.1.clone())).collect();
        if hosts.is_empty() { return; }
        let task = cx.background_executor().spawn(async move { hosts.into_iter().map(|(h, k)| { let r = model::account_user(&h, &k); (h, r) }).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| { for (h, res) in r { if let Some(a) = t.accounts.iter_mut().find(|a| a.0 == h) { a.2 = match res { Ok(u) => Acc::In(u), Err(e) => Acc::Bad(e) }; } } cx.notify(); }); }).detach();
    }
    fn save_token(&mut self, cx: &mut Context<Self>) {
        let (Some(host), tok) = (self.token_host.clone(), self.token.f.text.trim().to_string()) else { return };
        if tok.is_empty() { self.say(false, "Paste a token first."); return; }
        match model::token_store().set(&host, &tok) { Ok(()) => { self.token.f.clear(); self.token_host = None; self.say(true, format!("Token saved for {host}.")); self.refresh_accounts(cx); cx.emit(SettingsEvent::Changed); } Err(e) => self.say(false, format!("Cannot save the token: {e}")) }
    }
    fn import_cli(&mut self, host: String, kind: String, cx: &mut Context<Self>) {
        self.busy = true; self.say(true, format!("Looking for a {} login...", if kind == "gitlab" { "glab" } else { "gh" }));
        let h2 = host.clone();
        let task = cx.background_executor().spawn(async move { model::import_cli_token(&h2, &kind) });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| {
            t.busy = false;
            match r { Ok(tok) => { let _ = model::token_store().set(&host, &tok); t.say(true, format!("Imported the CLI login for {host}.")); t.refresh_accounts(cx); cx.emit(SettingsEvent::Changed); } Err(e) => t.say(false, e) }
            cx.notify(); }); }).detach();
    }
    fn device_login(&mut self, host: String, cx: &mut Context<Self>) {
        let cid = self.settings.oauth_client_ids.get(&host).cloned().unwrap_or_default();
        if cid.is_empty() { self.client_host = Some(host); self.say(false, "Enter an OAuth app client ID for this host first."); return; }
        self.busy = true; self.say(true, "Requesting a sign-in code...");
        let (h1, c1) = (host.clone(), cid.clone());
        let start = cx.background_executor().spawn(async move { model::device_start(&h1, &c1) });
        cx.spawn(async move |this, cx| {
            let code = start.await;
            let Ok(Some(code)) = this.update(cx, |t, cx| { match code { Ok(c) => { t.device = Some((host.clone(), c.clone())); t.say(true, format!("Enter the code {} at {}", c.user_code, c.verification_uri)); let _ = model::open_url(&c.verification_uri); cx.notify(); Some(c) } Err(e) => { t.busy = false; t.say(false, format!("Sign-in failed: {e}")); cx.notify(); None } } }) else { return };
            let (h2, c2, cc) = (host.clone(), cid.clone(), code.clone());
            let wait = cx.background_executor().spawn(async move { model::device_wait(&h2, &c2, &cc) });
            let tok = wait.await;
            let _ = this.update(cx, |t, cx| {
                t.busy = false; t.device = None;
                match tok { Ok(tk) => { let _ = model::token_store().set(&host, &tk); t.say(true, format!("Signed in to {host}.")); t.refresh_accounts(cx); cx.emit(SettingsEvent::Changed); } Err(e) => t.say(false, format!("Sign-in did not complete: {e}")) }
                cx.notify();
            });
        }).detach();
    }

    // ---- rules
    fn add_rule(&mut self, cx: &mut Context<Self>) {
        let (pat, label) = (self.pat.f.text.trim().to_string(), self.label.f.text.trim().to_string());
        if let Err(e) = perusal_diff::rules::validate_pattern(&pat) { self.rule_err = Some(e); return; }
        let id = format!("custom-{}", self.settings.group_rules.iter().filter(|r| !r.builtin).count() + 1);
        self.settings.group_rules.push(GroupRule { id, label: if label.is_empty() { pat.clone() } else { label }, pattern: pat, enabled: true, builtin: false });
        self.label.f.clear(); self.pat.f.clear(); self.rule_err = None; self.changed(cx);
    }
    fn blur(&mut self) { self.focus = None; self.dd = None; }
    fn commit_text(&mut self, cx: &mut Context<Self>) {
        let (c, m) = (self.cmd.f.text.trim().to_string(), self.model.f.text.trim().to_string());
        if c != self.settings.agent_command || m != self.settings.model { self.settings.agent_command = c; self.settings.model = m; self.changed(cx); }
        if let Some(h) = self.client_host.clone() { let id = self.client.f.text.trim().to_string(); if !id.is_empty() { self.settings.oauth_client_ids.insert(h, id); self.changed(cx); } }
    }

    pub fn key(&mut self, ev: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        if key == "escape" { if self.focus.is_some() || self.dd.is_some() { self.commit_text(cx); self.blur(); } else if self.onboarding.is_none() { cx.emit(SettingsEvent::Close); } cx.notify(); return; }
        let Some(f) = self.focus else { return };
        if key == "enter" {
            match f { F::Pat | F::Label => self.add_rule(cx), F::Token => self.save_token(cx), F::Host => self.add_host(cx), F::Client => { self.commit_text(cx); if let Some(h) = self.client_host.clone() { self.device_login(h, cx); } } _ => { self.commit_text(cx); self.blur(); } }
            cx.notify(); return;
        }
        let input = match f { F::Cmd => &mut self.cmd, F::Model => &mut self.model, F::Label => &mut self.label, F::Pat => &mut self.pat, F::Host => &mut self.host_in, F::Token => &mut self.token, F::Client => &mut self.client };
        kit::field_key(&mut input.f, &ev.keystroke, cx); if f == F::Pat { self.rule_err = None; }
        cx.notify();
    }
    fn add_host(&mut self, cx: &mut Context<Self>) {
        let h = self.host_in.f.text.trim().trim_start_matches("https://").trim_end_matches('/').to_lowercase();
        if h.is_empty() || h.contains(' ') { self.say(false, "Enter a host name such as git.example.com."); return; }
        if !self.settings.hosts.iter().any(|x| x.host == h) { self.settings.hosts.push(HostCfg { host: h.clone(), kind: self.new_kind.into() }); }
        self.host_in.f.clear(); self.say(true, format!("Added {h}. Sign in below.")); self.changed(cx); self.refresh_accounts(cx);
    }

    // ---- UI
    fn field_box(&self, id: &'static str, which: F, ph: &str, cx: &mut Context<Self>, secret: bool) -> gpui::Stateful<gpui::Div> {
        let p = self.pal; let input = match which { F::Cmd => &self.cmd, F::Model => &self.model, F::Label => &self.label, F::Pat => &self.pat, F::Host => &self.host_in, F::Token => &self.token, F::Client => &self.client };
        let focused = self.focus == Some(which);
        let fam: SharedString = if matches!(which, F::Pat) { self.fonts.mono.clone() } else { self.fonts.ui.clone() };
        let getter: fn(&mut Self) -> &mut Input = match which { F::Cmd => |t| &mut t.cmd, F::Model => |t| &mut t.model, F::Label => |t| &mut t.label, F::Pat => |t| &mut t.pat, F::Host => |t| &mut t.host_in, F::Token => |t| &mut t.token, F::Client => |t| &mut t.client };
        let el = div().id(id).flex().items_center().px_3().py(px(6.0)).min_h(px(32.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).child(input_text(input, focused, ph, &p, secret, 1));
        match which {
            F::Cmd => wire_input(el, cx, getter, |t, _| t.focus = Some(F::Cmd), fam, secret), F::Model => wire_input(el, cx, getter, |t, _| t.focus = Some(F::Model), fam, secret),
            F::Label => wire_input(el, cx, getter, |t, _| t.focus = Some(F::Label), fam, secret), F::Pat => wire_input(el, cx, getter, |t, _| t.focus = Some(F::Pat), fam, secret),
            F::Host => wire_input(el, cx, getter, |t, _| t.focus = Some(F::Host), fam, secret), F::Token => wire_input(el, cx, getter, |t, _| t.focus = Some(F::Token), fam, true), F::Client => wire_input(el, cx, getter, |t, _| t.focus = Some(F::Client), fam, secret),
        }
    }

    fn dropdown(&mut self, id: u8, current: &str, options: Vec<String>, cx: &mut Context<Self>, set: fn(&mut Settings, String)) -> gpui::AnyElement {
        let p = self.pal; let open = self.dd == Some(id);
        let builtin = theme::BUILTIN_NAMES;
        let items: Vec<_> = options.iter().enumerate().map(|(i, o)| {
            let on = o == current; let oo = o.clone(); let custom = !builtin.contains(&o.as_str());
            div().id(("dd-item", id as usize * 100 + i)).flex().items_center().gap_2().px_3().py(px(6.0)).cursor_pointer().when(on, |d| d.bg(p.accent.opacity(0.16))).hover(move |x| x.bg(p.hover)).text_sm()
                .child(div().w(px(16.0)).child(if on { icon(ic::CHECK, 16.0, p.accent).into_any_element() } else { div().into_any_element() })).child(div().flex_1().child(o.clone())).when(custom, |d| d.child(badge("custom", p.accent, p.accent.opacity(0.15))))
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .on_click(cx.listener(move |this, _, _, cx| { set(&mut this.settings, oo.clone()); this.dd = None; this.changed(cx); }))
        }).collect();
        div().relative().child(dd_button(("dd", id as usize), current.to_string(), open, &p)
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .on_click(cx.listener(move |this, _, _, cx| { this.dd = if this.dd == Some(id) { None } else { Some(id) }; cx.notify(); })))
            .children(open.then(|| deferred(div().absolute().top(px(36.0)).right_0().min_w(px(220.0)).max_h(px(260.0)).overflow_hidden().rounded_md().border_1().border_color(p.border).bg(p.surface).shadow_lg().py_1().occlude().children(items)).with_priority(30))).into_any_element()
    }

    fn accounts_section(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.pal;
        let rows: Vec<_> = self.accounts.clone().into_iter().enumerate().map(|(i, (host, kind, acc))| {
            let (status, color) = match &acc { Acc::In(u) => (format!("Signed in as {u}"), p.success), Acc::Unknown => ("Checking…".to_string(), p.muted), Acc::Bad(e) => (format!("Token problem: {e}"), p.danger), Acc::Out => ("Not signed in".to_string(), p.muted) };
            let signed = matches!(acc, Acc::In(_) | Acc::Bad(_) | Acc::Unknown);
            let (h1, h2, h3, h4, h5) = (host.clone(), host.clone(), host.clone(), host.clone(), host.clone()); let (k2, k3, k5) = (kind.clone(), kind.clone(), kind.clone());
            let editing = self.token_host.as_deref() == Some(host.as_str());
            let cli = if kind == "gitlab" { "glab" } else { "gh" };
            div().flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(p.border).bg(p.bg)
                .child(div().flex().items_center().gap_3().child(icon(if kind == "gitlab" { ic::CALL_MERGE } else { ic::PUBLIC }, 20.0, p.accent))
                    .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("{host}  ·  {}", if kind == "gitlab" { "GitLab" } else { "GitHub" }))).child(div().text_xs().text_color(color).child(status)))
                    .when(signed, |d| d.child(icon_text_btn(("signout", i), ic::LOGOUT, "Sign out", Btn::Secondary, &p).on_click(cx.listener(move |this, _, _, cx| { model::token_store().delete(&h1); this.refresh_accounts(cx); cx.emit(SettingsEvent::Changed); cx.notify(); })))))
                .when(!signed || editing, |d| d.child(div().flex().flex_wrap().gap_2()
                    .child(icon_text_btn(("tok", i), ic::KEY, "Paste a token", Btn::Secondary, &p).on_click(cx.listener(move |this, _, _, cx| { this.token_host = Some(h2.clone()); this.focus = Some(F::Token); cx.notify(); })))
                    .child(icon_text_btn(("cli", i), ic::TERMINAL, format!("Use my {cli} login"), Btn::Secondary, &p).on_click(cx.listener(move |this, _, _, cx| { this.import_cli(h3.clone(), k2.clone(), cx); })))
                    .child(icon_text_btn(("oauth", i), ic::OPEN_IN_BROWSER, "Sign in with the browser", Btn::Secondary, &p).on_click(cx.listener(move |this, _, _, cx| { this.device_login(h4.clone(), cx); cx.notify(); })))))
                .when(editing, |d| d.child(div().flex().flex_col().gap_2()
                    .child(self.field_box("f-token", F::Token, "Paste the access token", cx, true))
                    .child(div().flex().gap_2().child(icon_text_btn(("tok-save", i), ic::CHECK, "Save token", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_token(cx); cx.notify(); })))
                        .child(icon_text_btn(("tok-page", i), ic::OPEN_IN_NEW, "Create a token on the website", Btn::Ghost, &p).on_click(cx.listener(move |_, _, _, _| { let _ = model::open_url(&hosts::token_page(&h5, &k5)); }))))))
                .when(self.client_host.as_deref() == Some(host.as_str()), |d| d.child(div().flex().flex_col().gap_2()
                    .child(div().text_xs().text_color(p.muted).child(format!("Browser sign-in uses the OAuth device flow. Register an OAuth app on {host} with device flow enabled and paste its client ID (a token or the {cli} login works without this).")))
                    .child(self.field_box("f-client", F::Client, "OAuth client ID, then Enter", cx, false))))
                .when(false, |d| d.child(div().child(k3)))
        }).collect();
        let dev = self.device.clone().map(|(h, c)| div().flex().items_center().gap_3().p_3().rounded_md().border_1().border_color(p.accent).bg(p.accent.opacity(0.08))
            .child(kit::loading_dots(&p)).child(div().flex_1().text_sm().child(format!("Waiting for you to approve {} on {h}…", c.user_code))));
        div().flex().flex_col().gap_2().children(rows).children(dev)
            .child(div().flex().flex_col().gap_2().pt_2().child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Add a self-hosted GitHub Enterprise or GitLab"))
                .child(div().flex().items_center().gap_2().child(div().flex_1().child(self.field_box("f-host", F::Host, "git.example.com", cx, false)))
                    .child(seg_box(&p).child(seg_cell("nk-gh", "GitHub", self.new_kind == "github", &p).on_click(cx.listener(|this, _, _, cx| { this.new_kind = "github"; cx.notify(); }))).child(seg_cell("nk-gl", "GitLab", self.new_kind == "gitlab", &p).on_click(cx.listener(|this, _, _, cx| { this.new_kind = "gitlab"; cx.notify(); }))))
                    .child(icon_text_btn("add-host", ic::ADD, "Add", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.add_host(cx); cx.notify(); })))))
    }

    fn ai_section(&mut self, cx: &mut Context<Self>, row: &dyn Fn(&'static str, &'static str, gpui::AnyElement) -> gpui::Div) -> gpui::Div {
        let p = self.pal; let s = self.settings.clone();
        let chip = |id: (&'static str, usize), label: String, on: bool| div().id(id).px_3().py(px(4.0)).rounded_md().border_1().border_color(if on { p.accent } else { p.border }).bg(if on { p.accent.opacity(0.12) } else { gpui::transparent_black() }).cursor_pointer().text_sm().hover(move |x| x.bg(p.hover)).child(label);
        let agent_chips: Vec<_> = AGENTS.iter().enumerate().map(|(i, (k, l))| { let kk = k.to_string(); let installed = on_path(match *k { "claude" => "claude", "gemini" => "gemini", "codex" => "codex", _ => "opencode" });
            chip(("ag", i), format!("{l}{}", if installed { "  ✓" } else { "" }), s.agent == *k).on_click(cx.listener(move |this, _, _, cx| { this.settings.agent = kk.clone(); this.changed(cx); })) }).collect();
        let mut model_opts: Vec<String> = vec![String::new()]; model_opts.extend(model_suggestions(&s.agent).iter().map(|m| m.to_string()));
        let model_chips: Vec<_> = model_opts.iter().enumerate().map(|(i, m)| { let mm = m.clone(); chip(("md", i), if m.is_empty() { "Default".into() } else { m.clone() }, s.model == *m).on_click(cx.listener(move |this, _, _, cx| { this.settings.model = mm.clone(); this.model.f.set(&mm); this.changed(cx); })) }).collect();
        let effort_ok = supports_effort(&s.agent);
        let effort_chips: Vec<_> = [("", "Default"), ("low", "Low"), ("medium", "Medium"), ("high", "High")].iter().enumerate().map(|(i, (v, l))| { let vv = v.to_string(); chip(("ef", i), l.to_string(), s.effort == *v).on_click(cx.listener(move |this, _, _, cx| { this.settings.effort = vv.clone(); this.changed(cx); })) }).collect();
        div().flex().flex_col()
            .child(row("Agent", "Runs the CLI you already have installed (a check mark means it was found on your PATH); no keys are stored here", div().flex().flex_wrap().gap_2().max_w(px(380.0)).justify_end().children(agent_chips).into_any_element()))
            .child(row("Model", "One-click choices, or type any model name the agent accepts", div().flex().flex_col().items_end().gap_2().max_w(px(380.0)).child(div().flex().flex_wrap().gap_2().justify_end().children(model_chips)).child(div().w(px(260.0)).child(self.field_box("f-model", F::Model, "Custom model, then Enter", cx, false))).into_any_element()))
            .child(row("Effort / variant", if effort_ok { "How hard the model thinks (Claude: thinking budget, Codex: reasoning effort, opencode: variant)" } else { "Not supported by this agent" }, div().flex().gap_2().when(!effort_ok, |d| d.opacity(0.4)).children(effort_chips).into_any_element()))
            .child(row("Custom command", "Optional. Program and arguments; the prompt is added as the last argument", div().w(px(300.0)).child(self.field_box("f-cmd", F::Cmd, "e.g. my-agent --read-only", cx, false)).into_any_element()))
    }
}

impl gpui::Render for SettingsView {
    fn render(&mut self, _w: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pal;
        let row = move |label: &'static str, hint: &'static str, ctl: gpui::AnyElement| div().flex().items_center().justify_between().gap_4().py_2()
            .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label)).child(div().text_xs().text_color(p.muted).child(hint))).child(ctl);
        let section = move |t: &'static str| div().pt_4().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.muted).border_b_1().border_color(p.border).child(t.to_uppercase());
        let s = self.settings.clone();
        let msg = self.msg.clone().map(|(ok, m)| div().flex().items_start().gap_2().px_3().py_2().mt_2().rounded_md().border_1().border_color(if ok { p.success } else { p.danger }).bg(if ok { p.success.opacity(0.1) } else { p.danger.opacity(0.1) }).text_sm().text_color(if ok { p.success } else { p.danger })
            .child(icon(if ok { ic::CHECK_CIRCLE } else { ic::ERROR_OUTLINE }, 17.0, if ok { p.success } else { p.danger })).child(div().flex_1().min_w_0().child(m)));

        let (title, body, footer): (&str, gpui::AnyElement, gpui::AnyElement) = if let Some(step) = self.onboarding {
            let last = step >= 3;
            let feature = |i: ic::Icon, t: &'static str, d: &'static str| div().flex().items_start().gap_3()
                .child(div().size(px(36.0)).flex_none().rounded_lg().bg(p.accent.opacity(0.14)).flex().items_center().justify_center().child(icon(i, 20.0, p.accent)))
                .child(div().flex().flex_col().flex_1().min_w_0().gap(px(2.0)).child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(t)).child(div().text_sm().text_color(p.muted).child(d)));
            let signed_in = self.accounts.iter().filter(|a| matches!(a.2, Acc::In(_))).count();
            let agent_cards: Vec<_> = AGENTS.iter().enumerate().map(|(i, (k, l))| {
                let prog = match *k { "claude" => "claude", "gemini" => "gemini", "codex" => "codex", _ => "opencode" };
                let found = on_path(prog); let on = s.agent == *k; let kk = k.to_string();
                let blurb = match *k { "claude" => "Anthropic's coding agent", "gemini" => "Google's command-line agent", "codex" => "OpenAI's coding agent", _ => "Open-source agent for many providers" };
                div().id(("obag", i)).flex().items_center().gap_3().px_4().py_3().rounded_lg().border_1().border_color(if on { p.accent } else { p.border }).bg(if on { p.accent.opacity(0.10) } else { p.bg }).cursor_pointer().hover(move |x| x.bg(p.hover))
                    .child(icon(if on { ic::RADIO_BUTTON_CHECKED } else { ic::RADIO_BUTTON_UNCHECKED }, 20.0, if on { p.accent } else { p.muted }))
                    .child(div().flex().flex_col().flex_1().gap(px(1.0)).child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(*l)).child(div().text_xs().text_color(p.muted).child(blurb)))
                    .child(if found { badge("Found on PATH", p.success, p.success.opacity(0.15)) } else { badge("Not installed", p.muted, p.hover) })
                    .on_click(cx.listener(move |this, _, _, cx| { this.settings.agent = kk.clone(); this.changed(cx); }))
            }).collect();
            let content = match step {
                0 => div().flex().flex_col().gap_5().py_5()
                    .child(div().flex().items_center().gap_4()
                        .child(div().size(px(56.0)).rounded_xl().bg(p.accent).flex().items_center().justify_center().text_color(gpui::white()).text_size(px(28.0)).font_weight(FontWeight::BOLD).child("P"))
                        .child(div().flex().flex_col().flex_1().min_w_0().gap(px(2.0)).child(div().text_size(px(26.0)).font_weight(FontWeight::BOLD).text_color(p.fg).child("Welcome to Perusal")).child(div().text_color(p.muted).child("Review pull requests and merge requests quickly."))))
                    .child(div().flex().flex_col().gap_4()
                        .child(feature(ic::FILTER_LIST, "Filter and sort like a search", "Expressions such as additions > 50 && !viewed, with autocomplete and saved presets."))
                        .child(feature(ic::VIEW_AGENDA, "Review one batch at a time", "Related files are grouped, progress is tracked, and a commit graph lets you review commit by commit."))
                        .child(feature(ic::AUTO_AWESOME, "Bring your own AI agent", "Ask about selected lines or run a full review with the CLI you already use. Nothing leaves your machine except through it."))
                        .child(feature(ic::BOOKMARK, "Drafts are always safe", "Comments stay on your computer until you choose to post them.")))
                    .child(div().text_sm().text_color(p.muted).child("Two quick steps make it more useful. You can skip them and change everything later in Settings."))
                    .into_any_element(),
                1 => div().flex().flex_col().gap_3().py_4()
                    .child(div().text_size(px(20.0)).font_weight(FontWeight::BOLD).text_color(p.fg).child("Connect your accounts"))
                    .child(div().text_sm().text_color(p.muted).child("Public repositories open without signing in. Sign in to see your own reviews, open private repositories and post reviews. Use whichever method works for you."))
                    .child(self.accounts_section(cx)).children(msg).into_any_element(),
                2 => div().flex().flex_col().gap_3().py_4()
                    .child(div().text_size(px(20.0)).font_weight(FontWeight::BOLD).text_color(p.fg).child("Choose your AI agent"))
                    .child(div().text_sm().text_color(p.muted).child("Perusal runs the agent CLI that is already installed and logged in on your machine, read-only, on a copy of the code. No keys are stored here."))
                    .child(div().flex().flex_col().gap_2().children(agent_cards))
                    .child(div().text_xs().text_color(p.muted).child("Model and effort can be set in Settings later. Install an agent and reopen this step to see it detected."))
                    .into_any_element(),
                _ => div().flex().flex_col().items_center().gap_4().py_8()
                    .child(div().size(px(72.0)).rounded_full().bg(p.success.opacity(0.15)).flex().items_center().justify_center().child(icon(ic::CHECK_CIRCLE, 44.0, p.success)))
                    .child(div().text_size(px(24.0)).font_weight(FontWeight::BOLD).text_color(p.fg).child("You are all set"))
                    .child(div().flex().flex_col().gap_2().w(px(380.0)).p_4().rounded_lg().border_1().border_color(p.border).bg(p.bg)
                        .child(div().flex().items_center().gap_3().child(icon(if signed_in > 0 { ic::CHECK_CIRCLE } else { ic::RADIO_BUTTON_UNCHECKED }, 18.0, if signed_in > 0 { p.success } else { p.muted })).child(div().text_sm().text_color(p.fg).child(if signed_in > 0 { format!("{signed_in} account{} connected", if signed_in == 1 { "" } else { "s" }) } else { "No accounts connected (public repositories still work)".to_string() })))
                        .child(div().flex().items_center().gap_3().child(icon(ic::CHECK_CIRCLE, 18.0, p.success)).child(div().text_sm().text_color(p.fg).child(format!("AI agent: {}", s.agent_label())))))
                    .child(div().text_color(p.muted).child("Open a pull request URL, a local branch, or one of your own reviews from Home.")).into_any_element(),
            };
            let steps = ["Welcome", "Accounts", "AI agent", "Done"];
            let dots = div().flex().items_center().gap_2().children(steps.iter().enumerate().map(|(i, l)| {
                let (done, on) = (i < step as usize, i == step as usize);
                div().flex().items_center().gap_2()
                    .child(div().size(px(22.0)).rounded_full().flex().items_center().justify_center().text_xs().font_weight(FontWeight::BOLD).bg(if on { p.accent } else if done { p.success } else { p.border }).text_color(if on || done { gpui::white() } else { p.muted })
                        .child(if done { icon(ic::CHECK, 13.0, gpui::white()).into_any_element() } else { div().child(format!("{}", i + 1)).into_any_element() }))
                    .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).child(*l))
                    .when(i < 3, |d| d.child(div().w(px(18.0)).h(px(1.0)).bg(p.border)))
            }).collect::<Vec<_>>());
            let footer = div().flex().items_center().gap_2().px_5().py_3().border_t_1().border_color(p.border).bg(p.surface)
                .child(dots).child(div().flex_1())
                .child(div().id("ob-skip").px_3().text_sm().text_color(p.muted).cursor_pointer().child(if last { "" } else { "Skip setup" }).on_click(cx.listener(|this, _, _, cx| { this.settings.onboarded = true; this.changed(cx); cx.emit(SettingsEvent::Close); })))
                .when(step > 0, |d| d.child(icon_text_btn("ob-back", ic::ARROW_BACK, "Back", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.onboarding = this.onboarding.map(|s| s.saturating_sub(1)); this.msg = None; cx.notify(); }))))
                .child(if last { icon_text_btn("ob-done", ic::CHECK, "Start reviewing", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.commit_text(cx); this.settings.onboarded = true; this.changed(cx); cx.emit(SettingsEvent::Close); })).into_any_element() }
                    else { icon_text_btn("ob-next", ic::ARROW_FORWARD, if step == 0 { "Get started" } else { "Next" }, Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.commit_text(cx); this.onboarding = this.onboarding.map(|s| s + 1); this.msg = None; this.blur(); cx.notify(); })).into_any_element() });
            ("Setup", div().id("ob-scroll").flex_1().min_h_0().overflow_y_scroll().px_6().pb_4().text_color(p.fg).child(content).into_any_element(), footer.into_any_element())
        } else {
            let themes = theme::available(Some(&model::themes_dir()));
            let themes_dir = model::themes_dir();
            let example = "Read AGENTS.md in this folder, then create a warmer dark theme called warm.toml and validate it with `perusal theme check warm.toml`.";
            let rules: Vec<_> = s.group_rules.iter().enumerate().map(|(i, r)| {
                let id = r.id.clone(); let id2 = r.id.clone();
                div().flex().items_center().gap_3().py(px(6.0)).border_b_1().border_color(p.border)
                    .child(div().id(("rule-sw", i)).cursor_pointer().child(switch(r.enabled, &p)).on_click(cx.listener(move |this, _, _, cx| { if let Some(r) = this.settings.group_rules.iter_mut().find(|r| r.id == id) { r.enabled = !r.enabled; } this.changed(cx); })))
                    .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().child(r.label.clone())).child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).truncate().child(r.pattern.clone())))
                    .when(!r.builtin, |d| d.child(icon_btn(("rule-del", i), ic::DELETE, "Delete this rule", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.settings.group_rules.retain(|r| r.id != id2); this.changed(cx); }))))
            }).collect();
            let enabled = s.group_rules.iter().filter(|r| r.enabled).count();
            let rules_card = div().flex().flex_col().rounded_md().border_1().border_color(p.border).bg(p.bg)
                .child(div().id("rules-head").flex().items_center().gap_2().px_3().py_2().cursor_pointer().hover(move |x| x.bg(p.hover)).on_click(cx.listener(|this, _, _, cx| { this.rules_open = !this.rules_open; cx.notify(); }))
                    .child(icon(if self.rules_open { ic::EXPAND_MORE } else { ic::CHEVRON_RIGHT }, 18.0, p.muted)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child("Grouping rules")).child(badge(format!("{enabled} of {} on", s.group_rules.len()), p.muted, p.hover)))
                .when(self.rules_open, |d| d.child(div().flex().flex_col().px_3().pb_3().child(div().text_xs().text_color(p.muted).pb_2().child("Files that match the same rule with the same name (across folders) are reviewed together. The first matching rule wins; names like main, lib, index are ignored.")).children(rules)
                    .child(div().flex().flex_col().gap_2().pt_3().child(div().text_sm().font_weight(FontWeight::MEDIUM).child("Add a rule"))
                        .child(self.field_box("f-label", F::Label, "Name, for example Docs with images", cx, false))
                        .child(self.field_box("f-pat", F::Pat, r"Pattern with a stem group, e.g. ^docs/(?P<stem>[^/.]+)\.(?:md|png)$", cx, false))
                        .children(self.rule_err.clone().map(|e| div().text_xs().text_color(p.danger).child(e)))
                        .child(div().flex().child(icon_text_btn("add-rule", ic::ADD, "Add rule", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.add_rule(cx); cx.notify(); })))))));
            let theme_dd = self.dropdown(0, &s.theme, themes.clone(), cx, |st, v| st.theme = v);
            let light_dd = self.dropdown(1, &s.theme_light, themes.clone(), cx, |st, v| st.theme_light = v);
            let dark_dd = self.dropdown(2, &s.theme_dark, themes, cx, |st, v| st.theme_dark = v);
            let accounts = self.accounts_section(cx);
            let ai = self.ai_section(cx, &row);
            let scroll = div().id("settings-scroll").flex_1().min_h_0().overflow_y_scroll().px_5().pb_4().flex().flex_col()
                .child(section("Appearance"))
                .child(row("Theme", "Built-in themes and valid files from your themes folder", theme_dd))
                .child(row("Follow system appearance", "Switch between a light and a dark theme with the OS", div().id("sw-os").cursor_pointer().child(switch(s.follow_os, &p)).on_click(cx.listener(|this, _, _, cx| { this.settings.follow_os = !this.settings.follow_os; this.changed(cx); })).into_any_element()))
                .when(s.follow_os, |d| d.child(row("Light theme", "Used when the OS is light", light_dd)).child(row("Dark theme", "Used when the OS is dark", dark_dd)))
                .child(div().flex().flex_col().gap_2().p_3().mt_2().rounded_md().border_1().border_color(p.border).bg(p.bg)
                    .child(div().flex().items_center().gap_2().child(icon(ic::PALETTE, 18.0, p.accent)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child("Custom themes"))
                        .child(icon_text_btn("open-themes", ic::FOLDER_OPEN, "Open themes folder", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { let d = model::ensure_theme_docs(); if let Err(e) = model::open_dir(&d) { this.say(false, e); } cx.notify(); }))))
                    .child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).child(themes_dir.display().to_string()))
                    .child(div().text_sm().text_color(p.muted).child("Ask your AI agent to make one. Open an agent in the themes folder (for example run `claude` there). The folder already contains an AGENTS.md that explains the theme format, the colour rules and how to validate a file. Then ask, for example:"))
                    .child(div().flex().items_center().gap_2().px_3().py_2().rounded_md().border_1().border_color(p.border).bg(p.surface).child(div().flex_1().min_w_0().text_sm().child(format!("\"{example}\"")))
                        .child(icon_btn("copy-prompt", ic::CONTENT_COPY, "Copy this prompt", Btn::Ghost, &p).on_click(cx.listener(move |_, _, _, cx| { cx.write_to_clipboard(gpui::ClipboardItem::new_string(example.to_string())); }))))
                    .child(div().text_xs().text_color(p.muted).child("New valid theme files appear in the dropdown the next time you open Settings.")))
                .child(section("Accounts")).child(div().pt_2().child(accounts)).children(self.msg.clone().filter(|_| self.onboarding.is_none()).map(|(ok, m)| div().flex().items_start().gap_2().px_3().py_2().mt_2().rounded_md().border_1().border_color(if ok { p.success } else { p.danger }).text_sm().text_color(if ok { p.success } else { p.danger }).child(div().flex_1().min_w_0().child(m))))
                .child(section("Notifications"))
                .child(row("Desktop notifications", "Tell me when a merge request is merged, closed, updated, or its checks finish", div().id("sw-dn").cursor_pointer().child(switch(s.desktop_notifications, &p)).on_click(cx.listener(|this, _, _, cx| { this.settings.desktop_notifications = !this.settings.desktop_notifications; this.changed(cx); })).into_any_element()))
                .child(section("Review"))
                .child(row("Group related files", "Batch files that belong together using the rules below", div().id("sw-grp").cursor_pointer().child(switch(s.group_related, &p)).on_click(cx.listener(|this, _, _, cx| { this.settings.group_related = !this.settings.group_related; this.changed(cx); })).into_any_element()))
                .child(rules_card)
                .child(row("Files panel", "Flat list or a collapsible directory tree", seg_box(&p).child(seg_icon("fp-list", ic::VIEW_LIST, "Flat list", !s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.files_tree = false; this.changed(cx); })))
                    .child(seg_icon("fp-tree", ic::ACCOUNT_TREE, "Directory tree", s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.files_tree = true; this.changed(cx); }))).into_any_element()))
                .child(section("Diff"))
                .child(row("Default diff view", "How diffs open; switch per file with the floating bar", seg_box(&p).child(seg_icon("dm-uni", ic::VIEW_AGENDA, "Unified", s.diff_mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Unified; this.changed(cx); })))
                    .child(seg_icon("dm-split", ic::VERTICAL_SPLIT, "Side by side", s.diff_mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Split; this.changed(cx); }))).into_any_element()))
                .child(row("Markdown files", "Open .md files as a source diff or as rendered text", seg_box(&p).child(seg_icon("md-src", ic::CODE, "Source diff", s.markdown == MdMode::Source, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Source; this.changed(cx); })))
                    .child(seg_icon("md-rnd", ic::VISIBILITY, "Rendered", s.markdown == MdMode::Rendered, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Rendered; this.changed(cx); }))).into_any_element()))
                .child(section("AI")).child(ai)
                .child(section("Setup")).child(row("Run the first-time setup again", "Walks through accounts and the AI agent", icon_text_btn("rerun-ob", ic::ROCKET_LAUNCH, "Start setup", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.onboarding = Some(0); cx.notify(); })).into_any_element()));
            let footer = div().flex().items_center().justify_end().gap_2().px_5().py_3().border_t_1().border_color(p.border).bg(p.surface)
                .child(div().flex_1().text_xs().text_color(p.muted).child("Changes are saved as you make them."))
                .child(icon_text_btn("set-done", ic::CHECK, "Done", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.commit_text(cx); cx.emit(SettingsEvent::Close); })));
            ("Settings", scroll.into_any_element(), footer.into_any_element())
        };

        let onboarding = self.onboarding.is_some();
        let card = div().id("settings-card").text_color(p.fg).text_size(px(14.0)).flex().flex_col().w(px(if onboarding { 720.0 } else { 740.0 })).h(px(if onboarding { 680.0 } else { 800.0 })).max_h(relative(0.94)).rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.commit_text(cx); this.blur(); cx.notify(); }))
            .child(div().flex().items_center().px_5().py_3().border_b_1().border_color(p.border).child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title)))
            .child(body).child(footer);
        deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.55)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| { if this.onboarding.is_none() { this.commit_text(cx); cx.emit(SettingsEvent::Close); } }))
            .child(card)).with_priority(20)
    }
}
