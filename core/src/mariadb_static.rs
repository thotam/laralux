use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const MARIADB_VERSION: &str = "11.4.12";

#[derive(Debug, thiserror::Error)]
pub enum MariadbError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("layout error: {0}")]
    Layout(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn mariadb_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
}

pub fn mariadb_url(version: &str, arch: &str) -> String {
    format!("https://archive.mariadb.org/mariadb-{version}/bintar-linux-systemd-{arch}/mariadb-{version}-linux-systemd-{arch}.tar.gz")
}

/// Find a file/symlink-target named `name` anywhere under `root` (DFS); returns its path.
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

/// Make `link` (under basedir) a relative symlink to `target_rel` (a path relative to basedir).
fn rel_symlink(basedir: &std::path::Path, link_name: &str, target_rel: &str) -> std::io::Result<()> {
    let link = basedir.join(link_name);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    { std::os::unix::fs::symlink(target_rel, &link)?; }
    Ok(())
}

/// Curated MariaDB versions offered in the Setup modal (latest stable + LTS
/// lines). All verified present on archive.mariadb.org as
/// `bintar-linux-systemd-<arch>` tarballs. Note: the data directory is shared
/// across versions and is version-sensitive — switching to a version OLDER than
/// the one that created the datadir (a major downgrade) may refuse to start.
pub const KNOWN_MARIADB_VERSIONS: [&str; 4] = ["11.8.2", "11.4.12", "10.11.10", "10.6.20"];

/// Download + extract the default (pinned) MariaDB version.
pub fn install_mariadb(
    paths: &LaraluxPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MariadbError> {
    install_mariadb_version(paths, MARIADB_VERSION, downloader, runner, sink)
}

/// Download + extract a SPECIFIC MariaDB binary tarball into bin/mariadb/<version>/
/// (the basedir) with top-level mariadbd/mariadb-install-db/mariadb symlinks for
/// the resolver. Idempotent. An unknown version surfaces as `MariadbError::Download`.
pub fn install_mariadb_version(
    paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MariadbError> {
    let basedir = paths.version_dir("mariadb", version);
    // Require BOTH the server and the init tool — these are the last symlinks
    // created, so a half-finished prior install (process died mid-symlink) is
    // NOT treated as complete and is re-done rather than left broken.
    if basedir.join("mariadbd").exists() && basedir.join("mariadb-install-db").exists() {
        let _ = crate::layout::set_current(paths, "mariadb", version);
        return Ok(version.to_string());
    }
    let arch = mariadb_arch().ok_or_else(|| MariadbError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let tgz = paths.tmp().join("mariadb.tar.gz");
    downloader.fetch_with_progress(&mariadb_url(version, arch), &tgz, sink)
        .map_err(|e| MariadbError::Download(e.to_string()))?;
    let xdir = paths.tmp().join("mariadb-extract");
    let _ = std::fs::remove_dir_all(&xdir);
    std::fs::create_dir_all(&xdir)?;
    runner.run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), xdir.display().to_string()], None)
        .map_err(|e| MariadbError::Extract(e.to_string()))?;
    // The tarball nests under a single dir `mariadb-<ver>-...`; move it to basedir.
    let top = std::fs::read_dir(&xdir)?.flatten()
        .map(|e| e.path()).find(|p| p.is_dir())
        .ok_or_else(|| MariadbError::Extract("empty archive".into()))?;
    let _ = std::fs::remove_dir_all(&basedir);
    std::fs::create_dir_all(basedir.parent().unwrap())?;
    std::fs::rename(&top, &basedir).or_else(|_| {
        // cross-device: recursive copy then remove (best-effort via `cp -a`)
        runner.run("cp", &["-a".into(), top.display().to_string(), basedir.display().to_string()], None)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            .and_then(|_| std::fs::remove_dir_all(&top))
    })?;
    // Resolve the real binaries inside the basedir and symlink them at the top level.
    let mariadbd = find_under(&basedir, "mariadbd").ok_or_else(|| MariadbError::Layout("mariadbd not found".into()))?;
    let mariadbd_rel = mariadbd.strip_prefix(&basedir).map(|p| p.display().to_string()).unwrap_or_else(|_| "bin/mariadbd".into());
    rel_symlink(&basedir, "mariadbd", &mariadbd_rel)?;
    if let Some(idb) = find_under(&basedir, "mariadb-install-db") {
        let rel = idb.strip_prefix(&basedir).map(|p| p.display().to_string()).unwrap_or_else(|_| "scripts/mariadb-install-db".into());
        let _ = rel_symlink(&basedir, "mariadb-install-db", &rel);
    }
    if let Some(cli) = find_under(&basedir, "mariadb") {
        // skip if it's the basedir-relative "mariadb" we'd be creating; only the bin/ client
        if cli != basedir.join("mariadb") {
            let rel = cli.strip_prefix(&basedir).map(|p| p.display().to_string()).unwrap_or_else(|_| "bin/mariadb".into());
            let _ = rel_symlink(&basedir, "mariadb", &rel);
        }
    }
    // `mysql` is MariaDB's compat alias for the client; expose it top-level too so
    // it can be symlinked into /usr/local/bin alongside `mariadb`.
    if let Some(cli) = find_under(&basedir, "mysql") {
        if cli != basedir.join("mysql") {
            let rel = cli.strip_prefix(&basedir).map(|p| p.display().to_string()).unwrap_or_else(|_| "bin/mysql".into());
            let _ = rel_symlink(&basedir, "mysql", &rel);
        }
    }
    crate::layout::set_current(paths, "mariadb", version)?;
    Ok(version.to_string())
}

