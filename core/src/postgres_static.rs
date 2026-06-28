use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::io::Read;

pub const POSTGRES_VERSION: &str = "16.6.0";

/// Curated PostgreSQL versions offered in the Setup/version modal. All verified
/// present on Maven Central as zonky `embedded-postgres-binaries-linux-<arch>`
/// jars. The datadir is version-sensitive — a major downgrade may refuse to start.
pub const KNOWN_POSTGRES_VERSIONS: [&str; 4] = ["17.2.0", "16.6.0", "15.8.0", "14.13.0"];

#[derive(Debug, thiserror::Error)]
pub enum PostgresError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn postgres_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64v8"), _ => None }
}

pub fn postgres_url(version: &str, arch: &str) -> String {
    format!(
        "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-{arch}/{version}/embedded-postgres-binaries-linux-{arch}-{version}.jar"
    )
}

/// Make `link` (under basedir) a relative symlink to `target_rel`.
fn rel_symlink(basedir: &std::path::Path, link_name: &str, target_rel: &str) -> std::io::Result<()> {
    let link = basedir.join(link_name);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    { std::os::unix::fs::symlink(target_rel, &link)?; }
    Ok(())
}

/// Download + install the default (pinned) PostgreSQL version.
pub fn install_postgres(
    paths: &LaraluxPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, PostgresError> {
    install_postgres_version(paths, POSTGRES_VERSION, downloader, runner, sink)
}

/// Download the zonky jar, extract the inner `*.txz` (pure-Rust zip), untar it
/// (xz) into bin/postgres/<version>/ (yielding bin/, lib/, share/), and create
/// top-level symlinks to the executables. Idempotent.
pub fn install_postgres_version(
    paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, PostgresError> {
    let basedir = paths.version_dir("postgres", version);
    if basedir.join("bin").join("postgres").exists() {
        let _ = crate::layout::set_current(paths, "postgres", version);
        return Ok(version.to_string());
    }
    let arch = postgres_arch().ok_or_else(|| PostgresError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let jar = paths.tmp().join("postgres.jar");
    downloader.fetch_with_progress(&postgres_url(version, arch), &jar, sink)
        .map_err(|e| PostgresError::Download(e.to_string()))?;

    // Extract the single `*.txz` entry from the jar (a zip) — no `unzip` dependency.
    let txz = paths.tmp().join("postgres.txz");
    {
        let f = std::fs::File::open(&jar)?;
        let mut zipf = zip::ZipArchive::new(f).map_err(|e| PostgresError::Extract(e.to_string()))?;
        let name = (0..zipf.len())
            .filter_map(|i| zipf.by_index(i).ok().map(|e| e.name().to_string()))
            .find(|n| n.ends_with(".txz"))
            .ok_or_else(|| PostgresError::Extract("no .txz entry in jar".into()))?;
        let mut entry = zipf.by_name(&name).map_err(|e| PostgresError::Extract(e.to_string()))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&txz, &buf)?;
    }

    // Untar the xz tarball into the version dir (PG layout: bin/, lib/, share/).
    let _ = std::fs::remove_dir_all(&basedir);
    std::fs::create_dir_all(&basedir)?;
    runner.run("tar", &["-xJf".into(), txz.display().to_string(), "-C".into(), basedir.display().to_string()], None)
        .map_err(|e| PostgresError::Extract(e.to_string()))?;
    if !basedir.join("bin").join("postgres").exists() {
        return Err(PostgresError::Extract("postgres binary missing after extract".into()));
    }

    // Top-level symlinks so bin/postgres/current/<exe> resolves (resolver + CLI symlinks).
    for exe in ["postgres", "initdb", "pg_ctl", "psql", "pg_dump", "pg_restore", "createdb", "dropdb"] {
        let _ = rel_symlink(&basedir, exe, &format!("bin/{exe}"));
    }
    let _ = std::fs::remove_file(&jar);
    let _ = std::fs::remove_file(&txz);
    crate::layout::set_current(paths, "postgres", version)
        .map_err(|e| PostgresError::Extract(e.to_string()))?;
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_arch() {
        assert_eq!(
            postgres_url("16.6.0", "amd64"),
            "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-amd64/16.6.0/embedded-postgres-binaries-linux-amd64-16.6.0.jar"
        );
        assert_eq!(
            postgres_url("16.6.0", "arm64v8"),
            "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-arm64v8/16.6.0/embedded-postgres-binaries-linux-arm64v8-16.6.0.jar"
        );
        assert_eq!(
            postgres_arch(),
            match std::env::consts::ARCH { "x86_64" => Some("amd64"), "aarch64" => Some("arm64v8"), _ => None }
        );
    }

    #[test]
    fn known_versions_include_pinned_default() {
        assert!(KNOWN_POSTGRES_VERSIONS.contains(&POSTGRES_VERSION));
    }
}
