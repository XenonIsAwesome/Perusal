//! Opening reviews (local branch or remote MR/PR), data directories and tokens. Blocking; callers use a background task.
use perusal_app::{graph::GraphVm, vm::ReviewVm};
use perusal_core::store::Store;
use perusal_app::settings::HostCfg;
use perusal_providers::{auth::{FileStore, TokenStore}, github::GitHub, gitlab::GitLab, Ci, Kind, MrRef, Provider, RemoteMr, Role, StateFilter, Thread};
use std::path::{Path, PathBuf};

pub struct Model {
    pub repo: PathBuf, pub base_sha: String, pub branch: String, pub store: Store, pub vm: ReviewVm, pub graph: GraphVm, pub title: String,
    pub remote: Option<String>,
    /// Set for remote reviews: which host, project and number to post the review to.
    pub mr: Option<MrRef>,
    /// Web address of the MR (CI page, links).
    pub web_url: Option<String>,
    /// MR state when opened (open, merged, closed).
    pub mr_state: String,
    /// Host metadata of the MR when it was opened (description for the overview).
    pub info: Option<perusal_providers::MrInfo>,
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
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(|p| PathBuf::from(p).join("perusal")).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/perusal"))).unwrap_or_else(|| PathBuf::from("."))
}
pub fn themes_dir() -> PathBuf { config_dir().join("themes") }
/// Editable prompt templates (`~/.config/perusal/prompts`); the defaults and an AGENTS.md are written there on first use.
pub fn prompts_dir() -> PathBuf { config_dir().join("prompts") }
/// Prompt templates with the user's overrides. Creates the folder and missing default files once per run.
pub fn prompts() -> perusal_agents::Prompts {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| perusal_agents::ensure_prompt_files(&prompts_dir()));
    perusal_agents::Prompts::new(prompts_dir())
}
pub fn load_settings() -> perusal_app::settings::Settings { std::fs::read_to_string(config_dir().join("settings.toml")).map(|s| perusal_app::settings::Settings::from_toml(&s)).unwrap_or_default() }
pub fn save_settings(s: &perusal_app::settings::Settings) { let _ = std::fs::create_dir_all(config_dir()); let _ = std::fs::write(config_dir().join("settings.toml"), s.to_toml()); }
/// Where the read-only code snapshot for agents lives, per review and head.
pub fn snapshot_dir(review: i64, rev: &str) -> PathBuf { data_dir().join("snapshots").join(format!("{review}-{}", rev[..rev.len().min(12)].replace('/', "_"))) }
pub fn db_path() -> String { std::env::var("PERUSAL_DB").unwrap_or_else(|_| data_dir().join("perusal.sqlite").display().to_string()) }
pub fn token_store() -> FileStore { FileStore { dir: config_dir().join("tokens") } }
pub fn token_for(host: &str) -> Option<String> { token_store().get(host).or_else(|| std::env::var("PERUSAL_TOKEN").ok().filter(|t| !t.is_empty())) }

fn provider(r: &MrRef, token: String) -> Result<Box<dyn Provider>, String> {
    let ca = std::env::var_os("PERUSAL_CA_FILE").map(PathBuf::from);
    let http = perusal_providers::auth::client(ca.as_deref())?;
    let over = std::env::var("PERUSAL_API_BASE").ok().filter(|s| !s.is_empty());
    Ok(match r.kind { Kind::GitHub => Box::new(GitHub { http, api: over.unwrap_or_else(|| GitHub::api_base(&r.host)), token }), Kind::GitLab => Box::new(GitLab { http, api: over.unwrap_or_else(|| GitLab::api_base(&r.host)), token }) })
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
        // a name the user gave the review wins over the one derived from the branch or MR
        let title = store.review_title(review).ok().filter(|t| !t.is_empty()).unwrap_or(title);
        let vm = ReviewVm::new(&store, review, files);
        let graph = GraphVm::load(&repo, &base_sha, &rev, 6)?;
        Ok(Model { repo, base_sha, branch: rev, store, vm, graph, title, remote, mr: None, web_url: None, mr_state: String::new(), info: None })
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
        let clone_url = format!("{}/{}.git", std::env::var("PERUSAL_GIT_BASE").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("https://{}", r.host)), r.project);
        perusal_gitops::ensure_clone(&clone_url, &dir).map_err(|e| if token.is_none() { OpenErr::NeedsToken { host: r.host.clone(), why: format!("clone failed: {e}") } } else { OpenErr::Msg(format!("clone failed: {e}")) })?;
        let head = perusal_gitops::fetch_mr(&dir, r.kind == Kind::GitLab, r.number)?;
        perusal_gitops::git(&dir, &["fetch", "--filter=blob:none", "origin", &format!("+refs/heads/{}:refs/perusal/base", info.base_ref)])?;
        let base_sha = perusal_gitops::merge_base(&dir, "refs/perusal/base", &head)?;
        let canon = canonical(&r);
        let mut m = Self::finish(dir, base_sha, head, &format!("remote:{canon}"), info.title.clone(), Some(canon))?;
        crate::avatars::learn_info(&r.kind, &r.host, &info);
        m.info = Some(info.clone()); m.web_url = Some(info.web_url.clone()); m.mr_state = norm_state(&info.state); let _ = m.store.set_mr_state(m.vm.review, &m.mr_state); m.mr = Some(r); Ok(m)
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

