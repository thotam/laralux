use crate::hosts::apply_block;
use crate::paths::LaragonPaths;
use crate::privileged::Privileged;
use crate::sites::{list_all_sites, Site};
use crate::ssl::CertIssuer;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("sync io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sync ssl error: {0}")]
    Ssl(#[from] crate::ssl::SslError),
    #[error("sync privileged error: {0}")]
    Priv(#[from] crate::privileged::PrivError),
}

pub struct SyncOutcome {
    pub sites: Vec<Site>,
    pub warnings: Vec<String>,
    pub wildcard_bases: Vec<String>,
}

/// Scan sites, issue certs, write per-site vhosts, and update the managed
/// `/etc/hosts` block — writing hosts only when the block actually changes.
pub fn sync_sites(
    paths: &LaragonPaths,
    tld: &str,
    php_socket: &Path,
    hosts_path: &Path,
    issuer: &dyn CertIssuer,
    privileged: &dyn Privileged,
) -> Result<SyncOutcome, SyncError> {
    let (sites, warnings) = list_all_sites(paths, tld)?;
    let sites_dir = paths.etc_for("nginx").join("sites");
    std::fs::create_dir_all(&sites_dir)?;

    for site in &sites {
        let certs = issuer.ensure_cert(&site.name, &site.domains)?;
        let conf = site.vhost_config(paths, php_socket, &certs.cert, &certs.key);
        std::fs::write(sites_dir.join(format!("{}.conf", site.name)), conf)?;
    }

    let mut explicit: Vec<String> = Vec::new();
    let mut wildcard_bases: Vec<String> = Vec::new();
    for s in &sites {
        for d in &s.domains {
            match d.strip_prefix("*.") {
                Some(base) => {
                    if !wildcard_bases.iter().any(|b| b == base) {
                        wildcard_bases.push(base.to_string());
                    }
                }
                None => explicit.push(d.clone()),
            }
        }
    }

    let existing = match std::fs::read_to_string(hosts_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(SyncError::Io(e)),
    };
    let updated = apply_block(&existing, &explicit);
    if updated != existing {
        privileged.write_etc_hosts(&updated)?;
    }

    Ok(SyncOutcome { sites, warnings, wildcard_bases })
}

#[cfg(test)]
mod tests {
    use super::sync_sites;
    use crate::hosts::apply_block;
    use crate::paths::LaragonPaths;
    use crate::privileged::FakePrivileged;
    use crate::ssl::FakeCertIssuer;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ROOT_CTR: AtomicUsize = AtomicUsize::new(0);
    fn root() -> std::path::PathBuf {
        let n = ROOT_CTR.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("lara-sync-{}-{}", std::process::id(), n))
    }

    #[test]
    fn writes_vhosts_certs_and_hosts_block() {
        let r = root();
        let www = r.join("www");
        std::fs::create_dir_all(www.join("app")).unwrap();
        std::fs::create_dir_all(www.join("blog")).unwrap();
        let paths = LaragonPaths::new(r.clone());

        let hosts_path = r.join("hosts");
        std::fs::write(&hosts_path, "127.0.0.1 localhost\n").unwrap();

        let issuer = FakeCertIssuer::new(paths.ssl());
        let priv_ = FakePrivileged::new();
        let writes = priv_.hosts_writes();
        let sock = paths.tmp().join("php-fpm.sock");

        let out = sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();
        let sites = out.sites;

        assert_eq!(sites.len(), 2);
        // vhost files written
        assert!(paths.etc_for("nginx").join("sites").join("app.conf").is_file());
        assert!(paths.etc_for("nginx").join("sites").join("blog.conf").is_file());
        // certs requested for both
        assert_eq!(issuer.requested().lock().unwrap().len(), 2);
        // hosts written once, containing both hostnames and the preserved localhost line
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].contains("127.0.0.1 app.dev"));
        assert!(writes[0].contains("127.0.0.1 blog.dev"));
        assert!(writes[0].contains("127.0.0.1 localhost"));
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn skips_hosts_write_when_block_already_current() {
        let r = root();
        let www = r.join("www");
        std::fs::create_dir_all(www.join("app")).unwrap();
        let paths = LaragonPaths::new(r.clone());

        // Pre-populate hosts with the exact block sync would produce.
        let hosts_path = r.join("hosts");
        let already = apply_block("127.0.0.1 localhost\n", &["app.dev".to_string()]);
        std::fs::write(&hosts_path, &already).unwrap();

        let issuer = FakeCertIssuer::new(paths.ssl());
        let priv_ = FakePrivileged::new();
        let writes = priv_.hosts_writes();
        let sock = paths.tmp().join("php-fpm.sock");

        sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();

        // No write because the managed block is already correct.
        assert_eq!(writes.lock().unwrap().len(), 0);
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn sync_splits_explicit_hosts_and_wildcard_bases() {
        let r = root();
        std::fs::create_dir_all(r.join("www").join("demo")).unwrap();
        let paths = LaragonPaths::new(r.clone());
        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.set_domains("demo", &["demo.dev".into(), "api.demo.dev".into(), "*.demo.dev".into()]).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let hosts_path = r.join("hosts");
        std::fs::write(&hosts_path, "127.0.0.1 localhost\n").unwrap();
        let issuer = FakeCertIssuer::new(paths.ssl());
        let priv_ = FakePrivileged::new();
        let writes = priv_.hosts_writes();
        let sock = paths.tmp().join("php-fpm.sock");

        let out = sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();
        assert_eq!(out.wildcard_bases, vec!["demo.dev".to_string()]);
        let w = writes.lock().unwrap();
        assert!(w[0].contains("127.0.0.1 demo.dev"));
        assert!(w[0].contains("127.0.0.1 api.demo.dev"));
        assert!(!w[0].contains("*.demo.dev")); // wildcard not in hosts
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn writes_vhost_for_linked_site_outside_www() {
        let r = root();
        std::fs::create_dir_all(r.join("www")).unwrap();
        let external = r.join("ext").join("linked");
        std::fs::create_dir_all(&external).unwrap();
        let paths = LaragonPaths::new(r.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.add("linked", &external).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let hosts_path = r.join("hosts");
        std::fs::write(&hosts_path, "127.0.0.1 localhost\n").unwrap();
        let issuer = FakeCertIssuer::new(paths.ssl());
        let priv_ = FakePrivileged::new();
        let sock = paths.tmp().join("php-fpm.sock");

        let out = sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();
        assert!(out.sites.iter().any(|s| s.name == "linked"));
        assert!(paths.etc_for("nginx").join("sites").join("linked.conf").is_file());
        std::fs::remove_dir_all(&r).ok();
    }
}
