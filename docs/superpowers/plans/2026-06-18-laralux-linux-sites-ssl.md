# Laralux Linux — Plan 2: Sites, Pretty URLs & SSL Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scan `~/laralux/www/` for projects, generate an HTTPS nginx virtual host per site with a mkcert certificate, and register `*.dev` hostnames in `/etc/hosts` so `https://<name>.dev` works.

**Architecture:** Extends the Plan 1 `core` crate with pure, testable units (`sites`, `hosts`) plus two trait-gated boundaries for the things that need external tools or root: `CertIssuer` (mkcert) and `Privileged` (sudo cp to `/etc/hosts`, `setcap`, `mkcert -install`). A `sync_sites()` function composes them and is unit-tested end-to-end with fakes. `laraluxctl` wires the real implementations and runs `sync_sites` before starting nginx.

**Tech Stack:** Rust (edition 2021), reuses `laralux_core` from Plan 1 (`LaraluxPaths`, `SpawnSpec`, `PhpFpmService::socket_path`, `Config`). External runtime tools (live only): `mkcert`, `nginx`, `sudo`, `setcap`.

## Global Constraints

- Default TLD: `dev`. Hostname format: `<dir-name>.<tld>` (e.g. `app.dev`). TLD comes from `Config::tld`.
- `.dev` is HSTS-preloaded → every site MUST be served over HTTPS (443) with a mkcert cert; port 80 redirects to 443.
- Document root: if `<site>/public` is a directory use it (Laravel), else the site dir itself.
- `core` crate keeps zero Tauri deps. All root-requiring operations go through the `Privileged` trait; all cert issuance through the `CertIssuer` trait — both mockable, so `cargo test` needs no root and no mkcert.
- Managed `/etc/hosts` region is delimited by exact markers `# BEGIN laralux-linux` and `# END laralux-linux`; only that region is ever rewritten; unrelated lines are preserved.
- Per-site vhost files live in `~/laralux/etc/nginx/sites/<name>.conf` (already `include`d by Plan 1's `nginx.conf`). Certs live in `~/laralux/ssl/`.
- nginx fastcgi target is the php-fpm unix socket `~/laralux/tmp/php-fpm.sock` (from `PhpFpmService::new(php_version).socket_path(paths)` — thread it in, do not re-hardcode).
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- Follow TDD: failing test first, watch it fail, minimal implementation, watch it pass, commit.

---

### Task 1: Site model + scan_sites

**Files:**
- Create: `core/src/sites.rs`
- Modify: `core/src/lib.rs` (add `pub mod sites;`)

**Interfaces:**
- Consumes: `LaraluxPaths` (`www()`).
- Produces:
  - `struct Site { pub name: String, pub root: PathBuf, pub hostname: String }`
  - `impl Site { pub fn document_root(&self) -> PathBuf }` — `<root>/public` if it is a dir, else `<root>`.
  - `pub fn scan_sites(paths: &LaraluxPaths, tld: &str) -> std::io::Result<Vec<Site>>` — immediate subdirectories of `www/` only (skip files and hidden entries whose name starts with `.`), `hostname = "<name>.<tld>"`, sorted by `name` ascending for determinism. Returns empty vec if `www/` does not exist.

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs`, add after the existing `pub mod` lines:

```rust
pub mod sites;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/sites.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaraluxPaths;

    fn temp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("lara-sites-{}-{}", std::process::id(), line!()))
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
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laralux-core sites`
Expected: FAIL — `cannot find function scan_sites`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/sites.rs`:

```rust
use crate::paths::LaraluxPaths;
use std::path::PathBuf;

/// A project under `www/` exposed at `<name>.<tld>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub name: String,
    pub root: PathBuf,
    pub hostname: String,
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
        sites.push(Site {
            hostname: format!("{name}.{tld}"),
            root: entry.path(),
            name,
        });
    }
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sites)
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laralux-core sites`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/sites.rs core/src/lib.rs
git commit -m "feat(core): add Site model and scan_sites"
```

---

### Task 2: Per-site vhost generation

**Files:**
- Modify: `core/src/sites.rs`

