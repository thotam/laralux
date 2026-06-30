# Nginx Config Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make laralux's generated nginx config serve correct MIME types (fixing the lost-CSS/JS bug) and apply the P1/P2 correctness, security, and performance improvements from `NGINX-FIX.md`.

**Architecture:** All changes live in the two Rust functions that generate nginx config — `NginxService::write_config` (global `http{}` block, `mime.types`, `fastcgi_params`, default server) and `Site::vhost_config` (per-site `sites/*.conf`). Config bodies are embedded Rust string constants/`format!`; no new runtime deps, no shipped sidecar files, no downloads.

**Tech Stack:** Rust (laralux-core), `cargo test`, nginx 1.31.2 (built `--with-http_v2_module`).

## Global Constraints

- No `apt`, no network fetch, no packaged sidecar files — config is generated from embedded Rust strings (mirrors existing `fastcgi_params` write).
- `include mime.types;` MUST appear before `default_type` and before any `server`/`include` in `http{}`.
- HTTP/2 uses the `http2 on;` directive (nginx ≥ 1.25.1), NOT the deprecated `listen 443 ssl http2;`.
- Laravel detection is structural: a site is Laravel iff `<site root>/artisan` is a file. The `SiteTemplate` kind is not persisted and must not be used.
- Indentation in generated config uses the existing `\x20` leading-space convention; keep it consistent with surrounding lines.
- Keep all existing tests passing.

---

### Task 1: `http{}` block + `mime.types` (P0 bug + P1 charset + P2 perf/gzip + default-server dotfile)

