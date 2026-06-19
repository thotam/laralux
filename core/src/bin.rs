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

/// Parse "8.4" → (8, 4). Returns None if not exactly major.minor of integers.
fn parse_php_version(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor))
}

/// Find the highest installed `php-fpm<major>.<minor>` and return its version string.
pub fn detect_php_fpm_version(extra_dirs: &[PathBuf]) -> Option<String> {
    let mut dirs: Vec<PathBuf> = extra_dirs.to_vec();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));

    let mut best: Option<(u32, u32, String)> = None;
    for dir in dirs {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(ver) = name.strip_prefix("php-fpm") {
                if let Some((maj, min)) = parse_php_version(ver) {
                    if best.as_ref().map_or(true, |(bmaj, bmin, _)| (maj, min) > (*bmaj, *bmin)) {
                        best = Some((maj, min, ver.to_string()));
                    }
                }
            }
        }
    }
    best.map(|(_, _, ver)| ver)
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

    #[test]
    fn detects_highest_php_fpm_version() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("php-fpm8.3"), "x").unwrap();
        std::fs::write(dir.join("php-fpm8.4"), "x").unwrap();
        std::fs::write(dir.join("php-fpm"), "x").unwrap(); // unversioned: ignored
        assert_eq!(detect_php_fpm_version(&[dir.clone()]), Some("8.4".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_php_fpm_returns_none() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect_php_fpm_version(&[dir.clone()]), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
