//! Keymap with defaults, user overrides, vim preset and a fuzzy command palette.
use std::collections::BTreeMap;

pub const DEFAULTS: [(&str, &str); 13] = [("next_hunk", "]"), ("prev_hunk", "["), ("next_file", "n"), ("prev_file", "p"), ("toggle_viewed", "v"), ("next_batch", "Enter"),
    ("prev_batch", "Backspace"), ("comment", "c"), ("ask_ai", "a"), ("focus_filter", "/"), ("focus_graph", "g"), ("next_tab", "Ctrl+Tab"), ("help", "?")];

pub fn default_map() -> BTreeMap<String, String> { DEFAULTS.iter().map(|(a, k)| (a.to_string(), k.to_string())).collect() }

/// Applies `[keys]` overrides from keymap.toml text. Errors on unknown actions or duplicate keys.
pub fn load(user_toml: &str, vim: bool) -> Result<BTreeMap<String, String>, String> {
    let mut m = default_map();
    if vim { for (a, k) in [("next_file", "j"), ("prev_file", "k"), ("next_hunk", "}"), ("prev_hunk", "{")] { m.insert(a.into(), k.into()); } }
    let t: toml::Table = user_toml.parse().map_err(|e: toml::de::Error| e.to_string())?;
    if let Some(k) = t.get("keys").and_then(|v| v.as_table()) {
        for (a, v) in k { if !m.contains_key(a) { return Err(format!("unknown action '{a}'")); }
            m.insert(a.clone(), v.as_str().ok_or(format!("key for '{a}' must be a string"))?.to_string()); }
    }
    let mut seen: BTreeMap<&String, &String> = BTreeMap::new();
    for (a, k) in &m { if let Some(o) = seen.insert(k, a) { return Err(format!("key '{k}' is bound to both '{o}' and '{a}'")); } }
    Ok(m)
}

pub fn action_for<'a>(m: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> { m.iter().find(|(_, k)| *k == key).map(|(a, _)| a.as_str()) }

/// Subsequence fuzzy match, best (shortest) first.
pub fn palette<'a>(commands: &[&'a str], query: &str) -> Vec<&'a str> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut hits: Vec<&str> = commands.iter().copied().filter(|c| { let mut it = c.to_lowercase().chars().collect::<Vec<_>>().into_iter(); q.iter().all(|x| it.any(|y| y == *x)) }).collect();
    hits.sort_by_key(|c| c.len()); hits
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn overrides_conflicts_vim() {
        let m = load("[keys]\ncomment = \"m\"\n", false).unwrap(); assert_eq!(action_for(&m, "m"), Some("comment")); assert_eq!(action_for(&m, "c"), None);
        assert!(load("[keys]\ncomment = \"v\"\n", false).unwrap_err().contains("bound to both"));
        assert!(load("[keys]\nbogus = \"z\"\n", false).unwrap_err().contains("unknown action"));
        assert_eq!(load("", true).unwrap()["next_file"], "j");
    }
    #[test] fn fuzzy() { let c = ["toggle_viewed", "next_batch", "next_tab"]; assert_eq!(palette(&c, "ntb"), vec!["next_tab", "next_batch"]); assert_eq!(palette(&c, "tv"), vec!["toggle_viewed"]); assert!(palette(&c, "zzz").is_empty()); } }
