//! Version catalogs built from each tool's upstream release feed.

use crate::paths::LaraluxPaths;
use crate::setup::Downloader;
use crate::tools::ManagedTool;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PATCHES_PER_LINE: usize = 3;
/// Lines shown when the source does not say which ones are supported.
pub const DEFAULT_LINES: usize = 4;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Feed {
    pub versions: Vec<String>,
    /// Release lines the source marks as supported; `None` means "newest N".
    pub allowed_lines: Option<Vec<String>>,
}

pub enum Lines {
    Latest(usize),
    Allowed(Vec<String>),
}

/// Only plain numeric versions (`2.10.3`, `8.4`): drops RCs, tags and hashes.
pub fn is_stable(v: &str) -> bool {
    !v.is_empty() && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

pub fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn line_major(v: &str) -> String {
    v.split('.').next().unwrap_or("").to_string()
}

pub fn line_minor(v: &str) -> String {
    v.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// Newest `patches` versions of each selected line, newest first.
pub fn select(versions: &[String], line_of: fn(&str) -> String, lines: &Lines, patches: usize) -> Vec<String> {
    let mut stable: Vec<&String> = versions.iter().filter(|v| is_stable(v)).collect();
    stable.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    stable.dedup();

    let mut line_keys: Vec<String> = Vec::new();
    for v in &stable {
        let l = line_of(v);
        if !line_keys.contains(&l) {
            line_keys.push(l);
        }
    }
    let chosen: Vec<String> = match lines {
        Lines::Latest(n) => line_keys.into_iter().take(*n).collect(),
        Lines::Allowed(allowed) => line_keys.into_iter().filter(|l| allowed.contains(l)).collect(),
    };

    chosen
        .iter()
        .flat_map(|l| stable.iter().filter(move |v| &line_of(v) == l).take(patches).map(|v| v.to_string()))
        .collect()
}

const FEED_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Where a tool's versions come from.
pub struct Source {
    /// Offline fallback, also merged into the feed.
    pub fallback: &'static [&'static str],
    pub line_of: fn(&str) -> String,
    /// Download into the given scratch dir and parse; `None` on any failure or empty result.
    pub fetch: fn(&dyn Downloader, &Path) -> Option<Feed>,
}

pub fn source(tool: ManagedTool) -> Option<Source> {
    match tool {
        ManagedTool::Composer => Some(Source {
            fallback: &crate::php_cli::KNOWN_COMPOSER_VERSIONS,
            line_of: line_minor,
            fetch: crate::php_cli::composer_feed,
        }),
        ManagedTool::Node => Some(Source {
            fallback: &crate::node_static::KNOWN_NODE_VERSIONS,
            line_of: line_major,
            fetch: crate::node_static::node_feed,
        }),
        ManagedTool::Nginx => Some(Source {
            fallback: &crate::nginx_static::KNOWN_NGINX_VERSIONS,
            line_of: line_minor,
            fetch: crate::nginx_static::nginx_feed,
        }),
        ManagedTool::Postgres => Some(Source {
            fallback: &crate::postgres_static::KNOWN_POSTGRES_VERSIONS,
            line_of: line_major,
            fetch: crate::postgres_static::postgres_feed,
        }),
        _ => None,
    }
}

pub fn cache_path(paths: &LaraluxPaths, tool: ManagedTool) -> PathBuf {
    paths.tmp().join("feeds").join(format!("{}.json", crate::tools::info(tool).key))
}

/// Download `url` into `scratch` and return its body.
pub fn fetch_text(dl: &dyn Downloader, url: &str, scratch: &Path) -> Option<String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let file = scratch.join(format!("fetch-{}.part", SEQ.fetch_add(1, Ordering::SeqCst)));
    let text = dl.fetch_feed(url, &file).ok().and_then(|_| std::fs::read_to_string(&file).ok());
    let _ = std::fs::remove_file(&file);
    text
}

/// Refresh the tool's cached feed when missing or older than 24h. Best effort:
/// any failure keeps the previous cache.
pub fn refresh(paths: &LaraluxPaths, dl: &dyn Downloader, tool: ManagedTool) {
    let Some(src) = source(tool) else { return };
    let cache = cache_path(paths, tool);
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < FEED_TTL);
    let dir = cache.parent().unwrap_or(Path::new("."));
    if fresh || std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Some(feed) = (src.fetch)(dl, dir).filter(|f| !f.versions.is_empty()) else { return };
    let Ok(json) = serde_json::to_string(&feed) else { return };
    let part = cache.with_extension("json.part");
    if std::fs::write(&part, json).is_ok() && std::fs::rename(&part, &cache).is_err() {
        let _ = std::fs::remove_file(&part);
    }
}