**Interfaces:**
- Consumes: `LaraluxPaths` (`etc_for`, `log`), `Site::document_root`.
- Produces (added to `impl Site`):
  - `pub fn vhost_config(&self, paths: &LaraluxPaths, php_socket: &std::path::Path, cert: &std::path::Path, key: &std::path::Path) -> String` — an nginx config string with a port-80 server that 301-redirects to HTTPS and a port-443 `ssl` server rooted at `document_root()`, wired to php-fpm via `fastcgi_pass unix:<php_socket>`, including `<etc/nginx>/fastcgi_params`, setting `fastcgi_param HTTPS on;`, with per-site access/error logs in `log/`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/sites.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laralux-core sites::tests::vhost`
Expected: FAIL — `no method named vhost_config`.

- [ ] **Step 3: Write minimal implementation**

Add this method inside `impl Site` in `core/src/sites.rs`:

```rust
    /// Generate the nginx vhost (HTTP→HTTPS redirect + HTTPS server) for this site.
    pub fn vhost_config(
        &self,
        paths: &LaraluxPaths,
        php_socket: &std::path::Path,
        cert: &std::path::Path,
        key: &std::path::Path,
    ) -> String {
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laralux-core sites`
Expected: PASS — 4 tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/sites.rs
git commit -m "feat(core): add per-site nginx vhost generation"
```

---

### Task 3: /etc/hosts managed-block rendering

**Files:**
- Create: `core/src/hosts.rs`
- Modify: `core/src/lib.rs` (add `pub mod hosts;`)

**Interfaces:**
- Produces:
  - `pub const HOSTS_BEGIN: &str = "# BEGIN laralux-linux";`
  - `pub const HOSTS_END: &str = "# END laralux-linux";`
  - `pub fn render_block(hostnames: &[String]) -> String` — the managed block: BEGIN marker, one `127.0.0.1 <host>` line per hostname (in given order), END marker; trailing newline; no extra blank lines.
  - `pub fn apply_block(existing: &str, hostnames: &[String]) -> String` — return `existing` with any current managed block (BEGIN..END inclusive) removed, then the freshly rendered block appended. Non-managed lines are preserved in order. Idempotent: applying twice with the same hostnames yields the same output.

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add:

```rust
pub mod hosts;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/hosts.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn render_block_lists_each_host() {
        let b = render_block(&hosts(&["app.dev", "blog.dev"]));
        assert!(b.starts_with(HOSTS_BEGIN));
        assert!(b.contains("\n127.0.0.1 app.dev\n"));
        assert!(b.contains("\n127.0.0.1 blog.dev\n"));
        assert!(b.trim_end().ends_with(HOSTS_END));
    }

    #[test]
    fn apply_block_appends_to_clean_file_and_preserves_lines() {
        let existing = "127.0.0.1 localhost\n255.255.255.255 broadcasthost\n";
        let out = apply_block(existing, &hosts(&["app.dev"]));
        assert!(out.contains("127.0.0.1 localhost"));
        assert!(out.contains("broadcasthost"));
        assert!(out.contains("127.0.0.1 app.dev"));
        assert!(out.contains(HOSTS_BEGIN) && out.contains(HOSTS_END));
    }

    #[test]
    fn apply_block_replaces_existing_block_idempotently() {
        let existing = "127.0.0.1 localhost\n";
        let once = apply_block(existing, &hosts(&["app.dev"]));
        let twice = apply_block(&once, &hosts(&["app.dev"]));
        assert_eq!(once, twice); // idempotent
        // and the localhost line still appears exactly once
        assert_eq!(twice.matches("127.0.0.1 localhost").count(), 1);
    }

    #[test]
    fn apply_block_updates_when_hosts_change() {
        let existing = "127.0.0.1 localhost\n";
        let first = apply_block(existing, &hosts(&["app.dev"]));
        let second = apply_block(&first, &hosts(&["app.dev", "blog.dev"]));
        assert!(second.contains("127.0.0.1 blog.dev"));
        assert_eq!(second.matches("127.0.0.1 app.dev").count(), 1);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laralux-core hosts`
Expected: FAIL — `cannot find function render_block`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/hosts.rs`:

```rust
pub const HOSTS_BEGIN: &str = "# BEGIN laralux-linux";
pub const HOSTS_END: &str = "# END laralux-linux";

/// Render the managed block (markers + one mapping line per hostname).
pub fn render_block(hostnames: &[String]) -> String {
    let mut s = String::new();
    s.push_str(HOSTS_BEGIN);
    s.push('\n');
    for host in hostnames {
        s.push_str(&format!("127.0.0.1 {host}\n"));
    }
    s.push_str(HOSTS_END);
    s.push('\n');
    s
}

/// Strip any existing managed block from `existing`, then append a fresh one.
pub fn apply_block(existing: &str, hostnames: &[String]) -> String {
    // Collect lines that are NOT inside a managed block.
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        if line.trim() == HOSTS_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == HOSTS_END {
            inside = false;
            continue;
        }
        if !inside {
            kept.push(line);
        }
    }

    let mut out = String::new();
    for line in &kept {
        out.push_str(line);
        out.push('\n');
    }
    // Exactly one blank separator only if there is preceding content.
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&render_block(hostnames));
    out
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laralux-core hosts`
Expected: PASS — 4 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/hosts.rs core/src/lib.rs
git commit -m "feat(core): add /etc/hosts managed-block rendering"
```