/// Posts a finished review to the host (all comments in one call where the host allows). Blocking.
pub fn submit_review(mr: &MrRef, sub: &perusal_providers::Submission) -> Result<(), String> {
    let token = token_for(&mr.host).ok_or_else(|| format!("No access token for {}. Open the review from Home to enter one.", mr.host))?;
    let p = provider(mr, token)?;
    rt()?.block_on(p.submit(mr, sub)).map_err(|e| match e {
        perusal_providers::Error::Unauthorized => format!("{} rejected the token. Enter a new one from Home.", mr.host),
        perusal_providers::Error::Unsupported(w) => format!("{} does not support: {w}. Choose another verdict.", mr.host),
        other => other.to_string(),
    })
}

/// Opens a folder in the platform file manager, creating it first.
pub fn open_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    let (prog, args): (&str, Vec<&std::ffi::OsStr>) = if cfg!(target_os = "macos") { ("open", vec![path.as_os_str()]) } else if cfg!(target_os = "windows") { ("explorer", vec![path.as_os_str()]) } else { ("xdg-open", vec![path.as_os_str()]) };
    std::process::Command::new(prog).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("cannot start {prog}: {e}"))
}

/// Writes the AGENTS.md that explains the theme format to an AI agent, if the folder does not have one yet.
pub fn ensure_theme_docs() -> PathBuf {
    let d = themes_dir(); let _ = std::fs::create_dir_all(&d);
    let f = d.join("AGENTS.md");
    if !f.exists() { let _ = std::fs::write(&f, include_str!("../../app/themes/AGENTS.md")); }
    d
}

/// Saves a Markdown export of a local review and returns its path.
pub fn export_markdown_file(title: &str, md: &str) -> Result<PathBuf, String> {
    let d = data_dir().join("exports"); std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let f = d.join(format!("{safe}-{stamp}.md")); std::fs::write(&f, md).map_err(|e| e.to_string())?; Ok(f)
}

pub fn norm_state(s: &str) -> String { match s { "opened" | "open" => "open".into(), "merged" => "merged".into(), _ => "closed".into() } }

// ---- presets
pub fn presets_path() -> PathBuf { config_dir().join("presets.toml") }
pub fn load_presets() -> perusal_core::presets::Presets { std::fs::read_to_string(presets_path()).ok().and_then(|s| perusal_core::presets::Presets::from_toml(&s).ok()).unwrap_or_default() }
pub fn save_presets(p: &perusal_core::presets::Presets) { let _ = std::fs::create_dir_all(config_dir()); let _ = std::fs::write(presets_path(), p.to_toml()); }
/// The project a review belongs to for per-project presets: the MR's repository, or a local repo's origin remote.
pub fn project_of(m: &Model) -> String {
    if let Some(u) = &m.remote { return u.clone(); }
    perusal_gitops::git(&m.repo, &["remote", "get-url", "origin"]).unwrap_or_else(|_| m.repo.display().to_string())
}

// ---- system integration
pub fn open_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") { return Err("only web links can be opened".into()); }
    let prog = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    std::process::Command::new(prog).arg(url).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("cannot start {prog}: {e}"))
}

