//! perusal:// deep links and single-instance hand-off (Unix socket; token-checked loopback TCP on Windows).
use perusal_core::brand::DEEP_LINK_SCHEME;
use std::io::{Read, Write};
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
    // Windows (ShellExecute) and some browsers normalise `perusal://open?url=` to `perusal://open/?url=`.
    let rest = link.strip_prefix(&format!("{DEEP_LINK_SCHEME}://open")).and_then(|r| r.strip_prefix('/').unwrap_or(r).strip_prefix("?url=")).ok_or("not a perusal://open link")?;
    let url = unpercent(rest).ok_or("bad percent-encoding")?;
    if !url.starts_with("https://") || url.contains(char::is_whitespace) { return Err("only https MR URLs are accepted".into()); }
    perusal_providers_check(&url)?; Ok(url)
}
fn perusal_providers_check(url: &str) -> Result<(), String> {
    if url.contains("/pull/") || url.contains("/-/merge_requests/") { Ok(()) } else { Err("not a merge request or pull request URL".into()) }
}

#[cfg(unix)]
mod ipc {
    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};
    pub type Listener = UnixListener;
    pub fn send_to_running(sock: &Path, link: &str) -> std::io::Result<bool> {
        match UnixStream::connect(sock) { Ok(mut s) => { s.write_all(link.as_bytes())?; Ok(true) } Err(_) => Ok(false) }
    }
    pub fn listen(sock: &Path) -> std::io::Result<Listener> { let _ = std::fs::remove_file(sock); UnixListener::bind(sock) }
    pub fn accept_one(l: &Listener) -> std::io::Result<String> { let (mut s, _) = l.accept()?; let mut b = String::new(); s.read_to_string(&mut b)?; Ok(b) }
}

/// Windows has no Unix sockets in std: the running instance listens on a loopback TCP port and records `port token` in the file
/// at `sock` (in the user's own data folder). A client must send the token first, so another local process (or user) that can
/// reach the port but cannot read the file is ignored; reads have a size cap and a timeout so a silent client cannot block the listener.
#[cfg(windows)]
mod ipc {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::time::Duration;
    const MAX_LEN: u64 = 8 * 1024;
    pub struct Listener { inner: TcpListener, token: String }
    fn random_token() -> String {
        use std::hash::{BuildHasher, Hasher};
        // RandomState is seeded from the OS random generator; two independent hashers give 128 bits.
        (0..2).map(|i| { let mut h = std::collections::hash_map::RandomState::new().build_hasher(); h.write_u32(std::process::id() ^ i); format!("{:016x}", h.finish()) }).collect()
    }
    pub fn send_to_running(sock: &Path, link: &str) -> std::io::Result<bool> {
        let Ok(info) = std::fs::read_to_string(sock) else { return Ok(false) };
        let mut it = info.split_whitespace();
        let (Some(port), Some(token)) = (it.next().and_then(|p| p.parse::<u16>().ok()), it.next()) else { return Ok(false) };
        match TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(2)) {
            Ok(mut s) => { s.set_write_timeout(Some(Duration::from_secs(2)))?; s.write_all(format!("{token}\n{link}").as_bytes())?; Ok(true) }
            Err(_) => Ok(false),
        }
    }
    pub fn listen(sock: &Path) -> std::io::Result<Listener> {
        let inner = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?; let token = random_token();
        std::fs::write(sock, format!("{} {token}", inner.local_addr()?.port()))?; Ok(Listener { inner, token })
    }
    /// Accepts one connection and returns its link; a client with the wrong token, an oversized or a stalled message is an error (the caller just accepts again).
    pub fn accept_one(l: &Listener) -> std::io::Result<String> {
        let (s, _) = l.inner.accept()?; s.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut b = String::new(); s.take(MAX_LEN + 1).read_to_string(&mut b)?;
        if b.len() as u64 > MAX_LEN { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "message too long")); }
        match b.split_once('\n') { Some((t, link)) if t == l.token => Ok(link.to_string()), _ => Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "bad token")) }
    }
}
pub use ipc::{accept_one, listen, send_to_running, Listener};

