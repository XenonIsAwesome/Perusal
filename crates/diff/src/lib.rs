//! Per-file stats index, noisy-file rules and grouping. UI-free.
pub mod text;
use std::process::Command;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status { Added, Modified, Deleted, Renamed }

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self { Status::Added => "added", Status::Modified => "modified", Status::Deleted => "deleted", Status::Renamed => "renamed" }
    }
}

#[derive(Debug, Clone)]
pub struct FileStat {
    pub path: String,
    pub status: Status,
    pub additions: u32,
    pub deletions: u32,
    pub hunks: u32,
    pub size: u64,
    pub binary: bool,
    pub symlink: bool,
    pub generated: bool,
    pub viewed: bool,
    pub comments: u32,
    pub unresolved: u32,
    pub ai_flagged: bool,
    /// Changed lines when whitespace is ignored (`git diff -w`); equals `changed()` until computed.
    pub meaningful: u32,
}

impl FileStat {
    pub fn new(path: &str, status: Status, additions: u32, deletions: u32) -> Self {
        FileStat { path: path.into(), status, additions, deletions, hunks: 1, size: 0, binary: false, symlink: false,
            generated: is_noisy_path(path), viewed: false, comments: 0, unresolved: 0, ai_flagged: false, meaningful: additions + deletions }
    }
    pub fn name(&self) -> &str { self.path.rsplit('/').next().unwrap_or(&self.path) }
    pub fn ext(&self) -> &str { let n = self.name(); n.rsplit_once('.').map(|x| x.1).unwrap_or("") }
    pub fn net(&self) -> i64 { self.additions as i64 - self.deletions as i64 }
    pub fn changed(&self) -> u32 { self.additions + self.deletions }
    /// True when every change in the file is whitespace only (text files with changes).
    pub fn ws_only(&self) -> bool { !self.binary && self.changed() > 0 && self.meaningful == 0 }
    pub fn change_ratio(&self) -> f64 {
        let t = self.changed(); if t == 0 { 0.0 } else { self.additions.min(self.deletions) as f64 / t as f64 }
    }
    /// Noisy files collapse by default: patterns plus size limits.
    pub fn noisy(&self) -> bool {
        self.binary || self.generated || self.size > 1_000_000 || self.changed() > 5_000
    }
}

pub fn is_noisy_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    const LOCKS: [&str; 6] = ["Cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "poetry.lock", "go.sum"];
    LOCKS.contains(&name) || name.ends_with(".min.js") || name.ends_with(".min.css") || name.ends_with(".map")
        || path.starts_with("vendor/") || path.contains("/vendor/") || path.starts_with("dist/") || path.contains("/dist/")
}

/// Parses `git diff --raw -z` (modes, status, paths) and `git diff --numstat -z` (counts) into the stats index.
/// Handles renames (`old\0new`) and symlinks (mode 120000 on either side).
pub fn parse_diff_z(raw: &str, numstat: &str) -> Vec<FileStat> {
    use std::collections::HashMap;
    struct Meta { status: Status, symlink: bool }
    let mut meta: HashMap<String, Meta> = HashMap::new();
    let mut it = raw.split('\0').filter(|t| !t.is_empty());
    while let Some(m) = it.next() {
        let Some(m) = m.strip_prefix(':') else { continue };
        let f: Vec<&str> = m.split(' ').collect();
        if f.len() < 5 { continue; }
        let st = f[4].chars().next().unwrap_or('M');
        let path = if st == 'R' || st == 'C' { it.next(); it.next() } else { it.next() };
        let Some(path) = path else { break };
        let status = match st { 'A' => Status::Added, 'D' => Status::Deleted, 'R' => Status::Renamed, _ => Status::Modified };
        meta.insert(path.to_string(), Meta { status, symlink: f[0] == "120000" || f[1] == "120000" });
    }
    let mut out = Vec::new();
    let mut it = numstat.split('\0').filter(|t| !t.is_empty() || false);
    // numstat -z: "add\tdel\tpath\0" or, for renames, "add\tdel\t\0old\0new\0" (the empty path is significant)
    let toks: Vec<&str> = numstat.split('\0').collect();
    let mut i = 0; let _ = it.next();
    while i < toks.len() {
        let t = toks[i]; i += 1;
        let p: Vec<&str> = t.splitn(3, '\t').collect();
        if p.len() < 3 { continue; }
        let path = if p[2].is_empty() { i += 1; let n = toks.get(i).copied().unwrap_or(""); i += 1; n.to_string() } else { p[2].to_string() };
        let m = meta.get(&path);
        let mut f = FileStat::new(&path, m.map(|m| m.status).unwrap_or(Status::Modified), p[0].parse().unwrap_or(0), p[1].parse().unwrap_or(0));
        f.binary = p[0] == "-"; f.symlink = m.map(|m| m.symlink).unwrap_or(false);
        out.push(f);
    }
    out
}

