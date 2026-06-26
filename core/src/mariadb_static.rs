use crate::paths::LaragonPaths;
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

/// Download + extract the MariaDB binary tarball into bin/mariadb/<ver>/ (the basedir)
/// with top-level mariadbd/mariadb-install-db/mariadb symlinks for the resolver.
pub fn install_mariadb(
    paths: &LaragonPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MariadbError> {
    let basedir = paths.version_dir("mariadb", MARIADB_VERSION);
    if basedir.join("mariadbd").exists() {
        let _ = crate::layout::set_current(paths, "mariadb", MARIADB_VERSION);
        return Ok(MARIADB_VERSION.to_string());
    }
    let arch = mariadb_arch().ok_or_else(|| MariadbError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let tgz = paths.tmp().join("mariadb.tar.gz");
    downloader.fetch_with_progress(&mariadb_url(MARIADB_VERSION, arch), &tgz, sink)
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
    crate::layout::set_current(paths, "mariadb", MARIADB_VERSION)?;
    Ok(MARIADB_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_and_arch() {
        assert_eq!(mariadb_url("11.4.12", "x86_64"),
            "https://archive.mariadb.org/mariadb-11.4.12/bintar-linux-systemd-x86_64/mariadb-11.4.12-linux-systemd-x86_64.tar.gz");
        assert_eq!(mariadb_arch(), match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None });
    }
}
