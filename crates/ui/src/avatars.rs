//! User pictures. Hosts report an avatar URL per person; `register` remembers it by login, `request` downloads it once in the
//! background into the data dir, and `kit::avatar` draws the file when it is there and the generated placeholder otherwise
//! (while loading, on any failure, or for people with no host account such as a local git committer).
use crate::model;
use perusal_providers::auth::TokenStore;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct State { urls: HashMap<String, String>, inflight: HashSet<String>, failed: HashSet<String> }
fn state() -> &'static Mutex<State> { static S: OnceLock<Mutex<State>> = OnceLock::new(); S.get_or_init(Default::default) }
/// Bumped whenever a download finishes so the UI knows to repaint.
static VERSION: AtomicU64 = AtomicU64::new(0);
pub fn version() -> u64 { VERSION.load(Ordering::Relaxed) }

fn key(login: &str) -> String { login.trim().to_lowercase() }

/// Remembers where a person's picture lives. An empty URL is ignored; a later real URL replaces an earlier guess.
pub fn register(login: &str, url: &str) { if !login.trim().is_empty() && !url.is_empty() { state().lock().unwrap().urls.insert(key(login), url.to_string()); } }
/// Registers a person only if nothing better is known (used for guessed URLs).
pub fn register_guess(login: &str, url: Option<String>) { if let Some(u) = url { state().lock().unwrap().urls.entry(key(login)).or_insert(u); } }
pub fn url_for(login: &str) -> Option<String> { state().lock().unwrap().urls.get(&key(login)).cloned() }

fn path_of(url: &str) -> PathBuf {
    let h = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    model::data_dir().join("avatars").join(format!("{h:016x}"))
}
/// The downloaded file, if there is one.
pub fn cached(url: &str) -> Option<PathBuf> { let p = path_of(url); std::fs::metadata(&p).ok().filter(|m| m.len() > 0).map(|_| p) }

fn host_of(url: &str) -> Option<&str> { url.strip_prefix("https://")?.split(['/', '?', '#']).next() }

/// Starts a background download unless the file exists, one is running, or it failed earlier this run. Only https URLs are fetched, and the
/// access token saved for that exact host (if any) is sent along, for self-hosted instances that protect avatars.
pub fn request(url: &str) {
    if host_of(url).is_none() || cached(url).is_some() { return; }
    { let mut s = state().lock().unwrap(); if s.failed.contains(url) || !s.inflight.insert(url.to_string()) { return; } }
    let url = url.to_string();
    std::thread::spawn(move || {
        let tok = host_of(&url).and_then(|h| model::token_store().get(h));
        let res = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())
            .and_then(|rt| rt.block_on(perusal_providers::fetch_avatar(&url, tok.as_deref().map(|t| ("PRIVATE-TOKEN", t)))).map_err(|e| e.to_string()));
        let ok = res.map(|bytes| { let p = path_of(&url); let _ = std::fs::create_dir_all(p.parent().unwrap()); std::fs::write(&p, bytes).is_ok() }).unwrap_or(false);
        let mut s = state().lock().unwrap(); s.inflight.remove(&url); if !ok { s.failed.insert(url); }
        VERSION.fetch_add(1, Ordering::Relaxed);
    });
}

// ---- learning people from host responses
use perusal_providers::{guess_avatar, Kind, MrInfo, Person, RemoteMr, Thread};
fn learn(kind: &Kind, host: &str, login: &str, avatar: &str) { register(login, avatar); register_guess(login, guess_avatar(kind, host, login)); }
fn learn_all(kind: &Kind, host: &str, ps: &[Person]) { for p in ps { learn(kind, host, &p.login, &p.avatar); } }
pub fn learn_info(kind: &Kind, host: &str, i: &MrInfo) { learn(kind, host, &i.author, &i.author_avatar); learn_all(kind, host, &i.assignees); learn_all(kind, host, &i.reviewers); }
pub fn learn_threads(kind: &Kind, host: &str, ts: &[Thread]) { for t in ts { for (n, (who, _)) in t.comments.iter().enumerate() { learn(kind, host, who, t.avatars.get(n).map(String::as_str).unwrap_or("")); } } }
pub fn learn_mrs(l: &[RemoteMr]) { for m in l { learn(&m.kind, &m.host, &m.author, &m.author_avatar); learn_all(&m.kind, &m.host, &m.assignees); learn_all(&m.kind, &m.host, &m.reviewers); } }

/// Login of the signed-in user per host, to show "You" with their own picture.
fn mes() -> &'static Mutex<HashMap<String, String>> { static S: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new(); S.get_or_init(Default::default) }
pub fn set_me(host: &str, login: &str) { if !login.is_empty() { mes().lock().unwrap().insert(host.to_string(), login.to_string()); } }
pub fn me(host: &str) -> Option<String> { mes().lock().unwrap().get(host).cloned() }

/// The name to draw a git commit author under: the GitHub account in a noreply email, a known login equal to the author name or the
/// email's local part, else the plain git name (so a purely local committer gets a generated placeholder).
pub fn commit_identity(name: &str, email: &str) -> String {
    if let Some(l) = perusal_app::avatar::login_from_noreply(email) { return l; }
    let local = email.split('@').next().unwrap_or("");
    if url_for(name).is_some() { return name.to_string(); }
    if !local.is_empty() && url_for(local).is_some() { return local.to_string(); }
    if name.trim().is_empty() { local.to_string() } else { name.to_string() }
}
