use crate::paths::LaragonPaths;
use crate::php_static::{install_php_cli, PhpStaticError};
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const COMPOSER_URL: &str = "https://getcomposer.org/composer.phar";

/// Point `~/laragon/bin/php` at `php<version>` (replace any existing symlink/file).
pub fn set_active_php(paths: &LaragonPaths, version: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(paths.bin())?;
    let link = paths.bin().join("php");
    let _ = std::fs::remove_file(&link); // remove stale symlink/file if present
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(format!("php{version}"), &link)?;
    }
    Ok(())
}

/// Ensure the active version's cli binary exists (download if missing), then
/// point `~/laragon/bin/php` at it.
pub fn ensure_active_php_cli(
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    if !paths.bin().join(format!("php{version}")).exists() {
        let _ = install_php_cli(paths, version, downloader, runner)?;
    }
    set_active_php(paths, version)?;
    Ok(())
}

/// Download composer.phar and write a `composer` wrapper that runs it under the
/// active `~/laragon/bin/php`.
pub fn install_composer(paths: &LaragonPaths, downloader: &dyn Downloader) -> std::io::Result<()> {
    std::fs::create_dir_all(paths.bin())?;
    let phar = paths.bin().join("composer.phar");
    downloader
        .fetch(COMPOSER_URL, &phar)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let wrapper = paths.bin().join("composer");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nexec \"$(dirname \"$0\")/php\" \"$(dirname \"$0\")/composer.phar\" \"$@\"\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
    }
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
        std::fs::write(paths.bin().join("php8.4"), b"x").unwrap();
        std::fs::write(paths.bin().join("php8.3"), b"x").unwrap();

        set_active_php(&paths, "8.4").unwrap();
        let link = paths.bin().join("php");
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("php8.4"));

        // re-point
        set_active_php(&paths, "8.3").unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("php8.3"));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn ensure_active_php_cli_symlinks_without_download_when_present() {
        let paths = root();
        std::fs::write(paths.bin().join("php8.4"), b"x").unwrap();
        let dl = FakeDownloader::new(); // would write "fake"; must NOT be called
        let runner = crate::scaffold::FakeCommandRunner::new();
        ensure_active_php_cli(&paths, "8.4", &dl, &runner).unwrap();
        assert_eq!(std::fs::read_link(paths.bin().join("php")).unwrap(), Path::new("php8.4"));
        assert!(dl.requested().lock().unwrap().is_empty(), "no download when cli present");
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn install_composer_writes_phar_and_wrapper() {
        let paths = root();
        let dl = FakeDownloader::new();
        install_composer(&paths, &dl).unwrap();
        assert!(paths.bin().join("composer.phar").is_file());
        let wrapper = std::fs::read_to_string(paths.bin().join("composer")).unwrap();
        assert!(wrapper.contains("exec"));
        assert!(wrapper.contains("composer.phar"));
        assert!(wrapper.contains("/php\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths.bin().join("composer")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        std::fs::remove_dir_all(paths.root()).ok();
    }
}
