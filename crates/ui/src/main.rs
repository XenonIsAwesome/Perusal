//! Perusal GUI (GPUI). Thin: all review logic lives in the UI-free crates.
mod icons;
mod kit;
mod model;
mod settings_view;
mod root;
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
        [] => root::run(root::Initial::Home),
        [arg] if arg.starts_with("perusal://") || arg.starts_with("https://") => match deeplink::url_from_arg(arg) {
            Ok(url) => {
                // A window is already running: hand the link over and exit instead of opening a second one.
                if let Ok(true) = deeplink::send_to_running(&deeplink::socket_path(&model::data_dir()), &url) { return; }
                root::run(root::Initial::Url(url))
            }
            Err(e) => { eprintln!("rejected link: {e}"); std::process::exit(1) }
        },
        ["--help" | "-h"] => usage(),
        [repo, branch, rest @ ..] => {
            let base = rest.iter().position(|a| *a == "--base").and_then(|i| rest.get(i + 1)).map(|s| s.to_string());
            root::run(root::Initial::Local(repo.to_string(), branch.to_string(), base));
        }
        _ => usage(),
    }
}

fn usage() {
    println!("Usage: perusal                              open the home screen\n       perusal <repo> <branch> [--base <ref>]   review a local branch\n       perusal <https MR/PR URL>               review a remote MR/PR\n       perusal perusal://open?url=...           deep link\n       perusal theme check <file>");
}
