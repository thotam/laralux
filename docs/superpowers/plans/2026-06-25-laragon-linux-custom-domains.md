# Custom Domains per Site Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Edit a site's domains (add/remove explicit domains + subdomains, change/remove the auto primary, add wildcard `*.x`) for every site type, wiring each into a multi-SAN cert + `/etc/hosts` (explicit) + nginx `server_name`, with wildcard resolved by an app-managed CoreDNS (downloaded static binary) + a systemd-resolved drop-in.

**Architecture:** A per-site `[[domains]]` override in `sites.toml` drives `Site.domains`; `sync_sites` issues one SAN cert per site, writes explicit domains to `/etc/hosts`, and returns wildcard bases; a downloaded CoreDNS (run by the orchestrator) plus a reversible systemd-resolved routing drop-in resolve `*.x → 127.0.0.1`. An Edit-domains modal manages the list.

**Tech Stack:** Rust (laragon-core, zero Tauri deps), Tauri 2, vanilla JS; CoreDNS static binary (GitHub release), mkcert.

## Global Constraints

- `core` keeps **zero Tauri deps**. Commit messages MUST NOT contain a `Co-Authored-By` trailer. TDD: failing test first.
- Domain rule: DNS labels `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$` joined by `.`, or `*.<rest>` where the leftmost label is exactly `*` and `<rest>` is a valid hostname; no multi-`*`.
- A domain belongs to exactly one site (validated across all sites at write time); ≥1 domain per site.
- Cert: one mkcert cert per site covering all the site's domains (SAN), file basename = the site name, re-issued when the domain set changes (a `.san` sidecar tracks it).
- `/etc/hosts` gets explicit (non-wildcard) domains only; wildcard bases (`*.X` → `X`) go to CoreDNS.
- CoreDNS: downloaded static binary into `~/laragon/bin/coredns` (no apt, no root for the binary), run on `127.0.0.1:5353`; only the systemd-resolved drop-in needs pkexec; runs only while wildcard domains exist; reversible.
- CoreDNS source: `https://github.com/coredns/coredns/releases/download/v<ver>/coredns_<ver>_linux_<arch>.tgz` (`arch ∈ {amd64, arm64}`), version `1.14.4`. The tgz contains a single `coredns` binary.
- systemd-resolved drop-in: `/etc/systemd/resolved.conf.d/laragon.conf` with `[Resolve]\nDNS=127.0.0.1:5353\nDomains=~<base> …`.
- Run core tests `cargo test -p laragon-core`; build `cargo build -p laragon-desktop && cargo build -p laragonctl`. If `cargo`/`node` aren't on PATH use `$HOME/.cargo/bin/cargo` / `$HOME/.nvm/versions/node/v24.16.0/bin/node`.

Tasks are sequenced so each ends with a clean build.

---

### Task 1: registry — per-site domains

**Files:** Modify `core/src/site_registry.rs`, `core/src/lib.rs`.

**Interfaces produced:** `validate_domain(&str) -> Result<(), RegistryError>`, `SiteDomains { name, domains }`, `SiteRegistry::{set_domains, domains_for}`, `remove` also drops domains; new `RegistryError::{InvalidDomain, NoDomains, DomainTaken}`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/site_registry.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laragon-core site_registry`
Expected: FAIL to compile — items not found.

- [ ] **Step 3: Implement**

In `core/src/site_registry.rs`, add the error variants to `RegistryError`:

```rust
    #[error("invalid domain: {0}")]
    InvalidDomain(String),
    #[error("a site needs at least one domain")]
    NoDomains,
    #[error("domain already used by another site: {0}")]
    DomainTaken(String),
```

Add the type + field (after `ProxySite`):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteDomains {
    pub name: String,
    pub domains: Vec<String>,
}
```

In `struct SiteRegistry`, add:

```rust
    #[serde(default)]
    domains: Vec<SiteDomains>,
```

Add the free validator (top-level):

```rust
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
```

In `impl SiteRegistry`, add methods:

