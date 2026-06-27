use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::Path;

/// Default Node version installed during Setup (current active LTS line).
pub const NODE_VERSION: &str = "24.18.0";

/// Curated Node.js versions offered in the Setup modal: the latest patch of each
/// maintained LTS line. All permanently available on nodejs.org/dist as
/// `node-v<version>-linux-<arch>.tar.xz` tarballs (newest first).
pub const KNOWN_NODE_VERSIONS: [&str; 4] = ["24.18.0", "22.23.1", "20.20.2", "18.20.8"];

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Map the host arch to Node's release naming (`x64`/`arm64`).
pub fn node_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

pub fn node_url(version: &str, arch: &str) -> String {
    format!("https://nodejs.org/dist/v{version}/node-v{version}-linux-{arch}.tar.xz")
}

fn installed(node_bin: &Path) -> bool {
    std::fs::metadata(node_bin).map(|m| m.len() > 0).unwrap_or(false)
}

/// Expose `node`/`npm`/`npx` at the version-dir root as relative symlinks into
/// `bin/`, so `bin/node/current/node` resolves to the real binary while it keeps
/// its sibling `lib/` (npm needs the tree). Best-effort; missing targets skipped.
fn make_root_links(dir: &Path) {
    for name in ["node", "npm", "npx"] {
        if !dir.join("bin").join(name).exists() {
            continue;
        }
        let link = dir.join(name);
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(format!("bin/{name}"), &link);
        }
    }
}

/// Download + install the default (pinned) Node version.
pub fn install_node(
    paths: &LaraluxPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, NodeError> {
    install_node_version(paths, NODE_VERSION, downloader, runner, sink)
}

/// Download a SPECIFIC Node version, extract the official tarball tree directly
/// into `bin/node/<version>/` (preserving `bin/` + `lib/` so node/npm/npx work),
/// then add root convenience symlinks and point `current` at it. Idempotent;
/// an unknown version surfaces as `NodeError::Download`.
pub fn install_node_version(
    paths: &LaraluxPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, NodeError> {
    let dir = paths.version_dir("node", version);
    let node_bin = dir.join("bin").join("node");
    if installed(&node_bin) {
        let _ = crate::layout::set_current(paths, "node", version);
        return Ok(version.to_string());
    }
    let arch = node_arch().ok_or_else(|| NodeError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(&dir)?;
    let txz = paths.tmp().join("node.tar.xz");
    downloader
        .fetch_with_progress(&node_url(version, arch), &txz, sink)
        .map_err(|e| NodeError::Download(e.to_string()))?;
    // `--strip-components=1` drops the top-level `node-v.../` dir so the tree lands
    // directly under `dir` (-> dir/bin/node, dir/lib/...).
    runner
        .run(
            "tar",
            &[
                "-xJf".into(),
                txz.display().to_string(),
                "-C".into(),
                dir.display().to_string(),
                "--strip-components=1".into(),
            ],
            None,
        )
        .map_err(|e| NodeError::Extract(e.to_string()))?;
    if !installed(&node_bin) {
        return Err(NodeError::Extract("node binary not found in archive".into()));
    }
    make_root_links(&dir);
    crate::layout::set_current(paths, "node", version)?;
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_arch() {
        assert_eq!(
            node_url("24.18.0", "x64"),
            "https://nodejs.org/dist/v24.18.0/node-v24.18.0-linux-x64.tar.xz"
        );
        assert_eq!(
            node_arch(),
            match std::env::consts::ARCH {
                "x86_64" => Some("x64"),
                "aarch64" => Some("arm64"),
                _ => None,
            }
        );
    }

    #[test]
    fn known_versions_include_pinned_default() {
        assert!(KNOWN_NODE_VERSIONS.contains(&NODE_VERSION));
        assert_eq!(
            node_url("20.20.2", "arm64"),
            "https://nodejs.org/dist/v20.20.2/node-v20.20.2-linux-arm64.tar.xz"
        );
    }

    #[test]
    fn root_links_point_into_bin() {
        let dir = std::env::temp_dir().join(format!("lara-node-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("node"), b"x").unwrap();
        std::fs::write(dir.join("bin").join("npm"), b"x").unwrap();
        // npx intentionally absent: it must be skipped, not linked.
        make_root_links(&dir);
        #[cfg(unix)]
        {
            assert_eq!(std::fs::read_link(dir.join("node")).unwrap(), std::path::Path::new("bin/node"));
            assert_eq!(std::fs::read_link(dir.join("npm")).unwrap(), std::path::Path::new("bin/npm"));
            assert!(!dir.join("npx").exists());
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
