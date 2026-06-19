use std::path::PathBuf;

const FALLBACK_DIRS: [&str; 6] = [
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

/// Resolve a program name to an absolute path.
/// A name containing '/' is treated as a path and returned only if it is a file.
/// Otherwise searches: extra_dirs, then $PATH, then common system bin dirs.
pub fn resolve_bin(name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return if p.is_file() { Some(p) } else { None };
    }
    let mut dirs: Vec<PathBuf> = extra_dirs.to_vec();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolved absolute path as a string if found, else the bare name
/// (so PATH lookup still applies at spawn time).
pub fn resolve_or_name(name: &str, extra_dirs: &[PathBuf]) -> String {
    resolve_bin(name, extra_dirs)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static C: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!("lara-bin-{}-{}", std::process::id(), C.fetch_add(1, Ordering::SeqCst)))
    }

    #[test]
    fn resolves_a_binary_in_an_extra_dir() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("mybin");
        std::fs::write(&exe, "x").unwrap();
        assert_eq!(resolve_bin("mybin", &[dir.clone()]), Some(exe));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_binary_resolves_to_none_and_bare_name() {
        let nonexistent = "definitely-not-a-real-binary-xyz";
        assert_eq!(resolve_bin(nonexistent, &[]), None);
        assert_eq!(resolve_or_name(nonexistent, &[]), nonexistent.to_string());
    }

    #[test]
    fn path_with_slash_is_used_as_is() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("tool");
        std::fs::write(&exe, "x").unwrap();
        let abs = exe.display().to_string();
        assert_eq!(resolve_bin(&abs, &[]), Some(exe));
        assert_eq!(resolve_bin("/no/such/tool/here", &[]), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