```rust
    pub fn domains_for(&self, name: &str) -> Option<&[String]> {
        self.domains.iter().find(|d| d.name == name).map(|d| d.domains.as_slice())
    }

    pub fn set_domains(&mut self, name: &str, domains: &[String]) -> Result<(), RegistryError> {
        if domains.is_empty() {
            return Err(RegistryError::NoDomains);
        }
        for d in domains {
            validate_domain(d)?;
        }
        // reject a domain claimed by a *different* site
        for other in &self.domains {
            if other.name == name {
                continue;
            }
            for d in domains {
                if other.domains.iter().any(|x| x == d) {
                    return Err(RegistryError::DomainTaken(d.clone()));
                }
            }
        }
        self.domains.retain(|d| d.name != name);
        self.domains.push(SiteDomains { name: name.to_string(), domains: domains.to_vec() });
        Ok(())
    }
```

Extend `remove` to also drop the domains entry — add this line inside `remove` before the return:

```rust
        self.domains.retain(|d| d.name != name);
```

(and include `self.domains.len()` in the before/after count so removing only a domains entry still reports `true`; simplest: keep the existing `sites`/`proxies` count logic and additionally `self.domains.retain(...)` — the `[[domains]]` entry is auxiliary, so it need not affect the bool. Leave the bool based on sites+proxies.)

In `core/src/lib.rs`, add to the registry re-export: `validate_domain`, `SiteDomains`:

```rust
pub use site_registry::{
    validate_domain, ProxyRoute, ProxySite, RegisteredSite, RegistryError, SiteDomains, SiteRegistry,
};
```

- [ ] **Step 4: Run tests; Step 5: Commit**

Run: `cargo test -p laragon-core site_registry` (PASS). Then:

```bash
git add core/src/site_registry.rs core/src/lib.rs
git commit -m "feat(core): per-site domain overrides + validate_domain"
```

---

### Task 2: `Site.domains` + multi `server_name`

**Files:** Modify `core/src/sites.rs`, `core/src/lib.rs`.

**Interfaces:** `Site` gains `pub domains: Vec<String>`; `list_all_sites` populates it; `vhost_config` server_name uses all domains.

- [ ] **Step 1: Write the failing tests**

Add to `core/src/sites.rs` tests:

```rust
    #[test]
    fn site_has_domains_default_and_override() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("demo")).unwrap();
        let paths = LaragonPaths::new(root.clone());
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
    fn vhost_server_name_lists_all_domains() {
        let site = Site {
            name: "demo".into(),
            root: std::path::PathBuf::from("/x"),
            hostname: "demo.dev".into(),
            source: SiteSource::Scanned,
            proxy: None,
            domains: vec!["demo.dev".into(), "*.demo.dev".into()],
        };
        let paths = LaragonPaths::new(temp_root());
        let conf = site.vhost_config(&paths, std::path::Path::new("/x/php.sock"),
            std::path::Path::new("/x/c.pem"), std::path::Path::new("/x/k.pem"));
        assert!(conf.contains("server_name demo.dev *.demo.dev;"));
    }
```

- [ ] **Step 2: Run tests** — `cargo test -p laragon-core sites` → FAIL (no `domains` field).

- [ ] **Step 3: Implement**

In `core/src/sites.rs`, add `pub domains: Vec<String>` to `struct Site`.

In `scan_sites`, set it when pushing (the hostname is computed just above; build domains from it):

```rust
        let hostname = format!("{name}.{tld}");
        sites.push(Site {
            domains: vec![hostname.clone()],
            hostname,
            root: entry.path(),
            name,
            source: SiteSource::Scanned,
            proxy: None,
        });
```

In `list_all_sites`, load the registry once (it already loads it). After building the base `sites` (scanned + linked + proxy) and before the final sort, **rewrite each site's domains/hostname from any override**:

```rust
    for s in sites.iter_mut() {
        if let Some(over) = registry.domains_for(&s.name) {
            s.domains = over.to_vec();
            s.hostname = over[0].clone();
        }
    }
```

For the linked and proxy pushes inside `list_all_sites`, add `domains: vec![hostname.clone()]` next to `hostname` (mirror scan_sites: compute `let hostname = format!("{}.{}", ...)` into a binding, then `domains: vec![hostname.clone()], hostname,`).

