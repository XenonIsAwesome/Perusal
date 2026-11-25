//! Host abstraction: one trait, GitHub and GitLab implementations.
pub mod github;
pub mod gitlab;
pub mod auth;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind { GitHub, GitLab }

#[derive(Debug, Clone, PartialEq)]
pub struct MrRef { pub kind: Kind, pub host: String, pub project: String, pub number: u64 }

/// Accepts only https MR/PR URLs.
pub fn parse_mr_url(url: &str) -> std::result::Result<MrRef, String> {
    let rest = url.strip_prefix("https://").ok_or("only https MR URLs are accepted".to_string())?;
    let (host, path) = rest.split_once('/').ok_or("missing path".to_string())?;
    let path = path.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    if let Some((proj, n)) = path.split_once("/-/merge_requests/") {
        let n = n.split('/').next().unwrap_or("");
        return Ok(MrRef { kind: Kind::GitLab, host: host.into(), project: proj.into(), number: n.parse().map_err(|_| "bad MR number".to_string())? });
    }
    let p: Vec<&str> = path.split('/').collect();
    if p.len() >= 4 && p[2] == "pull" {
        return Ok(MrRef { kind: Kind::GitHub, host: host.into(), project: format!("{}/{}", p[0], p[1]), number: p[3].parse().map_err(|_| "bad PR number".to_string())? });
    }
    Err("not a merge request or pull request URL".into())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MrInfo { pub title: String, pub state: String, pub author: String, pub head_sha: String, pub base_ref: String, pub web_url: String }

/// Why a merge request is in your list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role { ReviewRequested, Authored, Assigned }

/// Which states to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateFilter { Open, Merged, Closed, All }

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteMr { pub url: String, pub title: String, pub project: String, pub number: u64, pub author: String, pub state: String, pub updated: String, pub host: String, pub kind: Kind }

#[derive(Debug, Clone, PartialEq)]
pub enum Ci { None, Pending, Running, Passed, Failed, Canceled }

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict { Approve, RequestChanges, Comment }

#[derive(Debug, Clone, PartialEq)]
pub struct InlineComment { pub path: String, pub start_line: u32, pub end_line: u32, pub body: String }

#[derive(Debug, Clone, PartialEq)]
pub struct Submission { pub head_sha: String, pub summary: String, pub verdict: Verdict, pub comments: Vec<InlineComment> }

#[derive(Debug, Clone, PartialEq)]
pub struct Thread { pub id: String, pub path: Option<String>, pub line: Option<u32>, pub resolved: bool, pub comments: Vec<(String, String)> }

#[derive(Debug)]
pub enum Error { Unauthorized, RateLimited(Option<u64>), Offline(String), Unsupported(&'static str), Http(u16, String) }

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Unauthorized => write!(f, "unauthorized: check or renew the access token"),
            Error::RateLimited(Some(s)) => write!(f, "rate limited, retry in {s}s"), Error::RateLimited(None) => write!(f, "rate limited"),
            Error::Offline(e) => write!(f, "offline or unreachable: {e}"), Error::Unsupported(w) => write!(f, "not supported by this host: {w}"),
            Error::Http(c, b) => write!(f, "http {c}: {b}"),
        }
    }
}
impl std::error::Error for Error {}
/// Page in the browser that shows this MR's CI (checks on GitHub, pipelines on GitLab).
pub fn ci_page(kind: &Kind, web_url: &str) -> String { match kind { Kind::GitHub => format!("{web_url}/checks"), Kind::GitLab => format!("{web_url}/pipelines") } }

/// What the app does about an error. Offline keeps local review fully usable; drafts are never lost.
#[derive(Debug, PartialEq)]
pub enum Action { Relogin, RetryAfter(u64), WorkOffline, HideControl, ShowMessage }

