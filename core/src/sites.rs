use crate::paths::LaragonPaths;
use std::path::PathBuf;

/// Where a site came from: the `www/` scan, or the explicit registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum SiteSource {
    Scanned,
    Linked,
    Proxy,
}

/// The proxy view of a site, sent to the frontend (routes + websocket flag).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProxySpec {
    pub routes: Vec<crate::site_registry::ProxyRoute>,
    pub websocket: bool,
}

/// A project under `www/` exposed at `<name>.<tld>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Site {
    pub name: String,
    pub root: PathBuf,
    pub hostname: String,
    pub source: SiteSource,
    pub proxy: Option<ProxySpec>,
}

impl Site {
    /// Laravel-style: serve `public/` if present, else the project dir.
    pub fn document_root(&self) -> PathBuf {
        let public = self.root.join("public");
        if public.is_dir() {
            public
        } else {
            self.root.clone()
        }
    }

    /// Generate the nginx vhost (HTTP→HTTPS redirect + HTTPS server) for this site.
    pub fn vhost_config(
        &self,
        paths: &LaragonPaths,
        php_socket: &std::path::Path,
        cert: &std::path::Path,
        key: &std::path::Path,
    ) -> String {
        if let Some(spec) = &self.proxy {
            let mut locations = String::new();
            for r in &spec.routes {
                locations.push_str(&format!(
                    "\x20 location {path} {{\n\
                     \x20   proxy_pass http://{up};\n\
                     \x20   proxy_http_version 1.1;\n\
                     \x20   proxy_set_header Host $host;\n\
                     \x20   proxy_set_header X-Real-IP $remote_addr;\n\
                     \x20   proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
                     \x20   proxy_set_header X-Forwarded-Proto $scheme;\n",
                    path = r.path,
                    up = r.upstream,
                ));
                if spec.websocket {
                    locations.push_str(
                        "\x20   proxy_set_header Upgrade $http_upgrade;\n\
                         \x20   proxy_set_header Connection $connection_upgrade;\n",
                    );
                }
                locations.push_str("\x20 }\n");
            }
            return format!(
                "server {{\n\
                 \x20 listen 80;\n\
                 \x20 server_name {host};\n\
                 \x20 return 301 https://$host$request_uri;\n\
                 }}\n\
                 server {{\n\
                 \x20 listen 443 ssl;\n\
                 \x20 server_name {host};\n\
                 \x20 ssl_certificate {cert};\n\
                 \x20 ssl_certificate_key {key};\n\
                 \x20 access_log {alog};\n\
                 \x20 error_log {elog};\n\
                 {locations}\
                 }}\n",
                host = self.hostname,
                cert = cert.display(),
                key = key.display(),
                alog = paths.log().join(format!("{}-access.log", self.name)).display(),
                elog = paths.log().join(format!("{}-error.log", self.name)).display(),
                locations = locations,
            );
        }
        format!(
            "server {{\n\
             \x20 listen 80;\n\
             \x20 server_name {host};\n\
             \x20 return 301 https://$host$request_uri;\n\
             }}\n\
             server {{\n\
             \x20 listen 443 ssl;\n\
             \x20 server_name {host};\n\
             \x20 ssl_certificate {cert};\n\
             \x20 ssl_certificate_key {key};\n\
             \x20 root {docroot};\n\
             \x20 index index.php index.html;\n\
             \x20 access_log {alog};\n\
             \x20 error_log {elog};\n\
             \x20 location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n\
             \x20 location ~ \\.php$ {{\n\
             \x20   fastcgi_pass unix:{sock};\n\
             \x20   fastcgi_index index.php;\n\
             \x20   include {nginx_etc}/fastcgi_params;\n\
             \x20   fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
             \x20   fastcgi_param HTTPS on;\n\
             \x20 }}\n\
             }}\n",
            host = self.hostname,
            cert = cert.display(),
            key = key.display(),
            docroot = self.document_root().display(),
            alog = paths.log().join(format!("{}-access.log", self.name)).display(),
            elog = paths.log().join(format!("{}-error.log", self.name)).display(),
            sock = php_socket.display(),
            nginx_etc = paths.etc_for("nginx").display(),
        )
    }
}

/// Discover sites in `www/`: immediate subdirectories, skipping hidden ones.
pub fn scan_sites(paths: &LaragonPaths, tld: &str) -> std::io::Result<Vec<Site>> {
    let www = paths.www();
    let entries = match std::fs::read_dir(&www) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut sites = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        sites.push(Site {
            hostname: format!("{name}.{tld}"),
            root: entry.path(),
            name,
            source: SiteSource::Scanned,
            proxy: None,
        });
    }
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sites)
}