In `vhost_config`, change BOTH the proxy branch and the php branch `server_name` from `self.hostname` to the joined list. Add at the top of `vhost_config`:

```rust
        let server_names = self.domains.join(" ");
```

and replace `server_name {host};` occurrences with `server_name {names};` using `names = server_names` (the redirect server, the ssl server, and the proxy server blocks). Keep `$host` in the 301 redirect line unchanged.

In `core/src/lib.rs`, the sites re-export is unchanged (no new public item beyond the field).

- [ ] **Step 4: Run tests** — `cargo test -p laragon-core sites` (PASS, plus existing tests updated for the new field where they construct `Site` directly — update any direct `Site{...}` in sites.rs tests to include `domains`).
- [ ] **Step 5: Commit**

```bash
git add core/src/sites.rs
git commit -m "feat(core): Site.domains + multi-domain server_name"
```

---

### Task 3: multi-SAN cert (`ensure_cert(basename, names)`)

**Files:** Modify `core/src/ssl.rs`, `core/src/sync.rs`.

**Interfaces:** `CertIssuer::ensure_cert(&self, basename: &str, names: &[String]) -> Result<CertFiles, SslError>`; `FakeCertIssuer::requested() -> Arc<Mutex<Vec<(String, Vec<String>)>>>`.

- [ ] **Step 1: Write the failing test**

Replace `fake_issuer_records_and_returns_paths` in `core/src/ssl.rs` tests and add a SAN test:

```rust
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
```

(Also update `cert_and_key_paths_under_ssl_dir`/`issue_command_targets_*` to use a basename like `"app"` and `&["app.dev".to_string()]` where they call the changed methods; `cert_path`/`key_path` keep taking a single `&str` basename.)

- [ ] **Step 2: Run tests** — `cargo test -p laragon-core ssl` → FAIL.

- [ ] **Step 3: Implement**

In `core/src/ssl.rs`:
- Change the trait method to `fn ensure_cert(&self, basename: &str, names: &[String]) -> Result<CertFiles, SslError>;`.
- `cert_path`/`key_path` keep `(&self, basename: &str)`. Add `fn san_path(&self, basename: &str) -> PathBuf { self.ssl_dir.join(format!("{basename}.san")) }`.
- Change `issue_command` to `(&self, basename: &str, names: &[String]) -> SpawnSpec`: build `mkcert -cert-file <cert> -key-file <key> <names...>`:

```rust
    pub fn issue_command(&self, basename: &str, names: &[String]) -> SpawnSpec {
        let mut spec = SpawnSpec::new("mkcert")
            .arg("-cert-file").arg(self.cert_path(basename).display().to_string())
            .arg("-key-file").arg(self.key_path(basename).display().to_string());
        for n in names {
            spec = spec.arg(n.clone());
        }
        spec
    }
```

- Rewrite `MkcertIssuer::ensure_cert`:

```rust
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
        let status = std::process::Command::new(&spec.program).args(&spec.args).status()
            .map_err(|e| SslError::Mkcert(format!("spawn mkcert: {e}")))?;
        if !status.success() {
            return Err(SslError::Mkcert(format!("mkcert failed for {basename}")));
        }
        std::fs::write(&san, &want)?;
        Ok(CertFiles { cert, key })
    }
```

- `FakeCertIssuer`: change `requested: Arc<Mutex<Vec<(String, Vec<String>)>>>` + accessor type; impl:

```rust
    fn ensure_cert(&self, basename: &str, names: &[String]) -> Result<CertFiles, SslError> {
        self.requested.lock().unwrap().push((basename.to_string(), names.to_vec()));
        Ok(CertFiles {
            cert: self.base.join(format!("{basename}.pem")),
            key: self.base.join(format!("{basename}-key.pem")),
        })
    }
```

In `core/src/sync.rs`, update the call (minimal for now): `let certs = issuer.ensure_cert(&site.name, &[site.hostname.clone()])?;` (Task 4 switches to `&site.domains`).

- [ ] **Step 4: Run tests** — `cargo test -p laragon-core ssl sync` (PASS; update sync tests that read `issuer.requested()` to the new `(String, Vec<String>)` shape).
- [ ] **Step 5: Commit**

