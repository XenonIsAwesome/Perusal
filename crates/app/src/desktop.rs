//! The user-level desktop entry that makes `perusal://` links open this binary (needed for AppImages, which are not installed).
use std::path::{Path, PathBuf};

/// Text of the entry for an executable at `exe`. Paths with spaces are quoted as the Desktop Entry spec requires.
pub fn entry(exe: &Path, icon: Option<&Path>) -> String {
    let q = |p: &Path| { let s = p.display().to_string(); if s.contains(' ') { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")) } else { s } };
    format!("[Desktop Entry]\nType=Application\nName=Perusal\nComment=Review merge requests and pull requests quickly\nExec={} %u\nIcon={}\nCategories=Development;\nMimeType=x-scheme-handler/perusal;\nTerminal=false\n",
        q(exe), icon.map(|i| i.display().to_string()).unwrap_or_else(|| "perusal".into()))
}

/// Where the entry goes: `$XDG_DATA_HOME/applications/perusal.desktop` (or `~/.local/share/applications`).
pub fn entry_path(home: &Path, xdg_data: Option<&Path>) -> PathBuf { xdg_data.map(Path::to_path_buf).unwrap_or_else(|| home.join(".local/share")).join("applications/perusal.desktop") }

/// The AppImage file the process runs from, if any.
pub fn appimage() -> Option<PathBuf> { std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.exists()) }

#[cfg(test)]
mod tests { use super::*;
    #[test] fn entry_text_and_path() {
        let e = entry(Path::new("/opt/Perusal.AppImage"), None); assert!(e.contains("Exec=/opt/Perusal.AppImage %u") && e.contains("x-scheme-handler/perusal;") && e.contains("Icon=perusal"));
        assert!(entry(Path::new("/my apps/p.AppImage"), None).contains("Exec=\"/my apps/p.AppImage\" %u"));
        assert_eq!(entry_path(Path::new("/h"), None), Path::new("/h/.local/share/applications/perusal.desktop"));
        assert_eq!(entry_path(Path::new("/h"), Some(Path::new("/x"))), Path::new("/x/applications/perusal.desktop"));
    }
}
