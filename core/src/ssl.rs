use crate::service::SpawnSpec;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum SslError {
    #[error("ssl io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("mkcert error: {0}")]
    Mkcert(String),
}

/// Issues (or reuses) a TLS certificate for a hostname.
pub trait CertIssuer: Send + Sync {
    fn ensure_cert(&self, hostname: &str) -> Result<CertFiles, SslError>;
}

// ---------- Real: mkcert ----------

pub struct MkcertIssuer {
    ssl_dir: PathBuf,
}

impl MkcertIssuer {
    pub fn new(ssl_dir: PathBuf) -> Self {
        Self { ssl_dir }
    }
    pub fn cert_path(&self, hostname: &str) -> PathBuf {
        self.ssl_dir.join(format!("{hostname}.pem"))
    }
    pub fn key_path(&self, hostname: &str) -> PathBuf {
        self.ssl_dir.join(format!("{hostname}-key.pem"))
    }
    pub fn issue_command(&self, hostname: &str) -> SpawnSpec {
        SpawnSpec::new("mkcert")
            .arg("-cert-file")
            .arg(self.cert_path(hostname).display().to_string())
            .arg("-key-file")
            .arg(self.key_path(hostname).display().to_string())
            .arg(hostname)
    }
}

impl CertIssuer for MkcertIssuer {
    fn ensure_cert(&self, hostname: &str) -> Result<CertFiles, SslError> {
        let cert = self.cert_path(hostname);
        let key = self.key_path(hostname);
        if cert.exists() && key.exists() {
            return Ok(CertFiles { cert, key });
        }
        std::fs::create_dir_all(&self.ssl_dir)?;
        let spec = self.issue_command(hostname);
        let status = std::process::Command::new(&spec.program)
            .args(&spec.args)
            .status()
            .map_err(|e| SslError::Mkcert(format!("spawn mkcert: {e}")))?;
        if !status.success() {
            return Err(SslError::Mkcert(format!("mkcert failed for {hostname}")));
        }
        Ok(CertFiles { cert, key })
    }
}

// ---------- Fake (used by sync tests) ----------

#[derive(Clone)]
pub struct FakeCertIssuer {
    base: PathBuf,
    requested: Arc<Mutex<Vec<String>>>,
}

impl FakeCertIssuer {
    pub fn new(base: PathBuf) -> Self {
        Self { base, requested: Arc::new(Mutex::new(Vec::new())) }
    }
    pub fn requested(&self) -> Arc<Mutex<Vec<String>>> {
        self.requested.clone()
    }
}

impl CertIssuer for FakeCertIssuer {
    fn ensure_cert(&self, hostname: &str) -> Result<CertFiles, SslError> {
        self.requested.lock().unwrap().push(hostname.to_string());
        Ok(CertFiles {
            cert: self.base.join(format!("{hostname}.pem")),
            key: self.base.join(format!("{hostname}-key.pem")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TMP_CTR: AtomicUsize = AtomicUsize::new(0);

    fn tmp_dir() -> std::path::PathBuf {
        let n = TMP_CTR.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("lara-ssl-{}-{}", std::process::id(), n))
    }

    #[test]
    fn cert_and_key_paths_under_ssl_dir() {
        let dir = tmp_dir();
        let m = MkcertIssuer::new(dir.clone());
        assert_eq!(m.cert_path("app.dev"), dir.join("app.dev.pem"));
        assert_eq!(m.key_path("app.dev"), dir.join("app.dev-key.pem"));
    }

    #[test]
    fn issue_command_targets_cert_key_and_host() {
        let dir = tmp_dir();
        let m = MkcertIssuer::new(dir.clone());
        let spec = m.issue_command("app.dev");
        assert_eq!(spec.program, "mkcert");
        let j = spec.args.join(" ");
        assert!(j.contains("-cert-file"));
        assert!(j.contains("-key-file"));
        assert!(j.contains("app.dev"));
    }

    #[test]
    fn ensure_cert_is_noop_when_files_exist() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let m = MkcertIssuer::new(dir.clone());
        std::fs::write(m.cert_path("app.dev"), "cert").unwrap();
        std::fs::write(m.key_path("app.dev"), "key").unwrap();
        // Must NOT invoke mkcert (which may be absent) when both files exist.
        let files = m.ensure_cert("app.dev").unwrap();
        assert_eq!(files.cert, m.cert_path("app.dev"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fake_issuer_records_and_returns_paths() {
        let dir = tmp_dir();
        let f = FakeCertIssuer::new(dir.clone());
        let files = f.ensure_cert("blog.dev").unwrap();
        assert_eq!(files.cert, dir.join("blog.dev.pem"));
        assert_eq!(f.requested().lock().unwrap().as_slice(), &["blog.dev".to_string()]);
    }
}
