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
    pub generated: bool,
    pub viewed: bool,
    pub comments: u32,
    pub unresolved: u32,
    pub ai_flagged: bool,
}

impl FileStat {
    pub fn new(path: &str, status: Status, additions: u32, deletions: u32) -> Self {
        FileStat { path: path.into(), status, additions, deletions, hunks: 1, size: 0, binary: false,
            generated: is_noisy_path(path), viewed: false, comments: 0, unresolved: 0, ai_flagged: false }
    }
    pub fn name(&self) -> &str { self.path.rsplit('/').next().unwrap_or(&self.path) }
    pub fn ext(&self) -> &str { let n = self.name(); n.rsplit_once('.').map(|x| x.1).unwrap_or("") }
    pub fn net(&self) -> i64 { self.additions as i64 - self.deletions as i64 }
    pub fn changed(&self) -> u32 { self.additions + self.deletions }
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

/// Parses `git diff --numstat -M` output (with `--name-status` merged separately is overkill: status is
/// derived from `git diff --name-status -M`).
pub fn parse_numstat(numstat: &str, name_status: &str) -> Vec<FileStat> {
    use std::collections::HashMap;
    let mut st: HashMap<String, Status> = HashMap::new();
    for l in name_status.lines() {
        let p: Vec<&str> = l.split('\t').collect();
        if p.len() < 2 { continue; }
        let s = match p[0].chars().next() { Some('A') => Status::Added, Some('D') => Status::Deleted, Some('R') => Status::Renamed, _ => Status::Modified };
        st.insert(p[p.len() - 1].to_string(), s);
    }
    numstat.lines().filter_map(|l| {
        let p: Vec<&str> = l.splitn(3, '\t').collect();
        if p.len() < 3 { return None; }
        let path = p[2].to_string();
        let binary = p[0] == "-";
        let status = st.get(&path).copied().unwrap_or(Status::Modified);
        let mut f = FileStat::new(&path, status, p[0].parse().unwrap_or(0), p[1].parse().unwrap_or(0));
        f.binary = binary;
        Some(f)
    }).collect()
}

/// Computes the stats index for `base..head` in the repo at `repo`, via the git CLI.
pub fn index_range(repo: &Path, base: &str, head: &str) -> std::io::Result<Vec<FileStat>> {
    let run = |args: &[&str]| -> std::io::Result<String> {
        let o = Command::new("git").args(args).arg(format!("{base}...{head}")).current_dir(repo).output()?;
        if !o.status.success() { return Err(std::io::Error::other(String::from_utf8_lossy(&o.stderr).to_string())); }
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    };
    Ok(parse_numstat(&run(&["diff", "--numstat", "-M"])?, &run(&["diff", "--name-status", "-M"])?))
}

/// Built-in name-based grouping: files sharing a stem across folders are batched together.
pub fn group_key(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut stem = name.rsplit_once('.').map(|x| x.0).unwrap_or(name).to_string();
    for suf in [".test", ".spec", "_test"] { if let Some(s) = stem.strip_suffix(suf) { stem = s.to_string(); } }
    if let Some(s) = stem.strip_prefix("test_") { stem = s.to_string(); }
    stem
}

/// Groups file indexes by stem; only groups that really relate several files are returned multi-member.
pub fn group_files(files: &[FileStat]) -> Vec<Vec<usize>> {
    use std::collections::BTreeMap;
    let mut m: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut order = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let k = group_key(&f.path);
        if !m.contains_key(&k) { order.push(k.clone()); }
        m.entry(k).or_default().push(i);
    }
    order.into_iter().map(|k| m.remove(&k).unwrap()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numstat_and_status() {
        let f = parse_numstat("3\t1\ta/b.rs\n-\t-\timg.png\n", "M\ta/b.rs\nA\timg.png\n");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].additions, 3); assert_eq!(f[0].status, Status::Modified);
        assert!(f[1].binary && f[1].noisy()); assert_eq!(f[1].status, Status::Added);
    }
    #[test]
    fn noisy_paths() {
        assert!(is_noisy_path("web/package-lock.json")); assert!(is_noisy_path("a/b.min.js")); assert!(!is_noisy_path("src/a.rs"));
    }
    #[test]
    fn grouping() {
        let fs: Vec<_> = ["a/foo.h", "b/foo.cpp", "x/foo.test.ts", "y/foo.ts", "t/test_bar.py", "bar.py", "z.rs"]
            .iter().map(|p| FileStat::new(p, Status::Modified, 1, 1)).collect();
        let g = group_files(&fs);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0], vec![0, 1, 2, 3]);
        assert_eq!(g[1], vec![4, 5]);
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
        let mut f = index_range(&d, "main", "f").unwrap(); f.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!((f[0].additions, f[0].deletions), (2, 1)); assert_eq!(f[1].status, Status::Added);
        let _ = std::fs::remove_dir_all(&d);
    }
}