---

### Task 4: SSL CertIssuer (mkcert) + fake

**Files:**
- Create: `core/src/ssl.rs`
- Modify: `core/src/lib.rs` (add `pub mod ssl;`)

**Interfaces:**
- Consumes: `LaraluxPaths` (`ssl()`).
- Produces:
  - `struct CertFiles { pub cert: PathBuf, pub key: PathBuf }`
  - `enum SslError { Io(std::io::Error), Mkcert(String) }` via `thiserror`.
  - `trait CertIssuer: Send + Sync { fn ensure_cert(&self, hostname: &str) -> Result<CertFiles, SslError>; }`
  - `struct MkcertIssuer { ssl_dir: PathBuf }` with `MkcertIssuer::new(ssl_dir: PathBuf) -> Self`, `cert_path(&self, hostname) -> PathBuf` (`<ssl_dir>/<host>.pem`), `key_path(&self, hostname) -> PathBuf` (`<ssl_dir>/<host>-key.pem`), `pub fn issue_command(&self, hostname) -> SpawnSpec` (mkcert with `-cert-file`/`-key-file`/hostname). `ensure_cert` returns the paths immediately if both files already exist; otherwise runs the mkcert command and errors if it exits non-zero.
  - `struct FakeCertIssuer` (NOT `#[cfg(test)]`-gated; reused by Task 6 tests) recording requested hostnames in `Arc<Mutex<Vec<String>>>` and returning `CertFiles` under a configured base dir; expose `requested()`.

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add:

```rust
pub mod ssl;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/ssl.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("lara-ssl-{}-{}", std::process::id(), line!()))
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
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laralux-core ssl`
Expected: FAIL — `cannot find type MkcertIssuer`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/ssl.rs`:

```rust
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
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laralux-core ssl`
Expected: PASS — 4 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/ssl.rs core/src/lib.rs
git commit -m "feat(core): add CertIssuer trait with mkcert and fake impls"
```

---

### Task 5: Privileged operations boundary

**Files:**
- Create: `core/src/privileged.rs`
- Modify: `core/src/lib.rs` (add `pub mod privileged;`)

**Interfaces:**
- Produces:
  - `enum PrivError { Io(std::io::Error), Command(String) }` via `thiserror`.
  - `trait Privileged: Send + Sync` with:
    - `fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError>;`
    - `fn install_mkcert_ca(&self) -> Result<(), PrivError>;`
    - `fn setcap_nginx(&self, nginx_bin: &std::path::Path) -> Result<(), PrivError>;`
  - `struct SudoPrivileged;` implementing `Privileged` (writes content to a temp file then `sudo cp <tmp> /etc/hosts`; `mkcert -install`; `sudo setcap cap_net_bind_service=+ep <bin>`), plus pure command builders for unit testing: `pub fn hosts_cp_command(src: &std::path::Path) -> (String, Vec<String>)` and `pub fn setcap_command(bin: &std::path::Path) -> (String, Vec<String>)`.
  - `struct FakePrivileged` (NOT `#[cfg(test)]`-gated; reused by Task 6) recording the last hosts content written, a write counter, and install/setcap flags via `Arc<Mutex<_>>`; expose `hosts_writes() -> Arc<Mutex<Vec<String>>>`.

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add:

```rust
pub mod privileged;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/privileged.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sudo_command_builders_are_correct() {
        let (prog, args) = SudoPrivileged::hosts_cp_command(Path::new("/tmp/hosts.new"));
        assert_eq!(prog, "sudo");
        assert_eq!(args, vec!["cp".to_string(), "/tmp/hosts.new".to_string(), "/etc/hosts".to_string()]);

        let (prog2, args2) = SudoPrivileged::setcap_command(Path::new("/usr/sbin/nginx"));
        assert_eq!(prog2, "sudo");
        assert_eq!(
            args2,
            vec![
                "setcap".to_string(),
                "cap_net_bind_service=+ep".to_string(),
                "/usr/sbin/nginx".to_string(),
            ]
        );
    }

    #[test]
    fn fake_records_hosts_write() {
        let f = FakePrivileged::new();
        let log = f.hosts_writes();
        f.write_etc_hosts("# BEGIN laralux-linux\n# END laralux-linux\n").unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(log.lock().unwrap()[0].contains("laralux-linux"));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laralux-core privileged`
Expected: FAIL — `cannot find type SudoPrivileged`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/privileged.rs`:

```rust
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum PrivError {
    #[error("privileged io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("privileged command failed: {0}")]
    Command(String),
}

/// Operations that require elevated privileges or external trust stores.
pub trait Privileged: Send + Sync {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError>;
    fn install_mkcert_ca(&self) -> Result<(), PrivError>;
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError>;
}

// ---------- Real: sudo / mkcert ----------

pub struct SudoPrivileged;

impl SudoPrivileged {
    pub fn hosts_cp_command(src: &Path) -> (String, Vec<String>) {
        (
            "sudo".to_string(),
            vec!["cp".to_string(), src.display().to_string(), "/etc/hosts".to_string()],
        )
    }
    pub fn setcap_command(bin: &Path) -> (String, Vec<String>) {
        (
            "sudo".to_string(),
            vec![
                "setcap".to_string(),
                "cap_net_bind_service=+ep".to_string(),
                bin.display().to_string(),
            ],
        )
    }

    fn run(prog: &str, args: &[String]) -> Result<(), PrivError> {
        let status = std::process::Command::new(prog)
            .args(args)
            .status()
            .map_err(|e| PrivError::Command(format!("spawn {prog}: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(PrivError::Command(format!("{prog} exited with failure")))
        }
    }
}

impl Privileged for SudoPrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        let tmp = std::env::temp_dir().join("laralux-hosts.new");
        std::fs::write(&tmp, new_content)?;
        let (prog, args) = Self::hosts_cp_command(&tmp);
        Self::run(&prog, &args)
    }
    fn install_mkcert_ca(&self) -> Result<(), PrivError> {
        Self::run("mkcert", &["-install".to_string()])
    }
    fn setcap_nginx(&self, nginx_bin: &Path) -> Result<(), PrivError> {
        let (prog, args) = Self::setcap_command(nginx_bin);
        Self::run(&prog, &args)
    }
}

// ---------- Fake (used by sync tests) ----------

#[derive(Clone, Default)]
pub struct FakePrivileged {
    hosts_writes: Arc<Mutex<Vec<String>>>,
    installed_ca: Arc<Mutex<bool>>,
    setcap_done: Arc<Mutex<bool>>,
}

impl FakePrivileged {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn hosts_writes(&self) -> Arc<Mutex<Vec<String>>> {
        self.hosts_writes.clone()
    }
    pub fn installed_ca(&self) -> bool {
        *self.installed_ca.lock().unwrap()
    }
}