/// Merge auto-discovered `www/` sites with the explicit registry.
/// Scanned sites shadow registry entries of the same name; registry entries
/// whose folder is missing are skipped. Returns `(sites, warnings)`.
pub fn list_all_sites(
    paths: &LaragonPaths,
    tld: &str,
) -> std::io::Result<(Vec<Site>, Vec<String>)> {
    let mut sites = scan_sites(paths, tld)?;
    let mut warnings = Vec::new();

    let registry = match crate::site_registry::SiteRegistry::load(&paths.sites_file()) {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!("sites.toml ignored ({e})"));
            crate::site_registry::SiteRegistry::default()
        }
    };

    for entry in registry.sites() {
        if sites.iter().any(|s| s.name == entry.name) {
            warnings.push(format!(
                "linked site `{}` is shadowed by a folder in www/",
                entry.name
            ));
            continue;
        }
        if !entry.root.is_dir() {
            warnings.push(format!(
                "linked site `{}`: folder `{}` not found",
                entry.name,
                entry.root.display()
            ));
            continue;
        }
        sites.push(Site {
            hostname: format!("{}.{}", entry.name, tld),
            root: entry.root.clone(),
            name: entry.name.clone(),
            source: SiteSource::Linked,
            proxy: None,
        });
    }

    for p in registry.proxies() {
        if sites.iter().any(|s| s.name == p.name) {
            warnings.push(format!("proxy site `{}` is shadowed by another site", p.name));
            continue;
        }
        sites.push(Site {
            hostname: format!("{}.{}", p.name, tld),
            root: std::path::PathBuf::new(),
            name: p.name.clone(),
            source: SiteSource::Proxy,
            proxy: Some(ProxySpec { routes: p.routes.clone(), websocket: p.websocket }),
        });
    }

    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((sites, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_root() -> std::path::PathBuf {
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("lara-sites-{}-{}-{}", std::process::id(), line!(), counter))
    }

    #[test]
    fn scans_only_dirs_builds_hostnames_sorted() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("beta")).unwrap();
        std::fs::create_dir_all(www.join("alpha")).unwrap();
        std::fs::create_dir_all(www.join(".hidden")).unwrap();
        std::fs::write(www.join("index.php"), "x").unwrap();

        let paths = LaragonPaths::new(root.clone());
        let sites = scan_sites(&paths, "dev").unwrap();

        let names: Vec<&str> = sites.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]); // sorted, no file, no hidden
        assert_eq!(sites[0].hostname, "alpha.dev");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn document_root_prefers_public_subdir() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("laravelapp").join("public")).unwrap();
        std::fs::create_dir_all(www.join("plain")).unwrap();

        let paths = LaragonPaths::new(root.clone());
        let sites = scan_sites(&paths, "dev").unwrap();
        let by = |n: &str| sites.iter().find(|s| s.name == n).unwrap().clone();

        assert_eq!(by("laravelapp").document_root(), www.join("laravelapp").join("public"));
        assert_eq!(by("plain").document_root(), www.join("plain"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_www_returns_empty() {
        let paths = LaragonPaths::new(temp_root());
        assert!(scan_sites(&paths, "dev").unwrap().is_empty());
    }

    #[test]
    fn vhost_has_https_redirect_ssl_and_fastcgi() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaragonPaths::new(root.clone());
        let site = scan_sites(&paths, "dev").unwrap().into_iter().find(|s| s.name == "app").unwrap();

        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");
        let conf = site.vhost_config(&paths, &sock, &cert, &key);

        assert!(conf.contains("server_name app.dev;"));
        assert!(conf.contains("return 301 https://$host$request_uri;"));
        assert!(conf.contains("listen 443 ssl;"));
        assert!(conf.contains(&format!("ssl_certificate {};", cert.display())));
        assert!(conf.contains(&format!("ssl_certificate_key {};", key.display())));
        assert!(conf.contains(&format!("root {};", www.join("app").join("public").display())));
        assert!(conf.contains(&format!("fastcgi_pass unix:{};", sock.display())));
        assert!(conf.contains("fastcgi_param HTTPS on;"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_marks_sites_as_scanned() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("a")).unwrap();
        let paths = LaragonPaths::new(root.clone());
        let sites = scan_sites(&paths, "dev").unwrap();
        assert_eq!(sites[0].source, SiteSource::Scanned);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_merges_scanned_and_linked_sorted() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("zeta")).unwrap();
        let external = root.join("external").join("alpha");
        std::fs::create_dir_all(&external).unwrap();
        let paths = LaragonPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.add("alpha", &external).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        let names: Vec<&str> = sites.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"]); // sorted
        let alpha = sites.iter().find(|s| s.name == "alpha").unwrap();
        assert_eq!(alpha.source, SiteSource::Linked);
        assert_eq!(alpha.hostname, "alpha.dev");
        assert!(warnings.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_skips_stale_root_with_warning() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www")).unwrap();
        let paths = LaragonPaths::new(root.clone());

        // Write a registry entry pointing at a folder that does not exist.
        let toml = format!(
            "[[sites]]\nname = \"ghost\"\nroot = \"{}\"\n",
            root.join("missing").display()
        );
        std::fs::write(paths.sites_file(), toml).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        assert!(sites.iter().all(|s| s.name != "ghost"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ghost"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_scanned_shadows_duplicate_registry_entry() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("dup")).unwrap();
        let external = root.join("external").join("dup");
        std::fs::create_dir_all(&external).unwrap();
        let paths = LaragonPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.add("dup", &external).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        let dups: Vec<&Site> = sites.iter().filter(|s| s.name == "dup").collect();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].source, SiteSource::Scanned); // scan wins
        assert_eq!(warnings.len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_includes_proxy_sites() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www")).unwrap();
        let paths = LaragonPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, _w) = list_all_sites(&paths, "dev").unwrap();
        let api = sites.iter().find(|s| s.name == "api").unwrap();
        assert_eq!(api.source, SiteSource::Proxy);
        assert_eq!(api.hostname, "api.dev");
        let spec = api.proxy.as_ref().expect("proxy spec");
        assert!(spec.websocket);
        assert_eq!(spec.routes[0].upstream, "127.0.0.1:3000");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_scanned_shadows_proxy_of_same_name() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("api")).unwrap();
        let paths = LaragonPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        let apis: Vec<&Site> = sites.iter().filter(|s| s.name == "api").collect();
        assert_eq!(apis.len(), 1);
        assert_eq!(apis[0].source, SiteSource::Scanned);
        assert_eq!(warnings.len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    fn proxy_site(name: &str, routes: Vec<crate::site_registry::ProxyRoute>, websocket: bool) -> Site {
        Site {
            name: name.to_string(),
            root: std::path::PathBuf::new(),
            hostname: format!("{name}.dev"),
            source: SiteSource::Proxy,
            proxy: Some(ProxySpec { routes, websocket }),
        }
    }

    #[test]
    fn proxy_vhost_has_proxy_pass_and_ws_headers() {
        let root = temp_root();
        let paths = LaragonPaths::new(root.clone());
        let route = crate::site_registry::ProxyRoute { path: "/".into(), upstream: "127.0.0.1:3000".into() };
        let site = proxy_site("app", vec![route], true);
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");

        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        assert!(conf.contains("server_name app.dev;"));
        assert!(conf.contains("listen 443 ssl;"));
        assert!(conf.contains("location / {"));
        assert!(conf.contains("proxy_pass http://127.0.0.1:3000;"));
        assert!(conf.contains("proxy_set_header X-Forwarded-Proto $scheme;"));
        assert!(conf.contains("proxy_set_header Upgrade $http_upgrade;"));
        assert!(conf.contains("proxy_set_header Connection $connection_upgrade;"));
        assert!(!conf.contains("fastcgi_pass"));
    }

    #[test]
    fn proxy_vhost_without_ws_omits_upgrade_and_supports_multiroute() {
        let root = temp_root();
        let paths = LaragonPaths::new(root.clone());
        let routes = vec![
            crate::site_registry::ProxyRoute { path: "/api".into(), upstream: "127.0.0.1:3001".into() },
            crate::site_registry::ProxyRoute { path: "/".into(), upstream: "127.0.0.1:3000".into() },
        ];
        let site = proxy_site("app", routes, false);
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");

        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        assert!(conf.contains("location /api {"));
        assert!(conf.contains("location / {"));
        assert!(conf.contains("proxy_pass http://127.0.0.1:3001;"));
        assert!(conf.contains("proxy_pass http://127.0.0.1:3000;"));
        assert!(!conf.contains("Upgrade $http_upgrade"));
    }
}
