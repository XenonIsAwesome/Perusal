//! Token storage, HTTP client construction (custom CA) and OAuth device flow.
use std::path::{Path, PathBuf};

pub fn client(ca_file: Option<&Path>) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder().user_agent("perusal");
    if let Some(p) = ca_file {
        let pem = std::fs::read(p).map_err(|e| format!("CA file: {e}"))?;
        b = b.add_root_certificate(reqwest::Certificate::from_pem(&pem).map_err(|e| format!("CA file: {e}"))?);
    }
    b.build().map_err(|e| e.to_string())
}

/// Token store. The OS keychain backend is behind a future `keychain` feature; the file store (mode 0600,
/// outside the SQLite database) is the fallback used here.
pub trait TokenStore { fn get(&self, host: &str) -> Option<String>; fn set(&self, host: &str, token: &str) -> std::io::Result<()>; fn delete(&self, host: &str); }

/// Tokens in the OS keychain through `secret-tool` (libsecret / Secret Service: GNOME Keyring, KWallet), falling back to the
/// 0600 token files when no keychain answers. A token found in a file is moved into the keychain the first time it is read.
/// The secret travels over the child's stdin, never on a command line. `PERUSAL_NO_KEYRING=1` forces files.
pub struct KeyringStore { pub files: FileStore, pub tool: Option<PathBuf> }
impl KeyringStore {
    pub fn detect(files: FileStore) -> Self {
        let tool = if std::env::var_os("PERUSAL_NO_KEYRING").is_some() { None }
            else { std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("secret-tool")).find(|f| f.is_file())) };
        KeyringStore { files, tool }
    }
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Option<std::process::Output> {
        use std::io::Write; use std::process::Stdio;
        let tool = self.tool.as_ref()?;
        let mut c = std::process::Command::new(tool).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        if let Some(mut i) = c.stdin.take() { if let Some(s) = stdin { let _ = i.write_all(s.as_bytes()); } }
        c.wait_with_output().ok()
    }
    fn lookup(&self, host: &str) -> Option<String> {
        let o = self.run(&["lookup", "service", "perusal", "host", host], None)?;
        let t = String::from_utf8_lossy(&o.stdout).trim().to_string(); (o.status.success() && !t.is_empty()).then_some(t)
    }
    fn store(&self, host: &str, token: &str) -> bool {
        let label = format!("Perusal token for {host}");
        self.run(&["store", &format!("--label={label}"), "service", "perusal", "host", host], Some(token)).map(|o| o.status.success()).unwrap_or(false)
    }
    /// Whether a keychain program was found (shown in Settings).
    pub fn has_keychain(&self) -> bool { self.tool.is_some() }
}
impl TokenStore for KeyringStore {
    fn get(&self, host: &str) -> Option<String> {
        if let Some(t) = self.lookup(host) { return Some(t); }
        let t = self.files.get(host)?;
        if self.store(host, &t) { self.files.delete(host); }
        Some(t)
    }
    fn set(&self, host: &str, token: &str) -> std::io::Result<()> {
        if self.store(host, token) { self.files.delete(host); Ok(()) } else { self.files.set(host, token) }
    }
    fn delete(&self, host: &str) { let _ = self.run(&["clear", "service", "perusal", "host", host], None); self.files.delete(host); }
}

pub struct FileStore { pub dir: PathBuf }
impl FileStore {
    fn path(&self, host: &str) -> PathBuf { self.dir.join(format!("{}.token", host.replace(['/', '\\', ':'], "_"))) }
}
impl TokenStore for FileStore {
    fn get(&self, host: &str) -> Option<String> { std::fs::read_to_string(self.path(host)).ok().map(|s| s.trim().to_string()) }
    fn set(&self, host: &str, token: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let p = self.path(host);
        std::fs::write(&p, token)?;
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?; }
        Ok(())
    }
    fn delete(&self, host: &str) { let _ = std::fs::remove_file(self.path(host)); }
}

