use crate::paths::LaragonPaths;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const STATIC_PHP_BASE: &str = "https://dl.static-php.dev/static-php-cli/bulk";

#[derive(Debug, thiserror::Error)]
pub enum PhpStaticError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("php {0} is not available as a static build")]
    Unavailable(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Pure arch mapping (testable without touching the host).
fn arch_from(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

/// The static-php arch tag for the current host, or None if unsupported.
pub fn arch_tag() -> Option<&'static str> {
    arch_from(std::env::consts::ARCH)
}

/// Find the newest `php-<version>.<patch>-fpm-linux-<arch>.tar.gz` in the
/// directory JSON and return its full download URL (or None if absent).
pub fn latest_patch_url(version: &str, arch: &str, listing_json: &str) -> Option<String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(listing_json).ok()?;
    let prefix = format!("php-{version}.");
    let suffix = format!("-fpm-linux-{arch}.tar.gz");
    let mut best: Option<(u32, String)> = None;
    for e in &entries {
        let name = match e.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };
        if let (true, true) = (name.starts_with(&prefix), name.ends_with(&suffix)) {
            let mid = &name[prefix.len()..name.len() - suffix.len()];
            if let Ok(patch) = mid.parse::<u32>() {
                if best.as_ref().map_or(true, |(b, _)| patch > *b) {
                    best = Some((patch, name.to_string()));
                }
            }
        }
    }
    best.map(|(_, name)| format!("{STATIC_PHP_BASE}/{name}"))
}

/// Download a static php-fpm `bulk` build for `version` and install it as
/// `~/laragon/bin/php-fpm<version>` (mode 0755). No privilege required.
pub fn install_php_static(
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(paths.bin())?;

    // 1. Fetch the directory index and resolve the newest patch URL.
    let index = paths.tmp().join("static-php-index.json");
    downloader
        .fetch(&format!("{STATIC_PHP_BASE}/?format=json"), &index)
        .map_err(|e| PhpStaticError::Download(e.to_string()))?;
    let json = std::fs::read_to_string(&index)?;
    let url = latest_patch_url(version, arch, &json)
        .ok_or_else(|| PhpStaticError::Unavailable(version.to_string()))?;

    // 2. Download + extract the single `php-fpm` binary into tmp.
    let tarball = paths.tmp().join(format!("php-{version}-fpm.tar.gz"));
    downloader
        .fetch(&url, &tarball)
        .map_err(|e| PhpStaticError::Download(e.to_string()))?;
    runner
        .run(
            "tar",
            &[
                "-xzf".to_string(),
                tarball.display().to_string(),
                "-C".to_string(),
                paths.tmp().display().to_string(),
                "php-fpm".to_string(),
            ],
            None,
        )
        .map_err(|e| PhpStaticError::Extract(e.to_string()))?;

    // 3. Move into place as php-fpm<version> with exec perms.
    let extracted = paths.tmp().join("php-fpm");
    let dest = paths.bin().join(format!("php-fpm{version}"));
    std::fs::rename(&extracted, &dest).or_else(|_| {
        std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scaffold::ScaffoldError;
    use crate::setup::{Downloader, SetupError};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    const SAMPLE: &str = r#"[
      {"name":"license/","is_dir":true},
      {"name":"php-8.3.31-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.9-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.22-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.22-cli-linux-x86_64.tar.gz"},
      {"name":"php-8.4.30-fpm-linux-aarch64.tar.gz"}
    ]"#;

    #[test]
    fn latest_patch_url_picks_highest_patch_for_arch() {
        let url = latest_patch_url("8.4", "x86_64", SAMPLE).unwrap();
        assert_eq!(url, format!("{STATIC_PHP_BASE}/php-8.4.22-fpm-linux-x86_64.tar.gz"));
    }

    #[test]
    fn latest_patch_url_none_for_missing_version_or_arch() {
        assert!(latest_patch_url("7.4", "x86_64", SAMPLE).is_none());
        assert!(latest_patch_url("8.4", "riscv64", SAMPLE).is_none());
    }

    #[test]
    fn arch_tag_maps_known() {
        // arch_from is the pure mapping behind arch_tag()
        assert_eq!(arch_from("x86_64"), Some("x86_64"));
        assert_eq!(arch_from("aarch64"), Some("aarch64"));
        assert_eq!(arch_from("riscv64"), None);
    }

    // A downloader that serves the index JSON for the `?format=json` URL and
    // dummy bytes for the tarball URL.
    struct StubDownloader {
        index_json: String,
        fetched: Arc<Mutex<Vec<String>>>,
    }
    impl Downloader for StubDownloader {
        fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError> {
            self.fetched.lock().unwrap().push(url.to_string());
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(SetupError::Io)?;
            }
            if url.ends_with("?format=json") {
                std::fs::write(dest, self.index_json.as_bytes()).map_err(SetupError::Io)?;
            } else {
                std::fs::write(dest, b"tarball").map_err(SetupError::Io)?;
            }
            Ok(())
        }
    }

    // A runner that emulates `tar -xzf <tarball> -C <dir> php-fpm` by creating
    // the extracted `php-fpm` file in the dest dir.
    struct TarRunner {
        calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    }
    impl CommandRunner for TarRunner {
        fn run(&self, program: &str, args: &[String], _cwd: Option<&Path>) -> Result<(), ScaffoldError> {
            self.calls.lock().unwrap().push((program.to_string(), args.to_vec()));
            // args: ["-xzf", <tarball>, "-C", <dir>, "php-fpm"]
            let dir = &args[3];
            std::fs::write(Path::new(dir).join("php-fpm"), b"bin").unwrap();
            Ok(())
        }
    }

    #[test]
    fn install_php_static_downloads_extracts_and_places_binary() {
        let root = std::env::temp_dir().join(format!("lara-spi-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let arch = arch_tag().expect("supported test arch");
        let json = format!(
            "[{{\"name\":\"php-8.4.22-fpm-linux-{arch}.tar.gz\"}},{{\"name\":\"php-8.4.9-fpm-linux-{arch}.tar.gz\"}}]"
        );
        let fetched = Arc::new(Mutex::new(Vec::new()));
        let dl = StubDownloader { index_json: json, fetched: fetched.clone() };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let runner = TarRunner { calls: calls.clone() };

        install_php_static(&paths, "8.4", &dl, &runner).unwrap();

        let f = fetched.lock().unwrap();
        assert!(f[0].ends_with("?format=json"), "index fetched first");
        assert!(f[1].ends_with("php-8.4.22-fpm-linux-{arch}.tar.gz".replace("{arch}", arch).as_str()));
        assert_eq!(calls.lock().unwrap()[0].0, "tar");
        let bin = paths.bin().join("php-fpm8.4");
        assert!(bin.is_file(), "binary placed in ~/laragon/bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bin).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_php_static_unavailable_version_errors() {
        let root = std::env::temp_dir().join(format!("lara-spi2-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let dl = StubDownloader { index_json: "[]".to_string(), fetched: Arc::new(Mutex::new(Vec::new())) };
        let runner = TarRunner { calls: Arc::new(Mutex::new(Vec::new())) };
        assert!(matches!(
            install_php_static(&paths, "8.4", &dl, &runner),
            Err(PhpStaticError::Unavailable(_))
        ));
        std::fs::remove_dir_all(&root).ok();
    }
}
