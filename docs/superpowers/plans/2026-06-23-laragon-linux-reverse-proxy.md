# Reverse Proxy Sites (Phase 2, Slice 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A reverse-proxy site that `proxy_pass`es to one or more `host:port` upstreams (per-path routes, per-site WebSocket toggle, editable), reachable at `https://<name>.dev` like any site.

**Architecture:** Extend `SiteRegistry` with a separate `proxies` list (`ProxySite { name, websocket, routes }`), merge proxies into `list_all_sites` as `SiteSource::Proxy` carrying a `ProxySpec`, branch `Site::vhost_config` to emit a proxy server block, add the websocket `map` to the generated `nginx.conf`, expose `add_proxy`/`update_proxy` IPC commands (reusing `unlink_site` for removal), and add a Reverse-proxy modal to the frontend.

**Tech Stack:** Rust (laragon-core, zero Tauri deps), Tauri 2, vanilla JS frontend (`dist/`, `withGlobalTauri`).

## Global Constraints

- `core` keeps **zero Tauri deps**.
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD: failing test first, watch it fail, implement, watch it pass, commit.
- Site name rule (valid DNS label): `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`, length 1–63 — reuse `scaffold::validate_site_name` in Rust; reuse the existing JS `validName`/`SITE_NAME_RE`.
- `sites.toml` backward compatibility: an existing file with only `[[sites]]` (no `[[proxies]]`) MUST still load (proxies default to empty).
- Upstream is HTTP only (`proxy_pass http://<host>:<port>;`); https-scheme upstreams are out of scope.
- WebSocket is a **per-site** flag (default on), applied to all routes.
- Run core tests with `cargo test -p laragon-core`; build with `cargo build -p laragon-desktop`. If `cargo` is not on PATH, use `$HOME/.cargo/bin/cargo`. Syntax-check JS with `node --check dist/app.js` (if `node` is missing, use `$HOME/.nvm/versions/node/v24.16.0/bin/node`).

---

### Task 1: Proxy registry model (`ProxySite`, `add_proxy`, `update_proxy`, `remove`)

**Files:**
- Modify: `core/src/site_registry.rs`
- Modify: `core/src/lib.rs` (re-export `ProxyRoute`, `ProxySite`)

**Interfaces:**
- Consumes: `validate_site_name`.
- Produces:
  - `struct ProxyRoute { path: String, upstream: String }` (serde Serialize+Deserialize).
  - `struct ProxySite { name: String, websocket: bool, routes: Vec<ProxyRoute> }` (serde).
  - `SiteRegistry` gains `#[serde(default)] proxies: Vec<ProxySite>` + `proxies() -> &[ProxySite]`.
  - `normalize_upstream(&str) -> Result<String, RegistryError>`, `validate_routes(&[ProxyRoute]) -> Result<Vec<ProxyRoute>, RegistryError>`.
  - `add_proxy(&mut self, name, routes, websocket) -> Result<(), RegistryError>`, `update_proxy(&mut self, name, routes, websocket) -> Result<(), RegistryError>`.
  - `remove(&mut self, name) -> bool` (now removes from `sites` OR `proxies`).
  - New `RegistryError` variants: `InvalidUpstream(String)`, `InvalidRoute(String)`, `NoRoutes`, `NotFound(String)`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/site_registry.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laragon-core site_registry`
Expected: FAIL to compile — `ProxyRoute`, `normalize_upstream`, `validate_routes`, `add_proxy`, `update_proxy`, `proxies` not found.

- [ ] **Step 3: Add the error variants**

In `core/src/site_registry.rs`, add to `enum RegistryError`:

```rust
    #[error("invalid upstream: {0}")]
    InvalidUpstream(String),
    #[error("invalid route: {0}")]
    InvalidRoute(String),
    #[error("a proxy needs at least one route")]
    NoRoutes,
    #[error("not found: {0}")]
    NotFound(String),
```

- [ ] **Step 4: Add the proxy types and the `proxies` field**

In `core/src/site_registry.rs`, after the `RegisteredSite` struct add:

```rust
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
```

And add the field to `SiteRegistry` (keep the existing `sites` field):

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRegistry {
    #[serde(default)]
    sites: Vec<RegisteredSite>,
    #[serde(default)]
    proxies: Vec<ProxySite>,
}
```

