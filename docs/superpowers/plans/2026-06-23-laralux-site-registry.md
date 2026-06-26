# Site Registry / Add Existing Folder (Phase 2, Slice 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a site point at a folder outside `~/laralux/www/` (persisted in `~/laralux/sites.toml`), reachable at `https://<name>.dev` like any scanned site, addable via a native folder picker, and unlinkable from the UI.

**Architecture:** Add a `SiteSource` discriminator to `Site`, a new `site_registry` module persisting `{name, root}` to `sites.toml`, and a `list_all_sites` merge that combines the `www/` scan with the registry. `sync_sites` switches to the merged list so vhosts/certs/hosts cover linked sites. Tauri gains `link_site`/`unlink_site` commands and the `tauri-plugin-dialog` for a native folder chooser; the frontend gets an "Add existing folder" modal, a "linked" badge, and a Remove (unlink) action.

**Tech Stack:** Rust (laralux-core, no Tauri deps), Tauri 2 (`tauri-plugin-dialog`), vanilla JS frontend (`dist/`, `withGlobalTauri`).

## Global Constraints

- `core` keeps **zero Tauri deps**. The registry/merge logic lives in `laralux-core` behind plain functions; only `commands.rs`/`main.rs` touch Tauri.
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD for all core changes: write the failing test first, watch it fail, implement, watch it pass, commit.
- Site name rule (a valid DNS label): `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`, length 1–63. Reuse `scaffold::validate_site_name` in core; mirror the same regex in JS (`SITE_NAME_RE`, already present).
- Registry file path: `~/laralux/sites.toml`. New site source values serialize as exactly `"Scanned"` / `"Linked"`.
- Run all core tests with `cargo test -p laralux-core`; build the app with `cargo build -p laralux-desktop`. If `cargo` is not on PATH, use `$HOME/.cargo/bin/cargo`.

---

### Task 1: `site_registry` module + `LaraluxPaths::sites_file()`

**Files:**
- Modify: `core/src/paths.rs` (add `sites_file()`)
- Create: `core/src/site_registry.rs`
- Modify: `core/src/lib.rs` (declare module + re-export)

**Interfaces:**
- Consumes: `crate::paths::LaraluxPaths`, `crate::scaffold::validate_site_name`.
- Produces:
  - `LaraluxPaths::sites_file(&self) -> PathBuf` = `root.join("sites.toml")`.
  - `struct RegisteredSite { name: String, root: PathBuf }` (serde).
  - `struct SiteRegistry { sites: Vec<RegisteredSite> }` (serde) with `load(&Path) -> Result<SiteRegistry, RegistryError>`, `save(&self, &Path) -> Result<(), RegistryError>`, `add(&mut self, name: &str, root: &Path) -> Result<(), RegistryError>`, `remove(&mut self, name: &str) -> bool`, `sites(&self) -> &[RegisteredSite]`.
  - `enum RegistryError` (thiserror): `Io`, `Parse`, `Serialize`, `InvalidName(String)`, `RootNotFound(String)`, `Duplicate(String)`.

- [ ] **Step 1: Add `sites_file()` to paths.rs**

In `core/src/paths.rs`, after `config_file`:

```rust
    pub fn sites_file(&self) -> PathBuf {
        self.root.join("sites.toml")
    }
```

- [ ] **Step 2: Write the failing registry tests**

Create `core/src/site_registry.rs` with only the test module first (the rest compiles after Step 4):

```rust
use crate::scaffold::validate_site_name;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p laralux-core site_registry`
Expected: FAIL to compile — `SiteRegistry`, `RegistryError` not found.

- [ ] **Step 4: Implement the module**

Add above the `#[cfg(test)]` block in `core/src/site_registry.rs`:

```rust
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredSite {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRegistry {
    #[serde(default)]
    sites: Vec<RegisteredSite>,
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

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sites.len();
        self.sites.retain(|s| s.name != name);
        self.sites.len() != before
    }
}
```

- [ ] **Step 5: Declare the module and re-export in lib.rs**

In `core/src/lib.rs`, add `pub mod site_registry;` after `pub mod sites;`, and add:

```rust
pub use site_registry::{RegisteredSite, SiteRegistry, RegistryError};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p laralux-core site_registry`
Expected: PASS — all four registry tests green.

- [ ] **Step 7: Commit**

```bash
git add core/src/paths.rs core/src/site_registry.rs core/src/lib.rs
git commit -m "feat(core): add site registry (sites.toml) with add/remove/validate"
```

---

### Task 2: `SiteSource` on `Site` + `list_all_sites` merge

**Files:**
- Modify: `core/src/sites.rs` (add `SiteSource`, `source` field, set it in `scan_sites`, add `list_all_sites`, update tests)
- Modify: `core/src/lib.rs` (re-export `SiteSource`, `list_all_sites`)

**Interfaces:**
- Consumes: `scan_sites`, `SiteRegistry::load`, `LaraluxPaths::sites_file`.
- Produces:
  - `enum SiteSource { Scanned, Linked }` (serde, `Copy`).
  - `Site` gains `pub source: SiteSource`.
  - `pub fn list_all_sites(paths: &LaraluxPaths, tld: &str) -> std::io::Result<(Vec<Site>, Vec<String>)>` — merged + sorted sites and human-readable warnings.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/sites.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core sites`
Expected: FAIL to compile — `SiteSource`, `source`, `list_all_sites` not found.

- [ ] **Step 3: Add `SiteSource` and the `source` field**

In `core/src/sites.rs`, add above `struct Site`:

```rust
/// Where a site came from: the `www/` scan, or the explicit registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum SiteSource {
    Scanned,
    Linked,
}
```

Add the field to `Site`:

```rust
pub struct Site {
    pub name: String,
    pub root: PathBuf,
    pub hostname: String,
    pub source: SiteSource,
}
```

In `scan_sites`, set the field when pushing:

```rust
        sites.push(Site {
            hostname: format!("{name}.{tld}"),
            root: entry.path(),
            name,
            source: SiteSource::Scanned,
        });
```

- [ ] **Step 4: Implement `list_all_sites`**

Add to `core/src/sites.rs` after `scan_sites` (it needs `crate::site_registry::SiteRegistry`):

```rust
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
        sites.push(Site {
            hostname: format!("{}.{}", entry.name, tld),
            root: entry.root.clone(),
            name: entry.name.clone(),
            source: SiteSource::Linked,
        });
    }

    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((sites, warnings))
}
```

- [ ] **Step 5: Re-export in lib.rs**

In `core/src/lib.rs`, change the sites re-export line to:

```rust
pub use sites::{list_all_sites, scan_sites, Site, SiteSource};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p laralux-core sites`
Expected: PASS — the new tests plus the existing `sites` tests (which don't assert `source`).

- [ ] **Step 7: Commit**

```bash
git add core/src/sites.rs core/src/lib.rs
git commit -m "feat(core): add SiteSource and list_all_sites (scan + registry merge)"
```

---

### Task 3: `sync_sites` over the merged list (return warnings)

**Files:**
- Modify: `core/src/sync.rs` (use `list_all_sites`, return `(Vec<Site>, Vec<String>)`, update tests)
- Modify: `src-tauri/src/commands.rs` (destructure the two existing `sync_sites` callers)

**Interfaces:**
- Consumes: `list_all_sites`.
- Produces: `sync_sites(...) -> Result<(Vec<Site>, Vec<String>), SyncError>` — sites synced plus merge warnings.

- [ ] **Step 1: Update the sync tests for the new return type**

In `core/src/sync.rs` tests, change the call in `writes_vhosts_certs_and_hosts_block`:

```rust
        let (sites, _warnings) = sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();
