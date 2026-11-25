//! Agent safety: minimal environment, per-review repo allowlist, visible access log, read-only permissions.
use std::collections::BTreeMap;

const KEEP: [&str; 9] = ["PATH", "HOME", "LANG", "TERM", "TMPDIR", "USER", "LOGNAME", "SHELL", "NODE_PATH"];
/// Variables that only describe where tools live (version managers, XDG dirs, locale); launchers and shims of agent CLIs need them to find themselves.
const KEEP_PREFIX: [&str; 16] = ["XDG_", "LC_", "VOLTA_", "ASDF_", "MISE_", "NVM_", "FNM_", "PNPM_", "BUN_", "NPM_CONFIG_PREFIX", "CARGO_HOME", "RUSTUP_HOME", "HOMEBREW_", "PYENV", "SDKMAN", "NODENV"];
const SECRET_HINTS: [&str; 7] = ["TOKEN", "SECRET", "KEY", "PASSWORD", "CREDENTIAL", "COOKIE", "AUTH"];

/// Minimal environment for an agent: a short allowlist, nothing that looks like a credential.
pub fn minimal_env(parent: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    parent.iter().filter(|(k, _)| (KEEP.contains(&k.as_str()) || KEEP_PREFIX.iter().any(|p| k.starts_with(p))) && !SECRET_HINTS.iter().any(|h| k.to_uppercase().contains(h))).map(|(k, v)| (k.clone(), v.clone())).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Access { WebSearch(String), WebFetch(String), RepoRead(String), Denied(String) }

#[derive(Default)]
pub struct Policy { pub repo_allowlist: Vec<String>, pub allow_edit_this_session: bool, pub log: Vec<Access> }

#[derive(Debug, PartialEq)]
pub enum Decision { Allow, Deny(String) }

impl Policy {
    /// Decides an ACP permission request. `kind` is the tool kind: read, search, fetch, edit, execute...
    /// Every web request and extra-repository read is logged so the chat can show it.
    pub fn decide(&mut self, kind: &str, target: &str) -> Decision {
        let d = match kind {
            "read" => if target.starts_with("repo:") { if self.repo_allowlist.iter().any(|r| target == format!("repo:{r}")) { self.log.push(Access::RepoRead(target.into())); Decision::Allow } else { Decision::Deny(format!("repository {target} is not on this review's allowlist")) } } else { Decision::Allow },
            "search" => { self.log.push(Access::WebSearch(target.into())); Decision::Allow }
            "fetch" => { self.log.push(Access::WebFetch(target.into())); Decision::Allow }
            "edit" | "delete" | "move" if self.allow_edit_this_session => Decision::Allow,
            "execute" if self.allow_edit_this_session => Decision::Allow,
            other => Decision::Deny(format!("'{other}' is not permitted: agents are read-only unless you grant more for this session")),
        };
        if let Decision::Deny(_) = d { self.log.push(Access::Denied(format!("{kind} {target}"))); }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn env_is_minimal_and_secretless() {
        let mut e = BTreeMap::new();
        for (k, v) in [("PATH", "/bin"), ("HOME", "/h"), ("GITHUB_TOKEN", "x"), ("PERUSAL_TOKEN", "x"), ("AWS_SECRET_ACCESS_KEY", "x"), ("SSH_AUTH_SOCK", "x"), ("LANG", "C")] { e.insert(k.to_string(), v.to_string()); }
        let m = minimal_env(&e); assert_eq!(m.keys().cloned().collect::<Vec<_>>(), vec!["HOME", "LANG", "PATH"]);
    }
    #[test] fn read_only_by_default_and_logged() {
        let mut p = Policy::default(); p.repo_allowlist.push("o/dep".into());
        assert_eq!(p.decide("read", "src/a.rs"), Decision::Allow);
        assert_eq!(p.decide("read", "repo:o/dep"), Decision::Allow); assert!(matches!(p.decide("read", "repo:evil/x"), Decision::Deny(_)));
        assert_eq!(p.decide("search", "rust lifetimes"), Decision::Allow); assert_eq!(p.decide("fetch", "https://docs.rs"), Decision::Allow);
        assert!(matches!(p.decide("edit", "a.rs"), Decision::Deny(_))); assert!(matches!(p.decide("execute", "rm -rf /"), Decision::Deny(_)));
        assert!(p.log.contains(&Access::WebFetch("https://docs.rs".into()))); assert!(p.log.contains(&Access::Denied("execute rm -rf /".into())));
        p.allow_edit_this_session = true; assert_eq!(p.decide("edit", "a.rs"), Decision::Allow);
    }
}