- [ ] **Step 5: Add the helpers and methods**

In `core/src/site_registry.rs`, add the free functions (top-level, after the structs):

```rust
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
```

In `impl SiteRegistry`, add the accessor and the two methods, and replace `remove`:

```rust
    pub fn proxies(&self) -> &[ProxySite] {
        &self.proxies
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
```

Replace the existing `remove` method body with one that clears either list:

```rust
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sites.len() + self.proxies.len();
        self.sites.retain(|s| s.name != name);
        self.proxies.retain(|p| p.name != name);
        self.sites.len() + self.proxies.len() != before
    }
```

- [ ] **Step 6: Re-export in lib.rs**

In `core/src/lib.rs`, change the registry re-export line to:

```rust
pub use site_registry::{ProxyRoute, ProxySite, RegisteredSite, RegistryError, SiteRegistry};
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p laragon-core site_registry`
Expected: PASS — the 5 new tests plus the existing registry tests.

- [ ] **Step 8: Commit**

```bash
git add core/src/site_registry.rs core/src/lib.rs
git commit -m "feat(core): add reverse-proxy entries to the site registry"
```

---

### Task 2: `SiteSource::Proxy` + `Site.proxy` + merge in `list_all_sites`

**Files:**
- Modify: `core/src/sites.rs`
- Modify: `core/src/lib.rs` (re-export `ProxySpec`)

**Interfaces:**
- Consumes: `SiteRegistry::proxies`, `ProxyRoute`.
- Produces:
  - `SiteSource` gains `Proxy`.
  - `struct ProxySpec { routes: Vec<crate::site_registry::ProxyRoute>, websocket: bool }` (serde Serialize).
  - `Site` gains `pub proxy: Option<ProxySpec>`.
  - `list_all_sites` appends proxy sites (source `Proxy`, `proxy: Some`).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/sites.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laragon-core sites`
Expected: FAIL to compile — `SiteSource::Proxy`, `proxy` field, `ProxySpec` not found.

- [ ] **Step 3: Add `SiteSource::Proxy`, `ProxySpec`, and the `proxy` field**

In `core/src/sites.rs`, extend `SiteSource`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum SiteSource {
    Scanned,
    Linked,
    Proxy,
}
```

Add the `ProxySpec` type (above `struct Site`):

```rust
/// The proxy view of a site, sent to the frontend (routes + websocket flag).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProxySpec {
    pub routes: Vec<crate::site_registry::ProxyRoute>,
    pub websocket: bool,
}
```

Add the field to `Site`:

```rust
pub struct Site {
    pub name: String,
    pub root: PathBuf,
    pub hostname: String,
    pub source: SiteSource,
    pub proxy: Option<ProxySpec>,
}
```

- [ ] **Step 4: Set `proxy: None` in `scan_sites` and the linked push, then append proxies**

In `scan_sites`, the `sites.push(Site { ... })` gains `proxy: None`:

```rust
        sites.push(Site {
            hostname: format!("{name}.{tld}"),
            root: entry.path(),
            name,
            source: SiteSource::Scanned,
            proxy: None,
        });
```

In `list_all_sites`, the existing linked-site push gains `proxy: None`:

```rust
        sites.push(Site {
            hostname: format!("{}.{}", entry.name, tld),
            root: entry.root.clone(),
            name: entry.name.clone(),
            source: SiteSource::Linked,
            proxy: None,
        });
```

Then, in `list_all_sites`, **after** the folder-entry loop and **before** the final `sites.sort_by(...)`, append proxies:

```rust
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
```

- [ ] **Step 5: Re-export `ProxySpec`**

In `core/src/lib.rs`, change the sites re-export line to:

