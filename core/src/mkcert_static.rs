use crate::paths::LaragonPaths;
use crate::progress::ProgressSink;
use crate::setup::Downloader;

pub const MKCERT_VERSION: &str = "1.4.4";

#[derive(Debug, thiserror::Error)]
pub enum MkcertError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn mkcert_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64"), _ => None }
}

pub fn mkcert_url(version: &str, arch: &str) -> String {
    format!("https://github.com/FiloSottile/mkcert/releases/download/v{version}/mkcert-v{version}-linux-{arch}")
}

/// True only if a non-empty regular file exists at `dest`.
fn installed(dest: &std::path::Path) -> bool {
    std::fs::metadata(dest).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Download the static mkcert binary into bin/mkcert/<version>/mkcert (no apt).
pub fn install_mkcert(
    paths: &LaragonPaths, downloader: &dyn Downloader, sink: &dyn ProgressSink,
) -> Result<String, MkcertError> {
    let dir = paths.version_dir("mkcert", MKCERT_VERSION);
    let dest = dir.join("mkcert");
    if installed(&dest) {
        let _ = crate::layout::set_current(paths, "mkcert", MKCERT_VERSION);
        return Ok(MKCERT_VERSION.to_string());
    }
    let arch = mkcert_arch().ok_or_else(|| MkcertError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(&dir)?;
    let tmp = paths.tmp().join("mkcert.download");
    let _ = std::fs::remove_file(&tmp);
    downloader.fetch_with_progress(&mkcert_url(MKCERT_VERSION, arch), &tmp, sink)
        .map_err(|e| MkcertError::Download(e.to_string()))?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?; }
    std::fs::rename(&tmp, &dest).or_else(|_| {
        std::fs::copy(&tmp, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&tmp))
    })?;
    crate::layout::set_current(paths, "mkcert", MKCERT_VERSION)?;
    Ok(MKCERT_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_and_arch() {
        assert_eq!(mkcert_url("1.4.4", "amd64"),
            "https://github.com/FiloSottile/mkcert/releases/download/v1.4.4/mkcert-v1.4.4-linux-amd64");
        assert_eq!(mkcert_arch(), match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64"), _ => None });
    }

    #[test]
    fn install_mkcert_downloads_to_versioned_dir() {
        let root = std::env::temp_dir().join(format!("lara-mkcert-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        let dl = crate::setup::FakeDownloader::new();
        let _ = install_mkcert(&paths, &dl, &crate::progress::NullProgress);
        // FakeDownloader writes "fake" bytes to dest; the binary should exist.
        let dest = paths.version_dir("mkcert", MKCERT_VERSION).join("mkcert");
        assert!(dest.exists(), "mkcert binary should be at {dest:?}");
        // current symlink should point to version
        let link = paths.current_link("mkcert");
        assert!(link.exists(), "current symlink should exist");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_mkcert_skips_if_already_installed() {
        let root = std::env::temp_dir().join(format!("lara-mkcert-skip-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        let dir = paths.version_dir("mkcert", MKCERT_VERSION);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mkcert"), b"ELF-fake").unwrap();
        let dl = crate::setup::FakeDownloader::new();
        let requested = dl.requested();
        let ver = install_mkcert(&paths, &dl, &crate::progress::NullProgress).unwrap();
        assert_eq!(ver, MKCERT_VERSION);
        assert!(requested.lock().unwrap().is_empty(), "should not re-download if already installed");
        std::fs::remove_dir_all(&root).ok();
    }
}
