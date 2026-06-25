use crate::paths::LaragonPaths;
use crate::php_static::{install_php_cli, PhpStaticError};
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const COMPOSER_URL: &str = "https://getcomposer.org/composer.phar";
pub const COMPOSER_FALLBACK_VERSION: &str = "2.8.9";

/// Point `bin/php/current` at `<version>` (via layout::set_current).
pub fn set_active_php(paths: &LaragonPaths, version: &str) -> std::io::Result<()> {
    crate::layout::set_current(paths, "php", version)
}

/// Ensure the active version's cli binary exists (download if missing), then
/// point `bin/php/current` at it.
pub fn ensure_active_php_cli(
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn crate::progress::ProgressSink,
) -> Result<(), PhpStaticError> {
    let full = match crate::layout::resolve_installed_version(paths, "php", version) {
        Some(f) if paths.version_dir("php", &f).join("php").is_file() => f,
        _ => install_php_cli(paths, version, downloader, runner, sink)?,
    };
    set_active_php(paths, &full).map_err(PhpStaticError::Io)?;
    Ok(())
}

/// Download composer.phar, probe its version, place it into `bin/composer/<version>/`,
/// write a wrapper script, and point `bin/composer/current` at the version dir.
pub fn install_composer(paths: &LaragonPaths, downloader: &dyn Downloader, sink: &dyn crate::progress::ProgressSink) -> std::io::Result<()> {
    // Download to tmp, read its version, then place into bin/composer/<version>/.
    let tmp_phar = paths.tmp().join("composer.phar");
    std::fs::create_dir_all(paths.tmp())?;
    downloader
        .fetch_with_progress(COMPOSER_URL, &tmp_phar, sink)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let php = paths.current_link("php").join("php");
    let version = crate::layout::probe_version(&php, &[tmp_phar.to_string_lossy().as_ref(), "--version"])
        .unwrap_or_else(|| COMPOSER_FALLBACK_VERSION.to_string());
    let dir = paths.version_dir("composer", &version);
    std::fs::create_dir_all(&dir)?;
    let phar = dir.join("composer.phar");
    std::fs::rename(&tmp_phar, &phar).or_else(|_| {
        std::fs::copy(&tmp_phar, &phar)
            .map(|_| ())
            .and_then(|_| std::fs::remove_file(&tmp_phar))
    })?;
    let wrapper = dir.join("composer");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nexec \"$HOME/laragon/bin/php/current/php\" \"$HOME/laragon/bin/composer/current/composer.phar\" \"$@\"\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
    }
    crate::layout::set_current(paths, "composer", &version)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::FakeDownloader;
    use std::path::Path;

    fn root() -> LaragonPaths {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("lara-phpcli-{}-{}", std::process::id(), id));
        let paths = LaragonPaths::new(p);
        paths.ensure_dirs().unwrap();
        paths
    }

    #[test]
    fn set_active_php_points_php_to_versioned_binary() {
        let paths = root();
        // Create version dirs so set_current has targets
        std::fs::create_dir_all(paths.version_dir("php", "8.4.10")).unwrap();
        std::fs::write(paths.version_dir("php", "8.4.10").join("php"), b"x").unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::write(paths.version_dir("php", "8.3.31").join("php"), b"x").unwrap();

        set_active_php(&paths, "8.4.10").unwrap();
        let link = paths.current_link("php");
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("8.4.10"));

        // re-point
        set_active_php(&paths, "8.3.31").unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("8.3.31"));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn ensure_active_php_cli_symlinks_without_download_when_present() {
        let paths = root();
        // Create version dir with php binary
        std::fs::create_dir_all(paths.version_dir("php", "8.4.10")).unwrap();
        std::fs::write(paths.version_dir("php", "8.4.10").join("php"), b"x").unwrap();
        let dl = FakeDownloader::new(); // would write "fake"; must NOT be called
        let runner = crate::scaffold::FakeCommandRunner::new();
        ensure_active_php_cli(&paths, "8.4.10", &dl, &runner, &crate::progress::NullProgress).unwrap();
        let link = paths.current_link("php");
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("8.4.10"));
        assert!(dl.requested().lock().unwrap().is_empty(), "no download when cli present");
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn install_composer_writes_phar_and_wrapper() {
        let paths = root();
        // Seed bin/php/current/php so probe_version has a target (will fail → fallback)
        std::fs::create_dir_all(paths.current_link("php")).unwrap();
        std::fs::write(paths.current_link("php").join("php"), b"x").unwrap();
        let dl = FakeDownloader::new();
        install_composer(&paths, &dl, &crate::progress::NullProgress).unwrap();
        // composer.phar lands in the fallback version dir
        let dir = paths.version_dir("composer", COMPOSER_FALLBACK_VERSION);
        assert!(dir.join("composer.phar").is_file());
        let wrapper = std::fs::read_to_string(dir.join("composer")).unwrap();
        assert!(wrapper.contains("exec"));
        assert!(wrapper.contains("composer.phar"));
        assert!(wrapper.contains("$HOME/laragon/bin/php/current/php"));
        // current symlink points at the fallback version
        assert_eq!(
            std::fs::read_link(paths.current_link("composer")).unwrap(),
            Path::new(COMPOSER_FALLBACK_VERSION)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("composer")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        std::fs::remove_dir_all(paths.root()).ok();
    }
}
