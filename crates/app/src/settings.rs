//! User settings (TOML). Unknown keys are ignored, missing keys take defaults.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffMode { #[default] Unified, Split }

/// What a pull/merge request review opens on: the overview (description and conversation) or the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirstView { #[default] Overview, Code }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MdMode { #[default] Source, Rendered }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostCfg { pub host: String, /// "github" or "gitlab"
    pub kind: String }
pub fn default_hosts() -> Vec<HostCfg> { vec![HostCfg { host: "github.com".into(), kind: "github".into() }, HostCfg { host: "gitlab.com".into(), kind: "gitlab".into() }] }

pub const AGENTS: [(&str, &str); 4] = [("claude", "Claude Code"), ("gemini", "Gemini CLI"), ("codex", "Codex"), ("opencode", "opencode")];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Batch related files (foo.h + foo.cpp, foo.ts + foo.test.ts) automatically.
    pub group_related: bool,
    /// Files panel as a directory tree instead of a flat list.
    pub files_tree: bool,
    /// Default diff layout for every file.
    pub diff_mode: DiffMode,
    /// Screen a pull/merge request review opens on (local branch reviews have no overview and always open on the code).
    pub first_view: FirstView,
    /// Markdown files open as rendered text or as a source diff.
    pub markdown: MdMode,
    /// Agent CLI used for Ask AI and AI review: claude, gemini, codex or opencode.
    pub agent: String,
    /// Theme name: a built-in (dark, light, github-dark, github-light) or a custom theme file name in the themes folder.
    pub theme: String,
    /// Follow the OS light/dark appearance, using `theme_light` and `theme_dark`.
    pub follow_os: bool,
    pub theme_light: String,
    pub theme_dark: String,
    /// Optional custom command (program and arguments); the prompt is appended as the last argument.
    pub agent_command: String,
    /// Full path of the agent executable (empty = find it on the PATH).
    pub agent_path: String,
    /// Model passed to the agent (empty = the agent's default).
    pub model: String,
    /// Reasoning effort / variant: empty (default), low, medium or high.
    pub effort: String,
    /// Minutes an AI agent run may take before it is stopped; 0 = no limit.
    pub agent_timeout_mins: u32,
    /// File grouping rules (built-in plus custom).
    pub group_rules: Vec<perusal_diff::GroupRule>,
    /// Hosts the user can sign in to (github.com and gitlab.com are always present).
    pub hosts: Vec<HostCfg>,
    /// OAuth app client ids per host for the device flow (optional; tokens and `gh`/`glab` login also work).
    pub oauth_client_ids: std::collections::BTreeMap<String, String>,
    /// First-run setup has been completed or skipped.
    pub onboarded: bool,
    /// Desktop notifications for important updates (master switch).
    pub desktop_notifications: bool,
    /// Per-event overrides of the master switch (key from `events::DESKTOP_EVENTS`); missing = on.
    pub desktop_events: std::collections::BTreeMap<String, bool>,
    /// Hide files whose changes are whitespace only.
    pub hide_whitespace: bool,
    /// Remote reviews: comments are published the moment they are added and the Finish review button is replaced (default off).
    pub publish_instantly: bool,
    /// Draw our own title bar and window buttons instead of the system's (restart-free where the platform allows).
    pub custom_titlebar: bool,
    /// Panel layout, remembered between sessions.
    pub left_width: f32,
    pub right_width: f32,
    pub left_open: bool,
    pub right_open: bool,
}

/// Agent to preselect at first run: keep `current` if it is installed, otherwise the first installed one in `AGENTS` order
/// (so a machine with only opencode does not start on Claude). With none installed, `current` stays.
pub fn pick_installed_agent(current: &str, installed: impl Fn(&str) -> bool) -> String {
    if installed(current) { return current.to_string(); }
    AGENTS.iter().map(|(k, _)| *k).find(|k| installed(k)).unwrap_or(current).to_string()
}

impl Default for Settings {
    fn default() -> Self { Settings { group_related: false, files_tree: false, diff_mode: DiffMode::Unified, first_view: FirstView::Overview, markdown: MdMode::Source, agent: "claude".into(), agent_command: String::new(), agent_path: String::new(), theme: "dark".into(), follow_os: false, theme_light: "github-light".into(), theme_dark: "dark".into(),
            model: String::new(), effort: String::new(), agent_timeout_mins: 15, hosts: default_hosts(), oauth_client_ids: Default::default(), onboarded: false, desktop_notifications: true, desktop_events: Default::default(), hide_whitespace: false, publish_instantly: false, custom_titlebar: true, group_rules: perusal_diff::rules::builtin_rules(), left_width: 380.0, right_width: 340.0, left_open: true, right_open: true } }
}

