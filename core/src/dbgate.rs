use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::PathBuf;

pub const DBGATE_VERSION: &str = "7.2.1";

#[derive(Debug, thiserror::Error)]
pub enum DbgateError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("not installed")]
    NotInstalled,
    #[error("failed to launch: {0}")]
    Spawn(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// AppImage asset arch token: x86_64 → "x86_64", aarch64 → "arm64".
pub fn dbgate_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

pub fn appimage_url(version: &str, arch: &str) -> String {
    format!(
        "https://github.com/dbgate/dbgate/releases/download/v{version}/dbgate-{version}-linux_{arch}.AppImage"
    )
}

pub fn install_dir(paths: &LaraluxPaths) -> PathBuf {
    paths.root().join("apps/dbgate")
}

pub fn apprun_path(paths: &LaraluxPaths) -> PathBuf {
    install_dir(paths).join("squashfs-root").join("AppRun")
}

pub fn is_installed(paths: &LaraluxPaths) -> bool {
    apprun_path(paths).is_file()
}

/// Download the AppImage and extract it once into apps/dbgate/squashfs-root/.
/// Idempotent: a no-op (returns the version) when already installed.
pub fn ensure_dbgate(
    paths: &LaraluxPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, DbgateError> {
    if is_installed(paths) {
        return Ok(DBGATE_VERSION.to_string());
    }
    let arch = dbgate_arch().ok_or_else(|| DbgateError::Arch(std::env::consts::ARCH.to_string()))?;
    let dir = install_dir(paths);
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(paths.tmp())?;
    let appimage = paths.tmp().join("dbgate.AppImage");
    downloader
        .fetch_with_progress(&appimage_url(DBGATE_VERSION, arch), &appimage, sink)
        .map_err(|e| DbgateError::Download(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&appimage, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = std::fs::remove_dir_all(dir.join("squashfs-root")); // clear any stale extract
    // `--appimage-extract` uses the AppImage runtime's built-in extractor (no FUSE);
    // it writes `squashfs-root` into the CWD, so run it with CWD = install dir.
    runner
        .run(&appimage.display().to_string(), &["--appimage-extract".into()], Some(&dir))
        .map_err(|e| DbgateError::Extract(e.to_string()))?;
    if !is_installed(paths) {
        return Err(DbgateError::Extract("AppRun not found after --appimage-extract".into()));
    }
    let _ = std::fs::remove_file(&appimage);
    Ok(DBGATE_VERSION.to_string())
}

/// Launch the extracted DbGate detached via its AppRun (with APPDIR set). DbGate's
/// AppRun auto-detects the AppDir using `$1` as a sentinel filename when `$APPDIR`
/// is empty, so passing a flag (e.g. `--no-sandbox`) breaks detection. We set
/// `APPDIR` explicitly and pass NO args: DbGate runs unprivileged without needing
/// `--no-sandbox` (its `dbgate` binary rejects that flag anyway).
pub fn open_dbgate(paths: &LaraluxPaths) -> Result<(), DbgateError> {
    if !is_installed(paths) {
        return Err(DbgateError::NotInstalled);
    }
    let appdir = install_dir(paths).join("squashfs-root");
    std::process::Command::new(apprun_path(paths))
        .env("APPDIR", &appdir)
        .spawn()
        .map_err(|e| DbgateError::Spawn(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_and_url() {
        assert_eq!(
            appimage_url("7.2.1", "x86_64"),
            "https://github.com/dbgate/dbgate/releases/download/v7.2.1/dbgate-7.2.1-linux_x86_64.AppImage"
        );
        assert_eq!(
            appimage_url("7.2.1", "arm64"),
            "https://github.com/dbgate/dbgate/releases/download/v7.2.1/dbgate-7.2.1-linux_arm64.AppImage"
        );
        assert_eq!(
            dbgate_arch(),
            match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("arm64"), _ => None }
        );
    }

    #[test]
    fn is_installed_reflects_apprun() {
        let root = std::env::temp_dir().join(format!("lara-dbg-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        assert!(!is_installed(&paths));
        std::fs::create_dir_all(install_dir(&paths).join("squashfs-root")).unwrap();
        std::fs::write(apprun_path(&paths), b"x").unwrap();
        assert!(is_installed(&paths));
        std::fs::remove_dir_all(&root).ok();
    }
}
