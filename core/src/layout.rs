use crate::paths::LaragonPaths;
use std::path::PathBuf;

/// (Re)point `bin/<tool>/current` at `<version>` (relative target, so it
/// resolves inside the tool dir). Removes any existing `current` first.
pub fn set_current(paths: &LaragonPaths, tool: &str, version: &str) -> std::io::Result<()> {
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
pub fn managed_bin_dirs(paths: &LaragonPaths) -> Vec<PathBuf> {
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
pub fn installed_versions(paths: &LaragonPaths, tool: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin::resolve_bin;

    fn root() -> LaragonPaths {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static C: AtomicUsize = AtomicUsize::new(0);
        let id = C.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("lara-layout-{}-{}", std::process::id(), id));
        let paths = LaragonPaths::new(p);
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
}
