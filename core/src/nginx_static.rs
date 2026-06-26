use crate::paths::LaragonPaths;
use crate::progress::ProgressSink;
use crate::setup::Downloader;

pub const NGINX_INDEX_URL: &str = "https://jirutka.github.io/nginx-binaries/index.json";
pub const NGINX_BASE_URL: &str = "https://jirutka.github.io/nginx-binaries";

#[derive(Debug, thiserror::Error)]
pub enum NginxError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("no nginx build for this platform")]
    NoBuild,
    #[error("download failed: {0}")]
    Download(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn nginx_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
}

fn vkey(v: &str) -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() }

/// Highest linux/<arch> nginx entry in the index → (version, filename). Tolerant of malformed entries.
pub fn latest_nginx(arch: &str, index_json: &str) -> Option<(String, String)> {
    // The real index is an object `{ "formatVersion": 2, "contents": [...] }`;
    // accept either that or a bare top-level array (test fixtures / older format).
    let root: serde_json::Value = serde_json::from_str(index_json).ok()?;
    let arr = root
        .as_array()
        .or_else(|| root.get("contents").and_then(|c| c.as_array()))?;
    let mut best: Option<(Vec<u32>, String, String)> = None;
    for e in arr {
        let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let os = e.get("os").and_then(|v| v.as_str()).unwrap_or("");
        let a = e.get("arch").and_then(|v| v.as_str()).unwrap_or("");
        if name != "nginx" || os != "linux" || a != arch { continue; }
        let ver = match e.get("version").and_then(|v| v.as_str()) { Some(v) => v, None => continue };
        let file = match e.get("filename").and_then(|v| v.as_str()) { Some(f) => f, None => continue };
        let key = vkey(ver);
        if best.as_ref().map_or(true, |(bk, _, _)| &key > bk) {
            best = Some((key, ver.to_string(), file.to_string()));
        }
    }
    best.map(|(_, v, f)| (v, f))
}

