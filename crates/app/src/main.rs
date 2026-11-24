//! `perusal` entry point. Subcommands that need no window run here; the GPUI window is pending (see DECISIONS.md).
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
            Ok(url) => println!("would open {url}"),
            Err(e) => { eprintln!("rejected deep link: {e}"); std::process::exit(1) }
        },
        _ => println!("{} (window UI pending; try `perusal theme check <file>`)", perusal_core::brand::PRODUCT_NAME),
    }
}
