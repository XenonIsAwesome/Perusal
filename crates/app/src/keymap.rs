//! Keymap with defaults, user overrides, vim preset and a fuzzy command palette.
use std::collections::BTreeMap;

/// (action, default key, label). The single source for the defaults, the `?` overlay and the command palette.
pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("next_file", "n", "Next file in the filtered order"), ("prev_file", "p", "Previous file"), ("toggle_viewed", "v", "Mark the current batch viewed / not viewed"),
    ("next_batch", "Enter", "Next batch"), ("prev_batch", "Backspace", "Back to the previous batch"), ("comment", "c", "Comment on the selected lines"),
    ("ask_ai", "a", "Ask AI about the selection"), ("focus_filter", "/", "Focus the filter box"), ("toggle_grouping", "g", "Toggle grouping of related files"),
    ("toggle_tree", "t", "Files panel: tree / list"), ("toggle_split", "u", "Unified / split diff"), ("toggle_markdown", "m", "Markdown rendered / source"),
    ("clear_scope", "x", "Clear the commit scope"), ("overview", "o", "Switch between the overview and the code"), ("refresh", "r", "Refresh commits, files, CI and comments"), ("finish", "f", "Finish review"), ("settings", "s", "Open settings"),
    ("palette", "Ctrl+p", "Command palette"), ("quick_open", "Ctrl+Shift+p", "Go to file…"), ("help", "?", "Show keyboard shortcuts"), ("next_tab", "Ctrl+Tab", "Next tab"),
];

pub fn default_map() -> BTreeMap<String, String> { ACTIONS.iter().map(|(a, k, _)| (a.to_string(), k.to_string())).collect() }

/// Label for an action id (falls back to the id).
pub fn label(action: &str) -> &str { ACTIONS.iter().find(|(a, _, _)| *a == action).map(|(_, _, l)| *l).unwrap_or(action) }

/// Canonical spelling of a key press: `Ctrl+`/`Alt+` prefixes, the typed character for printable keys (so `?` is `?`), otherwise the key name.
pub fn press_name(ctrl: bool, alt: bool, shift: bool, key: &str, typed: Option<&str>) -> String {
    let base = match typed { Some(t) if !ctrl && !alt && t.chars().count() == 1 && !t.chars().all(char::is_whitespace) => t.to_string(), _ => key.to_string() };
    // Shift is part of the name only with Ctrl/Alt (alone it already changed the typed character)
    format!("{}{}{}{base}", if ctrl { "Ctrl+" } else { "" }, if alt { "Alt+" } else { "" }, if shift && (ctrl || alt) { "Shift+" } else { "" })
}

/// Keys the app handles itself; they cannot be bound to an action.
pub const RESERVED: &[&str] = &["escape", "f5", "Ctrl+f", "Ctrl+q", "Ctrl+w", "Ctrl+Tab", "Ctrl+=", "Ctrl++", "Ctrl+-", "Ctrl+0"];
pub fn is_reserved(key: &str) -> bool { RESERVED.iter().any(|r| r.eq_ignore_ascii_case(key)) }

/// The `keymap.toml` text for `map`: only the keys that differ from the defaults (plus the vim preset when `vim`).
pub fn overrides_toml(map: &BTreeMap<String, String>, vim: bool) -> String {
    let base = load("", vim).unwrap_or_else(|_| default_map());
    let mut out = String::from("[keys]\n");
    let mut any = false;
    for (a, k) in map { if base.get(a) != Some(k) { out += &format!("{a} = \"{}\"\n", k.replace('\\', "\\\\").replace('"', "\\\"")); any = true; } }
    if any { out } else { String::new() }
}

/// Indices of `paths` matching `query`, best first: file-name substring, then path substring, then a loose subsequence.
pub fn fuzzy_files(paths: &[&str], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() { return (0..paths.len()).collect(); }
    let qc: Vec<char> = q.chars().filter(|c| !c.is_whitespace()).collect();
    let mut hits: Vec<(u8, usize, usize)> = paths.iter().enumerate().filter_map(|(i, p)| {
        let lp = p.to_lowercase(); let name = lp.rsplit('/').next().unwrap_or(&lp);
        let rank = if name.contains(&q) { 0 } else if lp.contains(&q) { 1 } else {
            let mut it = lp.chars(); if qc.iter().all(|x| it.any(|y| y == *x)) { 2 } else { return None }
        };
        Some((rank, p.len(), i))
    }).collect();
    hits.sort(); hits.into_iter().map(|h| h.2).collect()
}

