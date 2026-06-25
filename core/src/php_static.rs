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

pub fn latest_patch_url(version: &str, arch: &str, sapi: &str, listing_json: &str) -> Option<String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(listing_json).ok()?;
    let prefix = format!("php-{version}.");
    let suffix = format!("-{sapi}-linux-{arch}.tar.gz");
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

/// Fetch the `bulk` directory index JSON once.
fn fetch_index(paths: &LaragonPaths, downloader: &dyn Downloader) -> Result<String, PhpStaticError> {
    std::fs::create_dir_all(paths.tmp())?;
    let index = paths.tmp().join("static-php-index.json");
    downloader
        .fetch(&format!("{STATIC_PHP_BASE}/?format=json"), &index)
        .map_err(|e| PhpStaticError::Download(e.to_string()))?;
    Ok(std::fs::read_to_string(&index)?)
}

/// Download one SAPI tarball, extract its single `member` binary, and install
/// it as `~/laragon/bin/<dest_name>` (mode 0755).
fn download_static_php(
    paths: &LaragonPaths,
    version: &str,
    arch: &str,
    sapi: &str,
    member: &str,
    dest_name: &str,
    listing_json: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let url = latest_patch_url(version, arch, sapi, listing_json)
        .ok_or_else(|| PhpStaticError::Unavailable(version.to_string()))?;
    let tarball = paths.tmp().join(format!("php-{version}-{sapi}.tar.gz"));
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
                member.to_string(),
            ],
            None,
        )
        .map_err(|e| PhpStaticError::Extract(e.to_string()))?;
    let extracted = paths.tmp().join(member);
    let dest = paths.bin().join(dest_name);
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

/// Install both the php-fpm and php (cli) static binaries for `version`.
pub fn install_php_static(
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.bin())?;
    let json = fetch_index(paths, downloader)?;
    download_static_php(paths, version, arch, "fpm", "php-fpm", &format!("php-fpm{version}"), &json, downloader, runner)?;
    download_static_php(paths, version, arch, "cli", "php", &format!("php{version}"), &json, downloader, runner)?;
    Ok(())
}

/// Install only the php (cli) static binary as `~/laragon/bin/php<version>`.
pub fn install_php_cli(
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.bin())?;
    let json = fetch_index(paths, downloader)?;
    download_static_php(paths, version, arch, "cli", "php", &format!("php{version}"), &json, downloader, runner)
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
    fn latest_patch_url_picks_highest_patch_for_arch_and_sapi() {
        assert_eq!(
            latest_patch_url("8.4", "x86_64", "fpm", SAMPLE).unwrap(),
            format!("{STATIC_PHP_BASE}/php-8.4.22-fpm-linux-x86_64.tar.gz")
        );
        assert_eq!(
            latest_patch_url("8.4", "x86_64", "cli", SAMPLE).unwrap(),
            format!("{STATIC_PHP_BASE}/php-8.4.22-cli-linux-x86_64.tar.gz")
        );
    }

    #[test]
    fn latest_patch_url_none_for_missing_version_or_arch() {
        assert!(latest_patch_url("7.4", "x86_64", "fpm", SAMPLE).is_none());
        assert!(latest_patch_url("8.4", "riscv64", "fpm", SAMPLE).is_none());
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

    struct TarRunner {
        calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    }
    impl CommandRunner for TarRunner {
        fn run(&self, program: &str, args: &[String], _cwd: Option<&Path>) -> Result<(), ScaffoldError> {
            self.calls.lock().unwrap().push((program.to_string(), args.to_vec()));
            // args: ["-xzf", <tarball>, "-C", <dir>, <member>]
            let dir = &args[3];
            let member = &args[4];
            std::fs::write(Path::new(dir).join(member), b"bin").unwrap();
            Ok(())
        }
    }

    #[test]
    fn install_php_static_installs_fpm_and_cli() {
        let root = std::env::temp_dir().join(format!("lara-spi-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let arch = arch_tag().expect("supported test arch");
        let json = format!(
            "[{{\"name\":\"php-8.4.22-fpm-linux-{arch}.tar.gz\"}},{{\"name\":\"php-8.4.22-cli-linux-{arch}.tar.gz\"}}]"
        );
        let fetched = Arc::new(Mutex::new(Vec::new()));
        let dl = StubDownloader { index_json: json, fetched: fetched.clone() };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let runner = TarRunner { calls: calls.clone() };

        install_php_static(&paths, "8.4", &dl, &runner).unwrap();

        let f = fetched.lock().unwrap();
        assert!(f[0].ends_with("?format=json"), "index fetched first");
        assert!(f.iter().any(|u| u.ends_with(&format!("php-8.4.22-fpm-linux-{arch}.tar.gz"))));
        assert!(f.iter().any(|u| u.ends_with(&format!("php-8.4.22-cli-linux-{arch}.tar.gz"))));
        assert!(paths.bin().join("php-fpm8.4").is_file(), "fpm placed");
        assert!(paths.bin().join("php8.4").is_file(), "cli placed");
        assert_eq!(calls.lock().unwrap().len(), 2, "tar run for both SAPIs");
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