```

(the rest of that test is unchanged), and add a new test:

```rust
    #[test]
    fn writes_vhost_for_linked_site_outside_www() {
        let r = root();
        std::fs::create_dir_all(r.join("www")).unwrap();
        let external = r.join("ext").join("linked");
        std::fs::create_dir_all(&external).unwrap();
        let paths = LaraluxPaths::new(r.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.add("linked", &external).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let hosts_path = r.join("hosts");
        std::fs::write(&hosts_path, "127.0.0.1 localhost\n").unwrap();
        let issuer = FakeCertIssuer::new(paths.ssl());
        let priv_ = FakePrivileged::new();
        let sock = paths.tmp().join("php-fpm.sock");

        let (sites, _w) = sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();
        assert!(sites.iter().any(|s| s.name == "linked"));
        assert!(paths.etc_for("nginx").join("sites").join("linked.conf").is_file());
        std::fs::remove_dir_all(&r).ok();
    }
```

(The `skips_hosts_write_when_block_already_current` test does not bind the return value, so it stays as-is.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core sync`
Expected: FAIL to compile — `sync_sites` still returns `Vec<Site>`, so the `(sites, _warnings)` destructure and the new test won't compile.

- [ ] **Step 3: Switch `sync_sites` to `list_all_sites` and return warnings**

In `core/src/sync.rs`, change the signature and body. Replace:

```rust
) -> Result<Vec<Site>, SyncError> {
    let sites = scan_sites(paths, tld)?;
```

with:

```rust
) -> Result<(Vec<Site>, Vec<String>), SyncError> {
    let (sites, warnings) = list_all_sites(paths, tld)?;
```

and change the final `Ok(sites)` to `Ok((sites, warnings))`. Update the `use` line `use crate::sites::{scan_sites, Site};` to `use crate::sites::{list_all_sites, Site};`.

- [ ] **Step 4: Run core tests to verify they pass**

Run: `cargo test -p laralux-core`
Expected: PASS — all core tests green (sync tests updated, others unaffected).

- [ ] **Step 5: Update the two callers in commands.rs**

In `src-tauri/src/commands.rs`, both `stack_start_all` and `create_site` call `sync_sites` as `let _ = sync_sites(...)`. The new tuple return still works with `let _ = ...`, but to keep the warnings visible, change each to bind and ignore explicitly:

In `stack_start_all` replace `let _ = sync_sites(` block's binding so it reads:

```rust
        let _ = sync_sites(
            &state.paths,
            &config.tld,
            &php_socket,
            std::path::Path::new("/etc/hosts"),
            &issuer,
            &privileged,
        );
```

This already compiles with the tuple (the whole `Result` is discarded). No change is strictly required here, but confirm both call sites still build. (The explicit handling of warnings happens in Task 4's new commands.)

- [ ] **Step 6: Build the app to verify callers compile**

Run: `cargo build -p laralux-desktop`
Expected: PASS — builds cleanly with the new `sync_sites` return type.

- [ ] **Step 7: Commit**

```bash
git add core/src/sync.rs src-tauri/src/commands.rs
git commit -m "feat(core): sync_sites uses list_all_sites and returns merge warnings"
```

---

### Task 4: IPC commands — `list_sites` merge, `link_site`, `unlink_site`

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs` (register the two new commands)

**Interfaces:**
- Consumes: `list_all_sites`, `SiteRegistry`, `sync_sites`, `Orchestrator`, `PkexecPrivileged`, `MkcertIssuer`, `PhpFpmService`.
- Produces:
  - `list_sites` now returns the merged list.
  - `link_site(app, name: String, root: String) -> Result<Site, String>` (async).
  - `unlink_site(app, name: String) -> Result<(), String>` (async).

- [ ] **Step 1: Extend the core imports**

In `src-tauri/src/commands.rs`, update the first `use laralux_core::{...}` block to also import `list_all_sites`, `SiteRegistry`, and `SiteSource`:

```rust
use laralux_core::{
    build_services, create_site as core_create_site, detect_components, list_all_sites, run_setup,
    sync_sites, Config, CreateReport, LaraluxPaths, MkcertIssuer, Orchestrator, PkexecPrivileged,
    RealCommandRunner, RealSpawner, ServiceKind, ServiceState, ServiceStatus, Site, SiteRegistry,
    SiteSource, SiteTemplate,
};
```

(`SiteSource` may be unused directly; if the build warns, drop it from this list — it is only needed if referenced. Keep `SiteRegistry`, `list_all_sites`.)

- [ ] **Step 2: Switch `list_sites` to the merged list**

Replace the body of `list_sites`:

```rust
#[tauri::command]
pub fn list_sites(state: tauri::State<AppState>) -> Result<Vec<Site>, String> {
    let (sites, _warnings) = list_all_sites(&state.paths, &state.tld).map_err(|e| e.to_string())?;
    Ok(sites)
}
```

- [ ] **Step 3: Add the `link_site` command**

Append to `src-tauri/src/commands.rs`. It mirrors `create_site`'s sync+reload tail:

```rust
#[tauri::command]
pub async fn link_site(
    app: tauri::AppHandle,
    name: String,
    root: String,
) -> Result<Site, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Site, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        // Register the folder (validates name, existence, duplicates).
        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry
            .add(&name, std::path::Path::new(&root))
            .map_err(|e| e.to_string())?;
        registry
            .save(&state.paths.sites_file())
            .map_err(|e| e.to_string())?;

        // Make it reachable: sync vhost+cert+/etc/hosts, then reload nginx if running.
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
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            if orch.state(ServiceKind::Nginx) == ServiceState::Running {
                let _ = orch.stop(ServiceKind::Nginx);
                let _ = orch.start(ServiceKind::Nginx);
            }
        }

        // Return the freshly linked site from the merged list.
        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        sites
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("linked site `{name}` not found after sync"))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 4: Add the `unlink_site` command**

