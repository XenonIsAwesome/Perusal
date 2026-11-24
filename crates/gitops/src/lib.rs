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

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn redacts() { assert_eq!(redact("fatal: https://u:tok@host/x.git bad"), "fatal: https://***@host/x.git bad"); }
    #[test] fn refspecs() { assert!(mr_refspec(true, 5).contains("merge-requests/5")); assert!(mr_refspec(false, 5).contains("pull/5")); }
    #[test] fn clone_fetch_and_commits() {
        let d = std::env::temp_dir().join(format!("perusal-go-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d);
        let up = d.join("up"); std::fs::create_dir_all(&up).unwrap();
        let g = |a: &[&str]| { git(&up, &["-c", "user.name=t", "-c", "user.email=t@t"].iter().chain(a).copied().collect::<Vec<_>>().as_slice()).unwrap(); };
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
}