impl Settings {
    pub fn from_toml(s: &str) -> Settings {
        let mut st: Settings = toml::from_str(s).unwrap_or_default();
        st.group_rules = perusal_diff::rules::merge_with_builtins(std::mem::take(&mut st.group_rules));
        for d in default_hosts() { if !st.hosts.iter().any(|h| h.host == d.host) { st.hosts.insert(0, d); } }
        st.left_width = st.left_width.clamp(200.0, 700.0); st.right_width = st.right_width.clamp(220.0, 700.0);
        st
    }
    pub fn to_toml(&self) -> String { toml::to_string_pretty(self).unwrap_or_default() }
    /// Agent run limit; `None` when the user chose no timeout.
    pub fn agent_timeout(&self) -> Option<std::time::Duration> { (self.agent_timeout_mins > 0).then(|| std::time::Duration::from_secs(self.agent_timeout_mins as u64 * 60)) }
    pub fn agent_label(&self) -> &'static str { AGENTS.iter().find(|(k, _)| *k == self.agent).map(|(_, l)| *l).unwrap_or("Claude Code") }
    /// Theme to use now: the chosen one, or the light/dark pair when following the OS.
    pub fn effective_theme(&self, os_dark: bool) -> &str { if self.follow_os { if os_dark { &self.theme_dark } else { &self.theme_light } } else { &self.theme } }
    /// Whether a desktop notification should be raised for this event key.
    pub fn desktop_event_on(&self, key: &str) -> bool { self.desktop_notifications && self.desktop_events.get(key).copied().unwrap_or(true) }
    pub fn next_agent(&mut self) { let i = AGENTS.iter().position(|(k, _)| *k == self.agent).unwrap_or(0); self.agent = AGENTS[(i + 1) % AGENTS.len()].0.to_string(); }
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn defaults_roundtrip_and_tolerance() {
        let s = Settings::default(); assert_eq!(Settings::from_toml(&s.to_toml()), s);
        let t = Settings::from_toml("diff_mode = \"split\"\nmarkdown = \"rendered\"\nunknown = 3\n"); assert_eq!(t.diff_mode, DiffMode::Split); assert_eq!(t.markdown, MdMode::Rendered); assert!(!t.group_related);
        assert_eq!(Settings::from_toml("not toml {{").diff_mode, DiffMode::Unified);
    }
    #[test] fn rules_merge_and_layout_clamp() {
        let t = Settings::from_toml("left_width = 5000.0\nright_width = 1.0\n[[group_rules]]\nid = \"mine\"\nlabel = \"m\"\npattern = \"(?P<stem>x)\"\nenabled = true\n");
        assert_eq!((t.left_width, t.right_width), (700.0, 220.0)); assert_eq!(t.group_rules.len(), 6); assert!(t.group_rules.iter().any(|r| r.id == "mine" && !r.builtin));
        assert!(Settings::from_toml("").group_rules.iter().all(|r| r.builtin));
    }
    #[test] fn hosts_always_include_the_cloud_hosts() {
        let t = Settings::from_toml("[[hosts]]\nhost = \"git.corp\"\nkind = \"gitlab\"\n"); assert_eq!(t.hosts.iter().map(|h| h.host.as_str()).collect::<Vec<_>>(), vec!["gitlab.com", "github.com", "git.corp"]);
        assert!(!Settings::default().onboarded && Settings::default().desktop_notifications);
    }
    #[test] fn desktop_event_choices() {
        let mut s = Settings::default(); assert!(s.desktop_event_on("merged")); s.desktop_events.insert("merged".into(), false); assert!(!s.desktop_event_on("merged") && s.desktop_event_on("closed"));
        s.desktop_notifications = false; assert!(!s.desktop_event_on("closed")); assert!(Settings::default().custom_titlebar && !Settings::default().hide_whitespace && !Settings::default().publish_instantly);
        assert_eq!(Settings::from_toml(&s.to_toml()), s);
    }
    #[test] fn theme_choice() {
        let mut s = Settings::default(); assert_eq!(s.effective_theme(false), "dark");
        s.follow_os = true; assert_eq!(s.effective_theme(false), "github-light"); assert_eq!(s.effective_theme(true), "dark");
    }
    #[test] fn first_view_defaults_to_overview() {
        assert_eq!(Settings::default().first_view, FirstView::Overview);
        assert_eq!(Settings::from_toml("first_view = \"code\"\n").first_view, FirstView::Code);
        assert_eq!(Settings::from_toml("first_view = \"bogus\"\n").first_view, FirstView::Overview);
    }
    #[test] fn agent_timeout_setting() {
        assert_eq!(Settings::default().agent_timeout(), Some(std::time::Duration::from_secs(900)));
        assert_eq!(Settings::from_toml("agent_timeout_mins = 0\n").agent_timeout(), None);
        assert_eq!(Settings::from_toml("agent_timeout_mins = 2\n").agent_timeout(), Some(std::time::Duration::from_secs(120)));
    }
    #[test] fn agent_cycle() { let mut s = Settings::default(); assert_eq!(s.agent_label(), "Claude Code"); for _ in 0..4 { s.next_agent(); } assert_eq!(s.agent, "claude"); s.next_agent(); assert_eq!(s.agent_label(), "Gemini CLI"); }
    #[test] fn first_run_agent_follows_what_is_installed() {
        assert_eq!(pick_installed_agent("claude", |a| a == "opencode"), "opencode");
        assert_eq!(pick_installed_agent("claude", |a| a == "gemini" || a == "opencode"), "gemini");
        assert_eq!(pick_installed_agent("claude", |_| true), "claude");
        assert_eq!(pick_installed_agent("codex", |a| a == "codex" || a == "gemini"), "codex");
        assert_eq!(pick_installed_agent("claude", |_| false), "claude");
    }
}