**Files:**
- Modify: `core/src/service/nginx.rs` (add `MIME_TYPES` const; rewrite the `http{}` `format!` in `write_config`; add a `mime.types` file write; add dotfile-deny to the default server)
- Test: `core/src/service/nginx.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `LaraluxPaths::etc_for`, `::tmp`, `::log`, `::www` (unchanged).
- Produces: `write_config` now also writes `<etc>/nginx/mime.types`; `nginx.conf` `http{}` gains `include …/mime.types`, `charset utf-8`, `sendfile/tcp_nopush/tcp_nodelay/keepalive_timeout/server_tokens`, a `gzip` block; the default server gains a dotfile-deny location.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `core/src/service/nginx.rs`:

```rust
    #[test]
    fn write_config_emits_mime_types_and_http_hardening() {
        let tmp = std::env::temp_dir().join(format!("lara-nginx-mime-{}", std::process::id()));
        let p = LaraluxPaths::new(tmp.clone());
        let svc = NginxService::new(p.tmp().join("php-fpm.sock"));
        svc.write_config(&p).unwrap();

        let conf = std::fs::read_to_string(p.etc_for("nginx").join("nginx.conf")).unwrap();
        let etc = p.etc_for("nginx");
        assert!(conf.contains(&format!("include {}/mime.types;", etc.display())));
        // mime include must precede default_type and the server block.
        assert!(conf.find("mime.types").unwrap() < conf.find("default_type").unwrap());
        assert!(conf.find("mime.types").unwrap() < conf.find("server {").unwrap());
        assert!(conf.contains("charset utf-8;"));
        assert!(conf.contains("sendfile on;"));
        assert!(conf.contains("server_tokens off;"));
        assert!(conf.contains("gzip on;"));
        assert!(conf.contains("location ~ /\\.(?!well-known).* { deny all; }"));

        let mime = std::fs::read_to_string(etc.join("mime.types")).unwrap();
        assert!(mime.contains("text/css"));
        assert!(mime.contains("application/javascript"));
        assert!(mime.contains("js mjs;"));
        assert!(mime.contains("font/woff2"));
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laralux-core write_config_emits_mime_types_and_http_hardening`
Expected: FAIL (no `mime.types`, no `charset`/`gzip` in conf).

- [ ] **Step 3: Write minimal implementation**

In `core/src/service/nginx.rs`, add this const near the top (after the `use` lines):

```rust
/// Minimal, modern `mime.types`. Without it nginx serves every static file as
/// `default_type` (octet-stream) and browsers reject CSS and ES modules.
/// Trimmed from nginx's stock map; covers a modern web app (woff2, wasm, svg, mjs).
const MIME_TYPES: &str = r#"types {
    text/html                             html htm shtml;
    text/css                              css;
    text/xml                              xml;
    text/plain                            txt;
    image/gif                             gif;
    image/jpeg                            jpeg jpg;
    image/png                             png;
    image/svg+xml                         svg svgz;
    image/webp                            webp;
    image/avif                            avif;
    image/x-icon                          ico;
    image/tiff                            tif tiff;
    application/javascript                js mjs;
    application/json                      json map;
    application/ld+json                   jsonld;
    application/manifest+json             webmanifest;
    application/wasm                      wasm;
    application/pdf                       pdf;
    application/atom+xml                  atom;
    application/rss+xml                   rss;
    application/zip                       zip;
    font/woff                             woff;
    font/woff2                            woff2;
    font/ttf                              ttf;
    font/otf                              otf;
    application/vnd.ms-fontobject         eot;
    audio/mpeg                            mp3;
    audio/ogg                             ogg;
    video/mp4                             mp4;
    video/webm                            webm;
    application/octet-stream              bin exe dll iso img;
}
"#;
```

Replace the `let conf = format!( … );` block in `write_config` with:

```rust
        let conf = format!(
            "worker_processes auto;\n\
             pid {pid};\n\
             error_log {errlog};\n\
             events {{ worker_connections 1024; }}\n\
             http {{\n\
             \x20 include {nginx_etc}/mime.types;\n\
             \x20 default_type application/octet-stream;\n\
             \x20 charset utf-8;\n\
             \x20 sendfile on;\n\
             \x20 tcp_nopush on;\n\
             \x20 tcp_nodelay on;\n\
             \x20 keepalive_timeout 65;\n\
             \x20 server_tokens off;\n\
             \x20 gzip on;\n\
             \x20 gzip_vary on;\n\
             \x20 gzip_comp_level 5;\n\
             \x20 gzip_min_length 256;\n\
             \x20 gzip_types text/plain text/css application/javascript application/json image/svg+xml application/xml font/ttf font/otf;\n\
             \x20 map $http_upgrade $connection_upgrade {{ default upgrade; '' close; }}\n\
             \x20 access_log {acclog};\n\
             \x20 client_body_temp_path {tmp}/nginx-client;\n\
             \x20 proxy_temp_path {tmp}/nginx-proxy;\n\
             \x20 fastcgi_temp_path {tmp}/nginx-fastcgi;\n\
             \x20 server {{\n\
             \x20   listen {port};\n\
             \x20   server_name localhost;\n\
             \x20   root {www};\n\
             \x20   index index.php index.html;\n\
             \x20   location ~ /\\.(?!well-known).* {{ deny all; }}\n\
             \x20   location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n\
             \x20   location ~ \\.php$ {{\n\
             \x20     fastcgi_pass unix:{sock};\n\
             \x20     fastcgi_index index.php;\n\
             \x20     include {nginx_etc}/fastcgi_params;\n\
             \x20     fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
             \x20   }}\n\
             \x20 }}\n\
             \x20 include {nginx_etc}/sites/*.conf;\n\
             }}\n",
            pid = paths.tmp().join("nginx.pid").display(),
            errlog = paths.log().join("nginx-error.log").display(),
            acclog = paths.log().join("nginx-access.log").display(),
            tmp = paths.tmp().display(),
            port = self.http_port,
            www = paths.www().display(),
            sock = self.php_socket.display(),
            nginx_etc = paths.etc_for("nginx").display(),
        );
        std::fs::write(self.conf_path(paths), conf)?;
        // mime.types so the http-level include resolves (P0: correct MIME for assets).
        std::fs::write(paths.etc_for("nginx").join("mime.types"), MIME_TYPES)?;
```

Leave the existing `fastcgi_params` write below as-is (Task 2 replaces it).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laralux-core write_config_emits_mime_types_and_http_hardening`
Expected: PASS. Also run the existing nginx tests: `cargo test -p laralux-core nginx` → all PASS (websocket map, sites include, fastcgi_pass wiring still match).

- [ ] **Step 5: Commit**

```bash
git add core/src/service/nginx.rs
git commit -m "fix(nginx): ship mime.types + include (P0) and http hardening (charset/gzip/perf)"
```

---

### Task 2: full `fastcgi_params` + httpoxy (P1)

**Files:**
- Modify: `core/src/service/nginx.rs` (add `FASTCGI_PARAMS` const; replace the `fastcgi_params` file write in `write_config`)
- Test: `core/src/service/nginx.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `<etc>/nginx/fastcgi_params` now contains the full FastCGI param set plus `HTTP_PROXY ""`. All PHP locations that `include …/fastcgi_params` (default server + every site) inherit it.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `core/src/service/nginx.rs`:

```rust
    #[test]
    fn fastcgi_params_is_full_and_blocks_httpoxy() {
        let tmp = std::env::temp_dir().join(format!("lara-nginx-fcgi-{}", std::process::id()));
        let p = LaraluxPaths::new(tmp.clone());
        let svc = NginxService::new(p.tmp().join("php-fpm.sock"));
        svc.write_config(&p).unwrap();

        let f = std::fs::read_to_string(p.etc_for("nginx").join("fastcgi_params")).unwrap();
        for needle in [
            "REDIRECT_STATUS", "REQUEST_SCHEME", "HTTPS", "SERVER_PORT",
            "SERVER_ADDR", "REMOTE_PORT", "SCRIPT_NAME", "SERVER_SOFTWARE",
        ] {
            assert!(f.contains(needle), "missing fastcgi_param {needle}");
        }
        // httpoxy: HTTP_PROXY forced empty.
        assert!(f.contains("HTTP_PROXY"));
        assert!(f.contains("\"\""));
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laralux-core fastcgi_params_is_full_and_blocks_httpoxy`
Expected: FAIL (current file lacks REDIRECT_STATUS, HTTP_PROXY, etc.).

- [ ] **Step 3: Write minimal implementation**

In `core/src/service/nginx.rs`, add this const (next to `MIME_TYPES`):

```rust
/// Full FastCGI param set PHP-FPM expects, plus the httpoxy guard
/// (`HTTP_PROXY ""`). `HTTPS` uses `if_not_empty` so it is only set on TLS
/// connections; site 443 blocks additionally force `HTTPS on`.
const FASTCGI_PARAMS: &str = r#"fastcgi_param  QUERY_STRING       $query_string;
fastcgi_param  REQUEST_METHOD     $request_method;
fastcgi_param  CONTENT_TYPE       $content_type;
fastcgi_param  CONTENT_LENGTH     $content_length;

fastcgi_param  SCRIPT_NAME        $fastcgi_script_name;
fastcgi_param  REQUEST_URI        $request_uri;
fastcgi_param  DOCUMENT_URI       $document_uri;
fastcgi_param  DOCUMENT_ROOT      $document_root;
fastcgi_param  SERVER_PROTOCOL    $server_protocol;
fastcgi_param  REQUEST_SCHEME     $scheme;
fastcgi_param  HTTPS              $https if_not_empty;

fastcgi_param  GATEWAY_INTERFACE  CGI/1.1;
fastcgi_param  SERVER_SOFTWARE    nginx/$nginx_version;

fastcgi_param  REMOTE_ADDR        $remote_addr;
fastcgi_param  REMOTE_PORT        $remote_port;
fastcgi_param  SERVER_ADDR        $server_addr;
fastcgi_param  SERVER_PORT        $server_port;
fastcgi_param  SERVER_NAME        $server_name;

fastcgi_param  REDIRECT_STATUS    200;

fastcgi_param  HTTP_PROXY         "";
"#;
```

Replace the existing `// Provide a minimal fastcgi_params …` write with:

```rust
        // Full fastcgi_params (PHP-FPM expects these; httpoxy guarded here so it
        // covers the default server and every site PHP location at once).
        std::fs::write(
            paths.etc_for("nginx").join("fastcgi_params"),
            FASTCGI_PARAMS,
        )?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laralux-core fastcgi_params_is_full_and_blocks_httpoxy`
Expected: PASS. Re-run `cargo test -p laralux-core nginx` → all PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/service/nginx.rs
git commit -m "feat(nginx): full fastcgi_params + httpoxy HTTP_PROXY guard (P1)"
```

---

### Task 3: per-site vhost — http2 + SSL hardening + dotfile deny + Laravel `/build/` cache

**Files:**
- Modify: `core/src/sites.rs` (`Site::vhost_config` — both the proxy and non-proxy `format!` branches; compute `is_laravel` and a `build_cache` string)
- Test: `core/src/sites.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `Site { name, root, domains, source, proxy }`, `self.document_root()`, `LaraluxPaths::{log, etc_for}` (unchanged signature).
- Produces: both 443 blocks gain `http2 on;` + `ssl_protocols`/`ssl_ciphers`/`ssl_session_cache`/`ssl_session_timeout`; the non-proxy block gains a dotfile-deny location and, when `<root>/artisan` exists, a `location ^~ /build/` immutable-cache block.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `core/src/sites.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core vhost_has_http2_and_ssl_hardening_and_dotfile_deny vhost_build_cache_only_for_laravel`
Expected: FAIL (no `http2 on`, no `/build/`).

- [ ] **Step 3: Write minimal implementation**

In `core/src/sites.rs`, edit `Site::vhost_config`. Immediately after `let server_names = self.domains.join(" ");` add:

```rust
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
```

In the PROXY branch, replace its `return format!( … );` with (adds http2 + SSL hardening only):

```rust
            return format!(
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
```

Replace the final (non-proxy) `format!( … )` with (adds http2 + SSL hardening + dotfile deny + conditional `{build_cache}`):

```rust
        format!(
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
        )
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laralux-core vhost_has_http2_and_ssl_hardening_and_dotfile_deny vhost_build_cache_only_for_laravel`
Expected: PASS. Re-run the existing site test: `cargo test -p laralux-core vhost_has_https_redirect_ssl_and_fastcgi` → PASS (redirect/ssl_certificate/fastcgi_pass/HTTPS-on still present).

- [ ] **Step 5: Commit**

```bash
git add core/src/sites.rs
git commit -m "feat(nginx): per-site http2 + SSL hardening + dotfile deny + Laravel /build/ cache (P1/P2)"
```

---

### Task 4: full verification + changelog

**Files:**
- Modify: `CHANGELOG.md` (new entry), version bump files (`Cargo.toml`, `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, `docs/debian/ITP.md`, `docs/debian/RFS.md`) — patch bump.

- [ ] **Step 1: Run the full test suite + build**

Run: `cargo test && cargo build`
Expected: all tests PASS, build clean.

- [ ] **Step 2: Manual MIME verification on the running stack**

Regenerate config (restart laralux or re-run its config write) and reload nginx, then:

Run: `curl -skI https://<a-laravel-site>/build/assets/<hashed>.css | grep -i content-type`
Expected: `Content-Type: text/css` (was `application/octet-stream`). Spot-check a `.js` returns `application/javascript`.

- [ ] **Step 3: Version bump + CHANGELOG**

Bump patch version (e.g. 0.4.1 → 0.4.2) in `Cargo.toml`, `package.json`, `package-lock.json` (top two `version` keys), `src-tauri/tauri.conf.json`, `docs/debian/ITP.md`, `docs/debian/RFS.md`. Add a `## [x.y.z] - <date>` CHANGELOG entry under **Fixed** (mime.types/MIME bug) and **Added/Changed** (P1/P2 hardening) with a compare link. Run `cargo build` to sync `Cargo.lock`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore(release): <x.y.z> — nginx mime.types fix + config hardening"
```

---

## Self-Review

**Spec coverage:**
- P0 mime.types + include → Task 1 ✓
- P1 charset utf-8 → Task 1 ✓; full fastcgi_params → Task 2 ✓; httpoxy HTTP_PROXY "" → Task 2 ✓ (in fastcgi_params, covers all PHP locations); dotfile deny → Task 1 (default server) + Task 3 (sites) ✓
- P2 sendfile/tcp_nopush/tcp_nodelay/keepalive/server_tokens → Task 1 ✓; gzip → Task 1 ✓; http2 → Task 3 ✓; SSL hardening → Task 3 ✓; `/build/` cache (Laravel only) → Task 3 ✓
- Verification (`curl -I` → text/css) → Task 4 ✓

**Placeholder scan:** none — every code/step is concrete.

**Type/name consistency:** `MIME_TYPES`, `FASTCGI_PARAMS` consts defined in Task 1/2 and used in the same file; `is_laravel`/`build_cache` defined and used within `vhost_config` in Task 3; `self.root.join("artisan")` matches the `Site.root` field and the existing `document_root()` pattern; test helpers (`temp_root`, `scan_sites`) match existing usage.

Notes: dotfile-deny is added to both the default server (Task 1) and per-site non-proxy block (Task 3); httpoxy is centralized in `fastcgi_params` so no per-site change is needed.
