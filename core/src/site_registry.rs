use crate::scaffold::validate_site_name;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("registry io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("registry parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("registry serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("invalid site name: {0}")]
    InvalidName(String),
    #[error("folder not found: {0}")]
    RootNotFound(String),
    #[error("site already registered: {0}")]
    Duplicate(String),
    #[error("invalid upstream: {0}")]
    InvalidUpstream(String),
    #[error("invalid route: {0}")]
    InvalidRoute(String),
    #[error("a proxy needs at least one route")]
    NoRoutes,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid domain: {0}")]
    InvalidDomain(String),
    #[error("a site needs at least one domain")]
    NoDomains,
    #[error("domain already used by another site: {0}")]
    DomainTaken(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredSite {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyRoute {
    pub path: String,
    pub upstream: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxySite {
    pub name: String,
    #[serde(default = "default_true")]
    pub websocket: bool,
    pub routes: Vec<ProxyRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteDomains {
    pub name: String,
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SitePublicDomains {
    pub name: String,
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRegistry {
    #[serde(default)]
    sites: Vec<RegisteredSite>,
    #[serde(default)]
    proxies: Vec<ProxySite>,
    #[serde(default)]
    domains: Vec<SiteDomains>,
    #[serde(default)]
    public_domains: Vec<SitePublicDomains>,
}

/// Normalize a user-typed target into `host:port` (default host 127.0.0.1).
pub fn normalize_upstream(input: &str) -> Result<String, RegistryError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(RegistryError::InvalidUpstream(input.to_string()));
    }
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (if h.is_empty() { "127.0.0.1" } else { h }, p),
        None => ("127.0.0.1", s),
    };
    let port: u16 = port
        .parse()
        .map_err(|_| RegistryError::InvalidUpstream(input.to_string()))?;
    if port == 0 {
        return Err(RegistryError::InvalidUpstream(input.to_string()));
    }
    Ok(format!("{host}:{port}"))
}

/// Validate routes (≥1, each path starts with `/`, no duplicate paths) and
/// return them with normalized upstreams.
pub fn validate_routes(routes: &[ProxyRoute]) -> Result<Vec<ProxyRoute>, RegistryError> {
    if routes.is_empty() {
        return Err(RegistryError::NoRoutes);
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(routes.len());
    for r in routes {
        if !r.path.starts_with('/') {
            return Err(RegistryError::InvalidRoute(r.path.clone()));
        }
        if !seen.insert(r.path.clone()) {
            return Err(RegistryError::InvalidRoute(format!("duplicate path {}", r.path)));
        }
        let upstream = normalize_upstream(&r.upstream)?;
        out.push(ProxyRoute { path: r.path.clone(), upstream });
    }
    Ok(out)
}

/// A valid site domain: dotted DNS labels, optionally with a leading `*.`.
pub fn validate_domain(d: &str) -> Result<(), RegistryError> {
    let host = match d.strip_prefix("*.") {
        Some(rest) => rest,
        None => d,
    };
    if host.is_empty() || host.contains('*') {
        return Err(RegistryError::InvalidDomain(d.to_string()));
    }
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    if host.split('.').all(label_ok) && host.contains('.') {
        Ok(())
    } else {
        Err(RegistryError::InvalidDomain(d.to_string()))
    }
}

impl SiteRegistry {
    pub fn load(path: &Path) -> Result<SiteRegistry, RegistryError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SiteRegistry::default()),
            Err(e) => Err(RegistryError::Io(e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), RegistryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn sites(&self) -> &[RegisteredSite] {
        &self.sites
    }

    pub fn proxies(&self) -> &[ProxySite] {
        &self.proxies
    }

    pub fn add(&mut self, name: &str, root: &Path) -> Result<(), RegistryError> {
        validate_site_name(name).map_err(|_| RegistryError::InvalidName(name.to_string()))?;
        if !root.is_dir() {
            return Err(RegistryError::RootNotFound(root.display().to_string()));
        }
        if self.sites.iter().any(|s| s.name == name) {
            return Err(RegistryError::Duplicate(name.to_string()));
        }
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        self.sites.push(RegisteredSite { name: name.to_string(), root });
        Ok(())
    }

    pub fn add_proxy(
        &mut self,
        name: &str,
        routes: &[ProxyRoute],
        websocket: bool,
    ) -> Result<(), RegistryError> {
        validate_site_name(name).map_err(|_| RegistryError::InvalidName(name.to_string()))?;
        if self.sites.iter().any(|s| s.name == name) || self.proxies.iter().any(|p| p.name == name) {
            return Err(RegistryError::Duplicate(name.to_string()));
        }
        let routes = validate_routes(routes)?;
        self.proxies.push(ProxySite { name: name.to_string(), websocket, routes });
        Ok(())
    }

    pub fn update_proxy(
        &mut self,
        name: &str,
        routes: &[ProxyRoute],
        websocket: bool,
    ) -> Result<(), RegistryError> {
        let routes = validate_routes(routes)?;
        let p = self
            .proxies
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| RegistryError::NotFound(name.to_string()))?;
        p.routes = routes;
        p.websocket = websocket;
        Ok(())
    }

    pub fn domains_for(&self, name: &str) -> Option<&[String]> {
        self.domains.iter().find(|d| d.name == name).map(|d| d.domains.as_slice())
    }

    /// True nếu `domain` đang được một site có tên khác `skip` sử dụng,
    /// xét cả local domains lẫn public domains.
    fn domain_taken_by_other(&self, skip: &str, domain: &str) -> bool {
        self.domains.iter().any(|d| d.name != skip && d.domains.iter().any(|x| x == domain))
            || self
                .public_domains
                .iter()
                .any(|d| d.name != skip && d.domains.iter().any(|x| x == domain))
    }

    pub fn set_domains(&mut self, name: &str, domains: &[String]) -> Result<(), RegistryError> {
        // Normalize (trim + lowercase) and de-duplicate, preserving first-seen order
        // so domains[0] stays the leftmost the user intended.
        let mut norm: Vec<String> = Vec::new();
        for d in domains {
            let d = d.trim().to_ascii_lowercase();
            validate_domain(&d)?;
            if !norm.iter().any(|x| x == &d) {
                norm.push(d);
            }
        }
        if norm.is_empty() {
            return Err(RegistryError::NoDomains);
        }
        // reject a domain claimed by a *different* site (local hoặc public)
        for d in &norm {
            if self.domain_taken_by_other(name, d) {
                return Err(RegistryError::DomainTaken(d.clone()));
            }
        }
        // reject a domain that this same site already serves as a public domain
        // (else sync emits two blocks with an identical server_name → nginx conflict)
        if let Some(pd) = self.public_domains_for(name) {
            if let Some(dup) = norm.iter().find(|d| pd.iter().any(|x| &x == d)) {
                return Err(RegistryError::DomainTaken(dup.clone()));
            }
        }
        self.domains.retain(|d| d.name != name);
        self.domains.push(SiteDomains { name: name.to_string(), domains: norm });
        Ok(())
    }

    pub fn public_domains_for(&self, name: &str) -> Option<&[String]> {
        self.public_domains.iter().find(|d| d.name == name).map(|d| d.domains.as_slice())
    }

    pub fn set_public_domains(&mut self, name: &str, domains: &[String]) -> Result<(), RegistryError> {
        let mut norm: Vec<String> = Vec::new();
        for d in domains {
            let d = d.trim().to_ascii_lowercase();
            validate_domain(&d)?;
            if !norm.iter().any(|x| x == &d) {
                norm.push(d);
            }
        }
        if norm.is_empty() {
            return Err(RegistryError::NoDomains);
        }
        for d in &norm {
            if self.domain_taken_by_other(name, d) {
                return Err(RegistryError::DomainTaken(d.clone()));
            }
        }
        // reject a domain that this same site already serves as a local domain
        // (else sync emits two blocks with an identical server_name → nginx conflict)
        if let Some(ld) = self.domains_for(name) {
            if let Some(dup) = norm.iter().find(|d| ld.iter().any(|x| &x == d)) {
                return Err(RegistryError::DomainTaken(dup.clone()));
            }
        }
        self.public_domains.retain(|d| d.name != name);
        self.public_domains.push(SitePublicDomains { name: name.to_string(), domains: norm });
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sites.len() + self.proxies.len() + self.domains.len() + self.public_domains.len();
        self.sites.retain(|s| s.name != name);
        self.proxies.retain(|p| p.name != name);
        self.domains.retain(|d| d.name != name);
        self.public_domains.retain(|d| d.name != name);
        self.sites.len() + self.proxies.len() + self.domains.len() + self.public_domains.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CTR: AtomicUsize = AtomicUsize::new(0);
    fn root() -> PathBuf {
        let n = CTR.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("lara-reg-{}-{}", std::process::id(), n))
    }

    #[test]
    fn load_missing_file_is_empty() {
        let reg = SiteRegistry::load(&root().join("sites.toml")).unwrap();
        assert!(reg.sites().is_empty());
    }

    #[test]
    fn add_save_load_roundtrips() {
        let r = root();
        std::fs::create_dir_all(&r).unwrap();
        let proj = r.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let file = r.join("sites.toml");

        let mut reg = SiteRegistry::load(&file).unwrap();
        reg.add("blog", &proj).unwrap();
        reg.save(&file).unwrap();

        let back = SiteRegistry::load(&file).unwrap();
        assert_eq!(back.sites().len(), 1);
        assert_eq!(back.sites()[0].name, "blog");
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn add_rejects_invalid_name_missing_root_and_duplicate() {
        let r = root();
        let proj = r.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let mut reg = SiteRegistry::default();

        assert!(matches!(reg.add("Bad Name", &proj), Err(RegistryError::InvalidName(_))));
        assert!(matches!(
            reg.add("ok", &r.join("nope")),
            Err(RegistryError::RootNotFound(_))
        ));
        reg.add("dup", &proj).unwrap();
        assert!(matches!(reg.add("dup", &proj), Err(RegistryError::Duplicate(_))));
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn remove_reports_whether_removed() {
        let r = root();
        let proj = r.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let mut reg = SiteRegistry::default();
        reg.add("gone", &proj).unwrap();
        assert!(reg.remove("gone"));
        assert!(!reg.remove("gone"));
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn normalize_upstream_variants() {
        assert_eq!(normalize_upstream("3000").unwrap(), "127.0.0.1:3000");
        assert_eq!(normalize_upstream("127.0.0.1:5173").unwrap(), "127.0.0.1:5173");
        assert_eq!(normalize_upstream("localhost:8080").unwrap(), "localhost:8080");
        assert_eq!(normalize_upstream(":3000").unwrap(), "127.0.0.1:3000");
        for bad in ["", "0", "99999", "abc", "127.0.0.1:abc"] {
            assert!(normalize_upstream(bad).is_err(), "expected error for {bad:?}");
        }
    }

    #[test]
    fn validate_routes_checks_path_dupes_and_normalizes() {
        assert!(matches!(validate_routes(&[]), Err(RegistryError::NoRoutes)));
        let bad_path = vec![ProxyRoute { path: "api".into(), upstream: "3000".into() }];
        assert!(matches!(validate_routes(&bad_path), Err(RegistryError::InvalidRoute(_))));
        let dupe = vec![
            ProxyRoute { path: "/".into(), upstream: "3000".into() },
            ProxyRoute { path: "/".into(), upstream: "3001".into() },
        ];
        assert!(matches!(validate_routes(&dupe), Err(RegistryError::InvalidRoute(_))));
        let ok = validate_routes(&[ProxyRoute { path: "/".into(), upstream: "3000".into() }]).unwrap();
        assert_eq!(ok[0].upstream, "127.0.0.1:3000");
    }

    #[test]
    fn add_proxy_rejects_duplicate_across_lists() {
        let r = root();
        let proj = r.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let mut reg = SiteRegistry::default();
        reg.add("folder", &proj).unwrap();
        let routes = vec![ProxyRoute { path: "/".into(), upstream: "3000".into() }];

        assert!(matches!(reg.add_proxy("folder", &routes, true), Err(RegistryError::Duplicate(_))));
        assert!(matches!(reg.add_proxy("Bad Name", &routes, true), Err(RegistryError::InvalidName(_))));
        reg.add_proxy("api", &routes, true).unwrap();
        assert!(matches!(reg.add_proxy("api", &routes, true), Err(RegistryError::Duplicate(_))));
        assert_eq!(reg.proxies().len(), 1);
        assert_eq!(reg.proxies()[0].routes[0].upstream, "127.0.0.1:3000");
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn update_proxy_replaces_or_errors_not_found() {
        let mut reg = SiteRegistry::default();
        let routes = vec![ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true).unwrap();

        let new_routes = vec![ProxyRoute { path: "/".into(), upstream: "4000".into() }];
        reg.update_proxy("api", &new_routes, false).unwrap();
        assert_eq!(reg.proxies()[0].routes[0].upstream, "127.0.0.1:4000");
        assert!(!reg.proxies()[0].websocket);

        assert!(matches!(reg.update_proxy("ghost", &new_routes, true), Err(RegistryError::NotFound(_))));
    }

    #[test]
    fn validate_domain_accepts_and_rejects() {
        for ok in ["app2.dev", "api.demo.dev", "my-app.local", "*.demo.dev"] {
            assert!(validate_domain(ok).is_ok(), "should accept {ok}");
        }
        for bad in ["", "Demo.dev", "a b.dev", "*.*.dev", "*x.dev", "foo.*.dev", "-x.dev"] {
            assert!(validate_domain(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn set_domains_validates_uniqueness_and_roundtrips() {
        let mut reg = SiteRegistry::default();
        assert!(matches!(reg.set_domains("a", &[]), Err(RegistryError::NoDomains)));
        assert!(matches!(
            reg.set_domains("a", &["Bad".to_string()]),
            Err(RegistryError::InvalidDomain(_))
        ));
        reg.set_domains("a", &["a.dev".to_string(), "*.a.dev".to_string()]).unwrap();
        // another site can't claim a.dev
        assert!(matches!(
            reg.set_domains("b", &["a.dev".to_string()]),
            Err(RegistryError::DomainTaken(_))
        ));
        assert_eq!(reg.domains_for("a").unwrap(), &["a.dev".to_string(), "*.a.dev".to_string()]);
        assert!(reg.remove("a"));
        assert!(reg.domains_for("a").is_none());
    }

    #[test]
    fn set_domains_normalizes_and_dedups() {
        let mut reg = SiteRegistry::default();
        reg.set_domains("a", &["  Demo.DEV ".to_string(), "demo.dev".to_string(), "*.Demo.dev".to_string()]).unwrap();
        assert_eq!(reg.domains_for("a").unwrap(), &["demo.dev".to_string(), "*.demo.dev".to_string()]);
    }

    #[test]
    fn public_domains_set_get_remove_and_cross_uniqueness() {
        let mut reg = SiteRegistry::default();
        // empty bị từ chối
        assert!(matches!(reg.set_public_domains("a", &[]), Err(RegistryError::NoDomains)));
        // invalid bị từ chối
        assert!(matches!(
            reg.set_public_domains("a", &["Bad".to_string()]),
            Err(RegistryError::InvalidDomain(_))
        ));
        // set + get, có normalize/dedupe
        reg.set_public_domains("a", &["  App.Example.COM ".to_string(), "app.example.com".to_string()]).unwrap();
        assert_eq!(reg.public_domains_for("a").unwrap(), &["app.example.com".to_string()]);

        // local domain của site khác không được trùng public domain đã dùng
        assert!(matches!(
            reg.set_domains("b", &["app.example.com".to_string()]),
            Err(RegistryError::DomainTaken(_))
        ));
        // và ngược lại: public domain không được trùng local domain đã dùng
        reg.set_domains("c", &["c.dev".to_string()]).unwrap();
        assert!(matches!(
            reg.set_public_domains("d", &["c.dev".to_string()]),
            Err(RegistryError::DomainTaken(_))
        ));

        // remove xoá cả public domains
        assert!(reg.remove("a"));
        assert!(reg.public_domains_for("a").is_none());
    }

    #[test]
    fn same_site_cannot_reuse_domain_across_local_and_public() {
        // local trước, rồi public trùng -> bị từ chối
        let mut reg = SiteRegistry::default();
        reg.set_domains("a", &["a.dev".to_string()]).unwrap();
        assert!(matches!(
            reg.set_public_domains("a", &["a.dev".to_string()]),
            Err(RegistryError::DomainTaken(_))
        ));
        // public trước, rồi local trùng -> cũng bị từ chối
        let mut reg = SiteRegistry::default();
        reg.set_public_domains("b", &["app.example.com".to_string()]).unwrap();
        assert!(matches!(
            reg.set_domains("b", &["app.example.com".to_string()]),
            Err(RegistryError::DomainTaken(_))
        ));
    }

    #[test]
    fn old_sites_toml_without_public_domains_loads() {
        let r = root();
        std::fs::create_dir_all(&r).unwrap();
        let file = r.join("sites.toml");
        std::fs::write(&file, "[[sites]]\nname = \"blog\"\nroot = \"/tmp/blog\"\n").unwrap();
        let reg = SiteRegistry::load(&file).unwrap();
        assert!(reg.public_domains_for("blog").is_none());
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn remove_handles_proxies_and_old_file_loads() {
        // remove a proxy
        let mut reg = SiteRegistry::default();
        let routes = vec![ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true).unwrap();
        assert!(reg.remove("api"));
        assert!(!reg.remove("api"));

        // a sites.toml with only [[sites]] still loads, proxies empty
        let r = root();
        std::fs::create_dir_all(&r).unwrap();
        let file = r.join("sites.toml");
        std::fs::write(&file, "[[sites]]\nname = \"blog\"\nroot = \"/tmp/blog\"\n").unwrap();
        let loaded = SiteRegistry::load(&file).unwrap();
        assert_eq!(loaded.sites().len(), 1);
        assert!(loaded.proxies().is_empty());
        std::fs::remove_dir_all(&r).ok();
    }
}
