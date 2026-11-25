//! Filter/sort presets, global and per project (keyed by normalized remote URL).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Preset { #[serde(default)] pub filter: String, #[serde(default)] pub sort: String }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope { #[serde(default)] pub default: Option<String>, #[serde(default)] pub presets: BTreeMap<String, Preset> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Presets { #[serde(default)] pub global: Scope, #[serde(default)] pub projects: BTreeMap<String, Scope> }

/// `git@host:o/r.git`, `https://user@Host/o/r/` -> `host/o/r`
pub fn normalize_remote(url: &str) -> String {
    let mut u = url.trim().to_string();
    if let Some(rest) = u.strip_prefix("git@") { u = rest.replacen(':', "/", 1); }
    for p in ["https://", "http://", "ssh://", "git://"] { if let Some(r) = u.strip_prefix(p) { u = r.to_string(); break; } }
    if let Some((_, r)) = u.split_once('@') { if !r.starts_with('/') { u = r.to_string(); } }
    u.trim_end_matches('/').trim_end_matches(".git").to_lowercase()
}

impl Presets {
    pub fn from_toml(s: &str) -> Result<Self, String> { toml::from_str(s).map_err(|e| e.to_string()) }
    pub fn to_toml(&self) -> String { toml::to_string_pretty(self).unwrap_or_default() }
    /// Project default if set, else global default.
    pub fn effective_default(&self, remote: &str) -> Option<&Preset> {
        let key = normalize_remote(remote);
        if let Some(s) = self.projects.get(&key) {
            if let Some(p) = s.default.as_ref().and_then(|n| s.presets.get(n)) { return Some(p); }
        }
        self.global.default.as_ref().and_then(|n| self.global.presets.get(n))
    }
    pub fn save(&mut self, project: Option<&str>, name: &str, p: Preset) {
        let scope = match project { Some(r) => self.projects.entry(normalize_remote(r)).or_default(), None => &mut self.global };
        scope.presets.insert(name.into(), p);
    }
    pub fn set_default(&mut self, project: Option<&str>, name: Option<&str>) {
        let scope = match project { Some(r) => self.projects.entry(normalize_remote(r)).or_default(), None => &mut self.global };
        scope.default = name.map(String::from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn normalizes() {
        assert_eq!(normalize_remote("git@GitHub.com:Foo/bar.git"), "github.com/foo/bar");
        assert_eq!(normalize_remote("https://tok@github.com/foo/bar/"), "github.com/foo/bar");
    }
    #[test] fn project_default_beats_global_and_roundtrips() {
        let mut p = Presets::default();
        p.save(None, "g", Preset { filter: "!viewed".into(), sort: String::new() }); p.set_default(None, Some("g"));
        assert_eq!(p.effective_default("https://x/y/z").unwrap().filter, "!viewed");
        p.save(Some("git@h:a/b.git"), "mine", Preset { filter: "noisy".into(), sort: "name".into() }); p.set_default(Some("git@h:a/b.git"), Some("mine"));
        assert_eq!(p.effective_default("https://h/a/b").unwrap().filter, "noisy");
        let again = Presets::from_toml(&p.to_toml()).unwrap();
        assert_eq!(again.effective_default("https://h/a/b").unwrap().sort, "name");
        assert_eq!(again.effective_default("https://other/a/b").unwrap().filter, "!viewed");
    }
}
