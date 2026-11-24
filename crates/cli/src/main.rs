use clap::{Parser, Subcommand};
use perusal_core::store::Store;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "perusal-cli", about = "Headless harness for Perusal")]
struct Cli {
    /// SQLite database path
    #[arg(long, default_value = "perusal.sqlite")]
    db: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate a synthetic MR repository (main + feature branches).
    Gen { dir: PathBuf, #[arg(long, default_value_t = 10_000)] files: usize, #[arg(long, default_value_t = 500_000)] lines: usize },
    /// List files of a local branch review with stats, optionally filtered, sorted and grouped.
    Files { repo: PathBuf, branch: String, #[arg(long, default_value = "main")] base: String,
        #[arg(long, default_value = "")] filter: String, #[arg(long, default_value = "")] sort: String, #[arg(long)] group: bool },
    /// Mark a file viewed (or --unset).
    View { repo: PathBuf, branch: String, path: String, #[arg(long)] unset: bool },
    /// Add a draft comment.
    Draft { repo: PathBuf, branch: String, body: String, #[arg(long)] file: Option<String>, #[arg(long)] line: Option<u32>, #[arg(long)] ai: bool },
    /// Open an MR/PR from its https URL and print its info and CI state. `--api` overrides the API base (tests, mocks).
    Mr { url: String, #[arg(long)] api: Option<String>, #[arg(long, env = "PERUSAL_TOKEN", default_value = "")] token: String },
    /// Benchmark open/filter/sort on a generated synthetic MR and check the spec budgets (use a release build).
    Bench { #[arg(long, default_value_t = 10_000)] files: usize, #[arg(long, default_value_t = 500_000)] lines: usize },
    /// Export drafts as Markdown.
    Export { repo: PathBuf, branch: String },
}

fn key(repo: &std::path::Path, branch: &str) -> String { format!("local:{}#{branch}", repo.canonicalize().unwrap_or(repo.into()).display()) }

fn open(store: &Store, repo: &std::path::Path, branch: &str, base: &str) -> Result<i64, Box<dyn std::error::Error>> {
    let files = perusal_diff::index_range(repo, base, branch)?;
    Ok(store.open_review(&key(repo, branch), &format!("{branch} (local)"), branch, files.len() as u32)?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Gen { dir, files, lines } => perusal_cli::generate(&dir, files, lines)?,
        Cmd::Files { repo, branch, base, filter, sort, group } => {
            let store = Store::open(&cli.db)?;
            let r = open(&store, &repo, &branch, &base)?;
            let viewed = store.viewed(r)?;
            let mut files = perusal_diff::index_range(&repo, &base, &branch)?;
            for f in &mut files { f.viewed = viewed.contains(&f.path); }
            let mut idx = perusal_filter::filter(&files, &filter)?;
            perusal_filter::sort(&files, &mut idx, &sort)?;
            let (v, t) = store.progress(r)?;
            println!("viewed {v}/{t}");
            if group {
                let sel: Vec<_> = idx.iter().map(|&i| files[i].clone()).collect();
                for (n, g) in perusal_diff::group_files(&sel, &perusal_diff::rules::builtin_rules()).iter().enumerate() {
                    println!("batch {n}: {}", g.iter().map(|&i| sel[i].path.as_str()).collect::<Vec<_>>().join(", "));
                }
            } else {
                for i in idx {
                    let f = &files[i];
                    println!("{} {:>6} +{} -{} {}{}", if f.viewed { "v" } else { " " }, f.status.as_str(), f.additions, f.deletions, f.path, if f.noisy() { " [noisy]" } else { "" });
                }
            }
        }
        Cmd::View { repo, branch, path, unset } => {
            let store = Store::open(&cli.db)?; let r = open(&store, &repo, &branch, "main")?; store.set_viewed(r, &path, !unset)?;
        }
        Cmd::Draft { repo, branch, body, file, line, ai } => {
            let store = Store::open(&cli.db)?; let r = open(&store, &repo, &branch, "main")?;
            store.add_draft(r, file.as_deref(), line.map(|l| (l, l)), &body, ai)?;
        }
        Cmd::Mr { url, api, token } => {
            use perusal_providers::{github::GitHub, gitlab::GitLab, Kind, Provider};
            let r = perusal_providers::parse_mr_url(&url)?;
            let http = perusal_providers::auth::client(None)?;
            let p: Box<dyn Provider> = match r.kind {
                Kind::GitHub => Box::new(GitHub { http, api: api.unwrap_or(GitHub::api_base(&r.host)), token }),
                Kind::GitLab => Box::new(GitLab { http, api: api.unwrap_or(GitLab::api_base(&r.host)), token }),
            };
            let i = p.mr_info(&r).await?;
            println!("{} [{}] by {} @ {}", i.title, i.state, i.author, i.head_sha);
            println!("ci: {:?}", p.ci(&r, &i.head_sha).await?);
        }
        Cmd::Bench { files, lines } => {
            let d = std::env::temp_dir().join(format!("perusal-bench-{}", std::process::id()));
            perusal_cli::generate(&d, files, lines)?;
            let t = std::time::Instant::now();
            let idx = perusal_diff::index_range(&d, "main", "feature")?;
            let open = t.elapsed();
            let t = std::time::Instant::now();
            let mut sel = perusal_filter::filter(&idx, "additions > avg(additions) && path ~ /m4/ || top(50, additions + deletions)")?;
            let filt = t.elapsed();
            let t = std::time::Instant::now();
            perusal_filter::sort(&idx, &mut sel, "additions + deletions desc, name asc")?;
            let srt = t.elapsed();
            let rss_mb = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find(|l| l.starts_with("VmHWM")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())).unwrap_or(0) / 1024;
            println!("files={} open={open:?} filter={filt:?} sort={srt:?} peak_rss={rss_mb}MB", idx.len());
            let _ = std::fs::remove_dir_all(&d);
            let ok = open.as_millis() < 1000 && filt.as_millis() < 100 && srt.as_millis() < 100 && rss_mb < 1024;
            println!("budget (open<1s, filter<100ms, sort<100ms, mem<1GB): {}", if ok { "PASS" } else { "FAIL" });
            if !ok { std::process::exit(1); }
        }
        Cmd::Export { repo, branch } => {
            let store = Store::open(&cli.db)?; let r = open(&store, &repo, &branch, "main")?; print!("{}", store.export_markdown(r)?);
        }
    }
    Ok(())
}