```bash
git add core/src/ssl.rs core/src/sync.rs
git commit -m "feat(core): multi-SAN cert (ensure_cert basename+names, .san reissue)"
```

---

### Task 4: `sync_sites` — multi-domain cert/hosts + wildcard bases

**Files:** Modify `core/src/sync.rs`, `laragonctl/src/main.rs`.

**Interfaces:** `sync_sites(...) -> Result<SyncOutcome, SyncError>` where `pub struct SyncOutcome { pub sites: Vec<Site>, pub warnings: Vec<String>, pub wildcard_bases: Vec<String> }`.

- [ ] **Step 1: Write the failing test**

Add to `core/src/sync.rs` tests (adapt to the existing helpers):

```rust
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
```

- [ ] **Step 2: Run** — `cargo test -p laragon-core sync` → FAIL (return type / fields).

- [ ] **Step 3: Implement**

In `core/src/sync.rs`:
- Add `pub struct SyncOutcome { pub sites: Vec<Site>, pub warnings: Vec<String>, pub wildcard_bases: Vec<String> }`.
- Change the signature to `-> Result<SyncOutcome, SyncError>`.
- In the body: cert over the full domain set; hosts from explicit domains; collect wildcard bases:

```rust
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
    // ... existing hosts read/apply, but use `explicit` instead of `hostnames` ...
    let updated = apply_block(&existing, &explicit);
    if updated != existing {
        privileged.write_etc_hosts(&updated)?;
    }
    Ok(SyncOutcome { sites, warnings, wildcard_bases })
```

In `laragonctl/src/main.rs`, change the `sync_sites` match arm from `Ok(sites) => println!("Synced {} site(s).", sites.0.len())` to `Ok(out) => println!("Synced {} site(s).", out.sites.len())`.

(The Tauri `commands.rs` calls all use `let _ = sync_sites(...)`, so they still compile; Task 8 binds the outcome where needed.)

- [ ] **Step 4: Run tests + build** — `cargo test -p laragon-core` then `cargo build -p laragonctl && cargo build -p laragon-desktop` (PASS; update the other sync tests to read `out.sites`/`out.warnings`).
- [ ] **Step 5: Commit**

```bash
git add core/src/sync.rs laragonctl/src/main.rs
git commit -m "feat(core): sync_sites emits SyncOutcome (multi-domain cert/hosts + wildcard bases)"
```

---

### Task 5: `Privileged` — systemd-resolved drop-in

**Files:** Modify `core/src/privileged.rs`.

**Interfaces:** `Privileged::{write_resolved_dropin(&str), remove_resolved_dropin()}`; `FakePrivileged` records calls.

- [ ] **Step 1: Write the failing tests**

Add to `core/src/privileged.rs` tests:

```rust
    #[test]
    fn fake_records_resolved_dropin() {
        let f = FakePrivileged::new();
        f.write_resolved_dropin("[Resolve]\nDNS=127.0.0.1:5353\n").unwrap();
        assert_eq!(f.resolved_dropins().lock().unwrap().len(), 1);
        f.remove_resolved_dropin().unwrap();
        assert!(*f.resolved_removed().lock().unwrap());
    }
```

- [ ] **Step 2: Run** — `cargo test -p laragon-core privileged` → FAIL.

- [ ] **Step 3: Implement**

Add to `trait Privileged`:

```rust
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError>;
    fn remove_resolved_dropin(&self) -> Result<(), PrivError>;
```

Free helpers + a script (near `mariadb_apparmor_argv`):

```rust
const RESOLVED_DROPIN: &str = "/etc/systemd/resolved.conf.d/laragon.conf";

fn write_resolved_argv(contents: &str) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "mkdir -p /etc/systemd/resolved.conf.d && cat > {RESOLVED_DROPIN} <<'LARAGONEOF'\n{contents}\nLARAGONEOF\nsystemctl reload systemd-resolved || systemctl restart systemd-resolved || true"
        ),
    ]
}

fn remove_resolved_argv() -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("rm -f {RESOLVED_DROPIN}; systemctl reload systemd-resolved || systemctl restart systemd-resolved || true"),
    ]
}
```