fn installed(dest: &std::path::Path) -> bool {
    std::fs::metadata(dest).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Download the static nginx binary (latest from the index) into bin/nginx/<ver>/nginx.
pub fn install_nginx(
    paths: &LaragonPaths, downloader: &dyn Downloader, sink: &dyn ProgressSink,
) -> Result<String, NginxError> {
    let arch = nginx_arch().ok_or_else(|| NginxError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let idx = paths.tmp().join("nginx-index.json");
    downloader.fetch(NGINX_INDEX_URL, &idx).map_err(|e| NginxError::Download(e.to_string()))?;
    let json = std::fs::read_to_string(&idx)?;
    let (ver, filename) = latest_nginx(arch, &json).ok_or(NginxError::NoBuild)?;
    let dir = paths.version_dir("nginx", &ver);
    let dest = dir.join("nginx");
    if installed(&dest) {
        let _ = crate::layout::set_current(paths, "nginx", &ver);
        return Ok(ver);
    }
    std::fs::create_dir_all(&dir)?;
    let tmp = paths.tmp().join("nginx.download");
    let _ = std::fs::remove_file(&tmp);
    downloader.fetch_with_progress(&format!("{NGINX_BASE_URL}/{filename}"), &tmp, sink)
        .map_err(|e| NginxError::Download(e.to_string()))?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?; }
    std::fs::rename(&tmp, &dest).or_else(|_| {
        std::fs::copy(&tmp, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&tmp))
    })?;
    crate::layout::set_current(paths, "nginx", &ver)?;
    Ok(ver)
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDX: &str = r#"[
      {"name":"nginx","version":"1.24.0","arch":"x86_64","os":"linux","filename":"nginx-1.24.0-x86_64-linux"},
      {"name":"nginx","version":"1.31.2","arch":"x86_64","os":"linux","filename":"nginx-1.31.2-x86_64-linux"},
      {"name":"nginx","version":"1.31.2","arch":"aarch64","os":"linux","filename":"nginx-1.31.2-aarch64-linux"},
      {"name":"nginx","version":"1.9.0","arch":"x86_64","os":"linux","filename":"nginx-1.9.0-x86_64-linux"},
      {"name":"njs","version":"9.9.9","arch":"x86_64","os":"linux","filename":"x"}
    ]"#;
    #[test]
    fn picks_highest_linux_x86_64() {
        let (v, f) = latest_nginx("x86_64", IDX).unwrap();
        assert_eq!(v, "1.31.2"); // numeric: 1.31.2 > 1.24.0 > 1.9.0
        assert_eq!(f, "nginx-1.31.2-x86_64-linux");
        assert!(latest_nginx("riscv64", IDX).is_none());
    }
    #[test]
    fn arch_maps() {
        assert_eq!(nginx_arch(), match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None });
    }
    #[test]
    fn parses_real_contents_wrapped_index() {
        // The live index is `{ "formatVersion": 2, "contents": [...] }`, not a bare array.
        let wrapped = r#"{"formatVersion":2,"contents":[
          {"name":"nginx","version":"1.24.0","arch":"x86_64","os":"linux","filename":"nginx-1.24.0-x86_64-linux"},
          {"name":"nginx","version":"1.31.2","arch":"x86_64","os":"linux","filename":"nginx-1.31.2-x86_64-linux"}
        ]}"#;
        let (v, f) = latest_nginx("x86_64", wrapped).unwrap();
        assert_eq!(v, "1.31.2");
        assert_eq!(f, "nginx-1.31.2-x86_64-linux");
    }

    use crate::setup::{Downloader, SetupError};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    struct StubNginxDownloader {
        index_json: String,
        fetched: Arc<Mutex<Vec<String>>>,
    }
    impl Downloader for StubNginxDownloader {
        fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError> {
            self.fetched.lock().unwrap().push(url.to_string());
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(SetupError::Io)?;
            }
            if url.contains("index.json") {
                std::fs::write(dest, self.index_json.as_bytes()).map_err(SetupError::Io)?;
            } else {
                std::fs::write(dest, b"ELF-fake").map_err(SetupError::Io)?;
            }
            Ok(())
        }
        fn fetch_with_progress(&self, url: &str, dest: &Path, _sink: &dyn crate::progress::ProgressSink) -> Result<(), SetupError> {
            self.fetched.lock().unwrap().push(url.to_string());
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(SetupError::Io)?;
            }
            std::fs::write(dest, b"ELF-fake").map_err(SetupError::Io)?;
            Ok(())
        }
    }

    #[test]
    fn install_nginx_downloads_to_versioned_dir() {
        let root = std::env::temp_dir().join(format!("lara-nginx-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        let arch = nginx_arch().expect("supported test arch");
        let index_json = format!(
            r#"[{{"name":"nginx","version":"1.31.2","arch":"{arch}","os":"linux","filename":"nginx-1.31.2-{arch}-linux"}}]"#
        );
        let dl = StubNginxDownloader { index_json, fetched: Arc::new(Mutex::new(Vec::new())) };
        let _ = install_nginx(&paths, &dl, &crate::progress::NullProgress);
        let dest = paths.version_dir("nginx", "1.31.2").join("nginx");
        assert!(dest.exists(), "nginx binary should be at {dest:?}");
        let link = paths.current_link("nginx");
        assert!(link.exists(), "current symlink should exist");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_nginx_skips_if_already_installed() {
        let root = std::env::temp_dir().join(format!("lara-nginx-skip-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        let arch = nginx_arch().expect("supported test arch");
        let index_json = format!(
            r#"[{{"name":"nginx","version":"1.31.2","arch":"{arch}","os":"linux","filename":"nginx-1.31.2-{arch}-linux"}}]"#
        );
        // Pre-install the binary
        let dir = paths.version_dir("nginx", "1.31.2");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("nginx"), b"ELF-fake").unwrap();
        let fetched = Arc::new(Mutex::new(Vec::new()));
        let dl = StubNginxDownloader { index_json, fetched: fetched.clone() };
        let ver = install_nginx(&paths, &dl, &crate::progress::NullProgress).unwrap();
        assert_eq!(ver, "1.31.2");
        // Only the index should have been fetched, not the binary
        let reqs = fetched.lock().unwrap();
        assert!(reqs.iter().all(|u| u.contains("index.json")), "should not re-download binary if already installed");
        std::fs::remove_dir_all(&root).ok();
    }
}