pub fn cached_feed(paths: &LaraluxPaths, tool: ManagedTool) -> Option<Feed> {
    let text = std::fs::read_to_string(cache_path(paths, tool)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Versions to offer for `tool`: cached feed merged with the fallback list,
/// reduced to the newest patches of the selected lines.
pub fn catalog_versions(paths: &LaraluxPaths, tool: ManagedTool) -> Vec<String> {
    let Some(src) = source(tool) else { return Vec::new() };
    let mut versions: Vec<String> = src.fallback.iter().map(|v| v.to_string()).collect();
    let mut lines = Lines::Latest(DEFAULT_LINES);
    if let Some(feed) = cached_feed(paths, tool) {
        versions.extend(feed.versions);
        if let Some(allowed) = feed.allowed_lines {
            lines = Lines::Allowed(allowed);
        }
    }
    select(&versions, src.line_of, &lines, PATCHES_PER_LINE)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::paths::LaraluxPaths;
    use crate::setup::{Downloader, SetupError};
    use crate::tools::ManagedTool;
    use std::path::Path;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn root() -> LaraluxPaths {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-feed-{}-{}", std::process::id(), id)));
        paths.ensure_dirs().unwrap();
        paths
    }

    struct StubDownloader(&'static str);
    impl Downloader for StubDownloader {
        fn fetch(&self, _url: &str, dest: &Path) -> Result<(), SetupError> {
            std::fs::write(dest, self.0)?;
            Ok(())
        }
    }

    const COMPOSER_JSON: &str = r#"{"stable": [{"version": "2.11.0"}, {"version": "2.2.31"}],
        "preview": [{"version": "2.12.0-RC1"}], "snapshot": [{"version": "abc123"}]}"#;

    fn age_cache(paths: &LaraluxPaths, tool: ManagedTool) {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(25 * 60 * 60);
        std::fs::File::options().write(true).open(cache_path(paths, tool)).unwrap().set_modified(old).unwrap();
    }

    #[test]
    fn refresh_writes_cache_and_skips_while_fresh() {
        let paths = root();
        refresh(&paths, &StubDownloader(COMPOSER_JSON), ManagedTool::Composer);
        assert!(cached_feed(&paths, ManagedTool::Composer).unwrap().versions.contains(&"2.11.0".to_string()));
        let dl = crate::setup::FakeDownloader::new();
        refresh(&paths, &dl, ManagedTool::Composer);
        assert!(dl.requested().lock().unwrap().is_empty(), "fresh cache must not refetch");
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn refresh_keeps_old_cache_on_bad_feed() {
        let paths = root();
        refresh(&paths, &StubDownloader(COMPOSER_JSON), ManagedTool::Composer);
        age_cache(&paths, ManagedTool::Composer);
        refresh(&paths, &StubDownloader("garbage"), ManagedTool::Composer);
        assert!(cached_feed(&paths, ManagedTool::Composer).unwrap().versions.contains(&"2.11.0".to_string()));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn corrupt_cache_falls_back_to_known() {
        let paths = root();
        let cache = cache_path(&paths, ManagedTool::Composer);
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, "{\"versions\":[").unwrap();
        let known: Vec<String> = crate::php_cli::KNOWN_COMPOSER_VERSIONS.iter().map(|v| v.to_string()).collect();
        assert_eq!(
            catalog_versions(&paths, ManagedTool::Composer),
            select(&known, line_minor, &Lines::Latest(DEFAULT_LINES), PATCHES_PER_LINE)
        );
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn catalog_uses_allowed_lines_from_feed() {
        let paths = root();
        refresh(&paths, &StubDownloader(COMPOSER_JSON), ManagedTool::Composer);
        let vs = catalog_versions(&paths, ManagedTool::Composer);
        assert!(vs.contains(&"2.11.0".to_string()) && vs.contains(&"2.2.31".to_string()));
        assert!(vs.contains(&"2.2.30".to_string()), "known patches on an allowed line stay");
        assert!(!vs.contains(&"2.9.7".to_string()) && !vs.contains(&"2.8.12".to_string()));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn select_keeps_three_patches_of_four_newest_lines() {
        let vs = s(&["1.31.6", "1.31.5", "1.31.4", "1.31.3", "1.30.5", "1.29.8", "1.28.3", "1.27.5"]);
        assert_eq!(
            select(&vs, line_minor, &Lines::Latest(4), 3),
            s(&["1.31.6", "1.31.5", "1.31.4", "1.30.5", "1.29.8", "1.28.3"])
        );
    }

    #[test]
    fn select_allowed_lines_and_drops_prerelease_and_dupes() {
        let vs = s(&["13.1.0-rc", "12.3.3", "12.3.3", "11.8.6", "10.5.29", "abc123"]);
        let lines = Lines::Allowed(s(&["12.3", "11.8"]));
        assert_eq!(select(&vs, line_minor, &lines, 3), s(&["12.3.3", "11.8.6"]));
    }

    #[test]
    fn select_sorts_numerically_not_lexically() {
        assert_eq!(select(&s(&["9.0.0", "10.0.0"]), line_major, &Lines::Latest(4), 3), s(&["10.0.0", "9.0.0"]));
    }

    #[test]
    fn is_stable_rejects_non_numeric_parts() {
        assert!(is_stable("2.10.3") && is_stable("8.4"));
        assert!(!is_stable("2.12.0-RC1") && !is_stable("v1.4.4") && !is_stable("") && !is_stable("1..2"));
    }
}