Append to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn unlink_site(app: tauri::AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        let removed = registry.remove(&name);
        registry
            .save(&state.paths.sites_file())
            .map_err(|e| e.to_string())?;
        if !removed {
            return Err(format!("site `{name}` is not a linked site"));
        }

        // Remove the now-orphaned vhost so nginx stops serving it.
        let vhost = state
            .paths
            .etc_for("nginx")
            .join("sites")
            .join(format!("{name}.conf"));
        let _ = std::fs::remove_file(&vhost);

        // Re-sync (rewrites /etc/hosts without this host) and reload nginx.
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
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            if orch.state(ServiceKind::Nginx) == ServiceState::Running {
                let _ = orch.stop(ServiceKind::Nginx);
                let _ = orch.start(ServiceKind::Nginx);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 5: Register the commands in main.rs**

In `src-tauri/src/main.rs`, add to the `generate_handler!` list after `commands::create_site,`:

```rust
            commands::link_site,
            commands::unlink_site,
```

- [ ] **Step 6: Build the app**

Run: `cargo build -p laralux-desktop`
Expected: PASS — compiles cleanly. If `SiteSource` is unused, remove it from the import to clear the warning.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): add link_site/unlink_site commands; list merged sites"
```

---

### Task 5: Enable the native folder picker (`tauri-plugin-dialog`)

**Files:**
- Modify: `src-tauri/Cargo.toml` (add the plugin dependency)
- Modify: `src-tauri/src/main.rs` (register the plugin)
- Modify: `src-tauri/capabilities/default.json` (grant `dialog:allow-open`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `window.__TAURI__.dialog.open(...)` available in the webview (used by Task 6).

- [ ] **Step 1: Add the dependency**

In `src-tauri/Cargo.toml`, under `[dependencies]`, add:

```toml
tauri-plugin-dialog = "2"
```

- [ ] **Step 2: Register the plugin**

In `src-tauri/src/main.rs`, add `.plugin(tauri_plugin_dialog::init())` to the builder chain, immediately after `tauri::Builder::default()`:

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(build_state())
```

- [ ] **Step 3: Grant the open permission**

In `src-tauri/capabilities/default.json`, change `"permissions"` to:

```json
  "permissions": ["core:default", "dialog:allow-open"]
```

- [ ] **Step 4: Build to verify the plugin resolves**

Run: `cargo build -p laralux-desktop`
Expected: PASS — the plugin crate is fetched and the app builds. (First build downloads `tauri-plugin-dialog`.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/main.rs src-tauri/capabilities/default.json Cargo.lock
git commit -m "feat(desktop): enable tauri-plugin-dialog for native folder picker"
```

---

### Task 6: Frontend — Add-existing modal, Browse, linked badge, Remove

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes: `link_site({ name, root })`, `unlink_site({ name })`, `list_sites` (now returns `source`), `window.__TAURI__.dialog.open`.
- Produces: UI only.

The existing New Site modal (`state.modal === "newsite"`) is the template; the link modal reuses the same `ns-*` CSS classes via a parallel `state.modal === "linksite"` branch.

- [ ] **Step 1: Add link-site state and the basename→label helper**

In `dist/app.js`, in the `state` object near `newSite`, add:

```js
    linkSite: { root: "", name: "", busy: false, error: "" },
    confirmRemove: null,
```

After the `validName` function, add a sanitizer that turns a folder basename into a valid label:

```js
  function deriveName(path) {
    const base = (path || "").replace(/[\\/]+$/, "").split(/[\\/]/).pop() || "";
    return base.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 63);
  }
```

- [ ] **Step 2: Add open/close/browse/submit handlers for the link modal**

After `submitNewSite`, add:

```js
  function openLinkSite() {
    state.modal = "linksite";
    state.linkSite = { root: "", name: "", busy: false, error: "" };
    render();
    requestAnimationFrame(() => {
      const inp = document.getElementById("ls-name");
      if (inp) inp.focus();
    });
  }

  function closeLinkSite() {
    if (state.linkSite.busy) return;
    state.modal = null;
    render();
  }

  async function browseFolder() {
    try {
      const dlg = window.__TAURI__ && window.__TAURI__.dialog;
      if (!dlg) { toast({ type: "error", title: "Folder picker unavailable" }); return; }
      const picked = await dlg.open({ directory: true, multiple: false, title: "Choose project folder" });
      if (!picked) return; // cancelled
      const path = Array.isArray(picked) ? picked[0] : picked;
      state.linkSite.root = path;
      if (!state.linkSite.name) state.linkSite.name = deriveName(path);
      state.linkSite.error = "";
      render();
    } catch (e) {
      toast({ type: "error", title: "Folder picker failed", msg: String(e) });
    }
  }

  async function submitLinkSite() {
    const { root, name } = state.linkSite;
    if (!root) { state.linkSite.error = "Choose a folder first"; render(); return; }
    if (!validName(name)) { state.linkSite.error = "Use lowercase letters, digits, hyphens (e.g. my-app)"; render(); return; }
    state.linkSite.busy = true; state.linkSite.error = ""; render();
    try {
      const site = await invoke("link_site", { name, root });
      toast({ type: "success", title: "Linked " + site.name, msg: "https://" + site.hostname });
      state.modal = null;
      state.linkSite = { root: "", name: "", busy: false, error: "" };
      try {
        const sites = await invoke("list_sites");
        state.sites = Array.isArray(sites) ? sites : [];
      } catch (_) {}
      render();
    } catch (e) {
      state.linkSite.error = String(e);
      state.linkSite.busy = false;
      toast({ type: "error", title: "Link failed", msg: String(e) });
      render();
    } finally {
      if (state.linkSite.busy) { state.linkSite.busy = false; render(); }
    }
  }

  async function removeSite(name) {
    if (state.confirmRemove !== name) { state.confirmRemove = name; render(); return; }
    state.confirmRemove = null;
    try {
      await invoke("unlink_site", { name });
      toast({ type: "success", title: "Removed " + name });
      const sites = await invoke("list_sites");
      state.sites = Array.isArray(sites) ? sites : [];
      render();
    } catch (e) {
      toast({ type: "error", title: "Remove failed", msg: String(e) });
    }
  }
```

- [ ] **Step 3: Add the "Add existing folder" button and per-row badge/Remove in `sitesView`**

In `sitesView`, change the actions block (header) to include both buttons:

```js
      '<div class="sites-actions">' +
      '<button class="btn-newsite ghost" data-action="link-site">' + I.folder18 + "Add existing folder</button>" +
      '<button class="btn-newsite" data-action="new-site">' + I.plus + "New site</button></div></div>";
```

In the non-empty row `.map(...)`, replace the trailing buttons so linked sites get a badge + Remove. Change the row template's tail (after the `site-info` div) to:

```js
              (s.source === "Linked" ? '<span class="site-badge">linked</span>' : "") +
              '<button class="icon-btn sq32" data-action="copy-site" data-name="' + esc(s.name) + '" aria-label="Copy URL">' + I.copy + "</button>" +
              (s.source === "Linked"
                ? '<button class="btn-sm danger" data-action="remove-site" data-name="' + esc(s.name) + '">' +
                  (state.confirmRemove === s.name ? "Confirm?" : "Remove") + "</button>"
                : "") +
              '<a class="btn-sm" href="' + esc(url) + '" target="_blank" rel="noreferrer">' + I.external + "Open</a></div>"
```

(Remove the old disabled "More"/kebab button from the row to keep it tidy.)

- [ ] **Step 4: Add the link modal renderer**

After `newSiteModal`, add:

```js
  function linkSiteModal() {
    const ls = state.linkSite;
    const ok = ls.root && validName(ls.name);
    const preview = ls.name ? '<span class="ns-preview">→ https://' + esc(ls.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
    const errorHtml = ls.error ? '<div class="ns-error">' + esc(ls.error) + '</div>' : '';
    const d = ls.busy ? ' disabled' : '';
    const submitLabel = ls.busy ? '<span class="spin spinner on-primary"></span>Linking…' : 'Add site';
    return (
      '<div class="ns-overlay" data-action="ls-overlay-click" role="dialog" aria-modal="true" aria-labelledby="ls-title">' +
      '<div class="ns-card" role="document">' +
      '<div class="ns-head"><h2 class="ns-title" id="ls-title">Add existing folder</h2>' +
      '<button class="icon-btn" data-action="ls-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
      '<div class="ns-body">' +
      '<label class="ns-label" for="ls-root">Folder</label>' +
      '<div class="ls-row">' +
      '<input class="ns-input grow" type="text" id="ls-root" placeholder="/home/me/projects/my-app"' +
      ' value="' + esc(ls.root) + '" autocomplete="off" spellcheck="false"' + d + ' data-action="ls-root-input" />' +
      '<button class="btn btn-outline" data-action="ls-browse"' + d + '>Browse…</button>' +
      '</div>' +
      '<label class="ns-label" for="ls-name">Site name</label>' +
      '<input class="ns-input" type="text" id="ls-name" placeholder="my-app"' +
      ' value="' + esc(ls.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' + d + ' data-action="ls-name-input" />' +
      preview + errorHtml +
      '</div>' +
      '<div class="ns-foot">' +
      '<button class="btn btn-outline" data-action="ls-close"' + d + '>Cancel</button>' +
      '<button class="btn btn-primary' + (!ok || ls.busy ? ' btn-dim' : '') + '" data-action="ls-submit"' +
      (!ok || ls.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
      '</div></div></div>'
    );
  }
```

- [ ] **Step 5: Wire the link modal into `render` and the event handlers**

In `render`, change the modal line to render either modal:

```js
    const modalHtml = state.modal === "newsite" ? newSiteModal() : state.modal === "linksite" ? linkSiteModal() : "";
```

In the click handler, add cases (after the `ns-*` cases):

```js
    else if (a === "link-site") openLinkSite();
    else if (a === "remove-site") removeSite(el.getAttribute("data-name"));
    else if (a === "ls-close") closeLinkSite();
    else if (a === "ls-submit") submitLinkSite();
    else if (a === "ls-browse") browseFolder();
    else if (a === "ls-overlay-click") { if (e.target === el) closeLinkSite(); }
```

In the `input` handler, add link-modal field updates (after the `ns-name-input` block):

```js
    if (el.dataset.action === "ls-root-input") {
      state.linkSite.root = el.value;
      state.linkSite.error = "";
    }
    if (el.dataset.action === "ls-name-input") {
      state.linkSite.name = el.value;
      state.linkSite.error = "";
      const preview = document.querySelector(".ns-preview");
      if (preview) {
        if (el.value) { preview.classList.remove("muted"); preview.textContent = "→ https://" + el.value + ".dev"; }
        else { preview.classList.add("muted"); preview.innerHTML = "→ https://&lt;name&gt;.dev"; }
      }
      const submitBtn = document.querySelector('[data-action="ls-submit"]');
      if (submitBtn) { const ok = state.linkSite.root && validName(el.value); submitBtn.disabled = !ok; submitBtn.classList.toggle("btn-dim", !ok); }
    }
```

In the Esc handler, broaden the condition:

```js
    if (e.key === "Escape" && state.modal === "newsite") closeNewSite();
    else if (e.key === "Escape" && state.modal === "linksite") closeLinkSite();
```

In the focus-trap handler, change the guard `state.modal !== "newsite"` to also allow the link modal:

```js
    if (e.key !== "Tab" || (state.modal !== "newsite" && state.modal !== "linksite")) return;
```

- [ ] **Step 6: Add minimal CSS for the new bits**

Append to the end of `dist/styles.css` (or the existing stylesheet `dist/` uses — check `dist/index.html` for the linked CSS file and append there):

```css
.ls-row { display: flex; gap: 8px; align-items: center; }
.ls-row .grow { flex: 1 1 auto; }
.site-badge { font-size: 11px; padding: 2px 8px; border-radius: 999px; background: var(--chip-bg, #eef); color: var(--chip-fg, #446); align-self: center; }
.btn-sm.danger { color: #b42318; border-color: #f3c9c4; }
.btn-newsite.ghost { background: transparent; border: 1px solid var(--border, #d7d7e0); color: inherit; }
```

(If `dist/` has no separate stylesheet and styles are inline in `index.html`, append the same rules inside its `<style>` block instead.)

- [ ] **Step 7: Syntax-check the JS**

Run: `node --check dist/app.js`
Expected: PASS — no syntax errors (exit 0, no output).

- [ ] **Step 8: Manual verification (live)**

Run: `cargo run -p laralux-desktop` then: open **Sites → Add existing folder → Browse…**, pick a folder outside `www/`, confirm the name auto-fills and the preview shows `https://<name>.dev`, click **Add site**, confirm the toast and that the row shows a **linked** badge. Click **Remove → Confirm?**, confirm the site disappears. (No automated gate — UI is human-verified.)

- [ ] **Step 9: Commit**

```bash
git add dist/
git commit -m "feat(desktop): Add-existing-folder modal with native picker, linked badge, remove"
```

---

## Self-Review

**1. Spec coverage:**
- §3.1 registry module → Task 1. ✓
- §3.2 `SiteSource` + `list_all_sites` merge (shadow, stale-root warning, source) → Task 2. ✓
- §3.3 `sync_sites` over merged list + warnings + caller updates → Task 3. ✓
- §3.4 IPC `list_sites`/`link_site`/`unlink_site` (+ orphan vhost removal) → Task 4. ✓
- §3.5 native dialog plugin (Cargo + main.rs + capability) → Task 5. ✓
- §3.6 frontend (Add-existing modal, Browse, name derive, badge, Remove-with-confirm, a11y/Esc/focus-trap) → Task 6. ✓
- §4 decisions (inside-www shadowing, stale folder kept, hostname=name) covered by Task 2 logic + tests. ✓
- §5 error handling (typed errors → toast; malformed sites.toml degrades) → Task 1 errors + Task 2 warning path + Task 6 toasts. ✓
- §6 testing → tests in Tasks 1–3; frontend node-check + manual in Task 6. ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows full code. The only manual step (Task 6 Step 8) is explicitly a human-verified UI gate (no JS test runner exists in this repo).

**3. Type consistency:**
- `SiteRegistry::{load,save,add,remove,sites}` defined in Task 1, used identically in Tasks 2/3/4. ✓
- `list_all_sites(&LaraluxPaths, &str) -> io::Result<(Vec<Site>, Vec<String>)>` defined in Task 2, consumed in Tasks 3/4. ✓
- `sync_sites(...) -> Result<(Vec<Site>, Vec<String>), SyncError>` defined in Task 3, callers destructure or discard the whole `Result` (compatible). ✓
- `Site.source: SiteSource` (`"Scanned"`/`"Linked"`) defined in Task 2; JS compares `s.source === "Linked"` in Task 6. ✓
- IPC arg names `link_site({name, root})`, `unlink_site({name})` match between Task 4 (Rust params) and Task 6 (`invoke`). ✓

**Note:** First build of Task 5 downloads `tauri-plugin-dialog`; commit the updated `Cargo.lock`. The frontend stylesheet target in Task 6 Step 6 must be confirmed against `dist/index.html` (separate `.css` vs inline `<style>`) before appending.
