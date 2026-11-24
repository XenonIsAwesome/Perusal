//! The Settings panel (and the first-run setup), shared by the Home screen and every review.
use crate::icons as ic;
use crate::kit::{self, badge, dd_button, icon, icon_btn, icon_text_btn, input_text, seg_box, seg_cell, seg_icon, switch, wire_input, Btn, Fonts, Input, Pal};
use crate::model;
use gpui::{deferred, div, prelude::*, px, relative, Context, EventEmitter, FontWeight, MouseButton, SharedString};
use perusal_agents::{effort_suggestions, model_suggestions, on_path, supports_effort};
use perusal_app::hosts;
use perusal_app::settings::{DiffMode, HostCfg, MdMode, Settings, AGENTS};
use perusal_app::theme;
use perusal_diff::GroupRule;
use perusal_providers::auth::{DeviceCode, TokenStore};

pub enum SettingsEvent { Changed, Close }
impl EventEmitter<SettingsEvent> for SettingsView {}

#[derive(Clone, Copy, PartialEq)]
enum F { Cmd, Model, Label, Pat, Host, Token, Client, RuleGen, PName, PFilter, PSort, Effort }

#[derive(Clone)]
enum Acc { Unknown, Out, In(String), Bad(String) }

pub struct SettingsView {
    pub settings: Settings, pal: Pal, fonts: Fonts,
    cmd: Input, model: Input, label: Input, pat: Input, host_in: Input, token: Input, client: Input, rule_gen: Input, pname: Input, pfilter: Input, psort: Input, effort_in: Input,
    model_custom: bool, effort_custom: bool, stab: u8, editing_rule: Option<String>, presets: perusal_core::presets::Presets, pscope: Option<String>, pedit: bool, gen_busy: bool, gen_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    focus: Option<F>, rules_open: bool, dd: Option<u8>, rule_err: Option<String>, new_kind: &'static str,
    accounts: Vec<(String, String, Acc)>, token_host: Option<String>, client_host: Option<String>, device: Option<(String, DeviceCode)>, msg: Option<(bool, String)>, busy: bool,
    /// First-run setup: the step shown (0 welcome, 1 accounts, 2 agent, 3 done). `None` is the normal Settings panel.
    pub onboarding: Option<u8>,
}

