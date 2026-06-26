use crate::paths::LaragonPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const VALKEY_VERSION: &str = "9.1.0";

#[derive(Debug, thiserror::Error)]
pub enum RedisError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn valkey_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("arm64"), _ => None }
}

pub fn valkey_url(version: &str, arch: &str) -> String {
    format!("https://download.valkey.io/releases/valkey-{version}-jammy-{arch}.tar.gz")
}

fn installed(dest: &std::path::Path) -> bool {
    std::fs::metadata(dest).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Find a file named `name` anywhere under `root` (shallow walk of dirs).
fn find_under(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p); }
                else if p.file_name().map(|n| n == name).unwrap_or(false) { return Some(p); }
            }
        }
    }
    None
}

/// Curated Valkey versions offered in the Setup modal (latest patch per line).
/// All verified present on download.valkey.io as `jammy-<arch>` tarballs.
/// Note: Valkey/Redis persists an RDB (`dump.rdb`); switching to a version with
/// an older RDB format than the on-disk dump can refuse to start (recoverable by
/// switching back / moving the dump aside).
pub const KNOWN_REDIS_VERSIONS: [&str; 4] = ["9.1.0", "8.1.3", "8.0.4", "7.2.10"];

/// Download + install the default (pinned) Valkey version.
pub fn install_redis(
    paths: &LaragonPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, RedisError> {
    install_redis_version(paths, VALKEY_VERSION, downloader, runner, sink)
}

/// Download a SPECIFIC Valkey version and install valkey-server/valkey-cli as
/// bin/redis/<version>/{redis-server,redis-cli} (drop-in for redis). Idempotent.
/// An unknown version surfaces as `RedisError::Download`.
pub fn install_redis_version(
    paths: &LaragonPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, RedisError> {
    let dir = paths.version_dir("redis", version);
    let server = dir.join("redis-server");
    if installed(&server) {
        let _ = crate::layout::set_current(paths, "redis", version);
        return Ok(version.to_string());
    }
    let arch = valkey_arch().ok_or_else(|| RedisError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(&dir)?;
    let tgz = paths.tmp().join("valkey.tar.gz");
    downloader.fetch_with_progress(&valkey_url(version, arch), &tgz, sink)
        .map_err(|e| RedisError::Download(e.to_string()))?;
    let xdir = paths.tmp().join("valkey-extract");
    let _ = std::fs::remove_dir_all(&xdir);
    std::fs::create_dir_all(&xdir)?;
    runner.run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), xdir.display().to_string()], None)
        .map_err(|e| RedisError::Extract(e.to_string()))?;
    let vs = find_under(&xdir, "valkey-server").ok_or_else(|| RedisError::Extract("valkey-server not found in archive".into()))?;
    install_one(&vs, &server)?;
    if let Some(vc) = find_under(&xdir, "valkey-cli") { let _ = install_one(&vc, &dir.join("redis-cli")); }
    crate::layout::set_current(paths, "redis", version)?;
    Ok(version.to_string())
}

fn install_one(src: &std::path::Path, dest: &std::path::Path) -> Result<(), RedisError> {
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(src, std::fs::Permissions::from_mode(0o755)); }
    std::fs::rename(src, dest).or_else(|_| {
        std::fs::copy(src, dest).map(|_| ()).and_then(|_| std::fs::remove_file(src))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_and_arch() {
        assert_eq!(valkey_url("9.1.0", "x86_64"),
            "https://download.valkey.io/releases/valkey-9.1.0-jammy-x86_64.tar.gz");
        assert_eq!(valkey_arch(), match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("arm64"), _ => None });
    }

    #[test]
    fn known_versions_include_pinned_default() {
        assert!(KNOWN_REDIS_VERSIONS.contains(&VALKEY_VERSION));
        assert_eq!(valkey_url("8.0.4", "x86_64"),
            "https://download.valkey.io/releases/valkey-8.0.4-jammy-x86_64.tar.gz");
    }
}