impl Error {
    pub fn action(&self) -> Action {
        match self { Error::Unauthorized => Action::Relogin, Error::RateLimited(s) => Action::RetryAfter(s.unwrap_or(60)),
            Error::Offline(_) => Action::WorkOffline, Error::Unsupported(_) => Action::HideControl, Error::Http(..) => Action::ShowMessage }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[async_trait]
pub trait Provider: Send + Sync {
    async fn mr_info(&self, r: &MrRef) -> Result<MrInfo>;
    async fn threads(&self, r: &MrRef) -> Result<Vec<Thread>>;
    /// Posts everything or nothing.
    async fn submit(&self, r: &MrRef, s: &Submission) -> Result<()>;
    async fn ci(&self, r: &MrRef, head_sha: &str) -> Result<Ci>;
    async fn request_review(&self, r: &MrRef, users: &[String]) -> Result<()>;
    /// Sync a viewed mark to the host; hosts without support return `Unsupported` and the mark stays local.
    async fn set_viewed(&self, r: &MrRef, path: &str, viewed: bool) -> Result<()>;
    async fn list_open(&self, project: &str) -> Result<Vec<(u64, String)>>;
    /// Thread actions go to the host immediately, outside the pending review.
    async fn reply(&self, _r: &MrRef, _thread: &str, _body: &str) -> Result<()> { Err(Error::Unsupported("reply")) }
    async fn resolve(&self, _r: &MrRef, _thread: &str, _resolved: bool) -> Result<()> { Err(Error::Unsupported("resolve")) }
    async fn react(&self, _r: &MrRef, _thread: &str, _comment: &str, _emoji: &str) -> Result<()> { Err(Error::Unsupported("reactions")) }
    async fn edit_comment(&self, _r: &MrRef, _thread: &str, _comment: &str, _body: &str) -> Result<()> { Err(Error::Unsupported("edit")) }
    async fn delete_comment(&self, _r: &MrRef, _thread: &str, _comment: &str) -> Result<()> { Err(Error::Unsupported("delete")) }
    /// Login of the authenticated user.
    async fn current_user(&self) -> Result<String>;
    /// The user's own merge/pull requests by role and state (needs a token).
    async fn my_reviews(&self, host: &str, role: Role, state: StateFilter) -> Result<Vec<RemoteMr>>;
    /// Open MR/PR number whose source branch is `branch`, for attaching a local-branch review.
    async fn find_for_branch(&self, project: &str, branch: &str) -> Result<Option<u64>>;
}

pub(crate) async fn check(resp: std::result::Result<reqwest::Response, reqwest::Error>) -> Result<reqwest::Response> {
    let resp = resp.map_err(|e| Error::Offline(e.without_url().to_string()))?;
    let c = resp.status().as_u16();
    match c {
        200..=299 => Ok(resp),
        401 => Err(Error::Unauthorized),
        429 => Err(Error::RateLimited(resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()))),
        403 if resp.headers().get("x-ratelimit-remaining").map(|v| v == "0").unwrap_or(false) => Err(Error::RateLimited(None)),
        _ => Err(Error::Http(c, resp.text().await.unwrap_or_default().chars().take(300).collect())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn error_actions() {
        assert_eq!(Error::Unauthorized.action(), Action::Relogin); assert_eq!(Error::RateLimited(Some(5)).action(), Action::RetryAfter(5)); assert_eq!(Error::RateLimited(None).action(), Action::RetryAfter(60));
        assert_eq!(Error::Offline("x".into()).action(), Action::WorkOffline); assert_eq!(Error::Unsupported("x").action(), Action::HideControl);
    }
    #[test] fn urls() {
        assert_eq!(parse_mr_url("https://github.com/o/r/pull/12/files").unwrap(), MrRef { kind: Kind::GitHub, host: "github.com".into(), project: "o/r".into(), number: 12 });
        let g = parse_mr_url("https://git.corp/a/b/c/-/merge_requests/9?x=1").unwrap();
        assert_eq!((g.kind, g.project.as_str(), g.number), (Kind::GitLab, "a/b/c", 9));
        assert!(parse_mr_url("http://github.com/o/r/pull/1").is_err()); assert!(parse_mr_url("https://github.com/o/r").is_err());
        assert!(parse_mr_url("perusal://x").is_err());
    }
}