/// Applies `[keys]` overrides from keymap.toml text. Errors on unknown actions or duplicate keys.
pub fn load(user_toml: &str, vim: bool) -> Result<BTreeMap<String, String>, String> {
    let mut m = default_map();
    if vim { for (a, k) in [("next_file", "j"), ("prev_file", "k")] { m.insert(a.into(), k.into()); } }
    let t: toml::Table = user_toml.parse().map_err(|e: toml::de::Error| e.to_string())?;
    if let Some(k) = t.get("keys").and_then(|v| v.as_table()) {
        for (a, v) in k { if !m.contains_key(a) { return Err(format!("unknown action '{a}'")); }
            m.insert(a.clone(), v.as_str().ok_or(format!("key for '{a}' must be a string"))?.to_string()); }
    }
    let mut seen: BTreeMap<&String, &String> = BTreeMap::new();
    for (a, k) in &m { if let Some(o) = seen.insert(k, a) { return Err(format!("key '{k}' is bound to both '{o}' and '{a}'")); } }
    Ok(m)
}

pub fn action_for<'a>(m: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    m.iter().find(|(_, k)| *k == key).or_else(|| m.iter().find(|(_, k)| k.eq_ignore_ascii_case(key) && k.len() > 1)).map(|(a, _)| a.as_str())
}

/// Subsequence fuzzy match, best (shortest) first.
pub fn palette<'a>(commands: &[&'a str], query: &str) -> Vec<&'a str> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut hits: Vec<&str> = commands.iter().copied().filter(|c| { let mut it = c.to_lowercase().chars().collect::<Vec<_>>().into_iter(); q.iter().all(|x| it.any(|y| y == *x)) }).collect();
    hits.sort_by_key(|c| c.len()); hits
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn overrides_conflicts_vim() {
        let m = load("[keys]\ncomment = \"z\"\n", false).unwrap(); assert_eq!(action_for(&m, "z"), Some("comment")); assert_eq!(action_for(&m, "c"), None);
        assert!(load("[keys]\ncomment = \"v\"\n", false).unwrap_err().contains("bound to both"));
        assert!(load("[keys]\nbogus = \"z\"\n", false).unwrap_err().contains("unknown action"));
        assert_eq!(load("", true).unwrap()["next_file"], "j");
    }
    #[test] fn press_names() {
        assert_eq!(press_name(false, false, true, "/", Some("?")), "?"); assert_eq!(press_name(true, false, false, "p", Some("p")), "Ctrl+p");
        assert_eq!(press_name(false, false, false, "enter", None), "enter");
        let m = default_map(); assert_eq!(action_for(&m, "enter"), Some("next_batch")); assert_eq!(action_for(&m, "?"), Some("help")); assert_eq!(action_for(&m, "Ctrl+p"), Some("palette"));
        assert_eq!(action_for(&m, "N"), None);
    }
    #[test] fn shift_and_files_and_overrides() {
        assert_eq!(press_name(true, false, true, "p", Some("P")), "Ctrl+Shift+p"); assert_eq!(action_for(&default_map(), "Ctrl+Shift+p"), Some("quick_open"));
        assert!(is_reserved("ctrl+f") && !is_reserved("Ctrl+p"));
        let paths = ["src/deep/render.rs", "src/main.rs", "README.md", "docs/main_notes.md"];
        assert_eq!(fuzzy_files(&paths, "main"), vec![1, 3]); // file name hits first, shorter first
        assert_eq!(fuzzy_files(&paths, "deep"), vec![0]); assert_eq!(fuzzy_files(&paths, "srm"), vec![1]); assert!(fuzzy_files(&paths, "zzz").is_empty());
        let mut m = default_map(); assert_eq!(overrides_toml(&m, false), ""); m.insert("comment".into(), "z".into());
        let t = overrides_toml(&m, false); assert_eq!(t, "[keys]\ncomment = \"z\"\n"); assert_eq!(load(&t, false).unwrap()["comment"], "z");
    }
    #[test] fn defaults_have_no_conflicts() { assert!(load("", false).is_ok()); assert!(load("", true).is_ok()); }
    #[test] fn fuzzy() { let c = ["toggle_viewed", "next_batch", "next_tab"]; assert_eq!(palette(&c, "ntb"), vec!["next_tab", "next_batch"]); assert_eq!(palette(&c, "tv"), vec!["toggle_viewed"]); assert!(palette(&c, "zzz").is_empty()); } }
