use crate::config::Config;
use crate::paths::LaraluxPaths;
use std::path::{Path, PathBuf};

/// (Re)point `bin/<tool>/current` at `<version>` (relative target, so it
/// resolves inside the tool dir). Removes any existing `current` first.
pub fn set_current(paths: &LaraluxPaths, tool: &str, version: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(paths.tool_dir(tool))?;
    let link = paths.current_link(tool);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(version, &link)?;
    }
    Ok(())
}

/// Every `bin/<tool>/current` dir that exists — the search path for resolving
/// managed binaries. Sorted for determinism.
pub fn managed_bin_dirs(paths: &LaraluxPaths) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.bin()) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let cur = e.path().join("current");
                if cur.exists() {
                    dirs.push(cur);
                }
            }
        }
    }
    dirs.sort();
    dirs
}

/// Installed version dirs under `bin/<tool>`, excluding the `current` symlink.
/// Sorted by numeric version components.
pub fn installed_versions(paths: &LaraluxPaths, tool: &str) -> Vec<String> {
    let mut versions: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.tool_dir(tool)) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "current" {
                continue;
            }
            if e.path().is_dir() {
                versions.push(name);
            }
        }
    }
    versions.sort_by_key(|v| version_key(v));
    versions
}

/// Numeric version sort key, e.g. "8.3.31" -> [8,3,31]. Non-numeric parts -> 0.
fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Resolve a requested version to an installed one: an exact `version_dir` wins;
/// otherwise the highest installed full version whose major.minor equals
/// `requested` (so a minor like "8.3" maps to e.g. "8.3.31"). None if nothing matches.
pub fn resolve_installed_version(paths: &LaraluxPaths, tool: &str, requested: &str) -> Option<String> {
    if paths.version_dir(tool, requested).is_dir() {
        return Some(requested.to_string());
    }
    let mut best: Option<(Vec<u32>, String)> = None;
    for v in installed_versions(paths, tool) {
        let minor = v.split('.').take(2).collect::<Vec<_>>().join(".");
        if minor == requested {
            let key = version_key(&v);
            if best.as_ref().map_or(true, |(bk, _)| &key > bk) {
                best = Some((key, v));
            }
        }
    }
    best.map(|(_, v)| v)
}

/// Materialize `current` symlinks from config. Returns a warning per tool whose
/// configured version dir is missing. Best-effort: never aborts.
pub fn apply_versions(paths: &LaraluxPaths, config: &Config) -> Vec<String> {
    let mut warnings = Vec::new();
    for (tool, version) in &config.versions {
        if paths.version_dir(tool, version).is_dir() {
            if let Err(e) = set_current(paths, tool, version) {
                warnings.push(format!("{tool}: set_current failed: {e}"));
            }
        } else {
            warnings.push(format!("{tool}: version {version} not installed"));
        }
    }
    warnings
}

/// Run `program args`, capture stdout+stderr, return the first `N.N` or `N.N.N`
/// token found. Used to name mailpit/composer dirs by their real version.
pub fn probe_version(program: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    extract_version(&text)
}

/// First `\d+\.\d+(\.\d+)?` token in `s` (no regex dep — hand-scan).
fn extract_version(s: &str) -> Option<String> {
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || (bytes[i] == '.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())) {
                if bytes[i] == '.' { dots += 1; }
                i += 1;
            }
            if dots >= 1 {
                return Some(bytes[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin::resolve_bin;

    fn root() -> LaraluxPaths {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static C: AtomicUsize = AtomicUsize::new(0);
        let id = C.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("lara-layout-{}-{}", std::process::id(), id));
        let paths = LaraluxPaths::new(p);
        std::fs::create_dir_all(paths.bin()).unwrap();
        paths
    }

    #[test]
    fn set_current_points_and_repoints() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.4.10")).unwrap();
        std::fs::write(paths.version_dir("php", "8.3.31").join("php-fpm"), b"x").unwrap();
        set_current(&paths, "php", "8.3.31").unwrap();
        assert_eq!(std::fs::read_link(paths.current_link("php")).unwrap(), std::path::Path::new("8.3.31"));
        // resolves the binary through the current symlink
        let dirs = managed_bin_dirs(&paths);
        assert!(resolve_bin("php-fpm", &dirs).is_some());
        // repoint
        set_current(&paths, "php", "8.4.10").unwrap();
        assert_eq!(std::fs::read_link(paths.current_link("php")).unwrap(), std::path::Path::new("8.4.10"));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn installed_versions_lists_dirs_excluding_current() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.4.10")).unwrap();
        set_current(&paths, "php", "8.4.10").unwrap();
        assert_eq!(installed_versions(&paths, "php"), vec!["8.3.31".to_string(), "8.4.10".to_string()]);
        assert!(installed_versions(&paths, "nginx").is_empty());
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn managed_bin_dirs_collects_current_dirs() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::create_dir_all(paths.version_dir("coredns", "1.14.4")).unwrap();
        set_current(&paths, "php", "8.3.31").unwrap();
        set_current(&paths, "coredns", "1.14.4").unwrap();
        let dirs = managed_bin_dirs(&paths);
        assert!(dirs.contains(&paths.current_link("php")));
        assert!(dirs.contains(&paths.current_link("coredns")));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn apply_versions_materializes_present_and_warns_missing() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        let mut cfg = crate::config::Config::default();
        cfg.versions.insert("php".into(), "8.3.31".into());
        cfg.versions.insert("nginx".into(), "1.31.2".into()); // dir missing
        let warnings = apply_versions(&paths, &cfg);
        assert_eq!(std::fs::read_link(paths.current_link("php")).unwrap(), std::path::Path::new("8.3.31"));
        assert!(warnings.iter().any(|w| w.contains("nginx")));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn probe_version_extracts_semver() {
        // /bin/echo prints the arg; probe extracts the first semver token.
        assert_eq!(probe_version(std::path::Path::new("/bin/echo"), &["v1.2.3 extra"]), Some("1.2.3".to_string()));
        assert_eq!(probe_version(std::path::Path::new("/bin/echo"), &["no version here"]), None);
    }

    #[test]
    fn resolve_installed_version_maps_minor_to_latest_patch() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.40")).unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.4.10")).unwrap();
        assert_eq!(resolve_installed_version(&paths, "php", "8.3"), Some("8.3.40".to_string())); // latest patch of the minor
        assert_eq!(resolve_installed_version(&paths, "php", "8.4.10"), Some("8.4.10".to_string())); // exact full
        assert_eq!(resolve_installed_version(&paths, "php", "9.0"), None);
        std::fs::remove_dir_all(paths.root()).ok();
    }
}
