//! Perusal GUI (GPUI). Thin: all review logic lives in the UI-free crates.
// Release builds on Windows are GUI-subsystem (no console window); `attach_console` restores output for the CLI subcommands.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod icons;
mod kit;
mod model;
mod settings_view;
mod root;
mod view;

use perusal_app::{deeplink, theme};

#[cfg(all(windows, not(debug_assertions)))]
fn attach_console() { #[link(name = "kernel32")] extern "system" { fn AttachConsole(pid: u32) -> i32; } unsafe { AttachConsole(u32::MAX); } }
#[cfg(not(all(windows, not(debug_assertions))))]
fn attach_console() {}

/// Starts the GUI. On Windows it first makes sure `perusal://` is registered; only this path does, so a short-lived process
/// (`theme check`, `notify-test`, a rejected link) is never cut off between the registry writes.
fn run_gui(initial: root::Initial) {
    let _ = deeplink::register_scheme();
    root::run(initial)
}

fn main() {
    attach_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["theme", "check", file] => {
            let src = match std::fs::read_to_string(file) { Ok(s) => s, Err(e) => { eprintln!("cannot read {file}: {e}"); std::process::exit(2) } };
            let errs = theme::validate(&src);
            if errs.is_empty() { println!("{file}: ok"); } else { for e in &errs { eprintln!("{file}: {e}"); } std::process::exit(1); }
        }
        ["notify-test", title, body] => { model::desktop_notify(title, body); std::thread::sleep(std::time::Duration::from_secs(3)); }
        [] => run_gui(root::Initial::Home),
        [link] if link.starts_with("perusal://") => match deeplink::parse(link) {
            Ok(url) => run_gui(root::Initial::Url(url)),
            Err(e) => { eprintln!("rejected deep link: {e}"); std::process::exit(1) }
        },
        [url] if url.starts_with("https://") => run_gui(root::Initial::Url(url.to_string())),
        ["--help" | "-h"] => usage(),
        [repo, branch, rest @ ..] => {
            let base = rest.iter().position(|a| *a == "--base").and_then(|i| rest.get(i + 1)).map(|s| s.to_string());
            run_gui(root::Initial::Local(repo.to_string(), branch.to_string(), base));
        }
        _ => usage(),
    }
}

fn usage() {
    println!("Usage: perusal                              open the home screen\n       perusal <repo> <branch> [--base <ref>]   review a local branch\n       perusal <https MR/PR URL>               review a remote MR/PR\n       perusal perusal://open?url=...           deep link\n       perusal theme check <file>");
}
