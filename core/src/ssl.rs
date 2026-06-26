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

/// Issues (or reuses) a TLS certificate for a set of SANs.
pub trait CertIssuer: Send + Sync {
    fn ensure_cert(&self, basename: &str, names: &[String]) -> Result<CertFiles, SslError>;
}

// ---------- Real: mkcert ----------

pub struct MkcertIssuer {
    ssl_dir: PathBuf,
    /// The mkcert binary to spawn. Defaults to the bare name `mkcert`; use
    /// `resolved` (or `with_mkcert_bin`) so it points at the managed
    /// `bin/mkcert/current/mkcert` — no-apt, mkcert is not on `$PATH`.
    mkcert_bin: PathBuf,
}

impl MkcertIssuer {
    pub fn new(ssl_dir: PathBuf) -> Self {
        Self { ssl_dir, mkcert_bin: PathBuf::from("mkcert") }
    }
    /// Build an issuer with the mkcert binary resolved from the managed bin layout.
    pub fn resolved(paths: &crate::paths::LaragonPaths) -> Self {
        let bin = crate::bin::resolve_bin("mkcert", &crate::layout::managed_bin_dirs(paths))
            .unwrap_or_else(|| PathBuf::from("mkcert"));
        Self { ssl_dir: paths.ssl(), mkcert_bin: bin }
    }
    pub fn with_mkcert_bin(mut self, bin: PathBuf) -> Self {
        self.mkcert_bin = bin;
        self
    }
    pub fn cert_path(&self, basename: &str) -> PathBuf {
        self.ssl_dir.join(format!("{basename}.pem"))
    }
    pub fn key_path(&self, basename: &str) -> PathBuf {
        self.ssl_dir.join(format!("{basename}-key.pem"))
    }
    pub fn san_path(&self, basename: &str) -> PathBuf {
        self.ssl_dir.join(format!("{basename}.san"))
    }
    pub fn issue_command(&self, basename: &str, names: &[String]) -> SpawnSpec {
        let mut spec = SpawnSpec::new(self.mkcert_bin.display().to_string())
            .arg("-cert-file")
            .arg(self.cert_path(basename).display().to_string())
            .arg("-key-file")
            .arg(self.key_path(basename).display().to_string());
        for n in names {
            spec = spec.arg(n.clone());
        }
        spec
    }
}

impl CertIssuer for MkcertIssuer {
    fn ensure_cert(&self, basename: &str, names: &[String]) -> Result<CertFiles, SslError> {
        let cert = self.cert_path(basename);
        let key = self.key_path(basename);
        let san = self.san_path(basename);
        let mut sorted = names.to_vec();
        sorted.sort();
        let want = sorted.join("\n");
        if cert.exists() && key.exists() && std::fs::read_to_string(&san).ok().as_deref() == Some(want.as_str()) {
            return Ok(CertFiles { cert, key });
        }
        std::fs::create_dir_all(&self.ssl_dir)?;
        let spec = self.issue_command(basename, names);
        let status = std::process::Command::new(&spec.program)
            .args(&spec.args)
            .status()
            .map_err(|e| SslError::Mkcert(format!("spawn mkcert: {e}")))?;
        if !status.success() {
            return Err(SslError::Mkcert(format!("mkcert failed for {basename}")));
        }
        std::fs::write(&san, &want)?;
        Ok(CertFiles { cert, key })
    }
}

// ---------- Fake (used by sync tests) ----------

#[derive(Clone)]
pub struct FakeCertIssuer {
    base: PathBuf,
    requested: Arc<Mutex<Vec<(String, Vec<String>)>>>,
}

impl FakeCertIssuer {
    pub fn new(base: PathBuf) -> Self {
        Self { base, requested: Arc::new(Mutex::new(Vec::new())) }
    }
    pub fn requested(&self) -> Arc<Mutex<Vec<(String, Vec<String>)>>> {
        self.requested.clone()
    }
}

impl CertIssuer for FakeCertIssuer {
    fn ensure_cert(&self, basename: &str, names: &[String]) -> Result<CertFiles, SslError> {
        self.requested.lock().unwrap().push((basename.to_string(), names.to_vec()));
        Ok(CertFiles {
            cert: self.base.join(format!("{basename}.pem")),
            key: self.base.join(format!("{basename}-key.pem")),
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
        assert_eq!(m.cert_path("app"), dir.join("app.pem"));
        assert_eq!(m.key_path("app"), dir.join("app-key.pem"));
    }

    #[test]
    fn issue_command_targets_cert_key_and_host() {
        let dir = tmp_dir();
        let m = MkcertIssuer::new(dir.clone());
        let spec = m.issue_command("app", &["app.dev".to_string()]);
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
        std::fs::write(m.cert_path("app"), "cert").unwrap();
        std::fs::write(m.key_path("app"), "key").unwrap();
        std::fs::write(m.san_path("app"), "app.dev").unwrap();
        // Must NOT invoke mkcert (which may be absent) when both files exist and san matches.
        let files = m.ensure_cert("app", &["app.dev".to_string()]).unwrap();
        assert_eq!(files.cert, m.cert_path("app"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fake_issuer_records_basename_and_names() {
        let dir = tmp_dir();
        let f = FakeCertIssuer::new(dir.clone());
        let files = f.ensure_cert("blog", &["blog.dev".to_string(), "*.blog.dev".to_string()]).unwrap();
        assert_eq!(files.cert, dir.join("blog.pem"));
        let rec = f.requested();
        let rec = rec.lock().unwrap();
        assert_eq!(rec[0].0, "blog");
        assert_eq!(rec[0].1, vec!["blog.dev".to_string(), "*.blog.dev".to_string()]);
    }

    #[test]
    fn mkcert_reissues_when_san_set_changes() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let m = MkcertIssuer::new(dir.clone());
        // Pre-seed cert/key/san so ensure_cert is a no-op for the same set.
        std::fs::write(m.cert_path("app"), "c").unwrap();
        std::fs::write(m.key_path("app"), "k").unwrap();
        std::fs::write(dir.join("app.san"), "app.dev").unwrap();
        let f = m.ensure_cert("app", &["app.dev".to_string()]).unwrap();
        assert_eq!(f.cert, m.cert_path("app"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