/// Best-effort desktop notification (notify-send on Linux, osascript on macOS). Never blocks or fails the caller.
pub fn desktop_notify(title: &str, body: &str) {
    let (t, b) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let _ = if cfg!(target_os = "macos") {
            std::process::Command::new("osascript").args(["-e", &format!("display notification {:?} with title {:?}", b, format!("Perusal: {t}"))]).output()
        } else if cfg!(target_os = "windows") { Ok(std::process::Output { status: Default::default(), stdout: vec![], stderr: vec![] }) }
        else { std::process::Command::new("notify-send").args(["--app-name=Perusal", &format!("Perusal: {t}"), &b]).output() };
    });
}

// ---- remote state, listing, accounts
pub struct RemoteState { pub info: perusal_providers::MrInfo, pub ci: Ci, pub threads: Vec<Thread> }

pub fn fetch_remote_state(mr: &MrRef) -> Result<RemoteState, String> {
    let token = token_for(&mr.host).unwrap_or_default();
    let p = provider(mr, token)?; let rt = rt()?;
    let info = rt.block_on(p.mr_info(mr)).map_err(|e| e.to_string())?;
    let ci = rt.block_on(p.ci(mr, &info.head_sha)).unwrap_or(Ci::None);
    let threads = rt.block_on(p.threads(mr)).unwrap_or_default();
    crate::avatars::learn_info(&mr.kind, &mr.host, &info); crate::avatars::learn_threads(&mr.kind, &mr.host, &threads);
    if crate::avatars::me(&mr.host).is_none() { if let Ok(u) = rt.block_on(p.current_user()) { crate::avatars::set_me(&mr.host, &u); } }
    Ok(RemoteState { info, ci, threads })
}
pub fn ci_label(c: &Ci) -> &'static str { match c { Ci::None => "none", Ci::Pending => "pending", Ci::Running => "running", Ci::Passed => "passed", Ci::Failed => "failed", Ci::Canceled => "canceled" } }

/// The user's reviews across every host they are signed in to. Hosts without a token are skipped; failures are reported per host.
pub fn fetch_my_reviews(hosts: &[HostCfg], role: Role, state: StateFilter) -> (Vec<RemoteMr>, Vec<(String, String)>) {
    let (mut all, mut errs) = (vec![], vec![]);
    let Ok(rt) = rt() else { return (all, errs) };
    for h in hosts {
        let Some(token) = token_for(&h.host) else { continue };
        let kind = if h.kind == "gitlab" { Kind::GitLab } else { Kind::GitHub };
        let r = MrRef { kind, host: h.host.clone(), project: String::new(), number: 0 };
        let p = match provider(&r, token) { Ok(p) => p, Err(e) => { errs.push((h.host.clone(), e)); continue } };
        match rt.block_on(p.my_reviews(&h.host, role, state)) { Ok(mut l) => all.append(&mut l), Err(e) => errs.push((h.host.clone(), e.to_string())) }
    }
    all.sort_by(|a, b| b.updated.cmp(&a.updated));
    crate::avatars::learn_mrs(&all);
    (all, errs)
}

/// Signed-in user for a host (validates the token).
pub fn account_user(host: &str, kind: &str) -> Result<String, String> {
    let token = token_for(host).ok_or("not signed in")?;
    let r = MrRef { kind: if kind == "gitlab" { Kind::GitLab } else { Kind::GitHub }, host: host.into(), project: String::new(), number: 0 };
    rt()?.block_on(provider(&r, token)?.current_user()).map_err(|e| e.to_string())
}

