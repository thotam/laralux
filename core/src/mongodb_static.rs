use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const MONGODB_VERSION: &str = "8.0.4";

/// Curated MongoDB server versions offered in the Setup/version modal. Both
/// verified present on fastdl.mongodb.org as `ubuntu2204` static tarballs.
pub const KNOWN_MONGODB_VERSIONS: [&str; 2] = ["8.0.4", "7.0.15"];

/// Bundled `mongosh` shell version (Apache-2.0), pinned independently of the
/// server (MongoDB distributes the shell on its own cadence).
pub const MONGOSH_VERSION: &str = "2.3.8";

#[derive(Debug, thiserror::Error)]
pub enum MongodbError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Server tarball arch token.
pub fn mongodb_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
}

/// `mongosh` tarball arch token (different naming than the server).
pub fn mongosh_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x64"), "aarch64" => Some("arm64"), _ => None }
}

pub fn mongodb_url(version: &str, arch: &str) -> String {
    format!("https://fastdl.mongodb.org/linux/mongodb-linux-{arch}-ubuntu2204-{version}.tgz")
}

pub fn mongosh_url(version: &str, arch2: &str) -> String {
    format!("https://downloads.mongodb.com/compass/mongosh-{version}-linux-{arch2}.tgz")
}

/// Make `link` (under basedir) a relative symlink to `target_rel`.
fn rel_symlink(basedir: &std::path::Path, link_name: &str, target_rel: &str) -> std::io::Result<()> {
    let link = basedir.join(link_name);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    { std::os::unix::fs::symlink(target_rel, &link)?; }
    Ok(())
}