impl Privileged for FakePrivileged {
    fn write_etc_hosts(&self, new_content: &str) -> Result<(), PrivError> {
        self.hosts_writes.lock().unwrap().push(new_content.to_string());
        Ok(())
    }
    fn install_mkcert_ca(&self) -> Result<(), PrivError> {
        *self.installed_ca.lock().unwrap() = true;
        Ok(())
    }
    fn setcap_nginx(&self, _nginx_bin: &Path) -> Result<(), PrivError> {
        *self.setcap_done.lock().unwrap() = true;
        Ok(())
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laralux-core privileged`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/privileged.rs core/src/lib.rs
git commit -m "feat(core): add Privileged boundary with sudo and fake impls"
```

---

### Task 6: sync_sites integration

**Files:**
- Create: `core/src/sync.rs`
- Modify: `core/src/lib.rs` (add `pub mod sync;`)

**Interfaces:**
- Consumes: `LaraluxPaths`, `scan_sites`, `Site::vhost_config`, `CertIssuer`, `Privileged`, `hosts::apply_block`.
- Produces:
  - `enum SyncError { Io(std::io::Error), Ssl(crate::ssl::SslError), Priv(crate::privileged::PrivError) }` via `thiserror` (with `#[from]`).
  - `pub fn sync_sites(paths: &LaraluxPaths, tld: &str, php_socket: &std::path::Path, hosts_path: &std::path::Path, issuer: &dyn CertIssuer, privileged: &dyn Privileged) -> Result<Vec<Site>, SyncError>` — scans sites; for each site: `ensure_cert(hostname)`, then writes the vhost to `etc/nginx/sites/<name>.conf`; then reads `hosts_path` (empty string if it does not exist), computes `apply_block`, and calls `privileged.write_etc_hosts(new)` **only if** the content changed. Creates `etc/nginx/sites/` if missing. Returns the discovered sites.

- [ ] **Step 1: Add module declaration**

In `core/src/lib.rs` add:

```rust
pub mod sync;
```

- [ ] **Step 2: Write the failing test**

Create `core/src/sync.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::{apply_block, render_block};
    use crate::paths::LaraluxPaths;
    use crate::privileged::FakePrivileged;
    use crate::ssl::FakeCertIssuer;

    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("lara-sync-{}-{}", std::process::id(), line!()))
    }

    #[test]
    fn writes_vhosts_certs_and_hosts_block() {
        let r = root();
        let www = r.join("www");
        std::fs::create_dir_all(www.join("app")).unwrap();
        std::fs::create_dir_all(www.join("blog")).unwrap();
        let paths = LaraluxPaths::new(r.clone());

        let hosts_path = r.join("hosts");
        std::fs::write(&hosts_path, "127.0.0.1 localhost\n").unwrap();

        let issuer = FakeCertIssuer::new(paths.ssl());
        let priv_ = FakePrivileged::new();
        let writes = priv_.hosts_writes();
        let sock = paths.tmp().join("php-fpm.sock");

        let sites = sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();

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
        let paths = LaraluxPaths::new(r.clone());

        // Pre-populate hosts with the exact block sync would produce.
        let hosts_path = r.join("hosts");
        let already = apply_block("127.0.0.1 localhost\n", &["app.dev".to_string()]);
        std::fs::write(&hosts_path, &already).unwrap();
        let _ = render_block; // ensure import used

        let issuer = FakeCertIssuer::new(paths.ssl());
        let priv_ = FakePrivileged::new();
        let writes = priv_.hosts_writes();
        let sock = paths.tmp().join("php-fpm.sock");

        sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();

        // No write because the managed block is already correct.
        assert_eq!(writes.lock().unwrap().len(), 0);
        std::fs::remove_dir_all(&r).ok();
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laralux-core sync`
Expected: FAIL — `cannot find function sync_sites`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/sync.rs`:

```rust
use crate::hosts::apply_block;
use crate::paths::LaraluxPaths;
use crate::privileged::Privileged;
use crate::sites::{scan_sites, Site};
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

/// Scan sites, issue certs, write per-site vhosts, and update the managed
/// `/etc/hosts` block — writing hosts only when the block actually changes.
pub fn sync_sites(
    paths: &LaraluxPaths,
    tld: &str,
    php_socket: &Path,
    hosts_path: &Path,
    issuer: &dyn CertIssuer,
    privileged: &dyn Privileged,
) -> Result<Vec<Site>, SyncError> {
    let sites = scan_sites(paths, tld)?;
    let sites_dir = paths.etc_for("nginx").join("sites");
    std::fs::create_dir_all(&sites_dir)?;

    for site in &sites {
        let certs = issuer.ensure_cert(&site.hostname)?;
        let conf = site.vhost_config(paths, php_socket, &certs.cert, &certs.key);
        std::fs::write(sites_dir.join(format!("{}.conf", site.name)), conf)?;
    }

    let hostnames: Vec<String> = sites.iter().map(|s| s.hostname.clone()).collect();
    let existing = match std::fs::read_to_string(hosts_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(SyncError::Io(e)),
    };
    let updated = apply_block(&existing, &hostnames);
    if updated != existing {
        privileged.write_etc_hosts(&updated)?;
    }

    Ok(sites)
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laralux-core sync`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/sync.rs core/src/lib.rs
git commit -m "feat(core): add sync_sites integration"
```

---

### Task 7: laraluxctl wiring — sites, setup-perms, up sync

**Files:**
- Modify: `core/src/lib.rs` (add re-exports)
- Modify: `laraluxctl/src/main.rs`

**Interfaces:**
- Consumes: `sync_sites`, `MkcertIssuer`, `SudoPrivileged`, `scan_sites`, `Privileged`, `PhpFpmService` (for socket), `Config`, `LaraluxPaths`, `Orchestrator`, `build_services`, `RealSpawner`.
- Produces: CLI subcommands `sites` (list discovered sites + hostnames) and `setup-perms` (mkcert CA install + `setcap` on the resolved nginx binary). `up` now calls `sync_sites` (real `MkcertIssuer` + `SudoPrivileged`, `hosts_path = /etc/hosts`) before `start_all`.

- [ ] **Step 1: Add re-exports to `core/src/lib.rs`**

Append:

```rust
pub use privileged::{Privileged, SudoPrivileged};
pub use sites::{scan_sites, Site};
pub use ssl::MkcertIssuer;
pub use sync::sync_sites;
```

- [ ] **Step 2: Update the CLI**

Replace the body of `laraluxctl/src/main.rs` with the following (keeps the Plan 1 Ctrl-C helpers `wait_for_ctrl_c` and `ctrlc_lite` exactly as they are — do not delete them; only the `use` line, the `match`, and the addition of a `php_socket`/sync step change):

```rust
use laralux_core::service::php_fpm::PhpFpmService;
use laralux_core::{
    build_services, scan_sites, sync_sites, Config, LaraluxPaths, MkcertIssuer, Orchestrator,
    Privileged, RealSpawner, SudoPrivileged,
};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    let paths = LaraluxPaths::new(LaraluxPaths::default_root());

    match cmd.as_str() {
        "config-init" => {
            paths.ensure_dirs().expect("create dirs");
            let cfg = Config::default();
            cfg.save(&paths.config_file()).expect("save config");
            println!("Initialized {}", paths.config_file().display());
        }
        "sites" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            let sites = scan_sites(&paths, &cfg.tld).expect("scan sites");
            if sites.is_empty() {
                println!("No sites found in {}", paths.www().display());
            }
            for s in sites {
                println!("{:<20} https://{}", s.name, s.hostname);
            }
        }
        "setup-perms" => {
            let priv_ = SudoPrivileged;
            println!("Installing mkcert local CA (may prompt for sudo)...");
            priv_.install_mkcert_ca().expect("mkcert -install");
            let nginx_bin = which("nginx").unwrap_or_else(|| "/usr/sbin/nginx".into());
            println!("Granting nginx permission to bind low ports via setcap...");
            priv_.setcap_nginx(&nginx_bin).expect("setcap nginx");
            println!("Done.");
        }
        "up" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            paths.ensure_dirs().expect("create dirs");

            // Sync sites (vhosts + certs + /etc/hosts) before starting nginx.
            let php_socket = PhpFpmService::new(cfg.php_version.clone()).socket_path(&paths);
            let issuer = MkcertIssuer::new(paths.ssl());
            let privileged = SudoPrivileged;
            match sync_sites(
                &paths,
                &cfg.tld,
                &php_socket,
                std::path::Path::new("/etc/hosts"),
                &issuer,
                &privileged,
            ) {
                Ok(sites) => println!("Synced {} site(s).", sites.len()),
                Err(e) => {
                    eprintln!("site sync failed: {e}");
                    std::process::exit(1);
                }
            }

            let mut orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            match orch.start_all() {
                Ok(()) => println!("Started all services. Press Ctrl-C to stop."),
                Err(e) => {
                    eprintln!("start failed: {e}");
                    orch.stop_all();
                    std::process::exit(1);
                }
            }
            wait_for_ctrl_c();
            println!("Stopping...");
            orch.stop_all();
        }
        "status" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            let orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            for kind in orch.start_order() {
                println!("{:?}: {:?}", kind, orch.state(kind));
            }
        }
        "down" => {
            println!("`up` manages the process lifetime; stop it with Ctrl-C.");
        }
        _ => {
            println!("usage: laraluxctl <config-init|up|status|sites|setup-perms>");
        }
    }
}

/// Resolve a binary on PATH (minimal `which`, no external crate).
fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
```

Keep the existing `wait_for_ctrl_c()` and `ctrlc_lite()` functions from Plan 1 below `which` unchanged.

- [ ] **Step 3: Build the CLI**

Run: `cargo build -p laraluxctl`
Expected: PASS — compiles.

- [ ] **Step 4: Run the full test suite**

Run: `cargo test --workspace`
Expected: PASS — all existing tests plus the new `sites`/`hosts`/`ssl`/`privileged`/`sync` tests.

- [ ] **Step 5: CLI smoke test (no root needed for `sites`)**

```bash
cargo run -p laraluxctl -- config-init
mkdir -p ~/laralux/www/demo
cargo run -p laraluxctl -- sites
```
Expected: prints `demo                 https://demo.dev`.

The live `up` (which invokes `sudo` for `/etc/hosts` and needs `mkcert`, `nginx`, etc. installed) and `setup-perms` (sudo `setcap`, `mkcert -install`) require the stack and root; defer their live validation to the Plan 3 setup wizard. Document this in the report.

- [ ] **Step 6: Commit**

```bash
git add core/src/lib.rs laraluxctl/src/main.rs
git commit -m "feat(laraluxctl): add sites/setup-perms and sync sites on up"
```

---

## Self-Review

**1. Spec coverage (Plan 2 scope = sites, pretty URLs, SSL):**
- Scan `www/` → sites + docroot detection (spec §6, Phase 2 public/ detection) → Task 1 ✓
- Per-site vhost with HTTPS (spec §6 "SSL bắt buộc", `.dev` HSTS) → Task 2 ✓
- `*.dev` in `/etc/hosts`, managed block, preserve unrelated lines (spec §6, Global Constraints) → Tasks 3, 6 ✓
- mkcert cert per site, reuse if present (spec §6 auto SSL) → Task 4 ✓
- Privileged boundary: `/etc/hosts` write, `setcap` nginx, `mkcert -install` (spec §5) → Task 5 ✓
- Compose: scan → certs → vhosts → hosts, write-only-on-change (spec §6) → Task 6 ✓
- Wire into `laraluxctl`; `up` syncs before nginx; `setup-perms`; `sites` (spec §6, §7 Phase 1) → Task 7 ✓
- TDD throughout (spec §9) → every task ✓
- **Correctly deferred to Plan 3:** dnsmasq wildcard, GUI/tray, setup wizard, apt install of the stack, live root validation.

**2. Placeholder scan:** No "TBD/handle edge cases" — every code step is complete. Task 7 Step 5 is a concrete smoke test; the root-requiring parts are explicitly deferred with reasons, not left vague.

**3. Type consistency:** `Site{name,root,hostname}`, `Site::document_root`, `Site::vhost_config(paths, php_socket, cert, key)`, `scan_sites(paths, tld)`, `render_block`/`apply_block(existing, hostnames)`, `HOSTS_BEGIN`/`HOSTS_END`, `CertFiles{cert,key}`, `CertIssuer::ensure_cert(hostname)`, `MkcertIssuer::new(ssl_dir)` + `cert_path`/`key_path`/`issue_command`, `FakeCertIssuer::new(base)`+`requested()`, `Privileged::{write_etc_hosts,install_mkcert_ca,setcap_nginx}`, `SudoPrivileged::{hosts_cp_command,setcap_command}`, `FakePrivileged::new()`+`hosts_writes()`, `sync_sites(paths,tld,php_socket,hosts_path,issuer,privileged)` — all names/signatures consistent across tasks and consistent with Plan 1's `LaraluxPaths`/`SpawnSpec`/`PhpFpmService::socket_path`. ✓

**Note on the deferred Plan 1 finding:** `.display()` path interpolation (unquoted) recurs in the Task 2 vhost generator. Same low risk (default `~/laralux`, no spaces) and tracked for a later hardening pass; not introduced anew by this plan.
