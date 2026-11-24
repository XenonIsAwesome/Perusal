//! Opening reviews (local branch or remote MR/PR), data directories and tokens. Blocking; callers use a background task.
use perusal_app::{graph::GraphVm, vm::ReviewVm};
use perusal_core::store::Store;
use perusal_providers::{auth::{FileStore, TokenStore}, github::GitHub, gitlab::GitLab, Kind, MrRef, Provider};
use std::path::{Path, PathBuf};

pub struct Model {
    pub repo: PathBuf, pub base_sha: String, pub branch: String, pub store: Store, pub vm: ReviewVm, pub graph: GraphVm, pub title: String,
    pub remote: Option<String>,
}

#[derive(Debug)]
pub enum OpenErr { NeedsToken { host: String, why: String }, Msg(String) }
impl std::fmt::Display for OpenErr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { match self { OpenErr::NeedsToken { host, why } => write!(f, "{why} (enter a token for {host})"), OpenErr::Msg(m) => write!(f, "{m}") } }
}
impl From<String> for OpenErr { fn from(s: String) -> Self { OpenErr::Msg(s) } }

pub fn data_dir() -> PathBuf {
    let d = std::env::var_os("PERUSAL_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(|p| PathBuf::from(p).join("perusal")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/perusal")))
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&d); d
}
pub fn tree_pref() -> bool { std::fs::read_to_string(data_dir().join("files_tree")).map(|s| s.trim() == "1").unwrap_or(false) }
pub fn set_tree_pref(on: bool) { let _ = std::fs::write(data_dir().join("files_tree"), if on { "1" } else { "0" }); }
pub fn db_path() -> String { std::env::var("PERUSAL_DB").unwrap_or_else(|_| data_dir().join("perusal.sqlite").display().to_string()) }
pub fn token_store() -> FileStore {
    let d = std::env::var_os("XDG_CONFIG_HOME").map(|p| PathBuf::from(p).join("perusal")).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/perusal"))).unwrap_or_else(|| PathBuf::from("."));
    FileStore { dir: d.join("tokens") }
}
pub fn token_for(host: &str) -> Option<String> { token_store().get(host).or_else(|| std::env::var("PERUSAL_TOKEN").ok().filter(|t| !t.is_empty())) }

fn provider(r: &MrRef, token: String) -> Result<Box<dyn Provider>, String> {
    let ca = std::env::var_os("PERUSAL_CA_FILE").map(PathBuf::from);
    let http = perusal_providers::auth::client(ca.as_deref())?;
    Ok(match r.kind { Kind::GitHub => Box::new(GitHub { http, api: GitHub::api_base(&r.host), token }), Kind::GitLab => Box::new(GitLab { http, api: GitLab::api_base(&r.host), token }) })
}
fn rt() -> Result<tokio::runtime::Runtime, String> { tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string()) }

fn map_err(e: perusal_providers::Error, host: &str) -> OpenErr {
    match e { perusal_providers::Error::Unauthorized => OpenErr::NeedsToken { host: host.into(), why: "the host rejected the token".into() }, o => OpenErr::Msg(o.to_string()) }
}

fn canonical(r: &MrRef) -> String {
    match r.kind { Kind::GitHub => format!("https://{}/{}/pull/{}", r.host, r.project, r.number), Kind::GitLab => format!("https://{}/{}/-/merge_requests/{}", r.host, r.project, r.number) }
}

impl Model {
    fn finish(repo: PathBuf, base_sha: String, rev: String, key: &str, title: String, remote: Option<String>) -> Result<Self, OpenErr> {
        let files = perusal_diff::index_range(&repo, &base_sha, &rev).map_err(|e| e.to_string())?;
        let store = Store::open(&db_path()).map_err(|e| e.to_string())?;
        let review = store.open_review(key, &title, &rev, files.len() as u32).map_err(|e| e.to_string())?;
        let vm = ReviewVm::new(&store, review, files);
        let graph = GraphVm::load(&repo, &base_sha, &rev, 6)?;
        Ok(Model { repo, base_sha, branch: rev, store, vm, graph, title, remote })
    }

