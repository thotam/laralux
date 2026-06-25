use crate::paths::LaragonPaths;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

pub const COREDNS_VERSION: &str = "1.14.4";

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

/// Download the static CoreDNS binary into ~/laragon/bin/<version>/ (no apt/root) if missing.
pub fn ensure_coredns(
    paths: &LaragonPaths,
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
        let cf = corefile(&["demo.dev".to_string()], 5353);
        assert!(cf.contains("demo.dev:5353 {"));
        assert!(cf.contains("template IN A"));
        assert!(cf.contains("127.0.0.1"));
        let dp = resolved_dropin(&["demo.dev".to_string(), "test".to_string()], 5353);
        assert!(dp.contains("DNS=127.0.0.1:5353"));
        assert!(dp.contains("Domains=~demo.dev ~test"));
    }

    #[test]
    fn resolved_dropin_drops_unsafe_bases() {
        let dp = resolved_dropin(&["demo.dev".to_string(), "bad base".to_string()], 5353);
        assert!(dp.contains("~demo.dev"));
        assert!(!dp.contains("bad base"));
    }
}
