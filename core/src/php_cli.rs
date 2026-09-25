use crate::paths::LaraluxPaths;
use crate::php_static::{install_php_cli, PhpStaticError};
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

/// Latest stable release. The bare `getcomposer.org/composer.phar` is the dev
/// snapshot, which nags "development build is over 60 days old".
pub const COMPOSER_URL: &str = "https://getcomposer.org/download/latest-stable/composer.phar";
pub const COMPOSER_FALLBACK_VERSION: &str = "2.10.3";

/// Official release feed: `stable` lists the latest stable plus the LTS line.
pub const COMPOSER_VERSIONS_URL: &str = "https://getcomposer.org/versions";
const COMPOSER_VERSIONS_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Offline fallback for the Setup modal (recent 2.x + 2.2 LTS); the release
/// feed adds newer ones. All verified present as `getcomposer.org/download/<ver>/composer.phar`.
pub const KNOWN_COMPOSER_VERSIONS: [&str; 4] = ["2.10.3", "2.9.7", "2.8.12", "2.2.30"];

fn composer_versions_cache(paths: &LaraluxPaths) -> std::path::PathBuf {
    paths.tmp().join("composer-versions.json")
}

/// Stable version strings from the `getcomposer.org/versions` JSON.
pub fn parse_composer_versions(json: &str) -> Vec<String> {
    let root: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    root.get("stable")
        .and_then(|s| s.as_array())
        .map(|arr| arr.iter().filter_map(|e| e.get("version")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Refresh the cached release feed when missing or older than the TTL.
/// Best effort: on failure the previous cache (or the known list) is used.
pub fn refresh_composer_versions(paths: &LaraluxPaths, downloader: &dyn Downloader) {
    let cache = composer_versions_cache(paths);
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < COMPOSER_VERSIONS_TTL);
    if fresh || std::fs::create_dir_all(paths.tmp()).is_err() {
        return;
    }
    let tmp = paths.tmp().join("composer-versions.json.part");
    let ok = downloader.fetch(COMPOSER_VERSIONS_URL, &tmp).is_ok()
        && std::fs::read_to_string(&tmp).is_ok_and(|s| !parse_composer_versions(&s).is_empty());
    if ok {
        let _ = std::fs::rename(&tmp, &cache);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Versions offered in the catalog: the known list unioned with the cached feed.
pub fn composer_catalog_versions(paths: &LaraluxPaths) -> Vec<String> {
    let mut versions: Vec<String> = KNOWN_COMPOSER_VERSIONS.iter().map(|s| s.to_string()).collect();
    let cached = std::fs::read_to_string(composer_versions_cache(paths)).unwrap_or_default();
    for v in parse_composer_versions(&cached) {
        if !versions.contains(&v) {
            versions.push(v);
        }
    }
    versions
}

/// Versioned composer.phar download URL.
pub fn composer_versioned_url(version: &str) -> String {
    format!("https://getcomposer.org/download/{version}/composer.phar")
}

/// Point `bin/php/current` at `<version>` (via layout::set_current).
pub fn set_active_php(paths: &LaraluxPaths, version: &str) -> std::io::Result<()> {
    crate::layout::set_current(paths, "php", version)
}

/// Ensure the active version's cli binary exists (download if missing), then
/// point `bin/php/current` at it.
pub fn ensure_active_php_cli(
    paths: &LaraluxPaths,
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

/// Place a downloaded composer.phar into `bin/composer/<version>/composer.phar`,
/// write the `composer` wrapper (invokes the absolute managed php), chmod it, and
/// point `bin/composer/current` at the version dir.
fn place_composer_phar(paths: &LaraluxPaths, version: &str, tmp_phar: &std::path::Path) -> std::io::Result<()> {
    let dir = paths.version_dir("composer", version);
    std::fs::create_dir_all(&dir)?;
    let phar = dir.join("composer.phar");
    std::fs::rename(tmp_phar, &phar).or_else(|_| {
        std::fs::copy(tmp_phar, &phar).map(|_| ()).and_then(|_| std::fs::remove_file(tmp_phar))
    })?;
    write_composer_wrapper(paths, &dir)?;
    crate::layout::set_current(paths, "composer", version)
}

/// Write the `composer` wrapper into `dir` with ABSOLUTE paths (via the `current`
/// symlinks) rather than $HOME: the wrapper may be invoked as another user (e.g.
/// `sudo composer`, where $HOME becomes /root), so $HOME would resolve wrong.
fn write_composer_wrapper(paths: &LaraluxPaths, dir: &std::path::Path) -> std::io::Result<()> {
    let wrapper = dir.join("composer");
    let php = paths.current_link("php").join("php");
    let phar = paths.current_link("composer").join("composer.phar");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n", php.display(), phar.display()),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Download the latest composer.phar, probe its version, and install it into
/// `bin/composer/<version>/` with a wrapper. Used by the default Setup install.
pub fn install_composer(paths: &LaraluxPaths, downloader: &dyn Downloader, sink: &dyn crate::progress::ProgressSink) -> std::io::Result<()> {
    let tmp_phar = paths.tmp().join("composer.phar");
    std::fs::create_dir_all(paths.tmp())?;
    downloader
        .fetch_with_progress(COMPOSER_URL, &tmp_phar, sink)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let php = paths.current_link("php").join("php");
    let version = crate::layout::probe_version(&php, &[tmp_phar.to_string_lossy().as_ref(), "--version"])
        .unwrap_or_else(|| COMPOSER_FALLBACK_VERSION.to_string());
    place_composer_phar(paths, &version, &tmp_phar)
}

/// Download a SPECIFIC composer version into `bin/composer/<version>/` with a
/// wrapper. Idempotent. An unknown version surfaces as an io error (404).
pub fn install_composer_version(
    paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, sink: &dyn crate::progress::ProgressSink,
) -> std::io::Result<String> {
    if paths.version_dir("composer", version).join("composer").is_file() {
        let _ = crate::layout::set_current(paths, "composer", version);
        return Ok(version.to_string());
    }
    let tmp_phar = paths.tmp().join("composer.phar");
    std::fs::create_dir_all(paths.tmp())?;
    downloader
        .fetch_with_progress(&composer_versioned_url(version), &tmp_phar, sink)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    place_composer_phar(paths, version, &tmp_phar)?;
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::FakeDownloader;
    use std::path::Path;

    fn root() -> LaraluxPaths {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("lara-phpcli-{}-{}", std::process::id(), id));
        let paths = LaraluxPaths::new(p);
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

    const VERSIONS_JSON: &str = r#"{
        "stable": [{"path": "/download/2.11.0/composer.phar", "version": "2.11.0"},
                   {"path": "/download/2.2.31/composer.phar", "version": "2.2.31"}],
        "preview": [{"path": "/download/2.12.0-RC1/composer.phar", "version": "2.12.0-RC1"}],
        "snapshot": [{"path": "/composer.phar", "version": "abc123"}]
    }"#;

    struct JsonDownloader(&'static str);
    impl Downloader for JsonDownloader {
        fn fetch(&self, _url: &str, dest: &Path) -> Result<(), crate::setup::SetupError> {
            std::fs::write(dest, self.0)?;
            Ok(())
        }
    }

    #[test]
    fn composer_url_is_stable_channel_not_snapshot() {
        assert!(COMPOSER_URL.contains("latest-stable"));
    }

    #[test]
    fn parse_composer_versions_keeps_only_stable() {
        assert_eq!(parse_composer_versions(VERSIONS_JSON), vec!["2.11.0", "2.2.31"]);
        assert!(parse_composer_versions("not json").is_empty());
    }

    #[test]
    fn catalog_merges_cached_feed_with_known_list() {
        let paths = root();
        assert_eq!(composer_catalog_versions(&paths).len(), KNOWN_COMPOSER_VERSIONS.len());
        refresh_composer_versions(&paths, &JsonDownloader(VERSIONS_JSON));
        let vs = composer_catalog_versions(&paths);
        assert!(vs.contains(&"2.11.0".to_string()) && vs.contains(&"2.2.31".to_string()));
        assert!(!vs.iter().any(|v| v.contains("RC") || v == "abc123"));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn refresh_skips_download_while_cache_is_fresh_and_ignores_bad_feed() {
        let paths = root();
        refresh_composer_versions(&paths, &JsonDownloader("garbage"));
        assert!(!composer_versions_cache(&paths).exists(), "invalid feed must not be cached");
        refresh_composer_versions(&paths, &JsonDownloader(VERSIONS_JSON));
        let dl = FakeDownloader::new();
        refresh_composer_versions(&paths, &dl);
        assert!(dl.requested().lock().unwrap().is_empty(), "fresh cache must not refetch");
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
        // Absolute path baked in (not $HOME), so `sudo composer` works too.
        assert!(!wrapper.contains("$HOME"));
        assert!(wrapper.contains(&paths.current_link("php").join("php").display().to_string()));
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