Implement for `SudoPrivileged` (`run_escalated("sudo", ...)`) and `PkexecPrivileged` (`run_escalated("pkexec", ...)`):

```rust
    fn write_resolved_dropin(&self, contents: &str) -> Result<(), PrivError> {
        run_escalated("pkexec", &write_resolved_argv(contents))
    }
    fn remove_resolved_dropin(&self) -> Result<(), PrivError> {
        run_escalated("pkexec", &remove_resolved_argv())
    }
```

(Sudo impl uses `"sudo"`.) Extend `FakePrivileged`: fields `resolved_dropins: Arc<Mutex<Vec<String>>>`, `resolved_removed: Arc<Mutex<bool>>`; accessors `resolved_dropins()`, `resolved_removed()`; impl pushes/sets and returns `Ok(())`.

- [ ] **Step 4: Run tests; Step 5: Commit**

Run: `cargo test -p laragon-core privileged` (PASS).

```bash
git add core/src/privileged.rs
git commit -m "feat(core): Privileged systemd-resolved drop-in (write/remove)"
```

---

### Task 6: `core::coredns` — download + Corefile + drop-in text

**Files:** Create `core/src/coredns.rs`; modify `core/src/lib.rs`.

**Interfaces:** `coredns_url(version, arch) -> String`, `ensure_coredns(paths, downloader, runner) -> Result<(), CorednsError>`, `corefile(bases, port) -> String`, `resolved_dropin(bases, port) -> String`, `enum CorednsError`, `const COREDNS_VERSION`, `coredns_arch() -> Option<&'static str>`.

- [ ] **Step 1: Write the failing tests**

Create `core/src/coredns.rs` with imports + tests first:

```rust
use crate::paths::LaragonPaths;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::Path;

pub const COREDNS_VERSION: &str = "1.14.4";

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
```

- [ ] **Step 2: Run** — `cargo test -p laragon-core coredns` → FAIL.

- [ ] **Step 3: Implement**

Add above the test module:

```rust
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
```

In `core/src/lib.rs`, add `pub mod coredns;` and `pub use coredns::{ensure_coredns, corefile, resolved_dropin, CorednsError};`.

- [ ] **Step 4: Run tests; Step 5: Commit**

Run: `cargo test -p laragon-core coredns` (PASS).

```bash
git add core/src/coredns.rs core/src/lib.rs
git commit -m "feat(core): coredns module (download + Corefile + resolved drop-in)"
```

---

### Task 7: `Coredns` service + orchestrator `set_coredns`

**Files:** Create `core/src/service/coredns.rs`; modify `core/src/service/mod.rs` (ServiceKind), `core/src/orchestrator.rs`.

**Interfaces:** `ServiceKind::Coredns`; `CorednsService::new(bases: Vec<String>, port: u16)`; `Orchestrator::set_coredns(&mut self, bases: Vec<String>) -> Result<(), ServiceError>`.

- [ ] **Step 1: Write the failing test**

Add to `core/src/orchestrator.rs` tests:

```rust
    #[test]
    fn set_coredns_runs_when_bases_present_and_stops_when_empty() {
        let tmp = std::env::temp_dir().join(format!("lara-cdns-{}", std::process::id()));
        let paths = LaragonPaths::new(tmp.clone());
        let spawner = crate::process::FakeSpawner::new();
        let log = spawner.log();
        let mut orch = Orchestrator::new(paths, vec![], Box::new(spawner));

        orch.set_coredns(vec!["demo.dev".to_string()]).unwrap();
        assert_eq!(orch.state(ServiceKind::Coredns), ServiceState::Running);
        assert_eq!(log.lock().unwrap().last().unwrap().program, "coredns");

        orch.set_coredns(vec![]).unwrap();
        assert_eq!(orch.state(ServiceKind::Coredns), ServiceState::Stopped);
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: Run** — `cargo test -p laragon-core orchestrator` → FAIL.

- [ ] **Step 3: Implement**

In `core/src/service/mod.rs`, add `Coredns` to `enum ServiceKind`. Add `pub mod coredns;` to the service module list (wherever `pub mod php_fpm;` is).

Create `core/src/service/coredns.rs`:

```rust
use crate::coredns::corefile;
use crate::paths::LaragonPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};