impl SettingsView {
    pub fn new(settings: Settings, fonts: Fonts, onboarding: bool, cx: &mut Context<Self>) -> Self {
        let theme = theme::load(&settings.theme, Some(&model::themes_dir()));
        let mut v = SettingsView { pal: Pal::new(&theme), fonts, cmd: Input::new(&settings.agent_command), model: Input::new(&settings.model), label: Input::new(""), pat: Input::new(""), host_in: Input::new(""), token: Input::new(""), client: Input::new(""), rule_gen: Input::new(""), pname: Input::new(""), pfilter: Input::new(""), psort: Input::new(""), effort_in: Input::new(&settings.effort),
            model_custom: !settings.model.is_empty() && !model_suggestions(&settings.agent).contains(&settings.model.as_str()), effort_custom: !settings.effort.is_empty() && !effort_suggestions(&settings.agent).contains(&settings.effort.as_str()), stab: 0, editing_rule: None, presets: model::load_presets(), pscope: None, pedit: false, gen_busy: false, gen_cancel: Default::default(),
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
    /// Imports every gh/glab login, adding hosts we did not know (GitHub Enterprise, self-hosted GitLab).
    fn import_all(&mut self, cx: &mut Context<Self>) {
        self.busy = true; self.say(true, "Looking for gh and glab logins...");
        let task = cx.background_executor().spawn(async move { model::import_all_cli() });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| {
            t.busy = false;
            if r.is_empty() { t.say(false, "No gh or glab logins found. Run `gh auth login` or `glab auth login` first."); }
            else {
                for (host, kind, tok) in &r { let _ = model::token_store().set(host, tok); if !t.settings.hosts.iter().any(|h| &h.host == host) { t.settings.hosts.push(HostCfg { host: host.clone(), kind: kind.clone() }); } }
                t.say(true, format!("Imported {} login{}: {}.", r.len(), if r.len() == 1 { "" } else { "s" }, r.iter().map(|x| x.0.as_str()).collect::<Vec<_>>().join(", ")));
                t.changed(cx); t.refresh_accounts(cx);
            }
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
    fn next_rule_id(&self) -> String { (1..).map(|n| format!("custom-{n}")).find(|id| !self.settings.group_rules.iter().any(|r| &r.id == id)).unwrap() }
    fn add_rule(&mut self, cx: &mut Context<Self>) {
        let (pat, label) = (self.pat.f.text.trim().to_string(), self.label.f.text.trim().to_string());
        if let Err(e) = perusal_diff::rules::validate_pattern(&pat) { self.rule_err = Some(e); return; }
        let label = if label.is_empty() { pat.clone() } else { label };
        match self.editing_rule.take() {
            Some(id) => { if let Some(r) = self.settings.group_rules.iter_mut().find(|r| r.id == id && !r.builtin) { r.label = label; r.pattern = pat; } }
            None => { let id = self.next_rule_id(); self.settings.group_rules.push(GroupRule { id, label, pattern: pat, enabled: true, builtin: false }); }
        }
        self.label.f.clear(); self.pat.f.clear(); self.rule_err = None; self.changed(cx);
    }
    /// Copies any rule (built-in ones too) into an editable custom rule.
    fn duplicate_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(r) = self.settings.group_rules.iter().find(|r| r.id == id).cloned() else { return };
        let nid = self.next_rule_id();
        let pos = self.settings.group_rules.iter().position(|x| x.id == id).map(|i| i + 1).unwrap_or(self.settings.group_rules.len());
        self.settings.group_rules.insert(pos, GroupRule { id: nid, label: format!("{} (copy)", r.label), pattern: r.pattern, enabled: true, builtin: false });
        self.say(true, "Duplicated. Use the pencil on the copy to edit it."); self.changed(cx);
    }
    fn edit_rule(&mut self, id: &str) {
        if let Some(r) = self.settings.group_rules.iter().find(|r| r.id == id && !r.builtin) { self.label.f.set(&r.label.clone()); self.pat.f.set(&r.pattern.clone()); self.editing_rule = Some(id.to_string()); self.rule_err = None; self.rules_open = true; }
    }
    fn blur(&mut self) { self.focus = None; self.dd = None; }
    fn commit_text(&mut self, cx: &mut Context<Self>) {
        let (c, m) = (self.cmd.f.text.trim().to_string(), self.model.f.text.trim().to_string());
        let e = if self.effort_custom { self.effort_in.f.text.trim().to_string() } else { self.settings.effort.clone() };
        let m = if self.model_custom { m } else { self.settings.model.clone() };
        if c != self.settings.agent_command || m != self.settings.model || e != self.settings.effort { self.settings.agent_command = c; self.settings.model = m; self.settings.effort = e; self.changed(cx); }
        if let Some(h) = self.client_host.clone() { let id = self.client.f.text.trim().to_string(); if !id.is_empty() { self.settings.oauth_client_ids.insert(h, id); self.changed(cx); } }
    }

    fn input_mut(&mut self, f: F) -> &mut Input {
        match f { F::Cmd => &mut self.cmd, F::Model => &mut self.model, F::Label => &mut self.label, F::Pat => &mut self.pat, F::Host => &mut self.host_in, F::Token => &mut self.token, F::Client => &mut self.client,
            F::RuleGen => &mut self.rule_gen, F::PName => &mut self.pname, F::PFilter => &mut self.pfilter, F::PSort => &mut self.psort, F::Effort => &mut self.effort_in }
    }
    pub fn key(&mut self, ev: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        if key == "escape" { if self.focus.is_some() || self.dd.is_some() { self.commit_text(cx); self.blur(); } else if self.onboarding.is_none() { cx.emit(SettingsEvent::Close); } cx.notify(); return; }
        let Some(f) = self.focus else { return };
        if key == "enter" {
            match f {
                F::Pat | F::Label => self.add_rule(cx), F::Token => self.save_token(cx), F::Host => self.add_host(cx),
                F::Client => { self.commit_text(cx); if let Some(h) = self.client_host.clone() { self.device_login(h, cx); } }
                F::RuleGen => self.gen_rule(cx), F::PName | F::PFilter | F::PSort => self.save_preset(cx),
                _ => { self.commit_text(cx); self.blur(); }
            }
            cx.notify(); return;
        }
        let input = self.input_mut(f);
        kit::field_key(&mut input.f, &ev.keystroke, cx); if f == F::Pat { self.rule_err = None; }
        cx.notify();
    }
    /// Asks the agent for a grouping rule and drops its answer into the rule form for review.
    fn gen_rule(&mut self, cx: &mut Context<Self>) {
        if self.gen_busy { return; }
        let req = self.rule_gen.f.text.trim().to_string();
        if req.is_empty() { self.rule_err = Some("Describe which files belong together first.".into()); return; }
        self.gen_busy = true; self.rule_err = None; self.gen_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        let (st, cancel) = (self.settings.clone(), self.gen_cancel.clone());
        let task = cx.background_executor().spawn(async move { model::run_gen(&st, perusal_agents::GenKind::GroupRule, &req, &cancel) });
        cx.spawn(async move |this, cx| { let r = task.await; let _ = this.update(cx, |t, cx| {
            t.gen_busy = false;
            match r { Ok(v) => { t.label.f.set(&v[0]); t.pat.f.set(&v[1]); t.rule_gen.f.clear(); t.say(true, "The agent wrote a rule. Check it below, then add it."); } Err(e) => t.rule_err = Some(e) }
            cx.notify(); }); }).detach();
        cx.notify();
    }
    fn save_preset(&mut self, cx: &mut Context<Self>) {
        let name = self.pname.f.text.trim().to_string();
        if name.is_empty() { self.say(false, "Give the preset a name."); return; }
        let (f, so) = (self.pfilter.f.text.trim().to_string(), self.psort.f.text.trim().to_string());
        if !f.is_empty() { if let Err(e) = perusal_filter::compile(&f) { self.say(false, format!("The filter is not valid: {e}")); return; } }
        for (e, _) in perusal_filter::parse_tiers(&so) { if let Err(er) = perusal_filter::compile(&e) { self.say(false, format!("The sort is not valid: {er}")); return; } }
        self.presets.save(self.pscope.as_deref(), &name, perusal_core::presets::Preset { filter: f, sort: so });
        model::save_presets(&self.presets); self.pname.f.clear(); self.pfilter.f.clear(); self.psort.f.clear(); self.pedit = false;
        self.say(true, format!("Saved the preset \"{name}\".")); cx.emit(SettingsEvent::Changed); cx.notify();
    }
    fn add_host(&mut self, cx: &mut Context<Self>) {
        let h = self.host_in.f.text.trim().trim_start_matches("https://").trim_end_matches('/').to_lowercase();
        if h.is_empty() || h.contains(' ') { self.say(false, "Enter a host name such as git.example.com."); return; }
        if !self.settings.hosts.iter().any(|x| x.host == h) { self.settings.hosts.push(HostCfg { host: h.clone(), kind: self.new_kind.into() }); }
        self.host_in.f.clear(); self.say(true, format!("Added {h}. Sign in below.")); self.changed(cx); self.refresh_accounts(cx);
    }

    // ---- UI
    fn field_box(&self, id: &'static str, which: F, ph: &str, cx: &mut Context<Self>, secret: bool) -> gpui::Stateful<gpui::Div> {
        let p = self.pal;
        let input = match which { F::Cmd => &self.cmd, F::Model => &self.model, F::Label => &self.label, F::Pat => &self.pat, F::Host => &self.host_in, F::Token => &self.token, F::Client => &self.client,
            F::RuleGen => &self.rule_gen, F::PName => &self.pname, F::PFilter => &self.pfilter, F::PSort => &self.psort, F::Effort => &self.effort_in };
        let focused = self.focus == Some(which);
        let fam: SharedString = if matches!(which, F::Pat | F::PFilter | F::PSort) { self.fonts.mono.clone() } else { self.fonts.ui.clone() };
        let el = div().id(id).flex().items_center().px_3().py(px(6.0)).min_h(px(32.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).child(input_text(input, focused, ph, &p, secret, 1));
        macro_rules! w { ($f:ident, $v:ident, $sec:expr) => { wire_input(el, cx, |t| &mut t.$f, |t, _| t.focus = Some(F::$v), fam, $sec) } }
        match which {
            F::Cmd => w!(cmd, Cmd, secret), F::Model => w!(model, Model, secret), F::Label => w!(label, Label, secret), F::Pat => w!(pat, Pat, secret), F::Host => w!(host_in, Host, secret), F::Token => w!(token, Token, true), F::Client => w!(client, Client, secret),
            F::RuleGen => w!(rule_gen, RuleGen, secret), F::PName => w!(pname, PName, secret), F::PFilter => w!(pfilter, PFilter, secret), F::PSort => w!(psort, PSort, secret), F::Effort => w!(effort_in, Effort, secret),
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

    /// A dropdown of (label, value) options whose choice runs `act`.
    fn pick(&mut self, id: u8, current: &str, options: Vec<(String, String)>, cx: &mut Context<Self>, act: fn(&mut Self, String, &mut Context<Self>)) -> gpui::AnyElement {
        let p = self.pal; let open = self.dd == Some(id);
        let items: Vec<_> = options.iter().enumerate().map(|(i, (label, val))| {
            let on = label == current; let v = val.clone();
            div().id(("pk-item", id as usize * 100 + i)).flex().items_center().gap_2().px_3().py(px(6.0)).cursor_pointer().when(on, |d| d.bg(p.accent.opacity(0.16))).hover(move |x| x.bg(p.hover)).text_sm()
                .child(div().w(px(16.0)).child(if on { icon(ic::CHECK, 16.0, p.accent).into_any_element() } else { div().into_any_element() })).child(div().flex_1().child(label.clone()))
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .on_click(cx.listener(move |this, _, _, cx| { this.dd = None; act(this, v.clone(), cx); }))
        }).collect();
        div().relative().child(dd_button(("pk", id as usize), current.to_string(), open, &p)
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
            chip(("ag", i), format!("{l}{}", if installed { "  ✓" } else { "" }), s.agent == *k).on_click(cx.listener(move |this, _, _, cx| {
                this.settings.agent = kk.clone(); this.model_custom = false; this.effort_custom = false; this.settings.model.clear(); this.settings.effort.clear(); this.model.f.clear(); this.effort_in.f.clear(); this.changed(cx); })) }).collect();
        let mut model_opts: Vec<(String, String)> = vec![("Default".into(), String::new())]; model_opts.extend(model_suggestions(&s.agent).iter().map(|m| (m.to_string(), m.to_string()))); model_opts.push(("Custom…".into(), "\u{0}custom".into()));
        let model_label = if self.model_custom { "Custom…".to_string() } else if s.model.is_empty() { "Default".into() } else { s.model.clone() };
        let model_dd = self.pick(3, &model_label, model_opts, cx, |this, v, cx| {
            if v == "\u{0}custom" { this.model_custom = true; this.focus = Some(F::Model); } else { this.model_custom = false; this.model.f.set(&v); this.settings.model = v; this.changed(cx); } cx.notify(); });
        let mut eff_opts: Vec<(String, String)> = vec![("Default".into(), String::new())]; eff_opts.extend(effort_suggestions(&s.agent).iter().map(|m| (m.to_string(), m.to_string()))); eff_opts.push(("Custom…".into(), "\u{0}custom".into()));
        let eff_label = if self.effort_custom { "Custom…".to_string() } else if s.effort.is_empty() { "Default".into() } else { s.effort.clone() };
        let eff_dd = self.pick(4, &eff_label, eff_opts, cx, |this, v, cx| {
            if v == "\u{0}custom" { this.effort_custom = true; this.focus = Some(F::Effort); } else { this.effort_custom = false; this.effort_in.f.set(&v); this.settings.effort = v; this.changed(cx); } cx.notify(); });
        let effort_ok = supports_effort(&s.agent);
        let model_ctl = div().flex().flex_col().items_end().gap_2().child(model_dd).when(self.model_custom, |d| d.child(div().w(px(260.0)).child(self.field_box("f-model", F::Model, "Model name, e.g. claude-sonnet-4-5", cx, false)))).into_any_element();
        let eff_ctl = div().flex().flex_col().items_end().gap_2().when(!effort_ok, |d| d.opacity(0.4)).child(eff_dd).when(self.effort_custom && effort_ok, |d| d.child(div().w(px(260.0)).child(self.field_box("f-effort", F::Effort, if s.agent == "claude" { "Thinking tokens, e.g. 20000" } else { "Variant name" }, cx, false)))).into_any_element();
        div().flex().flex_col()
            .child(row("Agent", "Runs the CLI you already have installed (a check mark means it was found on your PATH); no keys are stored here", div().flex().flex_wrap().gap_2().max_w(px(380.0)).justify_end().children(agent_chips).into_any_element()))
            .child(row("Model", "Pick one, or choose Custom to type any model name the agent accepts", model_ctl))
            .child(row("Effort / variant", if effort_ok { "How hard the model thinks (Claude: thinking budget, Codex: reasoning effort, opencode: variant)" } else { "Not supported by this agent" }, eff_ctl))
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
            let sw = |id: &'static str, on: bool, cx: &mut Context<Self>, f: fn(&mut Settings)| div().id(id).cursor_pointer().child(switch(on, &p)).on_click(cx.listener(move |this, _, _, cx| { f(&mut this.settings); this.changed(cx); })).into_any_element();
            // ---- grouping rules
            let rules: Vec<_> = s.group_rules.iter().enumerate().map(|(i, r)| {
                let (id, id2, id3, id4) = (r.id.clone(), r.id.clone(), r.id.clone(), r.id.clone());
                div().flex().items_center().gap_2().py(px(6.0)).border_b_1().border_color(p.border)
                    .child(div().id(("rule-sw", i)).cursor_pointer().child(switch(r.enabled, &p)).on_click(cx.listener(move |this, _, _, cx| { if let Some(r) = this.settings.group_rules.iter_mut().find(|r| r.id == id) { r.enabled = !r.enabled; } this.changed(cx); })))
                    .child(div().flex().flex_col().flex_1().min_w_0().child(div().flex().items_center().gap_2().child(div().text_sm().child(r.label.clone())).when(r.builtin, |d| d.child(badge("built-in", p.muted, p.hover)))).child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).truncate().child(r.pattern.clone())))
                    .child(icon_btn(("rule-dup", i), ic::CONTENT_COPY, "Duplicate this rule", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.duplicate_rule(&id3, cx); })))
                    .when(!r.builtin, |d| d.child(icon_btn(("rule-edit", i), ic::EDIT, "Edit this rule", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.edit_rule(&id4); cx.notify(); })))
                        .child(icon_btn(("rule-del", i), ic::DELETE, "Delete this rule", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.settings.group_rules.retain(|r| r.id != id2); if this.editing_rule.as_deref() == Some(id2.as_str()) { this.editing_rule = None; } this.changed(cx); }))))
            }).collect();
            let enabled = s.group_rules.iter().filter(|r| r.enabled).count();
            let editing = self.editing_rule.is_some();
            let rules_card = div().flex().flex_col().rounded_md().border_1().border_color(p.border).bg(p.bg)
                .child(div().id("rules-head").flex().items_center().gap_2().px_3().py_2().cursor_pointer().hover(move |x| x.bg(p.hover)).on_click(cx.listener(|this, _, _, cx| { this.rules_open = !this.rules_open; cx.notify(); }))
                    .child(icon(if self.rules_open { ic::EXPAND_MORE } else { ic::CHEVRON_RIGHT }, 18.0, p.muted)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child("Grouping rules")).child(badge(format!("{enabled} of {} on", s.group_rules.len()), p.muted, p.hover)))
                .when(self.rules_open, |d| d.child(div().flex().flex_col().px_3().pb_3().child(div().text_xs().text_color(p.muted).pb_2().child("Files that match the same rule with the same name (across folders) are reviewed together. The first matching rule wins; names like main, lib, index are ignored. Built-in rules can be duplicated and then edited.")).children(rules)
                    .child(div().flex().flex_col().gap_2().pt_3().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(if editing { "Edit the rule" } else { "Add a rule" }))
                        .child(div().flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(p.border).bg(p.surface)
                            .child(div().flex().items_center().gap_2().text_sm().font_weight(FontWeight::SEMIBOLD).child(icon(ic::AUTO_AWESOME, 16.0, p.ai)).child("Generate with AI"))
                            .child(self.field_box("f-rgen", F::RuleGen, "Describe which files belong together, e.g. SQL migrations with their rollbacks", cx, false))
                            .child(div().flex().items_center().gap_2().when(self.gen_busy, |d| d.child(kit::loading_dots(&p)).child(div().text_xs().text_color(p.muted).child("The agent is writing the rule…"))).child(div().flex_1())
                                .child(if self.gen_busy { icon_text_btn("rgen-stop", ic::STOP, "Stop", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.gen_cancel.store(true, std::sync::atomic::Ordering::Relaxed); cx.notify(); })).into_any_element() }
                                    else { icon_text_btn("rgen-go", ic::AUTO_AWESOME, "Generate", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.gen_rule(cx); })).into_any_element() })))
                        .child(self.field_box("f-label", F::Label, "Name, for example Docs with images", cx, false))
                        .child(self.field_box("f-pat", F::Pat, r"Pattern with a stem group, e.g. ^docs/(?P<stem>[^/.]+)\.(?:md|png)$", cx, false))
                        .children(self.rule_err.clone().map(|e| div().text_xs().text_color(p.danger).child(e)))
                        .child(div().flex().gap_2().child(icon_text_btn("add-rule", if editing { ic::SAVE } else { ic::ADD }, if editing { "Save rule" } else { "Add rule" }, Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.add_rule(cx); cx.notify(); })))
                            .when(editing, |d| d.child(icon_text_btn("cancel-rule", ic::CLOSE, "Cancel", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.editing_rule = None; this.label.f.clear(); this.pat.f.clear(); cx.notify(); }))))))));
            let theme_dd = self.dropdown(0, &s.theme, themes.clone(), cx, |st, v| st.theme = v);
            let light_dd = self.dropdown(1, &s.theme_light, themes.clone(), cx, |st, v| st.theme_light = v);
            let dark_dd = self.dropdown(2, &s.theme_dark, themes, cx, |st, v| st.theme_dark = v);
            let accounts = self.accounts_section(cx);
            let ai = self.ai_section(cx, &row);
            let msgbox = self.msg.clone().filter(|_| self.onboarding.is_none()).map(|(ok, m)| div().flex().items_start().gap_2().px_3().py_2().mt_2().rounded_md().border_1().border_color(if ok { p.success } else { p.danger }).bg(if ok { p.success.opacity(0.1) } else { p.danger.opacity(0.1) }).text_sm().text_color(if ok { p.success } else { p.danger })
                .child(icon(if ok { ic::CHECK_CIRCLE } else { ic::ERROR_OUTLINE }, 17.0, if ok { p.success } else { p.danger })).child(div().flex_1().min_w_0().child(m)));
            // ---- notifications
            let master = s.desktop_notifications;
            let events: Vec<_> = perusal_app::events::DESKTOP_EVENTS.iter().enumerate().map(|(i, (key, label, hint))| {
                let on = s.desktop_event_on(key); let k = key.to_string();
                div().flex().items_center().justify_between().gap_4().py(px(6.0)).when(!master, |d| d.opacity(0.45))
                    .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(*label)).child(div().text_xs().text_color(p.muted).child(*hint)))
                    .child(div().id(("ev-sw", i)).cursor_pointer().child(switch(on, &p)).on_click(cx.listener(move |this, _, _, cx| { let cur = this.settings.desktop_events.get(&k).copied().unwrap_or(true); this.settings.desktop_events.insert(k.clone(), !cur); this.changed(cx); })))
            }).collect();
            // ---- presets
            let scope_label = |k: &Option<String>| k.clone().unwrap_or_else(|| "Global".to_string());
            let mut pscopes: Vec<(Option<String>, perusal_core::presets::Scope)> = vec![(None, self.presets.global.clone())];
            pscopes.extend(self.presets.projects.iter().map(|(k, v)| (Some(k.clone()), v.clone())));
            let preset_groups: Vec<_> = pscopes.iter().enumerate().map(|(si, (key, sc))| {
                let items: Vec<_> = sc.presets.iter().enumerate().map(|(pi, (name, pr))| {
                    let (k1, k2, k3) = (key.clone(), key.clone(), key.clone()); let (n1, n2, n3) = (name.clone(), name.clone(), name.clone()); let pr2 = pr.clone(); let is_def = sc.default.as_deref() == Some(name.as_str());
                    div().flex().items_center().gap_2().py(px(6.0)).border_b_1().border_color(p.border)
                        .child(div().flex().flex_col().flex_1().min_w_0().child(div().flex().items_center().gap_2().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(name.clone())).when(is_def, |d| d.child(badge("default", p.accent, p.accent.opacity(0.15)))))
                            .child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).truncate().child(format!("filter: {}", if pr.filter.is_empty() { "none" } else { &pr.filter })))
                            .child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).truncate().child(format!("sort: {}", if pr.sort.is_empty() { "none" } else { &pr.sort }))))
                        .child(icon_btn(("pe-edit", si * 1000 + pi), ic::EDIT, "Edit this preset", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| { this.pscope = k1.clone(); this.pname.f.set(&n1); this.pfilter.f.set(&pr2.filter); this.psort.f.set(&pr2.sort); this.pedit = true; cx.notify(); })))
                        .child(icon_btn(("pe-def", si * 1000 + pi), if is_def { ic::STAR } else { ic::STAR_BORDER }, if is_def { "Default for new reviews (click to clear)" } else { "Make this the default" }, Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| {
                            let cur = match &k2 { Some(k) => this.presets.projects.get(k).and_then(|s| s.default.clone()), None => this.presets.global.default.clone() };
                            this.presets.set_default(k2.as_deref(), if cur.as_deref() == Some(n2.as_str()) { None } else { Some(n2.as_str()) }); model::save_presets(&this.presets); cx.emit(SettingsEvent::Changed); cx.notify(); })))
                        .child(icon_btn(("pe-del", si * 1000 + pi), ic::DELETE, "Delete this preset", Btn::Ghost, &p).on_click(cx.listener(move |this, _, _, cx| {
                            let sc = match &k3 { Some(k) => this.presets.projects.get_mut(k), None => Some(&mut this.presets.global) };
                            if let Some(sc) = sc { sc.presets.remove(&n3); if sc.default.as_deref() == Some(n3.as_str()) { sc.default = None; } }
                            model::save_presets(&this.presets); cx.emit(SettingsEvent::Changed); cx.notify(); })))
                }).collect();
                div().flex().flex_col().rounded_md().border_1().border_color(p.border).bg(p.bg).px_3().py_2()
                    .child(div().flex().items_center().gap_2().pb_1().child(icon(if key.is_none() { ic::PUBLIC } else { ic::FOLDER }, 16.0, p.accent)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(if key.is_none() { "Global presets".to_string() } else { format!("Project: {}", scope_label(key)) })))
                    .when(sc.presets.is_empty(), |d| d.child(div().text_xs().text_color(p.muted).py_1().child("No presets here yet.")))
                    .children(items)
            }).collect();
            let scope_opts: Vec<(String, String)> = pscopes.iter().map(|(k, _)| (scope_label(k), k.clone().unwrap_or_default())).collect();
            let scope_dd = self.pick(5, &scope_label(&self.pscope), scope_opts, cx, |this, v, cx| { this.pscope = if v.is_empty() { None } else { Some(v) }; cx.notify(); });
            let preset_form = div().flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(p.border).bg(p.surface)
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(if self.pedit { "Edit preset" } else { "Add a preset" }))
                .child(self.field_box("f-pname", F::PName, "Preset name", cx, false))
                .child(self.field_box("f-pfilter", F::PFilter, "Filter, e.g. additions > 50 && !viewed", cx, false))
                .child(self.field_box("f-psort", F::PSort, "Sort, e.g. changed desc, path asc", cx, false))
                .child(div().flex().items_center().gap_2().child(div().text_xs().text_color(p.muted).child("Saved in")).child(scope_dd).child(div().flex_1())
                    .when(self.pedit, |d| d.child(icon_text_btn("pe-cancel", ic::CLOSE, "Cancel", Btn::Ghost, &p).on_click(cx.listener(|this, _, _, cx| { this.pedit = false; this.pname.f.clear(); this.pfilter.f.clear(); this.psort.f.clear(); cx.notify(); }))))
                    .child(icon_text_btn("pe-save", ic::SAVE, "Save preset", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.save_preset(cx); }))));
            // ---- tabs
            let tabs = ["Appearance", "Accounts", "Notifications", "Review", "Presets", "AI"];
            let tab_bar = div().flex().items_center().gap_1().px_5().border_b_1().border_color(p.border).children(tabs.iter().enumerate().map(|(i, t)| {
                let on = self.stab as usize == i;
                div().id(("stab", i)).px_3().py(px(8.0)).cursor_pointer().text_sm().font_weight(FontWeight::MEDIUM).text_color(if on { p.fg } else { p.muted }).border_b_2().border_color(if on { p.accent } else { gpui::transparent_black() }).hover(move |x| x.text_color(p.fg)).child(*t)
                    .on_click(cx.listener(move |this, _, _, cx| { this.stab = i as u8; this.dd = None; this.msg = None; if i == 4 { this.presets = model::load_presets(); } cx.notify(); }))
            }).collect::<Vec<_>>());
            let content: gpui::Div = match self.stab {
                0 => div().flex().flex_col()
                    .child(section("Appearance"))
                    .child(row("Theme", "Built-in themes and valid files from your themes folder", theme_dd))
                    .child(row("Follow system appearance", "Switch between a light and a dark theme with the OS", sw("sw-os", s.follow_os, cx, |st| st.follow_os = !st.follow_os)))
                    .when(s.follow_os, |d| d.child(row("Light theme", "Used when the OS is light", light_dd)).child(row("Dark theme", "Used when the OS is dark", dark_dd)))
                    .child(div().flex().flex_col().gap_2().p_3().mt_2().rounded_md().border_1().border_color(p.border).bg(p.bg)
                        .child(div().flex().items_center().gap_2().child(icon(ic::PALETTE, 18.0, p.accent)).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child("Custom themes"))
                            .child(icon_text_btn("open-themes", ic::FOLDER_OPEN, "Open themes folder", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { let d = model::ensure_theme_docs(); if let Err(e) = model::open_dir(&d) { this.say(false, e); } cx.notify(); }))))
                        .child(div().text_xs().font_family(self.fonts.mono.clone()).text_color(p.muted).child(themes_dir.display().to_string()))
                        .child(div().text_sm().text_color(p.muted).child("Ask your AI agent to make one. Open an agent in the themes folder (for example run `claude` there). The folder already contains an AGENTS.md that explains the theme format, the colour rules and how to validate a file. Then ask, for example:"))
                        .child(div().flex().items_center().gap_2().px_3().py_2().rounded_md().border_1().border_color(p.border).bg(p.surface).child(div().flex_1().min_w_0().text_sm().child(format!("\"{example}\"")))
                            .child(icon_btn("copy-prompt", ic::CONTENT_COPY, "Copy this prompt", Btn::Ghost, &p).on_click(cx.listener(move |_, _, _, cx| { cx.write_to_clipboard(gpui::ClipboardItem::new_string(example.to_string())); }))))
                        .child(div().text_xs().text_color(p.muted).child("New valid theme files appear in the dropdown the next time you open Settings.")))
                    .child(section("Window"))
                    .child(row("Built-in title bar", "Draw the minimize, maximize and close buttons inside the app. Turn off to use the system title bar and borders", sw("sw-tb", s.custom_titlebar, cx, |st| st.custom_titlebar = !st.custom_titlebar)))
                    .child(section("Setup")).child(row("Run the first-time setup again", "Walks through accounts and the AI agent", icon_text_btn("rerun-ob", ic::ROCKET_LAUNCH, "Start setup", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.onboarding = Some(0); cx.notify(); })).into_any_element())),
                1 => div().flex().flex_col().child(section("Accounts")).child(div().pt_2().flex().flex_col().gap_2()
                        .child(div().flex().items_center().gap_3().p_3().rounded_md().border_1().border_color(p.border).bg(p.bg).child(icon(ic::TERMINAL, 20.0, p.accent))
                            .child(div().flex().flex_col().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Import from gh and glab")).child(div().text_xs().text_color(p.muted).child("Finds every host you are logged in to, including GitHub Enterprise and self-hosted GitLab, and signs in to all of them.")))
                            .child(icon_text_btn("import-all", ic::DOWNLOAD, "Import all logins", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.import_all(cx); }))))
                        .child(accounts)).children(msgbox),
                2 => div().flex().flex_col().child(section("Desktop notifications"))
                    .child(row("Desktop notifications", "Master switch. In-app messages always appear", sw("sw-dn", s.desktop_notifications, cx, |st| st.desktop_notifications = !st.desktop_notifications)))
                    .child(div().pt_2().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child("NOTIFY ME WHEN"))
                    .child(div().flex().flex_col().children(events)),
                3 => div().flex().flex_col()
                    .child(section("Files"))
                    .child(row("Group related files", "Batch files that belong together using the rules below", sw("sw-grp", s.group_related, cx, |st| st.group_related = !st.group_related)))
                    .child(div().pt_2().child(rules_card))
                    .child(row("Files panel", "Flat list or a collapsible directory tree", seg_box(&p).child(seg_icon("fp-list", ic::VIEW_LIST, "Flat list", !s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.files_tree = false; this.changed(cx); })))
                        .child(seg_icon("fp-tree", ic::ACCOUNT_TREE, "Directory tree", s.files_tree, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.files_tree = true; this.changed(cx); }))).into_any_element()))
                    .child(row("Hide whitespace-only changes", "Hide files, and lines inside files, where only whitespace changed. Also togglable in the review", sw("sw-ws", s.hide_whitespace, cx, |st| st.hide_whitespace = !st.hide_whitespace)))
                    .child(section("Diff"))
                    .child(row("Default diff view", "How diffs open; switch per file with the floating bar", seg_box(&p).child(seg_icon("dm-uni", ic::VIEW_AGENDA, "Unified", s.diff_mode == DiffMode::Unified, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Unified; this.changed(cx); })))
                        .child(seg_icon("dm-split", ic::VERTICAL_SPLIT, "Side by side", s.diff_mode == DiffMode::Split, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.diff_mode = DiffMode::Split; this.changed(cx); }))).into_any_element()))
                    .child(row("Markdown files", "Open .md files as a source diff or as rendered text", seg_box(&p).child(seg_icon("md-src", ic::CODE, "Source diff", s.markdown == MdMode::Source, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Source; this.changed(cx); })))
                        .child(seg_icon("md-rnd", ic::VISIBILITY, "Rendered", s.markdown == MdMode::Rendered, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.markdown = MdMode::Rendered; this.changed(cx); }))).into_any_element())),
                4 => div().flex().flex_col().gap_3().child(section("Filter and sort presets"))
                    .child(div().text_xs().text_color(p.muted).child("Presets set a filter and a sort in one click. A project's default wins over the global default when a review opens. Save new ones from the bookmark button in a review, or add them here."))
                    .children(preset_groups).child(preset_form).children(self.msg.clone().map(|(ok, m)| div().text_sm().text_color(if ok { p.success } else { p.danger }).child(m))),
                _ => div().flex().flex_col().child(section("AI agent")).child(ai),
            };
            let scroll = div().flex().flex_col().flex_1().min_h_0().child(tab_bar)
                .child(div().id("settings-scroll").flex_1().min_h_0().overflow_y_scroll().px_5().pb_4().child(content));
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
