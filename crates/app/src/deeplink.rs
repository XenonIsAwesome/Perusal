//! perusal:// deep links and single-instance hand-off over a Unix socket.
use perusal_core::brand::DEEP_LINK_SCHEME;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

fn unpercent(s: &str) -> Option<String> {
    let b = s.as_bytes(); let mut out = Vec::new(); let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' { out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?); i += 3 } else { out.push(b[i]); i += 1 }
    }
    String::from_utf8(out).ok()
}

/// A deep link only opens an https MR URL for viewing; anything else is rejected.
pub fn parse(link: &str) -> Result<String, String> {
    let rest = link.strip_prefix(&format!("{DEEP_LINK_SCHEME}://open?url=")).ok_or("not a perusal://open link")?;
    let url = unpercent(rest).ok_or("bad percent-encoding")?;
    if !url.starts_with("https://") || url.contains(char::is_whitespace) { return Err("only https MR URLs are accepted".into()); }
    perusal_providers_check(&url)?; Ok(url)
}
fn perusal_providers_check(url: &str) -> Result<(), String> {
    if url.contains("/pull/") || url.contains("/-/merge_requests/") { Ok(()) } else { Err("not a merge request or pull request URL".into()) }
}

/// Tries to hand `link` to a running instance. Returns Ok(true) if one accepted it (caller should exit).
pub fn send_to_running(sock: &Path, link: &str) -> std::io::Result<bool> {
    match UnixStream::connect(sock) { Ok(mut s) => { s.write_all(link.as_bytes())?; Ok(true) } Err(_) => Ok(false) }
}

/// Becomes the running instance: returns a listener (stale socket files are replaced).
pub fn listen(sock: &Path) -> std::io::Result<UnixListener> { let _ = std::fs::remove_file(sock); UnixListener::bind(sock) }

pub fn accept_one(l: &UnixListener) -> std::io::Result<String> { let (s, _) = l.accept()?; let mut b = String::new(); s.take(8192).read_to_string(&mut b)?; Ok(b) }

/// What a second launch hands over: a `perusal://` link or a plain https MR URL. Returns the https URL to open.
pub fn url_from_arg(arg: &str) -> Result<String, String> {
    if arg.starts_with(&format!("{DEEP_LINK_SCHEME}://")) { parse(arg) }
    else if arg.starts_with("https://") && !arg.contains(char::is_whitespace) { perusal_providers_check(arg).map(|_| arg.to_string()) }
    else { Err("not a link Perusal opens".into()) }
}

/// Where the running instance listens (mode 0600 inside the user's data dir).
pub fn socket_path(data_dir: &Path) -> std::path::PathBuf { data_dir.join("perusal.sock") }

#[cfg(test)]
mod tests { use super::*;
    #[test] fn parses_and_rejects() {
        let l = perusal_core::brand::deep_link_for("https://github.com/o/r/pull/3"); assert_eq!(parse(&l).unwrap(), "https://github.com/o/r/pull/3");
        assert!(parse("perusal://open?url=http%3A%2F%2Fgithub.com%2Fo%2Fr%2Fpull%2F3").is_err());
        assert!(parse("perusal://open?url=javascript%3Aalert(1)").is_err()); assert!(parse("perusal://submit?url=x").is_err());
        assert!(parse("perusal://open?url=https%3A%2F%2Fgithub.com%2Fo%2Fr").is_err());
    }
    #[test] fn args_are_validated() {
        assert_eq!(url_from_arg("https://gitlab.com/g/p/-/merge_requests/9").unwrap(), "https://gitlab.com/g/p/-/merge_requests/9");
        assert!(url_from_arg("https://example.com/x").is_err()); assert!(url_from_arg("file:///etc/passwd").is_err());
        assert!(url_from_arg("perusal://open?url=https%3A%2F%2Fgithub.com%2Fo%2Fr%2Fpull%2F1").is_ok());
    }
    #[test] fn second_launch_hands_over() {
        let sock = std::env::temp_dir().join(format!("perusal-{}.sock", std::process::id()));
        assert!(!send_to_running(&sock, "x").unwrap()); // no instance yet
        let l = listen(&sock).unwrap();
        let s2 = sock.clone(); let h = std::thread::spawn(move || send_to_running(&s2, "perusal://open?url=a").unwrap());
        assert_eq!(accept_one(&l).unwrap(), "perusal://open?url=a"); assert!(h.join().unwrap()); let _ = std::fs::remove_file(&sock);
    } }
