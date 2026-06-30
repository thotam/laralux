//! Reaping of *managed* orphan processes left over from a prior session.
//!
//! A "managed" process is one whose running executable lives under
//! `~/laralux/bin`. We identify it via `/proc/<pid>/exe` (the kernel's canonical
//! path to the binary) rather than the cmdline: nginx and php-fpm rewrite their
//! argv (proctitle), but the exe symlink always points at the real file, and
//! php-fpm workers share the master's exe so they match too. All managed
//! processes run as the same user, so the symlink is readable without privilege.

use std::path::Path;
use std::time::{Duration, Instant};

// Minimal libc kill binding (avoids a libc dependency for two calls).
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

fn signal(pid: u32, sig: i32) {
    unsafe {
        libc_kill(pid as i32, sig);
    }
}

/// True when the process is still alive AND holding resources. `kill(pid, 0)`
/// reports a zombie as existing, but a zombie has already released its sockets
/// and locks, so we treat it as dead (the resource we care about is freed).
fn alive(pid: u32) -> bool {
    if unsafe { libc_kill(pid as i32, 0) } != 0 {
        return false; // ESRCH (gone) or not signalable
    }
    !is_zombie(pid)
}

/// Read the process state from `/proc/<pid>/stat` and report whether it is `Z`.
/// The `comm` field may contain spaces/parens, so the state char is the first
/// non-space after the final `)`. A missing stat file means the pid is gone.
fn is_zombie(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(s) => s
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.trim_start().chars().next())
            .map(|c| c == 'Z')
            .unwrap_or(false),
        Err(_) => true,
    }
}

/// True when `exe` (the resolved `/proc/<pid>/exe`) lives at or below `match_dir`.
/// A trailing `" (deleted)"` (kernel marker for an unlinked binary) is stripped
/// first. `Path::starts_with` is component-wise, so `/x/bin` matches
/// `/x/bin/php/8.4/sbin/php-fpm` but NOT `/x/binary`.
pub fn is_managed_exe(exe: &Path, match_dir: &Path) -> bool {
    let s = exe.to_string_lossy();
    let clean: &str = s.strip_suffix(" (deleted)").unwrap_or(&s);
    Path::new(clean).starts_with(match_dir)
}

/// PIDs of running processes whose executable is under `match_dir`, excluding our
/// own PID and any PID in `keep`. Returns an empty Vec if `/proc` is unreadable.
fn scan(match_dir: &Path, keep: &[u32]) -> Vec<u32> {
    let me = std::process::id();
    let canon = std::fs::canonicalize(match_dir).unwrap_or_else(|_| match_dir.to_path_buf());
    let mut out = Vec::new();
    let rd = match std::fs::read_dir("/proc") {
        Ok(r) => r,
        Err(_) => return out,
    };
    for e in rd.flatten() {
        let pid: u32 = match e.file_name().to_str().and_then(|s| s.parse().ok()) {
            Some(p) => p,
            None => continue, // non-numeric /proc entries (cpuinfo, self, …)
        };
        if pid == me || keep.contains(&pid) {
            continue;
        }
        let exe = match std::fs::read_link(format!("/proc/{pid}/exe")) {
            Ok(p) => p,
            Err(_) => continue, // gone, or not ours to read
        };
        if is_managed_exe(&exe, &canon) {
            out.push(pid);
        }
    }
    out
}

/// Kill every managed orphan under `match_dir` (excluding `keep` and ourselves):
/// SIGTERM, wait up to ~2s for graceful exit, then SIGKILL any survivors and wait
/// until all are gone — so the listening socket / lock is released before the
/// caller spawns a replacement. Returns the PIDs it acted on (empty if none).
pub fn reap(match_dir: &Path, keep: &[u32]) -> Vec<u32> {
    let targets = scan(match_dir, keep);
    if targets.is_empty() {
        return targets;
    }
    for &pid in &targets {
        signal(pid, 15); // SIGTERM
    }
    // Graceful window: poll until all dead or the deadline passes.
    let deadline = Instant::now() + Duration::from_millis(2000);
    while Instant::now() < deadline && targets.iter().any(|&p| alive(p)) {
        std::thread::sleep(Duration::from_millis(50));
    }
    // Force-kill stragglers, then wait until they are truly gone.
    for &pid in &targets {
        if alive(pid) {
            signal(pid, 9); // SIGKILL
        }
    }
    let hard = Instant::now() + Duration::from_millis(1000);
    while Instant::now() < hard && targets.iter().any(|&p| alive(p)) {
        std::thread::sleep(Duration::from_millis(20));
    }
    targets
}