/// `reg add` argument lists that register `perusal://` for the current user (no admin rights needed).
pub fn registry_commands(exe: &Path) -> Vec<Vec<String>> {
    let key = format!(r"HKCU\Software\Classes\{DEEP_LINK_SCHEME}");
    let cmd = format!("\"{}\" \"%1\"", exe.display());
    let a = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    vec![
        a(&["add", &key, "/ve", "/d", &format!("URL:{} link", perusal_core::brand::PRODUCT_NAME), "/f"]),
        a(&["add", &key, "/v", "URL Protocol", "/d", "", "/f"]),
        a(&["add", &format!(r"{key}\shell\open\command"), "/ve", "/d", &cmd, "/f"]),
    ]
}

/// Windows: makes `perusal://` links open this executable (per-user registry entry). Does nothing if already registered for it.
#[cfg(windows)]
pub fn register_scheme() -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe()?;
    // An MSIX install declares the protocol in its manifest; do not write a second registration pointing into the package.
    if exe.components().any(|c| c.as_os_str().eq_ignore_ascii_case("WindowsApps")) { return Ok(()); }
    let run = |args: &[String]| std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output();
    let key = format!(r"HKCU\Software\Classes\{DEEP_LINK_SCHEME}\shell\open\command");
    let current = run(&["query".into(), key, "/ve".into()])?;
    if String::from_utf8_lossy(&current.stdout).contains(&format!("\"{}\"", exe.display())) { return Ok(()); }
    for c in registry_commands(&exe) { let o = run(&c)?; if !o.status.success() { return Err(std::io::Error::other(String::from_utf8_lossy(&o.stderr).trim().to_string())); } }
    Ok(())
}
#[cfg(not(windows))]
pub fn register_scheme() -> std::io::Result<()> { Ok(()) }

#[cfg(test)]
mod tests { use super::*;
    #[test] fn parses_and_rejects() {
        let l = perusal_core::brand::deep_link_for("https://github.com/o/r/pull/3"); assert_eq!(parse(&l).unwrap(), "https://github.com/o/r/pull/3");
        assert!(parse("perusal://open?url=http%3A%2F%2Fgithub.com%2Fo%2Fr%2Fpull%2F3").is_err());
        assert!(parse("perusal://open?url=javascript%3Aalert(1)").is_err()); assert!(parse("perusal://submit?url=x").is_err());
        assert!(parse("perusal://open?url=https%3A%2F%2Fgithub.com%2Fo%2Fr").is_err());
        assert_eq!(parse("perusal://open/?url=https%3A%2F%2Fgithub.com%2Fo%2Fr%2Fpull%2F3").unwrap(), "https://github.com/o/r/pull/3");
        assert!(parse("perusal://openx?url=https%3A%2F%2Fgithub.com%2Fo%2Fr%2Fpull%2F3").is_err());
    }
    #[test] fn registry_entry_quotes_the_exe_and_link() {
        let c = registry_commands(Path::new(r"C:\Program Files\Perusal\perusal.exe"));
        assert_eq!(c.len(), 3); assert!(c[2].iter().any(|x| x == r#""C:\Program Files\Perusal\perusal.exe" "%1""#));
        assert!(c[0][1].starts_with(r"HKCU\Software\Classes\perusal"));
    }
    #[cfg(windows)]
    #[test] fn windows_handoff_rejects_wrong_token_and_stalls() {
        use std::io::Write;
        let sock = std::env::temp_dir().join(format!("perusal-auth-{}.sock", std::process::id()));
        let l = listen(&sock).unwrap();
        let port: u16 = std::fs::read_to_string(&sock).unwrap().split_whitespace().next().unwrap().parse().unwrap();
        let h = std::thread::spawn(move || { let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap(); s.write_all(b"wrong\nperusal://open?url=a").unwrap(); });
        assert!(accept_one(&l).is_err()); h.join().unwrap();
        let _silent = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap(); // connects and never sends
        let t = std::time::Instant::now(); assert!(accept_one(&l).is_err()); assert!(t.elapsed() < std::time::Duration::from_secs(5));
        let _ = std::fs::remove_file(&sock);
    }
    #[test] fn second_launch_hands_over() {
        let sock = std::env::temp_dir().join(format!("perusal-{}.sock", std::process::id()));
        assert!(!send_to_running(&sock, "x").unwrap()); // no instance yet
        let l = listen(&sock).unwrap();
        let s2 = sock.clone(); let h = std::thread::spawn(move || send_to_running(&s2, "perusal://open?url=a").unwrap());
        assert_eq!(accept_one(&l).unwrap(), "perusal://open?url=a"); assert!(h.join().unwrap()); let _ = std::fs::remove_file(&sock);
    } }
