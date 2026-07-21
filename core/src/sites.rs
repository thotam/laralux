use crate::paths::LaraluxPaths;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SiteFsError {
    #[error("invalid site name: {0}")]
    InvalidName(String),
    #[error("site folder not found: {0}")]
    NotFound(String),
    #[error("destination already exists: {0}")]
    AlreadyExists(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// True iff `name` is safe to treat as a direct child folder of `www`: non-empty,
/// not `.`/`..`, free of path separators, and not already hidden (leading `.`).
pub fn valid_scanned_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && !name.contains('/')
        && !name.contains('\\')
}

/// Hide a scanned site by renaming `www/<name>` → `www/.<name>`. `scan_sites`
/// skips dot-prefixed dirs, so the site vanishes from the list / hosts / nginx
/// after a re-sync while all files are kept. Reversible by renaming back.
pub fn hide_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError> {
    if !valid_scanned_name(name) {
        return Err(SiteFsError::InvalidName(name.to_string()));
    }
    let src = paths.www().join(name);
    if !src.is_dir() {
        return Err(SiteFsError::NotFound(name.to_string()));
    }
    let dst = paths.www().join(format!(".{name}"));
    if dst.exists() {
        return Err(SiteFsError::AlreadyExists(format!(".{name}")));
    }
    std::fs::rename(&src, &dst)?;
    Ok(())
}

/// Permanently delete a scanned site's folder `www/<name>`.
pub fn delete_scanned_site(paths: &LaraluxPaths, name: &str) -> Result<(), SiteFsError> {
    if !valid_scanned_name(name) {
        return Err(SiteFsError::InvalidName(name.to_string()));
    }
    let dir = paths.www().join(name);
    if !dir.is_dir() {
        return Err(SiteFsError::NotFound(name.to_string()));
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

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
    pub domains: Vec<String>,
    pub source: SiteSource,
    pub proxy: Option<ProxySpec>,
    pub public_domains: Vec<String>,
}

/// Serve `.well-known`, never execute it. ACME clients and OAuth libraries write
/// into this directory, and write access there must not imply code execution.
///
/// Three parts, all load-bearing:
/// - `^~` makes nginx skip the regex locations below, so the `\.php$` handler can
///   never reach anything here — the property is structural, not a matter of
///   declaration order or of the extension list being exhaustive.
/// - The nested dotfile deny restores what `^~` would otherwise bypass: without
///   it `/.well-known/.env` goes from 403 to 200 with its contents.
/// - The trailing rule is deliberately NOT anchored, because `^~` only matches at
///   the start: `/{tenant}/.well-known/…` (the shape OIDC discovery uses) would
///   otherwise still execute.
const WELL_KNOWN_GUARD: &str = "\x20 location ^~ /.well-known/ {\n\
     \x20   location ~ ^/\\.well-known/(.*/)?\\. { deny all; }\n\
     \x20   location ~ \\.(php|phar|phtml)$ { deny all; }\n\
     \x20   try_files $uri $uri/ /index.php?$query_string;\n\
     \x20 }\n\
     \x20 location ~ /\\.well-known/.*\\.(php|phar|phtml)$ { deny all; }\n";

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
        paths: &LaraluxPaths,
        php_socket: &std::path::Path,
        cert: &std::path::Path,
        key: &std::path::Path,
    ) -> String {
        let server_names = self.domains.join(" ");
        let is_laravel = self.root.join("artisan").is_file();
        let build_cache = if is_laravel {
            "\x20 location ^~ /build/ {\n\
             \x20   expires 1y;\n\
             \x20   add_header Cache-Control \"public, immutable\";\n\
             \x20   try_files $uri =404;\n\
             \x20 }\n"
        } else {
            ""
        };
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
            let local = format!(
                "server {{\n\
                 \x20 listen 80;\n\
                 \x20 server_name {names};\n\
                 \x20 return 301 https://$host$request_uri;\n\
                 }}\n\
                 server {{\n\
                 \x20 listen 443 ssl;\n\
                 \x20 http2 on;\n\
                 \x20 server_name {names};\n\
                 \x20 ssl_certificate {cert};\n\
                 \x20 ssl_certificate_key {key};\n\
                 \x20 ssl_protocols TLSv1.2 TLSv1.3;\n\
                 \x20 ssl_ciphers HIGH:!aNULL:!MD5;\n\
                 \x20 ssl_session_cache shared:SSL:10m;\n\
                 \x20 ssl_session_timeout 10m;\n\
                 \x20 access_log {alog};\n\
                 \x20 error_log {elog};\n\
                 {locations}\
                 }}\n",
                names = server_names,
                cert = cert.display(),
                key = key.display(),
                alog = paths.log().join(format!("{}-access.log", self.name)).display(),
                elog = paths.log().join(format!("{}-error.log", self.name)).display(),
                locations = locations,
            );
            return format!("{local}{}", self.public_vhost_block(paths, php_socket, cert, key));
        }
        let local = format!(
            "server {{\n\
             \x20 listen 80;\n\
             \x20 server_name {names};\n\
             \x20 return 301 https://$host$request_uri;\n\
             }}\n\
             server {{\n\
             \x20 listen 443 ssl;\n\
             \x20 http2 on;\n\
             \x20 server_name {names};\n\
             \x20 ssl_certificate {cert};\n\
             \x20 ssl_certificate_key {key};\n\
             \x20 ssl_protocols TLSv1.2 TLSv1.3;\n\
             \x20 ssl_ciphers HIGH:!aNULL:!MD5;\n\
             \x20 ssl_session_cache shared:SSL:10m;\n\
             \x20 ssl_session_timeout 10m;\n\
             \x20 root {docroot};\n\
             \x20 index index.php index.html;\n\
             \x20 access_log {alog};\n\
             \x20 error_log {elog};\n\
             \x20 location ~ /\\.(?!well-known).* {{ deny all; }}\n\
             {well_known}\
             {build_cache}\
             \x20 location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n\
             \x20 location ~ \\.php$ {{\n\
             \x20   fastcgi_pass unix:{sock};\n\
             \x20   fastcgi_index index.php;\n\
             \x20   include {nginx_etc}/fastcgi_params;\n\
             \x20   fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
             \x20   fastcgi_param HTTPS on;\n\
             \x20 }}\n\
             }}\n",
            names = server_names,
            cert = cert.display(),
            key = key.display(),
            docroot = self.document_root().display(),
            alog = paths.log().join(format!("{}-access.log", self.name)).display(),
            elog = paths.log().join(format!("{}-error.log", self.name)).display(),
            sock = php_socket.display(),
            nginx_etc = paths.etc_for("nginx").display(),
            build_cache = build_cache,
            well_known = WELL_KNOWN_GUARD,
        );
        format!("{local}{}", self.public_vhost_block(paths, php_socket, cert, key))
    }

    /// Nếu có public domains, sinh một server block phục vụ chúng trên CẢ `80`
    /// lẫn `443 ssl` (upstream có thể reverse-proxy vào cổng nào cũng được),
    /// KHÔNG redirect — upstream đã terminate TLS công khai (Let's Encrypt).
    /// Cert mkcert của site (upstream đặt `proxy_ssl_verify off`) dùng cho 443.
    /// Luồng gốc luôn là https nên `HTTPS` được set cứng `on`.
    fn public_vhost_block(
        &self,
        paths: &LaraluxPaths,
        php_socket: &std::path::Path,
        cert: &std::path::Path,
        key: &std::path::Path,
    ) -> String {
        if self.public_domains.is_empty() {
            return String::new();
        }
        let names = self.public_domains.join(" ");
        let alog = paths.log().join(format!("{}-public-access.log", self.name)).display().to_string();
        let elog = paths.log().join(format!("{}-public-error.log", self.name)).display().to_string();

        // Proxy-site: mirror routes. Served on both 80 and 443, no redirect.
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
                     \x20   proxy_set_header X-Forwarded-Proto https;\n",
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
                 \x20 listen 443 ssl;\n\
                 \x20 http2 on;\n\
                 \x20 server_name {names};\n\
                 \x20 ssl_certificate {cert};\n\
                 \x20 ssl_certificate_key {key};\n\
                 \x20 ssl_protocols TLSv1.2 TLSv1.3;\n\
                 \x20 ssl_ciphers HIGH:!aNULL:!MD5;\n\
                 \x20 ssl_session_cache shared:SSL:10m;\n\
                 \x20 ssl_session_timeout 10m;\n\
                 \x20 access_log {alog};\n\
                 \x20 error_log {elog};\n\
                 {locations}\
                 }}\n",
                cert = cert.display(),
                key = key.display(),
            );
        }

        // PHP site.
        let is_laravel = self.root.join("artisan").is_file();
        let build_cache = if is_laravel {
            "\x20 location ^~ /build/ {\n\
             \x20   expires 1y;\n\
             \x20   add_header Cache-Control \"public, immutable\";\n\
             \x20   try_files $uri =404;\n\
             \x20 }\n"
        } else {
            ""
        };
        format!(
            "server {{\n\
             \x20 listen 80;\n\
             \x20 listen 443 ssl;\n\
             \x20 http2 on;\n\
             \x20 server_name {names};\n\
             \x20 ssl_certificate {cert};\n\
             \x20 ssl_certificate_key {key};\n\
             \x20 ssl_protocols TLSv1.2 TLSv1.3;\n\
             \x20 ssl_ciphers HIGH:!aNULL:!MD5;\n\
             \x20 ssl_session_cache shared:SSL:10m;\n\
             \x20 ssl_session_timeout 10m;\n\
             \x20 root {docroot};\n\
             \x20 index index.php index.html;\n\
             \x20 access_log {alog};\n\
             \x20 error_log {elog};\n\
             \x20 location ~ /\\.(?!well-known).* {{ deny all; }}\n\
             {well_known}\
             {build_cache}\
             \x20 location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n\
             \x20 location ~ \\.php$ {{\n\
             \x20   fastcgi_pass unix:{sock};\n\
             \x20   fastcgi_index index.php;\n\
             \x20   include {nginx_etc}/fastcgi_params;\n\
             \x20   fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
             \x20   fastcgi_param HTTPS on;\n\
             \x20 }}\n\
             }}\n",
            cert = cert.display(),
            key = key.display(),
            docroot = self.document_root().display(),
            sock = php_socket.display(),
            nginx_etc = paths.etc_for("nginx").display(),
            well_known = WELL_KNOWN_GUARD,
        )
    }
}

