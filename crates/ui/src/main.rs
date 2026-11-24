//! Perusal GUI (GPUI). Thin: all review logic lives in the UI-free crates.
mod view;

use perusal_app::{deeplink, theme};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["theme", "check", file] => {
            let src = match std::fs::read_to_string(file) { Ok(s) => s, Err(e) => { eprintln!("cannot read {file}: {e}"); std::process::exit(2) } };
            let errs = theme::validate(&src);
            if errs.is_empty() { println!("{file}: ok"); } else { for e in &errs { eprintln!("{file}: {e}"); } std::process::exit(1); }
        }
        [link] if link.starts_with("perusal://") => match deeplink::parse(link) {
            Ok(url) => println!("would open {url} (remote MR opening in the GUI is not wired yet; use a local repo)"),
            Err(e) => { eprintln!("rejected deep link: {e}"); std::process::exit(1) }
        },
        [repo, branch, rest @ ..] => {
            let base = rest.iter().position(|a| *a == "--base").and_then(|i| rest.get(i + 1)).cloned();
            let db = std::env::var("PERUSAL_DB").unwrap_or_else(|_| "perusal.sqlite".into());
            match view::Model::open(std::path::Path::new(repo), branch, base, &db) {
                Ok(m) => view::run(m),
                Err(e) => { eprintln!("cannot open review: {e}"); std::process::exit(1) }
            }
        }
        _ => {
            println!("Usage: perusal <repo> <branch> [--base <ref>]\n       perusal theme check <file>\n       perusal perusal://open?url=...");
        }
    }
}