/// Reads a token from the `gh` / `glab` CLI login, if installed and logged in.
pub fn import_cli_token(host: &str, kind: &str) -> Result<String, String> {
    let (prog, args): (&str, Vec<&str>) = if kind == "gitlab" { ("glab", vec!["auth", "status", "--show-token", "--hostname", host]) } else { ("gh", vec!["auth", "token", "--hostname", host]) };
    let o = std::process::Command::new(prog).args(&args).stdin(std::process::Stdio::null()).output().map_err(|_| format!("`{prog}` is not installed"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let tok = if kind == "gitlab" { perusal_app::hosts::token_from_glab(&text) } else { Some(String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|t| !t.is_empty() && o.status.success()) };
    tok.ok_or_else(|| format!("`{prog}` has no login for {host}. Run `{prog} auth login` first."))
}

pub fn device_start(host: &str, client_id: &str) -> Result<perusal_providers::auth::DeviceCode, String> {
    let base = format!("https://{host}"); let c = perusal_providers::auth::client(None)?;
    rt()?.block_on(perusal_providers::auth::device_start(&c, &base, client_id))
}
/// Waits for the user to approve the device code (up to ~10 minutes) and returns the token.
pub fn device_wait(host: &str, client_id: &str, d: &perusal_providers::auth::DeviceCode) -> Result<String, String> {
    let base = format!("https://{host}"); let c = perusal_providers::auth::client(None)?;
    let mut dd = d.clone(); dd.interval = d.interval.max(5) * 1000;
    rt()?.block_on(perusal_providers::auth::device_poll(&c, &base, client_id, &dd, 120))
}

/// Finds the open MR/PR for a local branch on its origin host, if the user is signed in there (or the repo is public).
pub fn find_mr_for_branch(repo: &Path, branch: &str, hosts: &[HostCfg]) -> Option<(MrRef, String)> {
    let url = perusal_gitops::git(repo, &["remote", "get-url", "origin"]).ok()?;
    let (host, project) = perusal_app::hosts::parse_remote(&url)?;
    let kind = if perusal_app::hosts::kind_of(hosts, &host) == "gitlab" { Kind::GitLab } else { Kind::GitHub };
    let r = MrRef { kind: kind.clone(), host: host.clone(), project: project.clone(), number: 0 };
    let p = provider(&r, token_for(&host).unwrap_or_default()).ok()?;
    let n = rt().ok()?.block_on(p.find_for_branch(&project, branch)).ok()??;
    let web = match kind { Kind::GitHub => format!("https://{host}/{project}/pull/{n}"), Kind::GitLab => format!("https://{host}/{project}/-/merge_requests/{n}") };
    Some((MrRef { number: n, ..r }, web))
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

/// Asks the configured agent to write a filter, sort or grouping rule (blocking; run it on a background thread).
/// The answer is validated with the real parsers before it is returned: `[expr]` or `[label, pattern]`.
pub fn run_gen(s: &perusal_app::settings::Settings, kind: perusal_agents::GenKind, request: &str, cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<String>, String> {
    use perusal_agents::{launch_with, on_path, parse_generated, GenKind, Options};
    if request.trim().is_empty() { return Err("Describe what you want first.".into()); }
    let reference = if kind == GenKind::GroupRule { String::new() } else { perusal_filter::reference() };
    let prompt = prompts().generate(kind, request.trim(), &reference);
    let l = launch_with(&s.agent, &s.agent_command, &prompt, &Options { model: s.model.clone(), effort: s.effort.clone(), path: s.agent_path.clone() }).ok_or("No agent configured. Choose one in Settings.")?;
    if !on_path(&l.program) { return Err(format!("`{}` was not found on your PATH. Install it or pick another agent in Settings.", l.program)); }
    let dir = std::env::temp_dir().join("perusal-gen"); let _ = std::fs::create_dir_all(&dir);
    let out = perusal_agents::run::run_streaming(&l, &dir, &perusal_agents::run::agent_env(), std::time::Duration::from_secs(180), cancel, |_| {}).map_err(|e| e.to_string())?;
    let v = parse_generated(&out, match kind { GenKind::Filter => &["filter"][..], GenKind::Sort => &["sort"], GenKind::GroupRule => &["label", "pattern"] })?;
    match kind {
        GenKind::Filter => { perusal_filter::compile(&v[0]).map_err(|e| format!("The agent wrote an invalid filter ({e}): {}", v[0]))?; }
        GenKind::Sort => { for (e, _) in perusal_filter::parse_tiers(&v[0]) { perusal_filter::compile(&e).map_err(|er| format!("The agent wrote an invalid sort ({er}): {}", v[0]))?; } }
        GenKind::GroupRule => perusal_diff::rules::validate_pattern(&v[1]).map_err(|e| format!("The agent wrote an invalid rule: {e}"))?,
    }
    Ok(v)
}

/// Imports every login the `gh` and `glab` CLIs know about, including GitHub Enterprise and self-hosted GitLab.
/// Returns (host, kind, token) for each login whose token could be read.
pub fn import_all_cli() -> Vec<(String, String, String)> {
    let mut out = vec![];
    for (prog, kind, args) in [("gh", "github", vec!["auth", "status"]), ("glab", "gitlab", vec!["auth", "status"])] {
        let Ok(o) = std::process::Command::new(prog).args(&args).stdin(std::process::Stdio::null()).output() else { continue };
        let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        for host in perusal_app::hosts::hosts_from_status(&text) {
            if let Ok(tok) = import_cli_token(&host, kind) { out.push((host, kind.to_string(), tok)); }
        }
    }
    out
}