```rust
pub use sites::{list_all_sites, scan_sites, ProxySpec, Site, SiteSource};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p laragon-core sites`
Expected: PASS — the 2 new tests plus existing `sites` tests (existing tests don't assert `proxy`).

- [ ] **Step 7: Commit**

```bash
git add core/src/sites.rs core/src/lib.rs
git commit -m "feat(core): merge proxy sites into list_all_sites (SiteSource::Proxy)"
```

---

### Task 3: Proxy vhost generation in `Site::vhost_config`

**Files:**
- Modify: `core/src/sites.rs`

**Interfaces:**
- Consumes: `Site.proxy: Option<ProxySpec>`.
- Produces: `Site::vhost_config` emits a proxy server block when `proxy` is `Some`; otherwise the existing PHP/static block.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/sites.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laragon-core sites`
Expected: FAIL — `vhost_config` currently emits the PHP block (no `proxy_pass`), so the proxy assertions fail.

- [ ] **Step 3: Branch `vhost_config` for proxy sites**

In `core/src/sites.rs`, at the very top of `vhost_config` (before the existing PHP `format!`), add the proxy branch:

```rust
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
```

(The existing PHP/static `format!` remains as the fall-through for non-proxy sites. `php_socket` stays used by that branch.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laragon-core sites`
Expected: PASS — proxy vhost tests green; the existing `vhost_has_https_redirect_ssl_and_fastcgi` test still passes (non-proxy site).

- [ ] **Step 5: Commit**

```bash
git add core/src/sites.rs
git commit -m "feat(core): emit reverse-proxy vhost (per-route proxy_pass + optional websocket)"
```

---

### Task 4: WebSocket upgrade `map` in generated `nginx.conf`

**Files:**
- Modify: `core/src/service/nginx.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: the generated `nginx.conf` http block contains `map $http_upgrade $connection_upgrade { default upgrade; '' close; }`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/service/nginx.rs`:

```rust
    #[test]
    fn write_config_includes_websocket_map() {
        let tmp = std::env::temp_dir().join(format!("lara-nginx-ws-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        let svc = NginxService::new(p.tmp().join("php-fpm.sock"));
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("nginx").join("nginx.conf")).unwrap();
        assert!(conf.contains("map $http_upgrade $connection_upgrade"));
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core nginx`
Expected: FAIL — the generated config has no `map` directive yet.

- [ ] **Step 3: Add the map to the http block**

In `core/src/service/nginx.rs` `write_config`, in the `conf` format string, insert the map line immediately after `http {{\n` (before the `access_log` line):

```rust
             http {{\n\
             \x20 map $http_upgrade $connection_upgrade {{ default upgrade; '' close; }}\n\
             \x20 access_log {acclog};\n\
```

(The rest of the format string and its arguments are unchanged.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laragon-core nginx`
Expected: PASS — the new test plus the existing nginx tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/service/nginx.rs
git commit -m "feat(core): add websocket upgrade map to nginx.conf http block"
```

---

### Task 5: IPC commands `add_proxy` / `update_proxy`

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `SiteRegistry::{add_proxy, update_proxy}`, `ProxyRoute`, `list_all_sites`, `sync_sites`.
- Produces:
  - `add_proxy(app, name: String, routes: Vec<ProxyRoute>, websocket: bool) -> Result<Site, String>` (async).
  - `update_proxy(app, name: String, routes: Vec<ProxyRoute>, websocket: bool) -> Result<Site, String>` (async).
  - private helper `sync_and_reload(state, config)` shared by the two new commands.

- [ ] **Step 1: Extend the core imports**

In `src-tauri/src/commands.rs`, update the first `use laragon_core::{...}` block to also import `ProxyRoute`:

```rust
use laragon_core::{
    build_services, create_site as core_create_site, detect_components, list_all_sites,
    run_setup, sync_sites, Config, CreateReport, LaragonPaths, MkcertIssuer, Orchestrator,
    PkexecPrivileged, ProxyRoute, RealCommandRunner, RealSpawner, ServiceKind, ServiceState,
    ServiceStatus, Site, SiteRegistry, SiteTemplate,
};
```

(If a name is reported unused after the change, leave the others as-is; `ProxyRoute`, `SiteRegistry`, `list_all_sites` are all used by the new commands.)

- [ ] **Step 2: Add the shared `sync_and_reload` helper**

Append to `src-tauri/src/commands.rs` (a private fn, not a command):

```rust
/// Re-sync vhosts/certs/hosts and reload nginx if it is running. Best-effort,
/// matching `link_site`/`create_site` (a sync failure must not fail the call).
fn sync_and_reload(state: &AppState, config: &Config) {
    let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
    let issuer = MkcertIssuer::new(state.paths.ssl());
    let privileged = PkexecPrivileged;
    let _ = sync_sites(
        &state.paths,
        &config.tld,
        &php_socket,
        std::path::Path::new("/etc/hosts"),
        &issuer,
        &privileged,
    );
    if let Ok(mut orch) = state.orch.lock() {
        if orch.state(ServiceKind::Nginx) == ServiceState::Running {
            let _ = orch.stop(ServiceKind::Nginx);
            let _ = orch.start(ServiceKind::Nginx);
        }
    }
}
```

- [ ] **Step 3: Add the `add_proxy` command**

Append to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn add_proxy(
    app: tauri::AppHandle,
    name: String,
    routes: Vec<ProxyRoute>,
    websocket: bool,
) -> Result<Site, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Site, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry.add_proxy(&name, &routes, websocket).map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        sync_and_reload(&state, &config);

        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        sites
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("proxy `{name}` not found after sync"))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 4: Add the `update_proxy` command**

Append to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn update_proxy(
    app: tauri::AppHandle,
    name: String,
    routes: Vec<ProxyRoute>,
    websocket: bool,
) -> Result<Site, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Site, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry.update_proxy(&name, &routes, websocket).map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        sync_and_reload(&state, &config);

        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        sites
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("proxy `{name}` not found after sync"))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 5: Register the commands in main.rs**

