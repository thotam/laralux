use std::path::PathBuf;
use crate::paths::LaraluxPaths;
use crate::privileged::Privileged;

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

/// True iff `getcap` output reports the net-bind capability.
pub fn getcap_indicates_cap(output: &str) -> bool {
    output.contains("cap_net_bind_service")
}

/// Whether the nginx binary already has `cap_net_bind_service`. Uses the
/// unprivileged `getcap`; if `getcap` can't be run, assume present (don't nag).
pub fn nginx_has_bind_cap(nginx_bin: &std::path::Path) -> bool {
    match std::process::Command::new("getcap").arg(nginx_bin).output() {
        Ok(out) => getcap_indicates_cap(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => true,
    }
}

/// Preflight before starting nginx: if the resolved nginx binary lacks the
/// net-bind capability, re-apply it via `setcap` (best-effort; a failure does
/// not block startup). No-op (no prompt) when the capability is already present.
pub fn ensure_nginx_bind_cap(paths: &LaraluxPaths, privileged: &dyn Privileged) {
    if let Some(nginx) = resolve_bin("nginx", &crate::layout::managed_bin_dirs(paths)) {
        if !nginx_has_bind_cap(&nginx) {
            let _ = privileged.setcap_nginx(&nginx);
        }
    }
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
    fn getcap_output_detects_bind_cap() {
        assert!(getcap_indicates_cap("/usr/sbin/nginx cap_net_bind_service=ep"));
        assert!(getcap_indicates_cap("/usr/sbin/nginx cap_net_bind_service+ep"));
        assert!(!getcap_indicates_cap(""));
        assert!(!getcap_indicates_cap("/usr/sbin/nginx cap_chown=ep"));
    }
}
