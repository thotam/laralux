use crate::paths::LaragonPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::{Path, PathBuf};

/// Version recorded when `mailpit version` can't be probed (latest-install only).
pub const MAILPIT_FALLBACK_VERSION: &str = "1.20.0";

/// Curated mailpit versions offered in the Setup modal (latest + recent releases).
/// All verified present as GitHub release `mailpit-linux-<arch>.tar.gz` assets.
pub const KNOWN_MAILPIT_VERSIONS: [&str; 4] = ["1.30.2", "1.27.6", "1.25.0", "1.22.3"];

#[derive(Debug, thiserror::Error)]
pub enum MailpitError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn mailpit_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// Release-asset URL for a specific version.
pub fn mailpit_url(version: &str, arch: &str) -> String {
    format!("https://github.com/axllent/mailpit/releases/download/v{version}/mailpit-linux-{arch}.tar.gz")
}

/// Release-asset URL for the latest version (used by the default Setup install).
pub fn mailpit_latest_url(arch: &str) -> String {
    format!("https://github.com/axllent/mailpit/releases/latest/download/mailpit-linux-{arch}.tar.gz")
}

fn installed(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Extract the single `mailpit` entry from `tgz` into a fresh tmp dir; return its path.
fn extract_mailpit(paths: &LaragonPaths, tgz: &Path, runner: &dyn CommandRunner) -> Result<PathBuf, MailpitError> {
    let xdir = paths.tmp().join("mailpit-extract");
    let _ = std::fs::remove_dir_all(&xdir);
    std::fs::create_dir_all(&xdir)?;
    runner
        .run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), xdir.display().to_string(), "mailpit".into()], None)
        .map_err(|e| MailpitError::Extract(e.to_string()))?;
    let extracted = xdir.join("mailpit");
    if !extracted.is_file() {
        return Err(MailpitError::Extract("mailpit binary not found in archive".into()));
    }
    Ok(extracted)
}

/// Move the extracted binary into bin/mailpit/<version>/mailpit (chmod 0755) and
/// point `current` at it. Returns the version.
fn place_mailpit(paths: &LaragonPaths, version: &str, extracted: &Path) -> Result<String, MailpitError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(extracted, std::fs::Permissions::from_mode(0o755));
    }
    let dir = paths.version_dir("mailpit", version);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join("mailpit");
    std::fs::rename(extracted, &dest).or_else(|_| {
        std::fs::copy(extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(extracted))
    })?;
    crate::layout::set_current(paths, "mailpit", version)?;
    Ok(version.to_string())
}

/// Install the latest mailpit into bin/mailpit/<probed-version>/mailpit.
/// Returns the version reported by `mailpit version` (or the fallback).
pub fn install_mailpit(
    paths: &LaragonPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MailpitError> {
    let arch = mailpit_arch().ok_or_else(|| MailpitError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let tgz = paths.tmp().join("mailpit.tar.gz");
    downloader.fetch_with_progress(&mailpit_latest_url(arch), &tgz, sink)
        .map_err(|e| MailpitError::Download(e.to_string()))?;
    let extracted = extract_mailpit(paths, &tgz, runner)?;
    let ver = crate::layout::probe_version(&extracted, &["version"])
        .unwrap_or_else(|| MAILPIT_FALLBACK_VERSION.to_string());
    place_mailpit(paths, &ver, &extracted)
}

/// Install a SPECIFIC mailpit version into bin/mailpit/<version>/mailpit. Idempotent.
/// An unknown version surfaces as `MailpitError::Download` (the asset URL 404s).
pub fn install_mailpit_version(
    paths: &LaragonPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MailpitError> {
    let dest = paths.version_dir("mailpit", version).join("mailpit");
    if installed(&dest) {
        let _ = crate::layout::set_current(paths, "mailpit", version);
        return Ok(version.to_string());
    }
    let arch = mailpit_arch().ok_or_else(|| MailpitError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let tgz = paths.tmp().join("mailpit.tar.gz");
    downloader.fetch_with_progress(&mailpit_url(version, arch), &tgz, sink)
        .map_err(|e| MailpitError::Download(e.to_string()))?;
    let extracted = extract_mailpit(paths, &tgz, runner)?;
    place_mailpit(paths, version, &extracted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_arch() {
        assert_eq!(
            mailpit_url("1.27.6", "amd64"),
            "https://github.com/axllent/mailpit/releases/download/v1.27.6/mailpit-linux-amd64.tar.gz"
        );
        assert_eq!(mailpit_latest_url("amd64"),
            "https://github.com/axllent/mailpit/releases/latest/download/mailpit-linux-amd64.tar.gz");
        assert_eq!(mailpit_arch(), match std::env::consts::ARCH {
            "x86_64" => Some("amd64"), "aarch64" => Some("arm64"), _ => None });
    }
}
