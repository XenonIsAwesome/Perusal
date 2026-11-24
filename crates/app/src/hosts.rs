//! Pure helpers about git hosts: parsing remotes, guessing a host's kind, reading CLI login output.
use crate::settings::HostCfg;

/// (host, project) from a git remote URL: https, ssh:// and scp-style (git@host:owner/repo.git).
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    let (host, path) = if let Some(rest) = u.strip_prefix("git@") { rest.split_once(':')? }
        else {
            let r = ["https://", "http://", "ssh://", "git://"].iter().find_map(|p| u.strip_prefix(p))?; let r = r.split_once('@').map(|x| x.1).unwrap_or(r); r.split_once('/')? };
    let host = host.split(':').next()?.to_lowercase();
    let project = path.trim_matches('/').trim_end_matches(".git").to_string();
    (!host.is_empty() && project.contains('/')).then_some((host, project))
}

/// "github" or "gitlab" for a host: configured kind first, else guessed from the name.
pub fn kind_of(hosts: &[HostCfg], host: &str) -> &'static str {
    match hosts.iter().find(|h| h.host == host).map(|h| h.kind.as_str()) { Some("gitlab") => "gitlab", Some(_) => "github", None => if host.contains("gitlab") { "gitlab" } else { "github" } }
}

/// Token from `glab auth status --show-token` output ("Token found: glpat-..." or "Token: ...").
pub fn token_from_glab(out: &str) -> Option<String> {
    out.lines().find_map(|l| { let l = l.trim(); let i = l.to_lowercase().find("token")?; let rest = l[i..].split_once(':')?.1.trim(); let t = rest.split_whitespace().next()?; (t.len() > 8 && !t.contains('*')).then(|| t.to_string()) })
}

/// Hosts a `gh auth status` / `glab auth status` listing says the user is logged in to. Both CLIs print the host on its own
/// line (`github.com`, `ghe.corp`) followed by indented status lines, and `glab` also writes `Logged in to <host> as ...`.
pub fn hosts_from_status(out: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for l in out.lines() {
        let t = l.trim().trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
        let host = if let Some(r) = t.split(" to ").nth(1).filter(|_| t.to_lowercase().contains("logged in")) { r.split_whitespace().next() }
            else if !l.starts_with(char::is_whitespace) && !t.is_empty() && !t.contains(' ') && t.contains('.') { Some(t) } else { None };
        if let Some(h) = host { let h = h.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-').to_lowercase(); if h.contains('.') && !v.contains(&h) { v.push(h); } }
    }
    v
}

/// Page where a personal access token with the right scopes can be created.
pub fn token_page(host: &str, kind: &str) -> String {
    if kind == "gitlab" { format!("https://{host}/-/user_settings/personal_access_tokens?name=Perusal&scopes=api,read_repository") }
    else if host == "github.com" { "https://github.com/settings/tokens/new?description=Perusal&scopes=repo".into() } else { format!("https://{host}/settings/tokens/new?description=Perusal&scopes=repo") }
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn remotes_and_kinds() {
        assert_eq!(parse_remote("https://github.com/oven-sh/bun.git"), Some(("github.com".into(), "oven-sh/bun".into())));
        assert_eq!(parse_remote("git@gitlab.corp:grp/sub/proj.git"), Some(("gitlab.corp".into(), "grp/sub/proj".into())));
        assert_eq!(parse_remote("ssh://git@git.x:2222/o/r"), Some(("git.x".into(), "o/r".into()))); assert_eq!(parse_remote("https://tok@host.io/o/r/"), Some(("host.io".into(), "o/r".into())));
        assert!(parse_remote("/local/path").is_none() && parse_remote("https://host/onlyone").is_none());
        let hosts = vec![HostCfg { host: "git.corp".into(), kind: "gitlab".into() }, HostCfg { host: "ghe.corp".into(), kind: "github".into() }];
        assert_eq!((kind_of(&hosts, "git.corp"), kind_of(&hosts, "ghe.corp"), kind_of(&hosts, "gitlab.example"), kind_of(&hosts, "x.y")), ("gitlab", "github", "gitlab", "github"));
    }
    #[test] fn status_hosts() {
        let gh = "github.com\n  ✓ Logged in to github.com account me (keyring)\n  - Active account: true\nghe.corp.io\n  ✓ Logged in to ghe.corp.io account me (keyring)\n";
        assert_eq!(hosts_from_status(gh), vec!["github.com", "ghe.corp.io"]);
        let gl = "gitlab.com\n  ✓ Logged in to gitlab.com as me (/home/x/.config/glab-cli/config.yml)\n  ✓ Token found: **************************\ngit.acme.dev\n  ✓ Logged in to git.acme.dev as me\n";
        assert_eq!(hosts_from_status(gl), vec!["gitlab.com", "git.acme.dev"]); assert!(hosts_from_status("You are not logged in").is_empty());
    }
    #[test] fn glab_token_parsing() {
        assert_eq!(token_from_glab("gitlab.com\n  ✓ Logged in\n  ✓ Token found: glpat-abcdef123456\n").as_deref(), Some("glpat-abcdef123456"));
        assert_eq!(token_from_glab("  Token: **********").as_deref(), None); assert!(token_from_glab("nothing").is_none());
        assert!(token_page("github.com", "github").contains("scopes=repo") && token_page("g.corp", "gitlab").contains("g.corp"));
    } }
