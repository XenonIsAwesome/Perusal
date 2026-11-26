//! Host abstraction: one trait, GitHub and GitLab implementations.
pub mod github;
pub mod gitlab;
pub mod auth;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
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

/// A person on a host: login plus the avatar URL the host reported (empty when it did not).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Person { pub login: String, pub avatar: String }

/// Reads `{login_key, avatar_url}` objects (assignees, reviewers) into people.
pub(crate) fn people(v: &serde_json::Value, login_key: &str) -> Vec<Person> {
    v.as_array().into_iter().flatten().filter_map(|p| { let l = p[login_key].as_str()?; Some(Person { login: l.into(), avatar: p["avatar_url"].as_str().unwrap_or("").into() }) }).collect()
}

/// Best-guess avatar URL when the host did not report one: GitHub serves `/<login>.png`; GitLab needs the API, so no guess.
pub fn guess_avatar(kind: &Kind, host: &str, login: &str) -> Option<String> {
    if login.is_empty() { return None; }
    match kind { Kind::GitHub => Some(format!("https://{host}/{login}.png?size=80")), Kind::GitLab => None }
}

/// Largest avatar we accept.
const AVATAR_MAX: usize = 2_000_000;

/// Downloads an avatar image, never buffering more than `AVATAR_MAX` bytes (the body is streamed and aborted past the cap, so a
/// chunked reply without a Content-Length cannot exhaust memory). A token header is only ever sent to the exact host of `url`: with a
/// token the client follows redirects only within that host, so a redirect to another host (Gravatar, an attacker's server) never sees it.
pub async fn fetch_avatar(url: &str, token: Option<(&str, &str)>) -> Result<Vec<u8>> { fetch_avatar_from(url, token, true).await }