/// Computes the stats index for `base..head` in the repo at `repo`, via the git CLI.
pub fn index_range(repo: &Path, base: &str, head: &str) -> std::io::Result<Vec<FileStat>> {
    let run = |args: &[&str]| -> std::io::Result<String> {
        let mut c = Command::new("git");
        #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
        let o = c.args(args).arg(format!("{base}...{head}")).current_dir(repo).output()?;
        if !o.status.success() { return Err(std::io::Error::other(String::from_utf8_lossy(&o.stderr).to_string())); }
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    };
    let mut files = parse_diff_z(&run(&["diff", "--raw", "--no-abbrev", "-z", "-M"])?, &run(&["diff", "--numstat", "-z", "-M"])?);
    // files that disappear from a whitespace-ignoring diff are whitespace-only changes
    let w: std::collections::HashMap<String, u32> = parse_diff_z("", &run(&["diff", "-w", "--numstat", "-z", "-M"])?).into_iter().map(|f| (f.path.clone(), f.changed())).collect();
    for f in &mut files { if !f.binary { f.meaningful = w.get(&f.path).copied().unwrap_or(0); } }
    Ok(files)
}

pub mod rules;
pub use rules::{group_files, GroupRule};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diff_z_parses_modes_renames_and_symlinks() {
        let raw = ":100644 100644 aaa bbb M\0a/b.rs\0:000000 100644 000 ccc A\0img.png\0:100644 100644 ddd eee R090\0old/n.rs\0new/n.rs\0:120000 120000 f1 f2 M\0link\0";
        let num = "3\t1\ta/b.rs\0-\t-\timg.png\0" .to_string() + "2\t2\t\0old/n.rs\0new/n.rs\0" + "1\t1\tlink\0";
        let f = parse_diff_z(raw, &num);
        assert_eq!(f.len(), 4);
        assert_eq!((f[0].path.as_str(), f[0].additions, f[0].status), ("a/b.rs", 3, Status::Modified));
        assert!(f[1].binary && f[1].noisy() && f[1].status == Status::Added);
        assert_eq!((f[2].path.as_str(), f[2].status), ("new/n.rs", Status::Renamed));
        assert!(f[3].symlink && !f[0].symlink);
    }
    #[test]
    fn noisy_paths() {
        assert!(is_noisy_path("web/package-lock.json")); assert!(is_noisy_path("a/b.min.js")); assert!(!is_noisy_path("src/a.rs"));
    }
    #[test]
    fn index_real_repo() {
        let d = std::env::temp_dir().join(format!("perusal-idx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let run = |a: &[&str]| { assert!(Command::new("git").args(["-c","user.name=t","-c","user.email=t@t"]).args(a).current_dir(&d).status().unwrap().success()); };
        std::fs::create_dir_all(&d).unwrap();
        run(&["init","-q","-b","main"]);
        std::fs::write(d.join("a.txt"), "1\n2\n").unwrap(); run(&["add","-A"]); run(&["commit","-qm","a"]);
        run(&["checkout","-qb","f"]);
        std::fs::write(d.join("a.txt"), "1\n3\n4\n").unwrap(); std::fs::write(d.join("b.txt"), "x\n").unwrap();
        run(&["add","-A"]); run(&["commit","-qm","b"]);
        #[cfg(unix)] std::os::unix::fs::symlink("a.txt", d.join("ln")).unwrap(); #[cfg(not(unix))] std::fs::write(d.join("ln"), "a.txt").unwrap(); std::fs::write(d.join("w.txt"), "a\nb\n").unwrap(); run(&["add","-A"]); run(&["commit","-qm","ln"]);
        run(&["checkout","-qb","g","main"]); std::fs::write(d.join("w.txt"), "a\nb\n").unwrap(); run(&["add","-A"]); run(&["commit","-qm","w0"]);
        std::fs::write(d.join("w.txt"), "a\n    b\n").unwrap(); run(&["add","-A"]); run(&["commit","-qm","w1"]);
        let g = index_range(&d, "main", "g").unwrap(); let w = g.iter().find(|x| x.path == "w.txt").unwrap(); assert!(w.status == Status::Added && !w.ws_only());
        run(&["checkout","-q","f"]);
        run(&["checkout","-qb","h","f"]); std::fs::write(d.join("w.txt"), "a\n   b\n").unwrap(); run(&["add","-A"]); run(&["commit","-qm","w2"]);
        let h = index_range(&d, "f", "h").unwrap(); let w = h.iter().find(|x| x.path == "w.txt").unwrap(); assert!(w.ws_only(), "{w:?}");
        let mut f = index_range(&d, "main", "f").unwrap(); f.sort_by(|a, b| a.path.cmp(&b.path));
        #[cfg(unix)] assert!(f.iter().find(|x| x.path == "ln").unwrap().symlink); assert!(!f.iter().find(|x| x.path == "a.txt").unwrap().symlink);
        let f: Vec<FileStat> = f.into_iter().filter(|x| x.path != "ln" && x.path != "w.txt").collect();
        assert_eq!((f[0].additions, f[0].deletions), (2, 1)); assert_eq!(f[1].status, Status::Added);
        let _ = std::fs::remove_dir_all(&d);
    }
}
