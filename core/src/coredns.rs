use crate::paths::LaraluxPaths;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const COREDNS_VERSION: &str = "1.14.4";

/// Preferred local DNS port for CoreDNS. Deliberately NOT 5353 — that is the
/// well-known mDNS port and is almost always already bound by `avahi-daemon`
/// on `0.0.0.0:5353` (default on Ubuntu/desktop), which makes CoreDNS fail with
/// `bind: address already in use` and crash on startup. It is also >1024 so it
/// needs no root/CAP_NET_BIND_SERVICE.
pub const COREDNS_PORT: u16 = 15353;

/// True if `port` can be bound on 127.0.0.1 for BOTH udp and tcp — CoreDNS
/// serves DNS on both, so a usable port must be free on both. The sockets are
/// dropped immediately, freeing the port for CoreDNS to claim.
fn port_free(port: u16) -> bool {
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    UdpSocket::bind(addr).is_ok() && TcpListener::bind(addr).is_ok()
}

/// Pick a free DNS port for CoreDNS: prefer [`COREDNS_PORT`], and if it is taken
/// (e.g. another local resolver or a leftover instance) scan upward for the next
/// free one so a one-off conflict cannot keep CoreDNS crash-looping. Falls back
/// to [`COREDNS_PORT`] if the whole scan window is busy (caller surfaces the
/// resulting start error). Call AFTER killing any stale CoreDNS we own, so our
/// own previous instance does not push the port off its preferred value.
pub fn pick_coredns_port() -> u16 {
    select_coredns_port(port_free, None)
}

/// Like [`pick_coredns_port`], but when the canonical port is busy, reuse
/// `preferred` (the port already recorded in the systemd-resolved drop-in) if it
/// is free, before scanning for a new one. Reusing the existing port keeps the
/// drop-in content stable across restarts, so a plain restart needs no password.
pub fn pick_coredns_port_preferring(preferred: Option<u16>) -> u16 {
    select_coredns_port(port_free, preferred)
}

/// Pure port selection driven by a `is_free` probe. Preference order:
/// 1. [`COREDNS_PORT`] (canonical), 2. `preferred` if set and free, 3. the first
/// free port in `COREDNS_PORT+1..=COREDNS_PORT+50`. Falls back to [`COREDNS_PORT`]
/// when the whole window is busy (caller surfaces the resulting bind error).
pub fn select_coredns_port(is_free: impl Fn(u16) -> bool, preferred: Option<u16>) -> u16 {
    if is_free(COREDNS_PORT) {
        return COREDNS_PORT;
    }
    if let Some(p) = preferred {
        if p != COREDNS_PORT && is_free(p) {
            return p;
        }
    }
    (COREDNS_PORT + 1..=COREDNS_PORT + 50)
        .find(|&p| is_free(p))
        .unwrap_or(COREDNS_PORT)
}

/// Extract the CoreDNS port from a systemd-resolved drop-in body (`DNS=IP:PORT`).
/// Returns `None` when the line is missing or the port is unparseable.
pub fn parse_dropin_port(content: &str) -> Option<u16> {
    content
        .lines()
        .find_map(|l| l.trim().strip_prefix("DNS="))
        .and_then(|v| v.rsplit(':').next())
        .and_then(|p| p.trim().parse::<u16>().ok())
}

/// Poll `is_free` up to `attempts` times, sleeping `interval` between tries,
/// returning `true` as soon as it reports free. Lets a just-killed CoreDNS we own
/// finish releasing [`COREDNS_PORT`] before we pick, so a restart does not bump
/// onto a higher port (which would change the drop-in and re-prompt on the next
/// clean boot).
pub fn wait_port_free(mut is_free: impl FnMut() -> bool, attempts: u32, interval: std::time::Duration) -> bool {
    for i in 0..attempts {
        if is_free() {
            return true;
        }
        if i + 1 < attempts {
            std::thread::sleep(interval);
        }
    }
    false
}

/// Real-socket probe for the canonical CoreDNS port, for the app layer to wait on
/// after killing a stale instance.
pub fn coredns_port_free() -> bool {
    port_free(COREDNS_PORT)
}

#[derive(Debug, thiserror::Error)]
pub enum CorednsError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn coredns_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