    pub fn open_local(repo: &Path, branch: &str, base: Option<&str>) -> Result<Self, OpenErr> {
        if !repo.join(".git").exists() && !repo.join("HEAD").exists() { return Err(OpenErr::Msg(format!("{} is not a git repository", repo.display()))); }
        let base_sha = perusal_gitops::local_base(repo, branch, base)?;
        let abs = repo.canonicalize().unwrap_or(repo.into());
        Self::finish(repo.into(), base_sha, branch.into(), &format!("local:{}#{branch}", abs.display()), format!("{branch} (local)"), None)
    }

    /// Opens a remote MR/PR: host API for metadata, a partial clone cache for code.
    pub fn open_remote(url: &str) -> Result<Self, OpenErr> {
        let r = perusal_providers::parse_mr_url(url)?;
        let token = token_for(&r.host);
        if let Some(t) = &token { perusal_gitops::set_auth(&r.host, t); }
        let p = provider(&r, token.clone().unwrap_or_default())?;
        let rt = rt()?;
        let info = rt.block_on(p.mr_info(&r)).map_err(|e| match (&e, &token) {
            (perusal_providers::Error::Http(404, _), None) => OpenErr::NeedsToken { host: r.host.clone(), why: "not found (private repository?)".into() },
            _ => map_err(e, &r.host) })?;
        let norm = perusal_core::presets::normalize_remote(&format!("https://{}/{}", r.host, r.project));
        let dir = perusal_gitops::cache_dir(&data_dir().join("cache"), &norm);
        let clone_url = format!("https://{}/{}.git", r.host, r.project);
        perusal_gitops::ensure_clone(&clone_url, &dir).map_err(|e| if token.is_none() { OpenErr::NeedsToken { host: r.host.clone(), why: format!("clone failed: {e}") } } else { OpenErr::Msg(format!("clone failed: {e}")) })?;
        let head = perusal_gitops::fetch_mr(&dir, r.kind == Kind::GitLab, r.number)?;
        perusal_gitops::git(&dir, &["fetch", "--filter=blob:none", "origin", &format!("+refs/heads/{}:refs/perusal/base", info.base_ref)])?;
        let base_sha = perusal_gitops::merge_base(&dir, "refs/perusal/base", &head)?;
        let canon = canonical(&r);
        Self::finish(dir, base_sha, head, &format!("remote:{canon}"), info.title, Some(canon))
    }

    /// Reopens a review from the recent list using its stored key.
    pub fn reopen(key: &str) -> Result<Self, OpenErr> {
        if let Some(u) = key.strip_prefix("remote:") { return Self::open_remote(u); }
        let rest = key.strip_prefix("local:").ok_or_else(|| OpenErr::Msg("unknown review kind".into()))?;
        let (path, branch) = rest.rsplit_once('#').ok_or_else(|| OpenErr::Msg("bad review key".into()))?;
        Self::open_local(Path::new(path), branch, None)
    }
}

/// Open MRs of `host/project` (for the browse list): (number, title, url).
pub fn list_remote(target: &str) -> Result<Vec<(u64, String, String)>, OpenErr> {
    let t = target.trim().trim_start_matches("https://").trim_end_matches('/');
    let (host, project) = t.split_once('/').ok_or_else(|| OpenErr::Msg("use the form host/owner/repo, e.g. github.com/rust-lang/rust".into()))?;
    let kind = if host.contains("gitlab") { Kind::GitLab } else { Kind::GitHub };
    let r = MrRef { kind, host: host.into(), project: project.into(), number: 0 };
    let token = token_for(host).unwrap_or_default();
    let p = provider(&r, token)?;
    let list = rt()?.block_on(p.list_open(project)).map_err(|e| map_err(e, host))?;
    Ok(list.into_iter().map(|(n, title)| { let mut rr = r.clone(); rr.number = n; (n, title, canonical(&rr)) }).collect())
}

/// Makes pasted URLs forgiving: trims whitespace and quotes, adds https:// when no scheme is given.
pub fn normalize_url_input(s: &str) -> String {
    let t = s.trim().trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>').trim();
    if t.contains("://") { t.to_string() } else if t.is_empty() { String::new() } else { format!("https://{t}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn url_input_is_forgiving() {
        assert_eq!(normalize_url_input("  github.com/o/r/pull/1 \n"), "https://github.com/o/r/pull/1");
        assert_eq!(normalize_url_input("<https://gitlab.com/a/b/-/merge_requests/2>"), "https://gitlab.com/a/b/-/merge_requests/2");
        assert_eq!(normalize_url_input("http://x/y"), "http://x/y"); assert_eq!(normalize_url_input("   "), "");
    }
}
