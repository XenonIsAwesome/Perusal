//! git CLI wrapper: partial-clone cache, MR-ref fetch, worktrees.
use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T> = std::result::Result<T, String>;

pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git").args(args).current_dir(dir).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()) }
    else { Err(redact(&String::from_utf8_lossy(&o.stderr))) }
}

/// Strips credentials from URLs in log/error text.
pub fn redact(s: &str) -> String {
    let mut out = String::new();
    for (i, w) in s.split(' ').enumerate() {
        if i > 0 { out.push(' '); }
        match (w.find("://"), w.find('@')) {
            (Some(a), Some(b)) if b > a => { out.push_str(&w[..a + 3]); out.push_str("***@"); out.push_str(&w[b + 1..]); }
            _ => out.push_str(w),
        }
    }
    out
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { if i <= c.len() { out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char) } else { out.push('=') } }
    }
    out
}

static AUTH: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Registers a token for `host`, passed to git as a host-scoped `http.<url>.extraHeader` through the process
/// environment (GIT_CONFIG_*), so it is never written to disk and only sent to that host. This also covers the
/// lazy blob fetches of partial clones.
pub fn set_auth(host: &str, token: &str) {
    let header = format!("Authorization: Basic {}", b64(format!("x-access-token:{token}").as_bytes()));
    let mut a = AUTH.lock().unwrap();
    a.retain(|(h, _)| h != host); a.push((host.to_string(), header));
    for (i, (h, v)) in a.iter().enumerate() {
        std::env::set_var(format!("GIT_CONFIG_KEY_{i}"), format!("http.https://{h}/.extraHeader"));
        std::env::set_var(format!("GIT_CONFIG_VALUE_{i}"), v);
    }
    std::env::set_var("GIT_CONFIG_COUNT", a.len().to_string());
}

/// Cache directory for a repository, keyed by normalized remote.
pub fn cache_dir(root: &Path, normalized_remote: &str) -> PathBuf { root.join(normalized_remote.replace('/', "__")) }

/// Ensures a blobless partial clone (no checkout) exists at `dir`.
pub fn ensure_clone(url: &str, dir: &Path) -> Result<()> {
    if dir.join("HEAD").exists() { return Ok(()); }
    let parent = dir.parent().ok_or("bad cache dir")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    git(parent, &["clone", "--bare", "--filter=blob:none", url, dir.to_str().ok_or("bad path")?]).map(|_| ())
}

/// Refspec fetching only the MR/PR head ref: GitHub `pull/N/head`, GitLab `merge-requests/N/head`.
pub fn mr_refspec(gitlab: bool, n: u64) -> String {
    if gitlab { format!("+refs/merge-requests/{n}/head:refs/perusal/mr/{n}") } else { format!("+refs/pull/{n}/head:refs/perusal/mr/{n}") }
}

pub fn fetch_mr(dir: &Path, gitlab: bool, n: u64) -> Result<String> {
    git(dir, &["fetch", "--filter=blob:none", "origin", &mr_refspec(gitlab, n)])?;
    git(dir, &["rev-parse", &format!("refs/perusal/mr/{n}")])
}

pub fn merge_base(dir: &Path, a: &str, b: &str) -> Result<String> { git(dir, &["merge-base", a, b]) }

/// Detached worktree at `rev` (used as the agent's read-only file tree).
pub fn add_worktree(repo: &Path, path: &Path, rev: &str) -> Result<()> {
    git(repo, &["worktree", "add", "--detach", path.to_str().ok_or("bad path")?, rev]).map(|_| ())
}

