//! TOML themes: built-ins, validation with agent-actionable messages, contrast checks, zoom.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default)] pub dark: bool,
    pub ui: BTreeMap<String, String>,
    #[serde(default)] pub syntax: BTreeMap<String, String>,
    pub diff: BTreeMap<String, String>,
    pub status: BTreeMap<String, String>,
}

pub const UI_KEYS: [&str; 7] = ["background", "foreground", "accent", "border", "surface", "muted", "hover"];
pub const DIFF_KEYS: [&str; 4] = ["added", "removed", "added_word", "removed_word"];
pub const STATUS_KEYS: [&str; 5] = ["ci_running", "ci_passed", "ci_failed", "viewed", "ai"];
pub const SYNTAX_KEYS: [&str; 6] = ["keyword", "string", "comment", "number", "type", "function"];

pub const DARK: &str = include_str!("../themes/dark.toml");
pub const LIGHT: &str = include_str!("../themes/light.toml");
pub const GITHUB_DARK: &str = include_str!("../themes/github-dark.toml");
pub const GITHUB_LIGHT: &str = include_str!("../themes/github-light.toml");

fn rgb(h: &str) -> Option<(f64, f64, f64)> {
    let h = h.strip_prefix('#')?; if h.len() != 6 { return None; }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    Some((p(0)?, p(2)?, p(4)?))
}
fn lum(c: (f64, f64, f64)) -> f64 { let f = |x: f64| if x <= 0.03928 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }; 0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2) }
pub fn contrast(a: &str, b: &str) -> Option<f64> { let (x, y) = (lum(rgb(a)?), lum(rgb(b)?)); Some((x.max(y) + 0.05) / (x.min(y) + 0.05)) }

/// Returns every problem found, each saying what to change.
pub fn validate(src: &str) -> Vec<String> {
    let t: Theme = match toml::from_str(src) { Ok(t) => t, Err(e) => return vec![format!("cannot parse theme: {e}. Required tables: [ui], [diff], [status]; optional [syntax].")] };
    let mut errs = vec![];
    for (table, keys, map) in [("ui", &UI_KEYS[..], &t.ui), ("diff", &DIFF_KEYS[..], &t.diff), ("status", &STATUS_KEYS[..], &t.status)] {
        for k in keys { if !map.contains_key(*k) { errs.push(format!("[{table}] is missing key '{k}': add `{k} = \"#rrggbb\"`")); } }
        for (k, v) in map { if !keys.contains(&k.as_str()) { errs.push(format!("[{table}] has unknown key '{k}'; allowed: {}", keys.join(", "))); }
            if rgb(v).is_none() { errs.push(format!("[{table}] {k} = \"{v}\" is not a #rrggbb color")); } }
    }
    for (k, v) in &t.syntax { if rgb(v).is_none() { errs.push(format!("[syntax] {k} = \"{v}\" is not a #rrggbb color")); } }
    if let (Some(bg), Some(fg)) = (t.ui.get("background"), t.ui.get("foreground")) {
        if let Some(c) = contrast(bg, fg) { if c < 4.5 { errs.push(format!("[ui] foreground/background contrast is {c:.1}:1; need at least 4.5:1. Make foreground lighter on dark themes or darker on light themes.")); } }
        for k in SYNTAX_KEYS { if let Some(v) = t.syntax.get(k) { if let Some(c) = contrast(bg, v) { if c < 3.0 { errs.push(format!("[syntax] {k} has contrast {c:.1}:1 against the background; need at least 3:1")); } } } }
    }
    errs
}

pub fn builtin(name: &str) -> Option<Theme> {
    let s = match name { "dark" => DARK, "light" => LIGHT, "github-dark" => GITHUB_DARK, "github-light" => GITHUB_LIGHT, _ => return None };
    toml::from_str(s).ok()
}

/// Picks the light or dark theme name from the OS appearance.
pub fn follow_os(os_dark: bool, light: &str, dark: &str) -> String { if os_dark { dark.into() } else { light.into() } }

pub fn zoom(current: f32, step: i32) -> f32 { if step == 0 { 1.0 } else { (current + step as f32 * 0.1).clamp(0.5, 3.0) } }

#[cfg(test)]
mod tests { use super::*;
    #[test] fn builtins_valid() { for n in ["dark", "light", "github-dark", "github-light"] { assert!(validate(match n { "dark" => DARK, "light" => LIGHT, "github-dark" => GITHUB_DARK, _ => GITHUB_LIGHT }).is_empty(), "{n}: {:?}", validate(DARK)); assert!(builtin(n).is_some()); } }
    #[test] fn actionable_errors() {
        let bad = DARK.replace("keyword = \"#ff7b72\"", "keyword = \"red\"").replace("added = ", "adde = ");
        let e = validate(&bad).join("\n"); assert!(e.contains("not a #rrggbb")); assert!(e.contains("missing key 'added'")); assert!(e.contains("unknown key 'adde'"));
        let low = DARK.replace("foreground = \"#e6edf3\"", "foreground = \"#222222\""); assert!(validate(&low).iter().any(|m| m.contains("contrast")));
        assert!(validate("not toml [").iter().any(|m| m.contains("cannot parse")));
    }
    #[test] fn zoom_and_os() { assert_eq!(zoom(1.0, 0), 1.0); assert!((zoom(1.0, 2) - 1.2).abs() < 1e-6); assert_eq!(zoom(3.0, 5), 3.0); assert_eq!(follow_os(true, "light", "dark"), "dark"); }
}