/// Download + install the default (pinned) MongoDB version.
pub fn install_mongodb(
    paths: &LaraluxPaths, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MongodbError> {
    install_mongodb_version(paths, MONGODB_VERSION, downloader, runner, sink)
}

/// Download the server tarball + the `mongosh` tarball (both plain gzip),
/// flatten each (`--strip-components=1`) into bin/mongodb/<version>/, and create
/// top-level symlinks to the executables. Idempotent. The server tarball is
/// required; a `mongosh` failure is logged and tolerated.
pub fn install_mongodb_version(
    paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner, sink: &dyn ProgressSink,
) -> Result<String, MongodbError> {
    let basedir = paths.version_dir("mongodb", version);
    if basedir.join("bin").join("mongod").exists() {
        let _ = crate::layout::set_current(paths, "mongodb", version);
        return Ok(version.to_string());
    }
    let arch = mongodb_arch().ok_or_else(|| MongodbError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    let _ = std::fs::remove_dir_all(&basedir);
    std::fs::create_dir_all(&basedir)?;

    // Server tarball (required).
    let server = paths.tmp().join("mongodb.tgz");
    downloader.fetch_with_progress(&mongodb_url(version, arch), &server, sink)
        .map_err(|e| MongodbError::Download(e.to_string()))?;
    runner.run("tar", &[
        "-xzf".into(), server.display().to_string(),
        "--strip-components=1".into(),
        "-C".into(), basedir.display().to_string(),
    ], None).map_err(|e| MongodbError::Extract(e.to_string()))?;
    if !basedir.join("bin").join("mongod").exists() {
        return Err(MongodbError::Extract("mongod binary missing after extract".into()));
    }

    // mongosh shell (best-effort: server is usable without it).
    if let Some(arch2) = mongosh_arch() {
        let shell = paths.tmp().join("mongosh.tgz");
        let ok = downloader.fetch_with_progress(&mongosh_url(MONGOSH_VERSION, arch2), &shell, sink).is_ok()
            && runner.run("tar", &[
                "-xzf".into(), shell.display().to_string(),
                "--strip-components=1".into(),
                "-C".into(), basedir.display().to_string(),
            ], None).is_ok();
        if !ok {
            eprintln!("laralux: mongosh install skipped (download/extract failed); server is usable");
        }
        let _ = std::fs::remove_file(&shell);
    }

    // Top-level symlinks so bin/mongodb/current/<exe> resolves.
    for exe in ["mongod", "mongos", "mongosh"] {
        if basedir.join("bin").join(exe).exists() {
            let _ = rel_symlink(&basedir, exe, &format!("bin/{exe}"));
        }
    }
    let _ = std::fs::remove_file(&server);
    crate::layout::set_current(paths, "mongodb", version)
        .map_err(|e| MongodbError::Extract(e.to_string()))?;
    Ok(version.to_string())
}

pub const MONGODB_CURRENT_URL: &str = "https://downloads.mongodb.org/current.json";

/// Latest release of each maintained line from `current.json`, keeping only
/// those with an `ubuntu2204` build for `arch` (the tarball laralux installs).
pub fn parse_mongodb_feed(json: &str, arch: &str) -> crate::version_feed::Feed {
    let root: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let field = |e: &serde_json::Value, k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let versions: Vec<String> = root
        .get("versions")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|e| {
            e.get("downloads").and_then(|d| d.as_array()).into_iter().flatten()
                .any(|d| field(d, "target") == "ubuntu2204" && field(d, "arch") == arch)
        })
        .map(|e| field(e, "version"))
        .filter(|v| !v.is_empty())
        .collect();
    let lines: Vec<String> = versions.iter().map(|v| crate::version_feed::line_minor(v)).collect();
    crate::version_feed::Feed { allowed_lines: if lines.is_empty() { None } else { Some(lines) }, versions }
}

pub fn mongodb_feed(dl: &dyn Downloader, scratch: &std::path::Path) -> Option<crate::version_feed::Feed> {
    let arch = mongodb_arch()?;
    Some(parse_mongodb_feed(&crate::version_feed::fetch_text(dl, MONGODB_CURRENT_URL, scratch)?, arch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mongodb_feed_reads_current_json() {
        let j = r#"{"versions":[
            {"version":"9.0.4","downloads":[{"target":"ubuntu2204","arch":"x86_64"}]},
            {"version":"8.0.34","downloads":[{"target":"ubuntu2404","arch":"x86_64"},{"target":"ubuntu2204","arch":"x86_64"}]},
            {"version":"7.0.45","downloads":[{"target":"ubuntu2204","arch":"aarch64"}]},
            {"version":"5.0.34","downloads":[{"target":"ubuntu2004","arch":"x86_64"}]}]}"#;
        let f = parse_mongodb_feed(j, "x86_64");
        assert_eq!(f.versions, vec!["9.0.4".to_string(), "8.0.34".to_string()]);
        assert_eq!(f.allowed_lines, Some(vec!["9.0".to_string(), "8.0".to_string()]));
    }

    #[test]
    fn urls_and_arch() {
        assert_eq!(
            mongodb_url("8.0.4", "x86_64"),
            "https://fastdl.mongodb.org/linux/mongodb-linux-x86_64-ubuntu2204-8.0.4.tgz"
        );
        assert_eq!(
            mongodb_url("8.0.4", "aarch64"),
            "https://fastdl.mongodb.org/linux/mongodb-linux-aarch64-ubuntu2204-8.0.4.tgz"
        );
        assert_eq!(
            mongosh_url("2.3.8", "x64"),
            "https://downloads.mongodb.com/compass/mongosh-2.3.8-linux-x64.tgz"
        );
        assert_eq!(
            mongosh_url("2.3.8", "arm64"),
            "https://downloads.mongodb.com/compass/mongosh-2.3.8-linux-arm64.tgz"
        );
        assert_eq!(
            mongodb_arch(),
            match std::env::consts::ARCH { "x86_64" => Some("x86_64"), "aarch64" => Some("aarch64"), _ => None }
        );
        assert_eq!(
            mongosh_arch(),
            match std::env::consts::ARCH { "x86_64" => Some("x64"), "aarch64" => Some("arm64"), _ => None }
        );
    }

    #[test]
    fn known_versions_include_pinned_default() {
        assert!(KNOWN_MONGODB_VERSIONS.contains(&MONGODB_VERSION));
    }
}
