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
    let doms: Vec<String> = bases.iter().map(|b| format!("~{b}")).collect();
    format!("[Resolve]\nDNS=127.0.0.1:{port}\nDomains={}\n", doms.join(" "))
}

/// Download the static CoreDNS binary into ~/laragon/bin (no apt/root) if missing.
pub fn ensure_coredns(
    paths: &LaragonPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), CorednsError> {
    let dest = paths.bin().join("coredns");
    if dest.exists() {
        return Ok(());
    }
    let arch = coredns_arch().ok_or_else(|| CorednsError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(paths.bin())?;
    let tgz = paths.tmp().join("coredns.tgz");
    downloader
        .fetch(&coredns_url(COREDNS_VERSION, arch), &tgz)
        .map_err(|e| CorednsError::Download(e.to_string()))?;
    runner
        .run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), paths.bin().display().to_string(), "coredns".into()], None)
        .map_err(|e| CorednsError::Extract(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
