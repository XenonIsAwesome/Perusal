//! User settings (TOML). Unknown keys are ignored, missing keys take defaults.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffMode { #[default] Unified, Split }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MdMode { #[default] Source, Rendered }

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
    /// Model passed to the agent (empty = the agent's default).
    pub model: String,
    /// Reasoning effort / variant: empty (default), low, medium or high.
    pub effort: String,
    /// File grouping rules (built-in plus custom).
    pub group_rules: Vec<perusal_diff::GroupRule>,
    /// Panel layout, remembered between sessions.
    pub left_width: f32,
    pub right_width: f32,
    pub left_open: bool,
    pub right_open: bool,
}

impl Default for Settings {
    fn default() -> Self { Settings { group_related: false, files_tree: false, diff_mode: DiffMode::Unified, markdown: MdMode::Source, agent: "claude".into(), agent_command: String::new(), theme: "dark".into(), follow_os: false, theme_light: "github-light".into(), theme_dark: "dark".into(),
            model: String::new(), effort: String::new(), group_rules: perusal_diff::rules::builtin_rules(), left_width: 380.0, right_width: 340.0, left_open: true, right_open: true } }
}

impl Settings {
    pub fn from_toml(s: &str) -> Settings {
        let mut st: Settings = toml::from_str(s).unwrap_or_default();
        st.group_rules = perusal_diff::rules::merge_with_builtins(std::mem::take(&mut st.group_rules));
        st.left_width = st.left_width.clamp(200.0, 700.0); st.right_width = st.right_width.clamp(220.0, 700.0);
        st
    }
    pub fn to_toml(&self) -> String { toml::to_string_pretty(self).unwrap_or_default() }
    pub fn agent_label(&self) -> &'static str { AGENTS.iter().find(|(k, _)| *k == self.agent).map(|(_, l)| *l).unwrap_or("Claude Code") }
    /// Theme to use now: the chosen one, or the light/dark pair when following the OS.
    pub fn effective_theme(&self, os_dark: bool) -> &str { if self.follow_os { if os_dark { &self.theme_dark } else { &self.theme_light } } else { &self.theme } }
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
    #[test] fn theme_choice() {
        let mut s = Settings::default(); assert_eq!(s.effective_theme(false), "dark");
        s.follow_os = true; assert_eq!(s.effective_theme(false), "github-light"); assert_eq!(s.effective_theme(true), "dark");
    }
    #[test] fn agent_cycle() { let mut s = Settings::default(); assert_eq!(s.agent_label(), "Claude Code"); for _ in 0..4 { s.next_agent(); } assert_eq!(s.agent, "claude"); s.next_agent(); assert_eq!(s.agent_label(), "Gemini CLI"); }
}