/// Test helper: spawn `cmd`, retrying while execve reports ETXTBSY (raw os
/// error 26, "Text file busy"). `cargo test` runs tests in one multi-threaded
/// process; when another thread does fork+exec it transiently inherits the
/// writable fd of a just-`fs::copy`-ed executable, so exec'ing that file races
/// to ETXTBSY. The window is sub-millisecond and clears on its own.
#[cfg(test)]
pub(crate) fn spawn_retrying_etxtbsy(cmd: &mut std::process::Command) -> std::process::Child {
    for _ in 0..200 {
        match cmd.spawn() {
            Ok(child) => return child,
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => panic!("spawn failed: {e}"),
        }
    }
    panic!("spawn still ETXTBSY (os error 26) after retries");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn matches_dir_and_descendants() {
        let base = Path::new("/home/u/laralux/bin");
        assert!(is_managed_exe(Path::new("/home/u/laralux/bin"), base));
        assert!(is_managed_exe(Path::new("/home/u/laralux/bin/nginx/1.31.2/nginx"), base));
        assert!(is_managed_exe(Path::new("/home/u/laralux/bin/php/8.4/sbin/php-fpm"), base));
    }

    #[test]
    fn rejects_siblings_and_outsiders() {
        let base = Path::new("/home/u/laralux/bin");
        // Component-wise: a sibling that merely shares a name prefix must NOT match.
        assert!(!is_managed_exe(Path::new("/home/u/laralux/binary/x"), base));
        assert!(!is_managed_exe(Path::new("/usr/sbin/mariadbd"), base));
    }

    #[test]
    fn php_sub_scope_excludes_other_tools() {
        let php = Path::new("/home/u/laralux/bin/php");
        assert!(is_managed_exe(Path::new("/home/u/laralux/bin/php/8.4/sbin/php-fpm"), php));
        assert!(!is_managed_exe(Path::new("/home/u/laralux/bin/nginx/1.31.2/nginx"), php));
    }

    #[test]
    fn strips_deleted_suffix() {
        let base = Path::new("/home/u/laralux/bin");
        assert!(is_managed_exe(
            Path::new("/home/u/laralux/bin/mariadb/11.4.12/bin/mariadbd (deleted)"),
            base
        ));
    }

    /// Live: a real binary under <tmp>/bin is reaped; one excluded via `keep` survives.
    #[test]
    fn reaps_managed_process_but_keeps_excluded() {
        let sleep = ["/bin/sleep", "/usr/bin/sleep"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists());
        let sleep = match sleep {
            Some(s) => s,
            None => return, // no `sleep` available; skip on such a host
        };

        let root = std::env::temp_dir().join(format!("lara-orphan-{}", std::process::id()));
        let bindir = root.join("bin").join("sleeper");
        std::fs::create_dir_all(&bindir).unwrap();
        let exe = bindir.join("sleep");
        std::fs::copy(&sleep, &exe).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        // Case A: excluded via `keep` → survives.
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("30");
        let mut child = spawn_retrying_etxtbsy(&mut cmd);
        let pid = child.id();
        let acted = reap(&root.join("bin"), &[pid]);
        assert!(!acted.contains(&pid), "kept pid must not be reaped");
        assert!(alive(pid), "kept process must still be running");

        // Case B: not excluded → reaped.
        let acted = reap(&root.join("bin"), &[]);
        assert!(acted.contains(&pid), "managed process should be reaped");
        assert!(!alive(pid), "reaped process must be gone");

        let _ = child.wait();
        std::fs::remove_dir_all(&root).ok();
    }
}