/// Commits in `base..head`, oldest first: (sha, parents, subject).
pub fn commits(dir: &Path, base: &str, head: &str) -> Result<Vec<(String, Vec<String>, String)>> {
    let out = git(dir, &["log", "--reverse", "--format=%H%x09%P%x09%s", &format!("{base}..{head}")])?;
    Ok(out.lines().filter_map(|l| { let p: Vec<&str> = l.splitn(3, '\t').collect(); (p.len() == 3).then(|| (p[0].into(), p[1].split_whitespace().map(String::from).collect(), p[2].into())) }).collect())
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphRow { pub sha: String, pub parents: Vec<String>, pub subject: String, pub in_mr: bool, pub lane: usize }

/// Commit graph rows newest first: the MR's commits (`base..head`) plus `context` commits of target-branch history.
/// Lanes: the MR line is lane 0, target-branch context lane 1.
pub fn graph(dir: &Path, base: &str, head: &str, context: usize) -> Result<Vec<GraphRow>> {
    let mr: Vec<_> = commits(dir, base, head)?;
    let mut rows: Vec<GraphRow> = mr.iter().rev().map(|(s, p, t)| GraphRow { sha: s.clone(), parents: p.clone(), subject: t.clone(), in_mr: true, lane: 0 }).collect();
    let out = git(dir, &["log", &format!("-{context}"), "--format=%H%x09%P%x09%s", base])?;
    for l in out.lines() {
        let p: Vec<&str> = l.splitn(3, '\t').collect();
        if p.len() == 3 { rows.push(GraphRow { sha: p[0].into(), parents: p[1].split_whitespace().map(String::from).collect(), subject: p[2].into(), in_mr: false, lane: 1 }); }
    }
    Ok(rows)
}

/// Files touched by a commit (`from == None`) or by the range `from..to`.
pub fn files_in(dir: &Path, from: Option<&str>, to: &str) -> Result<Vec<String>> {
    let out = match from {
        None => git(dir, &["diff-tree", "--no-commit-id", "--name-only", "-r", "-m", "--root", to])?,
        Some(f) => git(dir, &["diff", "--name-only", f, to])?,
    };
    Ok(out.lines().map(String::from).collect())
}

/// Default branch: origin/HEAD target if known, else `main` or `master` if present.
pub fn default_branch(dir: &Path) -> Result<String> {
    if let Ok(r) = git(dir, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) { return Ok(r); }
    for b in ["main", "master"] { if git(dir, &["rev-parse", "--verify", "-q", b]).is_ok() { return Ok(b.into()); } }
    Err("cannot determine the default branch".into())
}

/// Base for a local-branch review: merge-base of the branch and the default branch (or an explicit override).
pub fn local_base(dir: &Path, branch: &str, override_base: Option<&str>) -> Result<String> {
    let b = match override_base { Some(b) => b.to_string(), None => default_branch(dir)? };
    merge_base(dir, &b, branch)
}

/// Exports the tree at `rev` into `dest` (a plain read-only snapshot without .git), for agents to read.
pub fn export_tree(dir: &Path, rev: &str, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut a = Command::new("git").args(["archive", "--format=tar", rev]).current_dir(dir).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let t = Command::new("tar").args(["-x", "-f", "-", "-C"]).arg(dest).stdin(a.stdout.take().unwrap()).output().map_err(|e| e.to_string())?;
    let o = a.wait_with_output().map_err(|e| e.to_string())?;
    if !o.status.success() { return Err(redact(&String::from_utf8_lossy(&o.stderr))); }
    if !t.status.success() { return Err(String::from_utf8_lossy(&t.stderr).to_string()); }
    Ok(())
}

/// Unified patch for `base..head` (three-dot), for review prompts.
pub fn patch(dir: &Path, base: &str, head: &str) -> Result<String> {
    let o = Command::new("git").args(["diff", "--no-color", "-U2", &format!("{base}...{head}")]).current_dir(dir).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).into_owned()) } else { Err(redact(&String::from_utf8_lossy(&o.stderr))) }
}