pub const MARIADB_RELEASES_API: &str = "https://downloads.mariadb.org/rest-api/mariadb/";

/// Release lines the MariaDB REST API marks `Stable` (LTS and rolling).
pub fn parse_mariadb_lines(json: &str) -> Vec<String> {
    let root: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    root.get("major_releases")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
        .filter(|r| r.get("release_status").and_then(|s| s.as_str()) == Some("Stable"))
        .filter_map(|r| r.get("release_id").and_then(|s| s.as_str()).map(str::to_string))
        .collect()
}

/// Patch versions of one line (`.../rest-api/mariadb/<line>/`).
pub fn parse_mariadb_releases(json: &str) -> Vec<String> {
    let root: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    root.get("releases").and_then(|r| r.as_object()).map(|o| o.keys().cloned().collect()).unwrap_or_default()
}

/// One request for the stable lines, then one per line; a failed line is skipped.
pub fn mariadb_feed(dl: &dyn Downloader, scratch: &std::path::Path) -> Option<crate::version_feed::Feed> {
    use crate::version_feed::fetch_text;
    let lines = parse_mariadb_lines(&fetch_text(dl, MARIADB_RELEASES_API, scratch)?);
    if lines.is_empty() {
        return None;
    }
    let versions = lines
        .iter()
        .filter_map(|l| fetch_text(dl, &format!("{MARIADB_RELEASES_API}{l}/"), scratch))
        .flat_map(|j| parse_mariadb_releases(&j))
        .collect();
    Some(crate::version_feed::Feed { versions, allowed_lines: Some(lines) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mariadb_lines_keep_only_stable() {
        let j = r#"{"major_releases":[{"release_id":"13.2","release_status":"Preview"},
            {"release_id":"13.1","release_status":"RC"},{"release_id":"12.3","release_status":"Stable"}]}"#;
        assert_eq!(parse_mariadb_lines(j), vec!["12.3".to_string()]);
    }

    struct RouteDownloader;
    impl Downloader for RouteDownloader {
        fn fetch(&self, url: &str, dest: &std::path::Path) -> Result<(), crate::setup::SetupError> {
            let body = if url.ends_with("/mariadb/") {
                r#"{"major_releases":[{"release_id":"12.3","release_status":"Stable"},
                    {"release_id":"11.8","release_status":"Stable"}]}"#
            } else if url.ends_with("/12.3/") {
                r#"{"releases":{"12.3.3":{},"12.3.2":{}}}"#
            } else {
                return Err(crate::setup::SetupError::Download("boom".into()));
            };
            std::fs::write(dest, body)?;
            Ok(())
        }
    }

    #[test]
    fn mariadb_feed_merges_lines_and_skips_failed_line() {
        let scratch = std::env::temp_dir().join(format!("lara-mdb-feed-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let mut f = mariadb_feed(&RouteDownloader, &scratch).unwrap();
        f.versions.sort();
        assert_eq!(f.versions, vec!["12.3.2".to_string(), "12.3.3".to_string()]);
        assert_eq!(f.allowed_lines, Some(vec!["12.3".to_string(), "11.8".to_string()]));
        std::fs::remove_dir_all(&scratch).ok();
    }
    #[test]
    fn url_and_arch() {
        assert_eq!(mariadb_url("11.4.12", "x86_64"),
            "https://archive.mariadb.org/mariadb-11.4.12/bintar-linux-systemd-x86_64/mariadb-11.4.12-linux-systemd-x86_64.tar.gz");
        assert_eq!(mariadb_arch(), match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None });
    }

    #[test]
    fn known_versions_include_pinned_default_and_build_urls() {
        assert!(KNOWN_MARIADB_VERSIONS.contains(&MARIADB_VERSION));
        // A non-default catalog version builds the expected systemd-bintar URL.
        assert_eq!(
            mariadb_url("10.11.10", "x86_64"),
            "https://archive.mariadb.org/mariadb-10.11.10/bintar-linux-systemd-x86_64/mariadb-10.11.10-linux-systemd-x86_64.tar.gz"
        );
    }
}