pub struct CorednsService {
    bases: Vec<String>,
    port: u16,
}

impl CorednsService {
    pub fn new(bases: Vec<String>, port: u16) -> Self {
        Self { bases, port }
    }
    fn conf_path(&self, paths: &LaragonPaths) -> std::path::PathBuf {
        paths.etc_for("coredns").join("Corefile")
    }
}

impl Service for CorednsService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Coredns
    }
    fn name(&self) -> &str {
        "coredns"
    }
    fn write_config(&self, paths: &LaragonPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("coredns"))?;
        std::fs::write(self.conf_path(paths), corefile(&self.bases, self.port))?;
        Ok(())
    }
    fn command(&self, paths: &LaragonPaths) -> SpawnSpec {
        SpawnSpec::new("coredns")
            .arg("-conf")
            .arg(self.conf_path(paths).display().to_string())
    }
    fn health_check(&self, _paths: &LaragonPaths) -> Result<(), ServiceError> {
        probe_tcp(self.port)
    }
}
```

(If `Service` has more required methods than the others implement, mirror the minimal set used by `MailpitService`/`RedisService`. `probe_tcp` is the helper used by other services; if its signature differs, match `redis.rs`'s health_check.)

In `core/src/orchestrator.rs`, add:

```rust
    pub fn set_coredns(&mut self, bases: Vec<String>) -> Result<(), ServiceError> {
        let was_running = self.state(ServiceKind::Coredns) == ServiceState::Running;
        if was_running {
            let _ = self.stop(ServiceKind::Coredns);
        }
        self.services.retain(|s| s.kind() != ServiceKind::Coredns);
        if bases.is_empty() {
            return Ok(());
        }
        self.services
            .push(Box::new(crate::service::coredns::CorednsService::new(bases, 5353)));
        self.start(ServiceKind::Coredns)
    }
```

- [ ] **Step 4: Run tests** — `cargo test -p laragon-core` (PASS; if `ServiceKind` is matched exhaustively anywhere, add the `Coredns` arm). Build `cargo build -p laragon-desktop`.
- [ ] **Step 5: Commit**

```bash
git add core/src/service/coredns.rs core/src/service/mod.rs core/src/orchestrator.rs
git commit -m "feat(core): CoreDNS service + Orchestrator::set_coredns"
```

---

### Task 8: IPC `set_site_domains` + wildcard wiring

**Files:** Modify `src-tauri/src/commands.rs`, `src-tauri/src/main.rs`.

**Interfaces:** `set_site_domains(app, name, domains) -> Result<Vec<Site>, String>`.

- [ ] **Step 1: Imports**

In `src-tauri/src/commands.rs`, add to the core imports: `ensure_coredns`, `resolved_dropin`, `SiteRegistry` (already imported), and a `sync` helper. Add `RealCommandRunner` (already imported), `CurlDownloader` (already imported).

- [ ] **Step 2: Add a private wildcard-apply helper**

Append to `commands.rs`:

```rust
/// Apply DNS state for the current wildcard bases: download+run CoreDNS and
/// write the resolved drop-in when present; otherwise stop CoreDNS and remove
/// the drop-in. Best-effort (failures are ignored; explicit domains still work).
fn apply_wildcard_dns(state: &AppState, bases: &[String]) {
    if bases.is_empty() {
        if let Ok(mut orch) = state.orch.lock() {
            let _ = orch.set_coredns(vec![]);
        }
        let _ = PkexecPrivileged.remove_resolved_dropin();
        return;
    }
    if ensure_coredns(&state.paths, &CurlDownloader, &RealCommandRunner).is_ok() {
        if let Ok(mut orch) = state.orch.lock() {
            let _ = orch.set_coredns(bases.to_vec());
        }
        let _ = PkexecPrivileged.write_resolved_dropin(&resolved_dropin(bases, 5353));
    }
}
```

- [ ] **Step 3: Add the command**

```rust
#[tauri::command]
pub async fn set_site_domains(
    app: tauri::AppHandle,
    name: String,
    domains: Vec<String>,
) -> Result<Vec<Site>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Site>, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry = SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry.set_domains(&name, &domains).map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::new(state.paths.ssl());
        let privileged = PkexecPrivileged;
        let outcome = sync_sites(
            &state.paths, &config.tld, &php_socket,
            std::path::Path::new("/etc/hosts"), &issuer, &privileged,
        );
        let bases = outcome.as_ref().map(|o| o.wildcard_bases.clone()).unwrap_or_default();
        apply_wildcard_dns(&state, &bases);
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            if orch.state(ServiceKind::Nginx) == ServiceState::Running {
                let _ = orch.stop(ServiceKind::Nginx);
                let _ = orch.start(ServiceKind::Nginx);
            }
        }
        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        Ok(sites)
    })
    .await
    .map_err(|e| e.to_string())?
}
```

(`sync_sites` now returns `Result<SyncOutcome, _>`; the existing `let _ = sync_sites(...)` calls elsewhere still compile.)

- [ ] **Step 4: Wildcard on Start All**

In `stack_start_all`, after the existing `sync_sites(...)` call (change it to bind the outcome) add, before locking the orchestrator to `start_all`:

```rust
        let bases = sync_sites(
            &state.paths, &config.tld, &php_socket,
            std::path::Path::new("/etc/hosts"), &issuer, &privileged,
        ).map(|o| o.wildcard_bases).unwrap_or_default();
        apply_wildcard_dns(&state, &bases);