/// File content at a revision, if present. Exact bytes as text (no trimming: line diffs depend on the final newline).
pub fn show(dir: &Path, rev: &str, path: &str) -> Option<String> {
    let o = Command::new("git").args(["show", &format!("{rev}:{path}")]).current_dir(dir).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn base64_matches_known() { assert_eq!(b64(b"x-access-token:abc"), "eC1hY2Nlc3MtdG9rZW46YWJj"); assert_eq!(b64(b"ab"), "YWI="); assert_eq!(b64(b"a"), "YQ=="); }
    #[test] fn redacts() { assert_eq!(redact("fatal: https://u:tok@host/x.git bad"), "fatal: https://***@host/x.git bad"); }
    #[test] fn refspecs() { assert!(mr_refspec(true, 5).contains("merge-requests/5")); assert!(mr_refspec(false, 5).contains("pull/5")); }
    #[test] fn clone_fetch_and_commits() {
        let d = std::env::temp_dir().join(format!("perusal-go-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d);
        let up = d.join("up"); std::fs::create_dir_all(&up).unwrap();
        let g = |a: &[&str]| { git(&up, ["-c", "user.name=t", "-c", "user.email=t@t"].iter().chain(a).copied().collect::<Vec<_>>().as_slice()).unwrap(); };
        g(&["init", "-q", "-b", "main"]); std::fs::write(up.join("a"), "1").unwrap(); g(&["add", "-A"]); g(&["commit", "-qm", "base"]);
        g(&["checkout", "-qb", "pr"]); std::fs::write(up.join("a"), "2").unwrap(); g(&["commit", "-qam", "change"]);
        g(&["update-ref", "refs/pull/7/head", "HEAD"]); g(&["checkout", "-q", "main"]);
        g(&["config", "uploadpack.allowFilter", "true"]); g(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
        let c = cache_dir(&d.join("cache"), "host/o/r");
        ensure_clone(&format!("file://{}", up.display()), &c).unwrap();
        let sha = fetch_mr(&c, false, 7).unwrap();
        let mb = merge_base(&c, "refs/perusal/mr/7", "main").unwrap();
        let cs = commits(&c, &mb, &sha).unwrap();
        assert_eq!(cs.len(), 1); assert_eq!(cs[0].2, "change");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn repo() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!("perusal-gr-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst))); let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap(); d
    }
    fn g(d: &Path, a: &[&str]) -> String { git(d, ["-c", "user.name=t", "-c", "user.email=t@t"].iter().chain(a).copied().collect::<Vec<_>>().as_slice()).unwrap() }
    #[test] fn graph_range_and_local_base() {
        let d = repo(); g(&d, &["init", "-q", "-b", "main"]);
        {
        let (f, c) = ("a", "1"); std::fs::write(d.join(f), c).unwrap(); } g(&d, &["add", "-A"]); g(&d, &["commit", "-qm", "base"]);
        g(&d, &["checkout", "-qb", "feat"]);
        std::fs::write(d.join("x"), "x").unwrap(); g(&d, &["add", "-A"]); g(&d, &["commit", "-qm", "c1"]);
        std::fs::write(d.join("y"), "y").unwrap(); g(&d, &["add", "-A"]); g(&d, &["commit", "-qm", "c2"]);
        let base = local_base(&d, "feat", None).unwrap();
        let rows = graph(&d, &base, "feat", 5).unwrap();
        assert_eq!(rows.iter().filter(|r| r.in_mr).count(), 2); assert_eq!(rows[0].subject, "c2"); assert!(rows.last().map(|r| !r.in_mr).unwrap());
        let c1 = rows[1].sha.clone(); let c2 = rows[0].sha.clone();
        assert_eq!(files_in(&d, None, &c1).unwrap(), vec!["x"]); assert_eq!(files_in(&d, None, &c2).unwrap(), vec!["y"]);
        let mut both = files_in(&d, Some(&base), "feat").unwrap(); both.sort(); assert_eq!(both, vec!["x", "y"]);
        let snap = d.join("snap"); export_tree(&d, "feat", &snap).unwrap(); assert_eq!(std::fs::read_to_string(snap.join("x")).unwrap(), "x"); assert!(!snap.join(".git").exists());
        assert!(patch(&d, "main", "feat").unwrap().contains("+x"));
        assert_eq!(show(&d, "feat", "x").unwrap(), "x"); std::fs::write(d.join("nl"), "a\nb\n").unwrap(); g(&d, &["add", "-A"]); g(&d, &["commit", "-qm", "nl"]); assert_eq!(show(&d, "feat", "nl").unwrap(), "a\nb\n"); assert!(show(&d, "main", "x").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