/// Discover sites in `www/`: immediate subdirectories, skipping hidden ones.
pub fn scan_sites(paths: &LaraluxPaths, tld: &str) -> std::io::Result<Vec<Site>> {
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
        let hostname = format!("{name}.{tld}");
        sites.push(Site {
            domains: vec![hostname.clone()],
            hostname,
            root: entry.path(),
            name,
            source: SiteSource::Scanned,
            proxy: None,
            public_domains: Vec::new(),
        });
    }
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sites)
}

/// Merge auto-discovered `www/` sites with the explicit registry.
/// Scanned sites shadow registry entries of the same name; registry entries
/// whose folder is missing are skipped. Returns `(sites, warnings)`.
pub fn list_all_sites(
    paths: &LaraluxPaths,
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
        let hostname = format!("{}.{}", entry.name, tld);
        sites.push(Site {
            domains: vec![hostname.clone()],
            hostname,
            root: entry.root.clone(),
            name: entry.name.clone(),
            source: SiteSource::Linked,
            proxy: None,
            public_domains: Vec::new(),
        });
    }

    for p in registry.proxies() {
        if sites.iter().any(|s| s.name == p.name) {
            warnings.push(format!("proxy site `{}` is shadowed by another site", p.name));
            continue;
        }
        // A proxy's folder is optional and only powers Procfile processes. If it
        // has gone missing, keep serving the route and just warn — unlike a
        // linked site, a proxy must never disappear because of its folder.
        let root = match &p.root {
            Some(r) if r.is_dir() => r.clone(),
            Some(r) => {
                warnings.push(format!(
                    "proxy site `{}`: folder `{}` not found; processes unavailable",
                    p.name,
                    r.display()
                ));
                std::path::PathBuf::new()
            }
            None => std::path::PathBuf::new(),
        };
        let hostname = format!("{}.{}", p.name, tld);
        sites.push(Site {
            domains: vec![hostname.clone()],
            hostname,
            root,
            name: p.name.clone(),
            source: SiteSource::Proxy,
            proxy: Some(ProxySpec { routes: p.routes.clone(), websocket: p.websocket }),
            public_domains: Vec::new(),
        });
    }

    for s in sites.iter_mut() {
        if let Some(over) = registry.domains_for(&s.name) {
            if let Some(first) = over.first() {
                s.domains = over.to_vec();
                s.hostname = first.clone();
            }
        }
    }

    for s in sites.iter_mut() {
        if let Some(pd) = registry.public_domains_for(&s.name) {
            s.public_domains = pd.to_vec();
        }
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

        let paths = LaraluxPaths::new(root.clone());
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

        let paths = LaraluxPaths::new(root.clone());
        let sites = scan_sites(&paths, "dev").unwrap();
        let by = |n: &str| sites.iter().find(|s| s.name == n).unwrap().clone();

        assert_eq!(by("laravelapp").document_root(), www.join("laravelapp").join("public"));
        assert_eq!(by("plain").document_root(), www.join("plain"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_www_returns_empty() {
        let paths = LaraluxPaths::new(temp_root());
        assert!(scan_sites(&paths, "dev").unwrap().is_empty());
    }

    #[test]
    fn vhost_has_https_redirect_ssl_and_fastcgi() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
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
    fn vhost_has_http2_and_ssl_hardening_and_dotfile_deny() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let site = scan_sites(&paths, "dev").unwrap().into_iter().find(|s| s.name == "app").unwrap();

        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");
        let conf = site.vhost_config(&paths, &sock, &cert, &key);

        assert!(conf.contains("http2 on;"));
        assert!(conf.contains("ssl_protocols TLSv1.2 TLSv1.3;"));
        assert!(conf.contains("ssl_ciphers HIGH:!aNULL:!MD5;"));
        assert!(conf.contains("ssl_session_cache shared:SSL:10m;"));
        assert!(conf.contains("location ~ /\\.(?!well-known).* { deny all; }"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn vhost_build_cache_only_for_laravel() {
        let root = temp_root();
        let www = root.join("www");
        // Laravel site: has an `artisan` marker in the project root.
        std::fs::create_dir_all(www.join("lara").join("public")).unwrap();
        std::fs::write(www.join("lara").join("artisan"), "#!/usr/bin/env php\n").unwrap();
        // Plain site: no artisan.
        std::fs::create_dir_all(www.join("plain")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let sites = scan_sites(&paths, "dev").unwrap();
        let by = |n: &str| sites.iter().find(|s| s.name == n).unwrap().clone();

        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("x.pem");
        let key = paths.ssl().join("x-key.pem");

        let lara = by("lara").vhost_config(&paths, &sock, &cert, &key);
        let plain = by("plain").vhost_config(&paths, &sock, &cert, &key);
        assert!(lara.contains("location ^~ /build/ {"), "laravel site should cache /build/");
        assert!(lara.contains("public, immutable"));
        assert!(!plain.contains("/build/"), "non-laravel site must not emit /build/");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn vhost_php_public_block_serves_80_and_443_no_redirect() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let mut site = scan_sites(&paths, "dev").unwrap().into_iter().find(|s| s.name == "app").unwrap();
        site.public_domains = vec!["app.example.com".to_string()];

        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");
        let conf = site.vhost_config(&paths, &sock, &cert, &key);

        // block local cũ vẫn còn (server_name riêng cho domain local)
        assert!(conf.contains("server_name app.dev;"));
        // block public: server_name domain thật, phục vụ cả 80 lẫn 443 ssl
        assert!(conf.contains("server_name app.example.com;"));
        assert!(conf.contains("\x20 listen 80;\n\x20 listen 443 ssl;"));
        // luồng gốc là https (upstream terminate TLS) -> HTTPS set cứng on
        assert!(conf.contains("fastcgi_param HTTPS on;"));
        assert!(!conf.contains("$lara_fwd_https"));
        // public block dùng cert của site cho 443
        assert!(conf.contains(&format!("ssl_certificate {};", cert.display())));
        // local và public là hai server_name tách biệt (không gộp chung)
        assert!(!conf.contains("server_name app.example.com app.dev;"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn php_vhost_blocks_execution_under_well_known() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let mut site = scan_sites(&paths, "dev").unwrap().into_iter().find(|s| s.name == "app").unwrap();
        site.public_domains = vec!["app.example.com".to_string()];

        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");
        let conf = site.vhost_config(&paths, &sock, &cert, &key);

        // Prefix block: `^~` stops nginx reaching the `\.php$` handler at all.
        assert_eq!(conf.matches("location ^~ /.well-known/").count(), 2,
            "cả block local lẫn block public đều phải có guard");
        // Nested denies: without the dotfile one, `^~` would expose /.well-known/.env.
        assert_eq!(conf.matches("location ~ ^/\\.well-known/(.*/)?\\. { deny all; }").count(), 2);
        assert_eq!(conf.matches("location ~ \\.(php|phar|phtml)$ { deny all; }").count(), 2);
        // `.well-known` nested at any depth — `^~` is anchored and misses these.
        assert_eq!(conf.matches("location ~ /\\.well-known/.*\\.(php|phar|phtml)$ { deny all; }").count(), 2);
        // The nested rule must NOT be anchored, or /mcp/.well-known/x.php executes.
        assert!(!conf.contains("location ~ ^/\\.well-known/.*\\.(php|phar|phtml)$"),
            "rule cho .well-known lồng không được neo ^");
        // OAuth/ACME still reach Laravel.
        assert!(conf.contains("try_files $uri $uri/ /index.php?$query_string;"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn proxy_vhost_has_no_well_known_guard() {
        let root = temp_root();
        let paths = LaraluxPaths::new(root.clone());
        let route = crate::site_registry::ProxyRoute { path: "/".into(), upstream: "127.0.0.1:3000".into() };
        let mut site = proxy_site("api", vec![route], true);
        site.public_domains = vec!["api.example.com".to_string()];
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("x.pem");
        let key = paths.ssl().join("x-key.pem");

        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        // A proxy has no root and no PHP handler — there is nothing to protect,
        // and a guard here would only add a confusing dead rule.
        assert!(!conf.contains(".well-known"), "nhánh proxy không được có guard");
    }

    #[test]
    fn vhost_no_public_block_when_public_domains_empty() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let site = scan_sites(&paths, "dev").unwrap().into_iter().find(|s| s.name == "app").unwrap();
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");
        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        assert!(!conf.contains("example.com"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_marks_sites_as_scanned() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("a")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
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
        let paths = LaraluxPaths::new(root.clone());

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
        let paths = LaraluxPaths::new(root.clone());

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
        let paths = LaraluxPaths::new(root.clone());

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
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true, None).unwrap();
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
    fn proxy_site_gets_root_from_registry() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www")).unwrap();
        let proj = root.join("nodeapp");
        std::fs::create_dir_all(&proj).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true, Some(&proj)).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        let api = sites.iter().find(|s| s.name == "api").unwrap();
        assert!(api.root.is_dir(), "proxy root should be populated");
        assert!(warnings.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn proxy_with_missing_folder_is_kept_with_warning() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www")).unwrap();
        let proj = root.join("gone");
        std::fs::create_dir_all(&proj).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true, Some(&proj)).unwrap();
        reg.save(&paths.sites_file()).unwrap();
        // Folder biến mất SAU khi đã đăng ký.
        std::fs::remove_dir_all(&proj).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        // Routing không được sập: site vẫn còn, chỉ mất khả năng chạy process.
        let api = sites.iter().find(|s| s.name == "api").expect("proxy must survive a missing folder");
        assert_eq!(api.source, SiteSource::Proxy);
        assert_eq!(api.root, std::path::PathBuf::new());
        assert!(api.proxy.is_some());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("api"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_scanned_shadows_proxy_of_same_name() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("api")).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true, None).unwrap();
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
            domains: vec![format!("{name}.dev")],
            source: SiteSource::Proxy,
            proxy: Some(ProxySpec { routes, websocket }),
            public_domains: Vec::new(),
        }
    }

    #[test]
    fn proxy_vhost_has_proxy_pass_and_ws_headers() {
        let root = temp_root();
        let paths = LaraluxPaths::new(root.clone());
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
        let paths = LaraluxPaths::new(root.clone());
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

    #[test]
    fn site_has_domains_default_and_override() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("demo")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        // default
        let (sites, _w) = list_all_sites(&paths, "dev").unwrap();
        let d = sites.iter().find(|s| s.name == "demo").unwrap();
        assert_eq!(d.domains, vec!["demo.dev".to_string()]);
        assert_eq!(d.hostname, "demo.dev");
        // override
        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.set_domains("demo", &["demo.dev".to_string(), "*.demo.dev".to_string()]).unwrap();
        reg.save(&paths.sites_file()).unwrap();
        let (sites, _w) = list_all_sites(&paths, "dev").unwrap();
        let d = sites.iter().find(|s| s.name == "demo").unwrap();
        assert_eq!(d.domains, vec!["demo.dev".to_string(), "*.demo.dev".to_string()]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_domain_override_is_ignored() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("demo")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        // Write a sites.toml with an empty domains list for "demo".
        let sites_file = paths.sites_file();
        std::fs::create_dir_all(sites_file.parent().unwrap()).unwrap();
        std::fs::write(&sites_file, "[[domains]]\nname = \"demo\"\ndomains = []\n").unwrap();
        // Should not panic; empty override is silently ignored.
        let (sites, _w) = list_all_sites(&paths, "dev").unwrap();
        let d = sites.iter().find(|s| s.name == "demo").unwrap();
        assert_eq!(d.domains, vec!["demo.dev".to_string()]);
        assert_eq!(d.hostname, "demo.dev");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_populates_public_domains_from_registry() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("demo")).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.set_public_domains("demo", &["app.example.com".to_string()]).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, _w) = list_all_sites(&paths, "dev").unwrap();
        let demo = sites.iter().find(|s| s.name == "demo").unwrap();
        // local domain giữ nguyên
        assert_eq!(demo.domains, vec!["demo.dev".to_string()]);
        // public domain điền từ registry
        assert_eq!(demo.public_domains, vec!["app.example.com".to_string()]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn vhost_server_name_lists_all_domains() {
        let site = Site {
            name: "demo".into(),
            root: std::path::PathBuf::from("/x"),
            hostname: "demo.dev".into(),
            source: SiteSource::Scanned,
            proxy: None,
            domains: vec!["demo.dev".into(), "*.demo.dev".into()],
            public_domains: Vec::new(),
        };
        let paths = LaraluxPaths::new(temp_root());
        let conf = site.vhost_config(&paths, std::path::Path::new("/x/php.sock"),
            std::path::Path::new("/x/c.pem"), std::path::Path::new("/x/k.pem"));
        assert!(conf.contains("server_name demo.dev *.demo.dev;"));
    }

    #[test]
    fn valid_scanned_name_accepts_plain_rejects_unsafe() {
        assert!(valid_scanned_name("myapp"));
        assert!(valid_scanned_name("my-app_2"));
        assert!(!valid_scanned_name(""));
        assert!(!valid_scanned_name("."));
        assert!(!valid_scanned_name(".."));
        assert!(!valid_scanned_name(".hidden"));
        assert!(!valid_scanned_name("a/b"));
        assert!(!valid_scanned_name("a\\b"));
    }

    #[test]
    fn hide_scanned_site_renames_to_dot_prefix() {
        let root = std::env::temp_dir().join(format!("lara-hide-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.www().join("foo")).unwrap();

        hide_scanned_site(&paths, "foo").unwrap();
        assert!(!paths.www().join("foo").exists());
        assert!(paths.www().join(".foo").is_dir());

        // NotFound when the source dir is absent.
        assert!(matches!(hide_scanned_site(&paths, "missing"), Err(SiteFsError::NotFound(_))));
        // InvalidName for a traversing name.
        assert!(matches!(hide_scanned_site(&paths, "../x"), Err(SiteFsError::InvalidName(_))));
        // AlreadyExists when the dot-target is taken.
        std::fs::create_dir_all(paths.www().join("bar")).unwrap();
        std::fs::create_dir_all(paths.www().join(".bar")).unwrap();
        assert!(matches!(hide_scanned_site(&paths, "bar"), Err(SiteFsError::AlreadyExists(_))));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn delete_scanned_site_removes_folder() {
        let root = std::env::temp_dir().join(format!("lara-del-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        std::fs::create_dir_all(paths.www().join("foo").join("public")).unwrap();

        delete_scanned_site(&paths, "foo").unwrap();
        assert!(!paths.www().join("foo").exists());

        assert!(matches!(delete_scanned_site(&paths, "missing"), Err(SiteFsError::NotFound(_))));
        assert!(matches!(delete_scanned_site(&paths, ".."), Err(SiteFsError::InvalidName(_))));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn proxy_site_public_block_uses_http_proxy_pass() {
        let root = temp_root();
        let paths = LaraluxPaths::new(root.clone());
        let route = crate::site_registry::ProxyRoute { path: "/".into(), upstream: "127.0.0.1:3000".into() };
        let mut site = proxy_site("api", vec![route], true);
        site.public_domains = vec!["api.example.com".to_string()];
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("x.pem");
        let key = paths.ssl().join("x-key.pem");

        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        // block public cho proxy-site: server_name domain thật, cả 80 lẫn 443 ssl
        assert!(conf.contains("server_name api.example.com;"));
        assert!(conf.contains("\x20 listen 80;\n\x20 listen 443 ssl;"));
        assert!(conf.contains("proxy_pass http://127.0.0.1:3000;"));
        // upstream đã terminate TLS -> báo app scheme gốc là https
        assert!(conf.contains("proxy_set_header X-Forwarded-Proto https;"));
        // ws headers vẫn có (websocket = true)
        assert!(conf.contains("proxy_set_header Upgrade $http_upgrade;"));
        // public block không có fastcgi
        assert!(!conf.contains("fastcgi_pass"));
    }
}