```

(Replace the prior `let _ = sync_sites(...)` in `stack_start_all` with this bound version. Leave the other `let _ = sync_sites(...)` sites — create_site/link_site/add_proxy/update_proxy — as-is.)

- [ ] **Step 5: Register**

In `src-tauri/src/main.rs` `generate_handler!`, add `commands::set_site_domains,`.

- [ ] **Step 6: Build** — `cargo build -p laragon-desktop` (clean).
- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): set_site_domains + wildcard CoreDNS/resolved wiring"
```

---

### Task 9: Frontend — Edit-domains modal

**Files:** Modify `dist/app.js`.

**Interfaces:** `set_site_domains({ name, domains })`.

- [ ] **Step 1: Add state + JS domain validator**

In `state`, add:

```js
    siteDomains: { name: "", domains: [""], busy: false, error: "" },
```

After `validName`, add:

```js
  const DOMAIN_RE = /^(\*\.)?([a-z0-9]([a-z0-9-]*[a-z0-9])?\.)+[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;
  function validDomain(d) { return DOMAIN_RE.test(d); }
```

- [ ] **Step 2: Add open/close/row/submit helpers**

After the proxy helpers, add:

```js
  function openDomains(site) {
    const ds = (site.domains && site.domains.length) ? site.domains.slice() : [site.hostname];
    state.siteDomains = { name: site.name, domains: ds, busy: false, error: "" };
    state.modal = "domains";
    render();
  }
  function closeDomains() { if (state.siteDomains.busy) return; state.modal = null; render(); }
  function addDomainRow() { state.siteDomains.domains.push(""); render(); }
  function delDomainRow(i) { state.siteDomains.domains.splice(i, 1); if (!state.siteDomains.domains.length) state.siteDomains.domains.push(""); render(); }

  async function submitDomains() {
    const sd = state.siteDomains;
    const domains = sd.domains.map((d) => d.trim()).filter((d) => d.length);
    if (!domains.length) { sd.error = "Add at least one domain"; render(); return; }
    for (const d of domains) { if (!validDomain(d)) { sd.error = "Invalid domain: " + d; render(); return; } }
    sd.busy = true; sd.error = ""; render();
    try {
      const sites = await invoke("set_site_domains", { name: sd.name, domains });
      state.sites = Array.isArray(sites) ? sites : [];
      toast({ type: "success", title: "Domains updated", msg: domains.join(", ") });
      state.modal = null; render();
    } catch (e) {
      sd.error = String(e); sd.busy = false;
      toast({ type: "error", title: "Update failed", msg: String(e) });
      render();
    } finally {
      if (sd.busy) { sd.busy = false; render(); }
    }
  }
```

- [ ] **Step 3: Add the modal renderer**

