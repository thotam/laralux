use std::path::{Path, PathBuf};

/// Resolve the XDG config base from the relevant env values (pure — no env read,
/// so it is deterministically testable).
fn resolve_base(xdg_config_home: Option<&str>, home: &str) -> PathBuf {
    match xdg_config_home {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(home).join(".config"),
    }
}

/// Path to the XDG autostart desktop entry for Laralux:
/// `$XDG_CONFIG_HOME/autostart/laralux.desktop` (else `~/.config/autostart/...`).
pub fn autostart_path() -> PathBuf {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    resolve_base(xdg.as_deref(), &home)
        .join("autostart")
        .join("laralux.desktop")
}

fn entry_contents(exec_path: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Laralux\n\
         Exec={}\n\
         Icon=com.laralux.linux\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         Comment=Local web-development environment manager\n",
        exec_path.display()
    )
}

fn write_entry(path: &Path, exec_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, entry_contents(exec_path))
}

fn remove_entry(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Write the autostart entry pointing at `exec_path` (caller passes the running
/// executable's path, e.g. `std::env::current_exe()`).
pub fn enable_autostart(exec_path: &Path) -> std::io::Result<()> {
    write_entry(&autostart_path(), exec_path)
}

/// Remove the autostart entry. A missing file is success (idempotent).
pub fn disable_autostart() -> std::io::Result<()> {
    remove_entry(&autostart_path())
}

/// Whether the autostart entry currently exists.
pub fn is_autostart_enabled() -> bool {
    autostart_path().exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_base_prefers_xdg_then_home_config() {
        assert_eq!(resolve_base(Some("/x/cfg"), "/home/u"), PathBuf::from("/x/cfg"));
        assert_eq!(resolve_base(Some(""), "/home/u"), PathBuf::from("/home/u/.config"));
        assert_eq!(resolve_base(None, "/home/u"), PathBuf::from("/home/u/.config"));
    }

    #[test]
    fn write_entry_then_remove_idempotent() {
        let dir = std::env::temp_dir().join(format!("lara-autostart-{}", std::process::id()));
        let path = dir.join("autostart").join("laralux.desktop");
        write_entry(&path, Path::new("/usr/bin/laralux")).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("Exec=/usr/bin/laralux"));
        assert!(body.contains("Name=Laralux"));
        assert!(body.contains("Type=Application"));
        assert!(body.contains("Icon=com.laralux.linux"));
        // remove, then remove again — both succeed
        remove_entry(&path).unwrap();
        assert!(!path.exists());
        remove_entry(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
