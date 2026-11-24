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
    #[test] fn redaction() { assert_eq!(redact("Bearer abc123 x", "abc123"), "Bearer *** x"); }
}