After `proxyModal`, add `domainsModal()` mirroring its structure (`ns-*` classes): a title "Edit domains — <name>", one `ns-input` per domain (data-action `dm-input` data-idx), a remove button per row when >1, an "+ Add domain" link (`dm-add`), an error line, and Cancel/Save (`dm-close`/`dm-submit`). Use `esc()` on values. (Follow `proxyModal`'s exact markup conventions.)

- [ ] **Step 4: Wire render + handlers**

In `render`, extend the modal selection with `: state.modal === "domains" ? domainsModal()`.

In the click handler, add:

```js
    else if (a === "edit-domains") openDomains(state.sites.find((s) => s.name === el.getAttribute("data-name")));
    else if (a === "dm-close") closeDomains();
    else if (a === "dm-submit") submitDomains();
    else if (a === "dm-add") addDomainRow();
    else if (a === "dm-del") delDomainRow(parseInt(el.getAttribute("data-idx"), 10));
    else if (a === "dm-overlay-click") { if (e.target === el) closeDomains(); }
```

In the `input` handler, add: `if (el.dataset.action === "dm-input") { state.siteDomains.domains[parseInt(el.dataset.idx, 10)] = el.value; }`.

In the Esc handler and focus-trap guard, include `state.modal === "domains"`.

- [ ] **Step 5: Add the Edit-domains button to every site row**

In `sitesView`'s row builder, add (next to the other buttons, for ALL sites):

```js
            const domBtn = '<button class="btn-sm" data-action="edit-domains" data-name="' + esc(s.name) + '">Domains</button>';
```

and insert `domBtn` into the row markup (e.g. right after `editBtn`).

- [ ] **Step 6: Syntax-check** — `node --check dist/app.js` (exit 0).

- [ ] **Step 7: Manual verification (live)**

Run `cargo run -p laragon-desktop`. On a site, click **Domains** → add `app2.dev` and `*.demo.dev`, Save → toast; a pkexec prompt appears (hosts + resolved drop-in). `https://app2.dev` resolves (hosts) and `https://anything.demo.dev` resolves (CoreDNS). Removing the wildcard and saving stops CoreDNS + removes the drop-in. (DNS/cert/pkexec can't run in unit tests — human-verified.)

- [ ] **Step 8: Commit**

```bash
git add dist/app.js
git commit -m "feat(desktop): Edit-domains modal (explicit + wildcard) per site"
```

---

## Self-Review

**1. Spec coverage:** registry domains+validate (T1); Site.domains+server_name (T2); SAN cert (T3); sync split + wildcard bases (T4); resolved drop-in privileged (T5); coredns module download/configs (T6); CoreDNS service + set_coredns (T7); IPC set_site_domains + wildcard wiring + Start-All (T8); frontend modal (T9). All §3.x covered. ✓

**2. Placeholder scan:** No TBD. Manual gates (T9 Step 7) are explicit human-verified (DNS/cert/pkexec). Two steps say "mirror X's markup/minimal method set" (T7 service methods, T9 modal) — these reference a concrete existing file (redis.rs / proxyModal) the implementer copies from; acceptable since the exact shape lives in-repo.

**3. Type consistency:** `ensure_cert(basename, names)` (T3) used by sync (T4) and FakeCertIssuer; `SyncOutcome{sites,warnings,wildcard_bases}` (T4) consumed by laragonctl + commands (T8); `set_coredns(Vec<String>)` (T7) called by `apply_wildcard_dns` (T8); `ensure_coredns`/`resolved_dropin` (T6) used in T8; `write/remove_resolved_dropin` (T5) used in T8; `validate_domain`/`set_domains`/`domains_for` (T1) used in T2/T8; IPC `set_site_domains({name,domains})` matches the JS invoke (T9). Sequencing keeps each task compiling (commands' other `sync_sites` calls stay `let _`). ✓

**Note:** CoreDNS port is fixed at 5353 (matches `resolved_dropin`/`corefile`/`set_coredns`). The cert basename is the site name (stable across domain edits; `.san` sidecar forces re-issue when the domain set changes). `apply_wildcard_dns` is best-effort so a missing CoreDNS download or a cancelled pkexec never breaks the explicit-domain path.