/// `https_only` is false only in tests (mock servers speak plain http).
pub(crate) async fn fetch_avatar_from(url: &str, token: Option<(&str, &str)>, https_only: bool) -> Result<Vec<u8>> {
    let parsed = reqwest::Url::parse(url).map_err(|_| Error::Http(0, "bad avatar URL".into()))?;
    if https_only && parsed.scheme() != "https" { return Err(Error::Http(0, "avatar URL must be https".into())); }
    let origin = (parsed.host_str().unwrap_or("").to_ascii_lowercase(), parsed.port_or_known_default());
    let mut builder = reqwest::Client::builder();
    if token.is_some() {
        builder = builder.redirect(reqwest::redirect::Policy::custom(move |a| {
            let same = a.url().host_str().map(|h| h.to_ascii_lowercase()) == Some(origin.0.clone()) && a.url().port_or_known_default() == origin.1 && (!https_only || a.url().scheme() == "https");
            if same && a.previous().len() < 5 { a.follow() } else { a.stop() }
        }));
    }
    let client = builder.build().map_err(|e| Error::Offline(e.to_string()))?;
    let mut rq = client.get(url).timeout(std::time::Duration::from_secs(15));
    if let Some((h, v)) = token { rq = rq.header(h, v); }
    let mut resp = check(rq.send().await).await?;
    if resp.content_length().map(|n| n as usize > AVATAR_MAX).unwrap_or(false) { return Err(Error::Http(0, "avatar too large".into())); }
    let mut out: Vec<u8> = vec![];
    while let Some(chunk) = resp.chunk().await.map_err(|e| Error::Offline(e.to_string()))? {
        if out.len() + chunk.len() > AVATAR_MAX { return Err(Error::Http(0, "avatar too large".into())); }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MrInfo { pub title: String, pub state: String, pub author: String, pub head_sha: String, pub base_ref: String, pub web_url: String, pub description: String, pub source_ref: String,
    #[serde(default)] pub author_avatar: String,
    #[serde(default)] pub assignees: Vec<Person>,
    #[serde(default)] pub reviewers: Vec<Person> }

/// Why a merge request is in your list. `Reviewable` is every request in a project you can review in (the default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role { ReviewRequested, Authored, Assigned, Reviewable }

/// Which states to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateFilter { Open, Merged, Closed, All }

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteMr { pub url: String, pub title: String, pub project: String, pub number: u64, pub author: String, pub state: String, pub updated: String, pub host: String, pub kind: Kind,
    pub author_avatar: String, pub assignees: Vec<Person>, pub reviewers: Vec<Person>, pub labels: Vec<String>, pub description: String, pub draft: bool }

#[derive(Debug, Clone, PartialEq)]
pub enum Ci { None, Pending, Running, Passed, Failed, Canceled }

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict { Approve, RequestChanges, Comment }

#[derive(Debug, Clone, PartialEq)]
pub struct InlineComment { pub path: String, pub start_line: u32, pub end_line: u32, pub body: String,
    /// Old-side line numbers paired with `start_line`/`end_line` (GitLab line codes need them); 0 when unknown.
    pub start_old: u32, pub end_old: u32,
    /// Line numbers are on the old side of the diff (a comment on a removed line).
    pub old_side: bool }

#[derive(Debug, Clone, PartialEq)]
pub struct Submission { pub head_sha: String, pub summary: String, pub verdict: Verdict, pub comments: Vec<InlineComment>,
    /// Whole-MR comments: each is posted as its own conversation thread, not folded into the review summary.
    pub general: Vec<String> }

#[derive(Debug, Clone, PartialEq)]
pub struct Thread { pub id: String, pub path: Option<String>, pub line: Option<u32>, pub resolved: bool, pub comments: Vec<(String, String)>,
    /// Host ids of `comments`, same order (used to react to, edit or delete a single comment).
    pub comment_ids: Vec<String>,
    /// GitHub only: the GraphQL id needed to resolve/unresolve the thread (`None` when it could not be fetched).
    pub node_id: Option<String>,
    /// Avatar URL of each comment's author, parallel to `comments` (empty string when unknown).
    pub avatars: Vec<String> }
impl Thread {
    /// The id to pass to `resolve` for this host, if resolving is possible.
    pub fn resolve_id(&self, kind: Kind) -> Option<&str> { match kind { Kind::GitLab => Some(self.id.as_str()), Kind::GitHub => self.node_id.as_deref() } }
}

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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    #[tokio::test] async fn avatar_token_is_not_sent_to_a_redirect_target() {
        let (a, b) = (MockServer::start().await, MockServer::start().await);
        Mock::given(method("GET")).and(path("/img")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8, 2, 3])).mount(&b).await;
        Mock::given(method("GET")).and(path("/av")).respond_with(ResponseTemplate::new(302).insert_header("location", format!("{}/img", b.uri()))).mount(&a).await;
        // a redirect to another origin is not followed, so the token header never reaches it
        assert!(fetch_avatar_from(&format!("{}/av", a.uri()), Some(("PRIVATE-TOKEN", "secret")), false).await.is_err());
        assert!(b.received_requests().await.unwrap().is_empty());
        // without a token redirects are followed as usual
        assert_eq!(fetch_avatar_from(&format!("{}/av", a.uri()), None, false).await.unwrap(), vec![1u8, 2, 3]);
        // a same-origin redirect keeps working with a token
        Mock::given(method("GET")).and(path("/same")).respond_with(ResponseTemplate::new(302).insert_header("location", "/real")).mount(&a).await;
        Mock::given(method("GET")).and(path("/real")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9u8])).mount(&a).await;
        assert_eq!(fetch_avatar_from(&format!("{}/same", a.uri()), Some(("PRIVATE-TOKEN", "secret")), false).await.unwrap(), vec![9u8]);
    }
    #[tokio::test] async fn avatar_download_is_capped_even_without_content_length() {
        let s = MockServer::start().await;
        Mock::given(method("GET")).and(path("/big")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; AVATAR_MAX + 1])).mount(&s).await;
        assert!(matches!(fetch_avatar_from(&format!("{}/big", s.uri()), None, false).await, Err(Error::Http(0, _))));
        assert!(fetch_avatar("http://example.com/x.png", None).await.is_err());
    }
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
