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

/// Windows Credential Manager (generic credentials, one per host, target `Perusal/<host>`). Tokens never touch the disk.
#[cfg(windows)]
pub mod wincred {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND, FALSE};
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};

    fn target(host: &str) -> Vec<u16> { format!("Perusal/{host}").encode_utf16().chain(Some(0)).collect() }

    pub fn read(host: &str) -> Option<String> {
        let t = target(host); let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `t` is NUL terminated; on success `p` points to a credential we free with CredFree after copying the blob out.
        unsafe {
            if CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) == FALSE { return None; }
            let c = &*p; let s = if c.CredentialBlob.is_null() { Vec::new() } else { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec() };
            CredFree(p as *const _); String::from_utf8(s).ok()
        }
    }
    pub fn write(host: &str, token: &str) -> std::io::Result<()> {
        let mut t = target(host); let mut user: Vec<u16> = host.encode_utf16().chain(Some(0)).collect(); let mut blob = token.as_bytes().to_vec();
        // SAFETY: a zeroed CREDENTIALW is valid; the pointers stay alive for the call.
        unsafe {
            let mut c: CREDENTIALW = std::mem::zeroed();
            c.Type = CRED_TYPE_GENERIC; c.TargetName = t.as_mut_ptr(); c.UserName = user.as_mut_ptr();
            c.CredentialBlobSize = blob.len() as u32; c.CredentialBlob = blob.as_mut_ptr(); c.Persist = CRED_PERSIST_LOCAL_MACHINE;
            if CredWriteW(&c, 0) == FALSE { return Err(std::io::Error::from_raw_os_error(GetLastError() as i32)); }
        }
        Ok(())
    }
    pub fn delete(host: &str) -> bool {
        let t = target(host);
        // SAFETY: `t` is NUL terminated.
        unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) != FALSE || GetLastError() == ERROR_NOT_FOUND }
    }
}

/// The store the app uses: the OS credential store where there is one (Windows Credential Manager), else 0600 files.
/// On Windows an older token file is moved into the Credential Manager the first time it is read.
pub struct DefaultStore { pub files: FileStore }
impl DefaultStore { pub fn new(dir: PathBuf) -> Self { Self { files: FileStore { dir } } } }
#[cfg(windows)]
impl TokenStore for DefaultStore {
    fn get(&self, host: &str) -> Option<String> {
        if let Some(t) = wincred::read(host) { return Some(t); }
        let t = self.files.get(host)?; // migrate a token saved by an earlier version
        if wincred::write(host, &t).is_ok() { self.files.delete(host); }
        Some(t)
    }
    fn set(&self, host: &str, token: &str) -> std::io::Result<()> {
        // No credential store (for example a service session without a logon session): fall back to the token file.
        match wincred::write(host, token) { Ok(()) => { self.files.delete(host); Ok(()) } Err(_) => self.files.set(host, token) }
    }
    fn delete(&self, host: &str) { wincred::delete(host); self.files.delete(host); }
}
#[cfg(not(windows))]
impl TokenStore for DefaultStore {
    fn get(&self, host: &str) -> Option<String> { self.files.get(host) }
    fn set(&self, host: &str, token: &str) -> std::io::Result<()> { self.files.set(host, token) }
    fn delete(&self, host: &str) { self.files.delete(host) }
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
    #[cfg(windows)] #[test] fn credential_manager_roundtrip_and_migration() {
        let host = format!("perusal-test-{}.example", std::process::id());
        let d = std::env::temp_dir().join(format!("perusal-cm-{}", std::process::id())); let s = DefaultStore::new(d.clone());
        assert!(s.get(&host).is_none());
        if wincred::write(&host, "probe").is_err() { eprintln!("no Credential Manager in this session; skipped"); return; } wincred::delete(&host);
        s.set(&host, "tok-ü-1").unwrap(); assert_eq!(s.get(&host).as_deref(), Some("tok-ü-1")); assert!(s.files.get(&host).is_none(), "no file on disk");
        s.set(&host, "tok2").unwrap(); assert_eq!(s.get(&host).as_deref(), Some("tok2"));
        s.delete(&host); assert!(s.get(&host).is_none());
        // a token file from an older version is moved into the Credential Manager
        s.files.set(&host, "old").unwrap(); assert_eq!(s.get(&host).as_deref(), Some("old")); assert!(s.files.get(&host).is_none()); assert_eq!(wincred::read(&host).as_deref(), Some("old"));
        s.delete(&host); let _ = std::fs::remove_dir_all(d);
    }
    #[test] fn redaction() { assert_eq!(redact("Bearer abc123 x", "abc123"), "Bearer *** x"); }
}
