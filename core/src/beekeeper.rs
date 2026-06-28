use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::PathBuf;

pub const BEEKEEPER_VERSION: &str = "5.8.1";

#[derive(Debug, thiserror::Error)]
pub enum BeekeeperError {
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

/// AppImage asset arch suffix: x86_64 → "" , aarch64 → "-arm64".
pub fn beekeeper_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some(""),
        "aarch64" => Some("-arm64"),
        _ => None,
    }
}

pub fn appimage_url(version: &str, arch_suffix: &str) -> String {
    format!(
        "https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v{version}/Beekeeper-Studio-{version}{arch_suffix}.AppImage"
    )
}

pub fn install_dir(paths: &LaraluxPaths) -> PathBuf {
    paths.root().join("apps/beekeeper")
}

pub fn apprun_path(paths: &LaraluxPaths) -> PathBuf {
    install_dir(paths).join("squashfs-root").join("AppRun")
}

pub fn is_installed(paths: &LaraluxPaths) -> bool {
    apprun_path(paths).is_file()
}

/// Download the AppImage and extract it once into apps/beekeeper/squashfs-root/.
/// Idempotent: a no-op (returns the version) when already installed.
pub fn ensure_beekeeper(
    paths: &LaraluxPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, BeekeeperError> {
    if is_installed(paths) {
        return Ok(BEEKEEPER_VERSION.to_string());
    }
    let arch = beekeeper_arch().ok_or_else(|| BeekeeperError::Arch(std::env::consts::ARCH.to_string()))?;
    let dir = install_dir(paths);
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(paths.tmp())?;
    let appimage = paths.tmp().join("beekeeper.AppImage");
    downloader
        .fetch_with_progress(&appimage_url(BEEKEEPER_VERSION, arch), &appimage, sink)
        .map_err(|e| BeekeeperError::Download(e.to_string()))?;
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
        .map_err(|e| BeekeeperError::Extract(e.to_string()))?;
    if !is_installed(paths) {
        return Err(BeekeeperError::Extract("AppRun not found after --appimage-extract".into()));
    }
    let _ = std::fs::remove_file(&appimage);
    Ok(BEEKEEPER_VERSION.to_string())
}

/// Launch the extracted Beekeeper detached via its AppRun (with APPDIR set). The
/// bundled wrapper adds `--no-sandbox` for the unprivileged Electron app itself.
pub fn open_beekeeper(paths: &LaraluxPaths) -> Result<(), BeekeeperError> {
    if !is_installed(paths) {
        return Err(BeekeeperError::NotInstalled);
    }
    // Run the extracted AppRun with APPDIR set. AppRun's own AppDir auto-detection
    // uses $1 as a sentinel filename and breaks if we pass any flag (yielding an
    // empty APPDIR → "/beekeeper-studio: No such file or directory"). Pass NO args:
    // the bundled `beekeeper-studio` wrapper already adds `--no-sandbox` itself.
    let appdir = install_dir(paths).join("squashfs-root");
    std::process::Command::new(apprun_path(paths))
        .env("APPDIR", &appdir)
        .spawn()
        .map_err(|e| BeekeeperError::Spawn(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_and_url() {
        assert_eq!(
            appimage_url("5.8.1", ""),
            "https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v5.8.1/Beekeeper-Studio-5.8.1.AppImage"
        );
        assert_eq!(
            appimage_url("5.8.1", "-arm64"),
            "https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v5.8.1/Beekeeper-Studio-5.8.1-arm64.AppImage"
        );
        assert_eq!(
            beekeeper_arch(),
            match std::env::consts::ARCH { "x86_64" => Some(""), "aarch64" => Some("-arm64"), _ => None }
        );
    }

    #[test]
    fn is_installed_reflects_apprun() {
        let root = std::env::temp_dir().join(format!("lara-bk-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        assert!(!is_installed(&paths));
        std::fs::create_dir_all(install_dir(&paths).join("squashfs-root")).unwrap();
        std::fs::write(apprun_path(&paths), b"x").unwrap();
        assert!(is_installed(&paths));
        std::fs::remove_dir_all(&root).ok();
    }
}