/// Redacts a token for logs.
pub fn redact(s: &str, token: &str) -> String { if token.is_empty() { s.into() } else { s.replace(token, "***") } }

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCode { pub device_code: String, pub user_code: String, pub verification_uri: String, pub interval: u64 }

/// GitHub-style device flow. `base` is e.g. https://github.com.
pub async fn device_start(c: &reqwest::Client, base: &str, client_id: &str) -> Result<DeviceCode, String> {
    let v: serde_json::Value = c.post(format!("{base}/login/device/code")).header("accept", "application/json")
        .form(&[("client_id", client_id), ("scope", "repo")]).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let s = |k: &str| v[k].as_str().map(String::from).ok_or(format!("missing {k}"));
    Ok(DeviceCode { device_code: s("device_code")?, user_code: s("user_code")?, verification_uri: s("verification_uri")?, interval: v["interval"].as_u64().unwrap_or(5) })
}

pub async fn device_poll(c: &reqwest::Client, base: &str, client_id: &str, d: &DeviceCode, max_tries: u32) -> Result<String, String> {
    for _ in 0..max_tries {
        let v: serde_json::Value = c.post(format!("{base}/login/oauth/access_token")).header("accept", "application/json")
            .form(&[("client_id", client_id), ("device_code", &d.device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])
            .send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
        if let Some(t) = v["access_token"].as_str() { return Ok(t.into()); }
        match v["error"].as_str() { Some("authorization_pending") | Some("slow_down") => tokio::time::sleep(std::time::Duration::from_millis(d.interval)).await,
            Some(e) => return Err(e.into()), None => return Err("bad response".into()) }
    }
    Err("timed out".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn file_store_roundtrip_and_perms() {
        let d = std::env::temp_dir().join(format!("perusal-tok-{}", std::process::id())); let s = FileStore { dir: d.clone() };
        s.set("git.corp:8443", "sekret").unwrap(); assert_eq!(s.get("git.corp:8443").unwrap(), "sekret");
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(s.path("git.corp:8443")).unwrap().permissions().mode() & 0o777, 0o600); }
        s.delete("git.corp:8443"); assert!(s.get("git.corp:8443").is_none()); let _ = std::fs::remove_dir_all(d);
    }
    #[cfg(unix)] #[test] fn keyring_store_uses_tool_then_files() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("perusal-kr-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap();
        // a fake secret-tool keeping secrets in a directory; arguments: store --label=.. service perusal host H | lookup .. | clear ..
        let tool = d.join("secret-tool"); let vault = d.join("vault"); std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(&tool, format!("#!/bin/sh\nV={}\ncmd=$1; shift\ncase $cmd in\n store) shift; host=$4; cat > \"$V/$host\";;\n lookup) host=$4; cat \"$V/$host\" 2>/dev/null;;\n clear) rm -f \"$V/$4\";;\nesac\n", vault.display())).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let files = FileStore { dir: d.join("tokens") }; files.set("old.host", "from-file").unwrap();
        let s = KeyringStore { files: FileStore { dir: d.join("tokens") }, tool: Some(tool) };
        s.set("h.example", "sekret").unwrap(); assert_eq!(s.get("h.example").unwrap(), "sekret");
        assert_eq!(std::fs::read_to_string(vault.join("h.example")).unwrap(), "sekret"); assert!(files.get("h.example").is_none()); // never written to a file
        assert_eq!(s.get("old.host").unwrap(), "from-file"); assert!(files.get("old.host").is_none()); assert_eq!(std::fs::read_to_string(vault.join("old.host")).unwrap(), "from-file"); // migrated
        s.delete("h.example"); assert!(s.get("h.example").is_none());
        // no keychain program: plain 0600 files
        let f = KeyringStore { files: FileStore { dir: d.join("t2") }, tool: None }; f.set("x", "tok").unwrap(); assert_eq!(f.get("x").unwrap(), "tok"); assert!(!f.has_keychain());
        let _ = std::fs::remove_dir_all(d);
    }
    #[test] fn redaction() { assert_eq!(redact("Bearer abc123 x", "abc123"), "Bearer *** x"); }
}
