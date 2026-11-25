//! Synthetic merge-request generator used for tests and benchmarks.
use std::{fs, io, path::Path, process::Command};

/// Creates a git repo at `dir` with a `main` commit and a `feature` branch
/// changing `files` files with about `total_lines` changed lines in total.
pub fn generate(dir: &Path, files: usize, total_lines: usize) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let git = |args: &[&str]| -> io::Result<()> {
        let s = Command::new("git").args(["-c","user.name=gen","-c","user.email=gen@example.com"]).args(args).current_dir(dir).status()?;
        if s.success() { Ok(()) } else { Err(io::Error::other(format!("git {args:?} failed"))) }
    };
    git(&["init", "-q", "-b", "main"])?;
    let per = (total_lines / files.max(1)).max(1);
    let path = |i: usize| dir.join(format!("src/m{}/f{}.txt", i % 100, i));
    for i in 0..files {
        let p = path(i);
        fs::create_dir_all(p.parent().unwrap())?;
        fs::write(&p, (0..per).map(|l| format!("line {l}\n")).collect::<String>())?;
    }
    git(&["add", "-A"])?;
    git(&["commit", "-q", "-m", "base"])?;
    git(&["checkout", "-q", "-b", "feature"])?;
    for i in 0..files {
        fs::write(path(i), (0..per).map(|l| format!("changed {l}\n")).collect::<String>())?;
    }
    git(&["add", "-A"])?;
    git(&["commit", "-q", "-m", "change everything"])
}

#[cfg(test)]
mod tests {
    #[test]
    fn generates_small_repo() {
        let d = std::env::temp_dir().join(format!("perusal-gen-{}", std::process::id()));
        super::generate(&d, 10, 100).unwrap();
        let out = std::process::Command::new("git").args(["diff","--numstat","main","feature"]).current_dir(&d).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 10);
        let _ = std::fs::remove_dir_all(d);
    }
}