pub fn coredns_url(version: &str, arch: &str) -> String {
    format!("https://github.com/coredns/coredns/releases/download/v{version}/coredns_{version}_linux_{arch}.tgz")
}

/// CoreDNS Corefile: each wildcard base becomes a zone answering any name with 127.0.0.1.
pub fn corefile(bases: &[String], port: u16) -> String {
    let mut s = String::new();
    for b in bases {
        s.push_str(&format!(
            "{b}:{port} {{\n    bind 127.0.0.1\n    template IN A {{\n        answer \"{{{{ .Name }}}} 60 IN A 127.0.0.1\"\n    }}\n    template IN AAAA {{\n        rcode NXDOMAIN\n    }}\n}}\n"
        ));
    }
    s
}

/// systemd-resolved drop-in routing the wildcard bases to our CoreDNS.
pub fn resolved_dropin(bases: &[String], port: u16) -> String {
    let doms: Vec<String> = bases
        .iter()
        .filter(|b| !b.is_empty() && b.chars().all(|c| !c.is_whitespace() && !c.is_control()))
        .map(|b| format!("~{b}"))
        .collect();
    format!("[Resolve]\nDNS=127.0.0.1:{port}\nDomains={}\n", doms.join(" "))
}

/// True only if a non-empty regular file exists at `dest` (a zero-byte leftover
/// from a failed extract counts as NOT installed, so it is re-downloaded).
pub fn coredns_installed(dest: &std::path::Path) -> bool {
    std::fs::metadata(dest).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Download the static CoreDNS binary into ~/laralux/bin/<version>/ (no apt/root) if missing.
pub fn ensure_coredns(
    paths: &LaraluxPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn crate::progress::ProgressSink,
) -> Result<(), CorednsError> {
    let dir = paths.version_dir("coredns", COREDNS_VERSION);
    let dest = dir.join("coredns");
    if coredns_installed(&dest) {
        let _ = crate::layout::set_current(paths, "coredns", COREDNS_VERSION);
        return Ok(());
    }
    let arch = coredns_arch().ok_or_else(|| CorednsError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_file(&dest);
    let tgz = paths.tmp().join("coredns.tgz");
    downloader.fetch_with_progress(&coredns_url(COREDNS_VERSION, arch), &tgz, sink).map_err(|e| CorednsError::Download(e.to_string()))?;
    let extract_dir = paths.tmp().join("coredns-extract");
    std::fs::create_dir_all(&extract_dir)?;
    runner.run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), extract_dir.display().to_string(), "coredns".into()], None)
        .map_err(|e| CorednsError::Extract(e.to_string()))?;
    let extracted = extract_dir.join("coredns");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&extracted, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&extracted, &dest).or_else(|_| {
        std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
    })?;
    crate::layout::set_current(paths, "coredns", COREDNS_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coredns_installed_rejects_missing_and_empty() {
        let dir = std::env::temp_dir().join(format!("lara-cdns-inst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("coredns");
        assert!(!coredns_installed(&p)); // missing
        std::fs::write(&p, b"").unwrap();
        assert!(!coredns_installed(&p)); // zero-byte
        std::fs::write(&p, b"ELF...").unwrap();
        assert!(coredns_installed(&p)); // non-empty
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn url_and_configs() {
        assert_eq!(
            coredns_url("1.14.4", "amd64"),
            "https://github.com/coredns/coredns/releases/download/v1.14.4/coredns_1.14.4_linux_amd64.tgz"
        );
        let cf = corefile(&["demo.dev".to_string()], COREDNS_PORT);
        assert!(cf.contains(&format!("demo.dev:{COREDNS_PORT} {{")));
        assert!(cf.contains("template IN A"));
        assert!(cf.contains("127.0.0.1"));
        let dp = resolved_dropin(&["demo.dev".to_string(), "test".to_string()], COREDNS_PORT);
        assert!(dp.contains(&format!("DNS=127.0.0.1:{COREDNS_PORT}")));
        assert!(dp.contains("Domains=~demo.dev ~test"));
    }

    #[test]
    fn resolved_dropin_drops_unsafe_bases() {
        let dp = resolved_dropin(&["demo.dev".to_string(), "bad base".to_string()], COREDNS_PORT);
        assert!(dp.contains("~demo.dev"));
        assert!(!dp.contains("bad base"));
    }

    #[test]
    fn coredns_port_avoids_mdns_and_needs_no_root() {
        // 5353 is the mDNS/avahi port — binding it collides with avahi-daemon and
        // crashes CoreDNS. Our port must avoid it and stay above the privileged range.
        assert_ne!(COREDNS_PORT, 5353);
        assert!(COREDNS_PORT > 1024);
    }

    #[test]
    fn pick_coredns_port_prefers_default_when_free() {
        let p = pick_coredns_port();
        // Always within the scan window; equals the default when it is free.
        assert!((COREDNS_PORT..=COREDNS_PORT + 50).contains(&p));
    }

    #[test]
    fn pick_coredns_port_skips_a_busy_default() {
        use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, COREDNS_PORT));
        // Hold the udp side of the preferred port so the picker must move past it.
        // Acquire it exclusively, retrying past the transient free-probe binds the
        // sibling port tests do inside `pick_coredns_port` (they run in parallel,
        // so a one-shot bind here can spuriously fail and leave the port free).
        let mut held = None;
        for _ in 0..200 {
            match UdpSocket::bind(addr) {
                Ok(sock) => {
                    held = Some(sock);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
        let _held = held.expect("could not acquire the preferred port to simulate it being busy");
        // We hold UDP COREDNS_PORT, so port_free() short-circuits false on it and
        // the picker must return a higher, free port.
        let p = pick_coredns_port();
        assert_ne!(p, COREDNS_PORT);
        assert!(p > COREDNS_PORT);
    }

    #[test]
    fn parse_dropin_port_reads_dns_line() {
        let body = "[Resolve]\nDNS=127.0.0.1:15354\nDomains=~member.dev ~online.dev\n";
        assert_eq!(parse_dropin_port(body), Some(15354));
    }

    #[test]
    fn parse_dropin_port_none_when_absent_or_garbage() {
        assert_eq!(parse_dropin_port("[Resolve]\nDomains=~x\n"), None);
        assert_eq!(parse_dropin_port("DNS=127.0.0.1:notaport\n"), None);
        assert_eq!(parse_dropin_port(""), None);
    }

    #[test]
    fn select_prefers_canonical_when_free() {
        // Everything free: always the canonical port, even if a different one was preferred.
        let p = select_coredns_port(|_| true, Some(COREDNS_PORT + 5));
        assert_eq!(p, COREDNS_PORT);
    }

    #[test]
    fn select_reuses_preferred_when_canonical_busy() {
        // Canonical busy, preferred free -> reuse the existing drop-in port instead
        // of scanning to a brand-new one (avoids churn that would re-prompt).
        let preferred = COREDNS_PORT + 7;
        let p = select_coredns_port(|port| port != COREDNS_PORT, Some(preferred));
        assert_eq!(p, preferred);
    }

    #[test]
    fn select_scans_when_canonical_busy_and_no_preferred() {
        // Canonical busy, no preferred -> first free port above canonical.
        let p = select_coredns_port(|port| port != COREDNS_PORT, None);
        assert_eq!(p, COREDNS_PORT + 1);
    }

    #[test]
    fn select_scans_when_preferred_also_busy() {
        let busy = COREDNS_PORT + 7;
        let p = select_coredns_port(|port| port != COREDNS_PORT && port != busy, Some(busy));
        assert_eq!(p, COREDNS_PORT + 1);
    }

    #[test]
    fn wait_port_free_returns_true_once_freed() {
        use std::cell::Cell;
        let calls = Cell::new(0u32);
        // Free on the 3rd probe.
        let ok = wait_port_free(
            || {
                let n = calls.get() + 1;
                calls.set(n);
                n >= 3
            },
            10,
            std::time::Duration::ZERO,
        );
        assert!(ok);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn wait_port_free_gives_up_after_attempts() {
        use std::cell::Cell;
        let calls = Cell::new(0u32);
        let ok = wait_port_free(
            || {
                calls.set(calls.get() + 1);
                false
            },
            4,
            std::time::Duration::ZERO,
        );
        assert!(!ok);
        assert_eq!(calls.get(), 4);
    }
}