In `src-tauri/src/main.rs`, add to the `generate_handler!` list after `commands::unlink_site,`:

```rust
            commands::add_proxy,
            commands::update_proxy,
```

- [ ] **Step 6: Build the app**

Run: `cargo build -p laragon-desktop`
Expected: PASS — compiles cleanly. If any import is unused, remove it to keep the build warning-clean.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): add add_proxy/update_proxy commands"
```

---

### Task 6: Frontend — Reverse-proxy modal (create + edit), badge, routes

**Files:**
- Modify: `dist/app.js`
- Modify: `dist/styles.css`

**Interfaces:**
- Consumes: `add_proxy({name, routes, websocket})`, `update_proxy({name, routes, websocket})`, `unlink_site({name})`, `list_sites` (now returns `source: "Proxy"` and `proxy: {routes, websocket}`), `openExternal`.
- Produces: UI only.

The existing New Site / Add-existing modals (`ns-*` classes, `state.modal`, delegated handlers) are the template; add a parallel `state.modal === "proxy"` branch.

- [ ] **Step 1: Add proxy state and the open/close/route helpers**

In `dist/app.js`, in the `state` object near `linkSite`, add:

```js
    proxy: { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" },
```

After `submitLinkSite` (or near the link helpers), add:

```js
  function openProxy(site) {
    if (site && site.proxy) {
      state.proxy = {
        mode: "edit", name: site.name, websocket: !!site.proxy.websocket,
        routes: (site.proxy.routes || []).map((r) => ({ path: r.path, upstream: r.upstream })),
        busy: false, error: "",
      };
      if (!state.proxy.routes.length) state.proxy.routes = [{ path: "/", upstream: "" }];
    } else {
      state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" };
    }
    state.modal = "proxy";
    render();
    requestAnimationFrame(() => { const inp = document.getElementById("px-name"); if (inp && !inp.readOnly) inp.focus(); });
  }

  function closeProxy() {
    if (state.proxy.busy) return;
    state.modal = null;
    render();
  }

  function addProxyRoute() { state.proxy.routes.push({ path: "", upstream: "" }); render(); }
  function delProxyRoute(i) { state.proxy.routes.splice(i, 1); if (!state.proxy.routes.length) state.proxy.routes.push({ path: "/", upstream: "" }); render(); }

  async function submitProxy() {
    const p = state.proxy;
    if (!validName(p.name)) { p.error = "Use lowercase letters, digits, hyphens (e.g. my-app)"; render(); return; }
    if (!p.routes.length) { p.error = "Add at least one route"; render(); return; }
    for (const r of p.routes) {
      if (!r.path.startsWith("/")) { p.error = "Each path must start with /"; render(); return; }
      if (!String(r.upstream).trim()) { p.error = "Each route needs a target (host:port)"; render(); return; }
    }
    p.busy = true; p.error = ""; render();
    try {
      const cmd = p.mode === "edit" ? "update_proxy" : "add_proxy";
      const site = await invoke(cmd, {
        name: p.name, websocket: p.websocket,
        routes: p.routes.map((r) => ({ path: r.path, upstream: r.upstream })),
      });
      toast({ type: "success", title: (p.mode === "edit" ? "Updated " : "Proxy ") + site.name, msg: "https://" + site.hostname });
      state.modal = null;
      state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" };
      try { const sites = await invoke("list_sites"); state.sites = Array.isArray(sites) ? sites : []; } catch (_) {}
      render();
    } catch (e) {
      p.error = String(e); p.busy = false;
      toast({ type: "error", title: "Proxy failed", msg: String(e) });
      render();
    } finally {
      if (p.busy) { p.busy = false; render(); }
    }
  }
```

- [ ] **Step 2: Add the "Reverse proxy" button and proxy-aware rows in `sitesView`**

In `sitesView`, change the header actions block to include the proxy button:

```js
      '<div class="sites-actions">' +
      '<button class="btn-newsite ghost" data-action="proxy-site">' + I.navSites + "Reverse proxy</button>" +
      '<button class="btn-newsite ghost" data-action="link-site">' + I.folder18 + "Add existing folder</button>" +
      '<button class="btn-newsite" data-action="new-site">' + I.plus + "New site</button></div></div>";
```

Replace the non-empty row `.map((s) => {...})` body with one that handles all three sources:

```js
          .map((s) => {
            const url = "https://" + s.hostname;
            const isProxy = s.source === "Proxy";
            const isLinked = s.source === "Linked";
            const target = isProxy && s.proxy && s.proxy.routes && s.proxy.routes.length
              ? s.proxy.routes[0].upstream + (s.proxy.routes.length > 1 ? " +" + (s.proxy.routes.length - 1) : "")
              : "";
            const badge = isProxy
              ? '<span class="site-badge">proxy → ' + esc(target) + "</span>"
              : (isLinked ? '<span class="site-badge">linked</span>' : "");
            const subRight = isProxy ? "" : '<span class="site-root" title="' + esc(s.root) + '">' + esc(s.root) + "</span>";
            const editBtn = isProxy
              ? '<button class="btn-sm" data-action="edit-proxy" data-name="' + esc(s.name) + '">Edit</button>'
              : "";
            const removeBtn = (isProxy || isLinked)
              ? '<button class="btn-sm danger" data-action="remove-site" data-name="' + esc(s.name) + '">' +
                (state.confirmRemove === s.name ? "Confirm?" : "Remove") + "</button>"
              : "";
            return (
              '<div class="card site-row"><div class="site-tile">' + I.folder18 + "</div>" +
              '<div class="site-info"><div class="site-name">' + esc(s.name) + "</div>" +
              '<div class="site-sub"><a class="site-url" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + esc(url) + "</a>" +
              subRight + "</div></div>" +
              badge +
              '<button class="icon-btn sq32" data-action="copy-site" data-name="' + esc(s.name) + '" aria-label="Copy URL">' + I.copy + "</button>" +
              editBtn + removeBtn +
              '<a class="btn-sm" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + I.external + "Open</a></div>"
            );
          })
```

- [ ] **Step 3: Add the proxy modal renderer**

After `linkSiteModal` (or `newSiteModal`), add:

```js
  function proxyModal() {
    const p = state.proxy;
    const ok = validName(p.name) && p.routes.length > 0;
    const isEdit = p.mode === "edit";
    const preview = p.name ? '<span class="ns-preview">→ https://' + esc(p.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
    const errorHtml = p.error ? '<div class="ns-error">' + esc(p.error) + '</div>' : '';
    const d = p.busy ? ' disabled' : '';
    const rows = p.routes.map((r, i) =>
      '<div class="pr-row">' +
      '<input class="ns-input pr-path" type="text" placeholder="/" value="' + esc(r.path) + '" autocomplete="off" spellcheck="false" data-action="pr-path" data-idx="' + i + '"' + d + ' />' +
      '<input class="ns-input pr-up" type="text" placeholder="3000 or 127.0.0.1:5173" value="' + esc(r.upstream) + '" autocomplete="off" spellcheck="false" data-action="pr-upstream" data-idx="' + i + '"' + d + ' />' +
      (p.routes.length > 1 ? '<button class="icon-btn sq32" data-action="pr-del" data-idx="' + i + '" aria-label="Remove route"' + d + '>' + I.close + '</button>' : '') +
      '</div>'
    ).join('');
    const submitLabel = p.busy
      ? '<span class="spin spinner on-primary"></span>' + (isEdit ? 'Saving…' : 'Creating…')
      : (isEdit ? 'Save' : 'Create proxy');
    return (
      '<div class="ns-overlay" data-action="px-overlay-click" role="dialog" aria-modal="true" aria-labelledby="px-title">' +
      '<div class="ns-card" role="document">' +
      '<div class="ns-head"><h2 class="ns-title" id="px-title">' + (isEdit ? 'Edit reverse proxy' : 'Reverse proxy') + '</h2>' +
      '<button class="icon-btn" data-action="px-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
      '<div class="ns-body">' +
      '<label class="ns-label" for="px-name">Site name</label>' +
      '<input class="ns-input" type="text" id="px-name" placeholder="my-app" value="' + esc(p.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' + (isEdit ? ' readonly' : '') + d + ' data-action="px-name-input" />' +
      preview +
      '<label class="ns-label">Routes</label>' +
      rows +
      '<button class="link-btn" data-action="pr-add"' + d + '>+ Add route</button>' +
      '<label class="ns-check"><input type="checkbox" data-action="px-ws"' + (p.websocket ? ' checked' : '') + d + ' /> WebSocket support</label>' +
      errorHtml +
      '</div>' +
      '<div class="ns-foot">' +
      '<button class="btn btn-outline" data-action="px-close"' + d + '>Cancel</button>' +
      '<button class="btn btn-primary' + (!ok || p.busy ? ' btn-dim' : '') + '" data-action="px-submit"' + (!ok || p.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
      '</div></div></div>'
    );
  }
```

- [ ] **Step 4: Wire the proxy modal into `render` and the event handlers**

In `render`, extend the modal selection:

```js
    const modalHtml = state.modal === "newsite" ? newSiteModal()
      : state.modal === "linksite" ? linkSiteModal()
      : state.modal === "proxy" ? proxyModal()
      : "";
```

In the click handler, add cases (after the `ls-*` cases):

```js
    else if (a === "proxy-site") openProxy();
    else if (a === "edit-proxy") openProxy(state.sites.find((s) => s.name === el.getAttribute("data-name")));
    else if (a === "px-close") closeProxy();
    else if (a === "px-submit") submitProxy();
    else if (a === "pr-add") addProxyRoute();
    else if (a === "pr-del") delProxyRoute(parseInt(el.getAttribute("data-idx"), 10));
    else if (a === "px-overlay-click") { if (e.target === el) closeProxy(); }
```

In the `input` handler, add (after the `ls-*` blocks):

```js
    if (el.dataset.action === "px-name-input") {
      state.proxy.name = el.value;
      state.proxy.error = "";
      const preview = document.querySelector(".ns-preview");
      if (preview) {
        if (el.value) { preview.classList.remove("muted"); preview.textContent = "→ https://" + el.value + ".dev"; }
        else { preview.classList.add("muted"); preview.innerHTML = "→ https://&lt;name&gt;.dev"; }
      }
      const submitBtn = document.querySelector('[data-action="px-submit"]');
      if (submitBtn) { const ok = validName(el.value) && state.proxy.routes.length > 0; submitBtn.disabled = !ok; submitBtn.classList.toggle("btn-dim", !ok); }
    }
    if (el.dataset.action === "pr-path") { state.proxy.routes[parseInt(el.dataset.idx, 10)].path = el.value; }
    if (el.dataset.action === "pr-upstream") { state.proxy.routes[parseInt(el.dataset.idx, 10)].upstream = el.value; }
```

In the `change` handler (where `ns-template-change` is handled), add:

```js
    if (el.dataset.action === "px-ws") { state.proxy.websocket = el.checked; }
```

In the Esc handler, add:

```js
    else if (e.key === "Escape" && state.modal === "proxy") closeProxy();
```

In the focus-trap guard, extend the condition to include the proxy modal:

```js
    if (e.key !== "Tab" || (state.modal !== "newsite" && state.modal !== "linksite" && state.modal !== "proxy")) return;
```

- [ ] **Step 5: Add CSS for route rows and the checkbox**

Append to `dist/styles.css`:

```css
.pr-row { display:flex; gap:8px; align-items:center; margin-bottom:6px; }
.pr-row .pr-path { flex:0 0 96px; }
.pr-row .pr-up { flex:1 1 auto; min-width:0; }
.ns-check { display:flex; align-items:center; gap:8px; font-size:13px; margin-top:10px; cursor:pointer; }
```

- [ ] **Step 6: Syntax-check the JS**

Run: `node --check dist/app.js`
Expected: PASS — exit 0, no output. (If `node` is missing, use `$HOME/.nvm/versions/node/v24.16.0/bin/node --check dist/app.js`.)

- [ ] **Step 7: Manual verification (live)**

Run: `cargo run -p laragon-desktop`, then **Sites → Reverse proxy**: enter a name, a target (e.g. `3000`), optionally add a second route (`/api` → `3001`), toggle WebSocket, **Create proxy** → confirm the toast and a row with badge "proxy → 127.0.0.1:3000". Click **Edit**, change the target, **Save** → badge updates. Click **Remove → Confirm?** → the row disappears. (No JS test runner — this step is human-verified.)

- [ ] **Step 8: Commit**

```bash
git add dist/
git commit -m "feat(desktop): reverse-proxy modal (create/edit), proxy badge, routes"
```

---

## Self-Review

**1. Spec coverage:**
- §3.1 proxy registry model (ProxyRoute/ProxySite, normalize_upstream, validate_routes, add_proxy/update_proxy/remove, error variants) → Task 1. ✓
- §3.2 SiteSource::Proxy + Site.proxy + ProxySpec + list_all_sites merge (shadow + warning) → Task 2. ✓
- §3.2/§4 proxy vhost (per-route proxy_pass, forwarded headers, websocket on/off, multi-route) → Task 3. ✓
- §3.3 nginx.conf websocket map → Task 4. ✓
- §3.4 IPC add_proxy/update_proxy (+ reuse unlink_site) → Task 5. ✓
- §3.5 frontend (Reverse proxy button, modal create+edit with locked name, websocket checkbox, dynamic routes, badge "proxy → upstream", Edit/Remove) → Task 6. ✓
- §5 error handling (typed RegistryError → toast; NotFound on update) → Task 1 errors + Task 5 mapping + Task 6 toasts. ✓
- §6 testing → Tasks 1–4 unit tests; Task 6 node-check + manual. ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows full code. The only manual step (Task 6 Step 7) is an explicit human-verified UI gate (no JS test runner exists in this repo).

**3. Type consistency:**
- `ProxyRoute { path, upstream }` (Task 1) is used identically in Tasks 2/3/5 and the JS `routes` array (Task 6). ✓
- `ProxySite { name, websocket, routes }` (Task 1) consumed by `list_all_sites` (Task 2). ✓
- `ProxySpec { routes, websocket }` (Task 2) consumed by `vhost_config` (Task 3) and serialized to JS `site.proxy` (Task 6). ✓
- `add_proxy/update_proxy` IPC arg names `{name, routes, websocket}` match between Task 5 (Rust params) and Task 6 (`invoke`). ✓
- `SiteSource::Proxy` (Task 2) compared as `s.source === "Proxy"` in JS (Task 6). ✓
- `remove` extended in Task 1 is what `unlink_site` (existing) calls to delete a proxy — no command change needed. ✓

**Note:** Task 6 mutates `state.proxy.routes[idx]` from the input handler without a full re-render (to preserve focus); the submit button's enabled state only depends on name + route *count* (which change only via add/del route, both of which re-render), so live route-field typing correctly does not need to touch the button.
