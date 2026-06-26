# Versioned Binary Layout Implementation Plan (Spec 0)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure `~/laralux/bin` into a per-tool, per-version layout (`bin/<tool>/<version>/<binary>`) with a config-driven `current` symlink per tool, so versions are managed like Laralux for Windows.

**Architecture:** Add a `layout` module (symlink materialization + resolution helpers) and a `Config.versions` map (source of truth). Switch every binary resolver call from the flat `bin/` dir to the set of `bin/*/current` dirs, move every installer to write `bin/<tool>/<version>/`, and adapt PHP version management, the php-fpm service, shell PATH, and the composer wrapper. Existing flat installs are not migrated — setup re-downloads cleanly into the new layout.

**Tech Stack:** Rust (laralux-core, zero Tauri deps), Tauri 2, serde/toml, unix symlinks.

## Global Constraints

- `core` keeps **zero Tauri deps**. Commit messages MUST NOT contain a `Co-Authored-By` trailer. TDD: failing test first.
- Layout: `~/laralux/bin/<tool>/<version>/<binary…>`; active pointer `~/laralux/bin/<tool>/current → <version>` (relative symlink, target = the bare version string).
- Tools and their binaries: `php`→`php`,`php-fpm`; `nginx`→`nginx`; `redis`→`redis-server`,`redis-cli`; `mailpit`→`mailpit`; `coredns`→`coredns`; `mkcert`→`mkcert`; `composer`→`composer.phar`,`composer`.
- Versions are **full**: PHP `8.3.31` (not `8.3`); mailpit/composer read from the binary (with a pinned fallback const); coredns uses `COREDNS_VERSION`.
- Config (`config.versions: BTreeMap<String,String>`, tool→version) is authoritative; `current` symlinks are materialized from it via `apply_versions`. Migrate the legacy `php_version` into `versions["php"]` on load.
- Resolver: `bin::resolve_bin(name, &layout::managed_bin_dirs(paths))` — the function is unchanged; only the dirs passed change. Its `$PATH` fallback still resolves apt nginx/redis during the Spec 0→1 interim.
- nginx/redis/mkcert installers are **Spec 1**; mariadb is **Spec 2**. This plan only restructures the layout + the already-downloaded tools (php, mailpit, coredns, composer).
- Run core tests `cargo test -p laralux-core`; build `cargo build -p laralux-desktop && cargo build -p laraluxctl`. If `cargo` isn't on PATH use `$HOME/.cargo/bin/cargo`.

Tasks are sequenced so each ends with a clean build: new helpers are added first (additive), consumers are switched next, and legacy functions are removed last.

---

### Task 1: `layout` module — symlink + resolution helpers

**Files:**
- Create: `core/src/layout.rs`
- Modify: `core/src/paths.rs` (add tool/version/current path accessors)
- Modify: `core/src/lib.rs` (declare `pub mod layout;` + re-export)

**Interfaces produced:**
- `LaraluxPaths::tool_dir(&self, tool: &str) -> PathBuf`, `version_dir(&self, tool, version) -> PathBuf`, `current_link(&self, tool) -> PathBuf`
- `layout::set_current(paths, tool, version) -> std::io::Result<()>`
- `layout::managed_bin_dirs(paths) -> Vec<PathBuf>`
- `layout::installed_versions(paths, tool) -> Vec<String>`

- [ ] **Step 1: Write the failing tests** — create `core/src/layout.rs` with:

```rust
use crate::paths::LaraluxPaths;
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin::resolve_bin;

    fn root() -> LaraluxPaths {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static C: AtomicUsize = AtomicUsize::new(0);
        let id = C.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("lara-layout-{}-{}", std::process::id(), id));
        let paths = LaraluxPaths::new(p);
        std::fs::create_dir_all(paths.bin()).unwrap();
        paths
    }

    #[test]
    fn set_current_points_and_repoints() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.4.10")).unwrap();
        std::fs::write(paths.version_dir("php", "8.3.31").join("php-fpm"), b"x").unwrap();
        set_current(&paths, "php", "8.3.31").unwrap();
        assert_eq!(std::fs::read_link(paths.current_link("php")).unwrap(), std::path::Path::new("8.3.31"));
        // resolves the binary through the current symlink
        let dirs = managed_bin_dirs(&paths);
        assert!(resolve_bin("php-fpm", &dirs).is_some());
        // repoint
        set_current(&paths, "php", "8.4.10").unwrap();
        assert_eq!(std::fs::read_link(paths.current_link("php")).unwrap(), std::path::Path::new("8.4.10"));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn installed_versions_lists_dirs_excluding_current() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::create_dir_all(paths.version_dir("php", "8.4.10")).unwrap();
        set_current(&paths, "php", "8.4.10").unwrap();
        assert_eq!(installed_versions(&paths, "php"), vec!["8.3.31".to_string(), "8.4.10".to_string()]);
        assert!(installed_versions(&paths, "nginx").is_empty());
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn managed_bin_dirs_collects_current_dirs() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        std::fs::create_dir_all(paths.version_dir("coredns", "1.14.4")).unwrap();
        set_current(&paths, "php", "8.3.31").unwrap();
        set_current(&paths, "coredns", "1.14.4").unwrap();
        let dirs = managed_bin_dirs(&paths);
        assert!(dirs.contains(&paths.current_link("php")));
        assert!(dirs.contains(&paths.current_link("coredns")));
        std::fs::remove_dir_all(paths.root()).ok();
    }
}
```

- [ ] **Step 2: Run** — `cargo test -p laralux-core layout` → FAIL (functions/accessors missing).

- [ ] **Step 3: Add the paths accessors** — in `core/src/paths.rs`, next to `bin()`:

```rust
    pub fn tool_dir(&self, tool: &str) -> PathBuf {
        self.bin().join(tool)
    }
    pub fn version_dir(&self, tool: &str, version: &str) -> PathBuf {
        self.bin().join(tool).join(version)
    }
    pub fn current_link(&self, tool: &str) -> PathBuf {
        self.bin().join(tool).join("current")
    }
```

- [ ] **Step 4: Implement the helpers** — above the `tests` module in `core/src/layout.rs`:

```rust
/// (Re)point `bin/<tool>/current` at `<version>` (relative target, so it
/// resolves inside the tool dir). Removes any existing `current` first.
pub fn set_current(paths: &LaraluxPaths, tool: &str, version: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(paths.tool_dir(tool))?;
    let link = paths.current_link(tool);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(version, &link)?;
    }
    Ok(())
}

/// Every `bin/<tool>/current` dir that exists — the search path for resolving
/// managed binaries. Sorted for determinism.
pub fn managed_bin_dirs(paths: &LaraluxPaths) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.bin()) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let cur = e.path().join("current");
                if cur.exists() {
                    dirs.push(cur);
                }
            }
        }
    }
    dirs.sort();
    dirs
}

/// Installed version dirs under `bin/<tool>`, excluding the `current` symlink.
/// Sorted by numeric version components.
pub fn installed_versions(paths: &LaraluxPaths, tool: &str) -> Vec<String> {
    let mut versions: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.tool_dir(tool)) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "current" {
                continue;
            }
            if e.path().is_dir() {
                versions.push(name);
            }
        }
    }
    versions.sort_by_key(|v| version_key(v));
    versions
}

/// Numeric version sort key, e.g. "8.3.31" -> [8,3,31]. Non-numeric parts -> 0.
fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}
```

- [ ] **Step 5: Declare + re-export** — in `core/src/lib.rs`, add `pub mod layout;` to the module list and `pub use layout::{managed_bin_dirs, set_current, installed_versions};` to the re-export block.

- [ ] **Step 6: Run** — `cargo test -p laralux-core layout` → PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

- [ ] **Step 7: Commit**

```bash
git add core/src/layout.rs core/src/paths.rs core/src/lib.rs
git commit -m "feat(core): layout helpers (tool/version dirs + current symlink + managed_bin_dirs)"
```

---

### Task 2: `Config.versions` map + legacy migration

**Files:** Modify `core/src/config.rs`.

**Interfaces produced:** `Config.versions: BTreeMap<String,String>`; `Config::tool_version(&self, tool) -> Option<&str>`; `Config::load` migrates `php_version` → `versions["php"]`.

- [ ] **Step 1: Write the failing tests** — add to `core/src/config.rs` tests:

```rust
    #[test]
    fn versions_defaults_empty_and_roundtrips() {
        let mut c = Config::default();
        assert!(c.versions.is_empty());
        c.versions.insert("php".into(), "8.3.31".into());
        let tmp = std::env::temp_dir().join(format!("lara-cfg-ver-{}.toml", std::process::id()));
        c.save(&tmp).unwrap();
        let back = Config::load(&tmp).unwrap();
        assert_eq!(back.tool_version("php"), Some("8.3.31"));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn legacy_php_version_migrates_into_versions_on_load() {
        let tmp = std::env::temp_dir().join(format!("lara-cfg-mig-{}.toml", std::process::id()));
        std::fs::write(&tmp, "tld = \"dev\"\nphp_version = \"8.3\"\n").unwrap();
        let c = Config::load(&tmp).unwrap();
        assert_eq!(c.tool_version("php"), Some("8.3"));
        std::fs::remove_file(&tmp).ok();
    }
```

- [ ] **Step 2: Run** — `cargo test -p laralux-core config` → FAIL.

- [ ] **Step 3: Implement** — in `core/src/config.rs`:
  - Add `use std::collections::BTreeMap;` at the top.
  - Add the field to `Config`: `#[serde(default)] pub versions: BTreeMap<String, String>,`
  - Add `versions: BTreeMap::new(),` to `Config::default()`.
  - Add a normalize step + accessor:

```rust
impl Config {
    fn normalize(mut self) -> Self {
        if !self.versions.contains_key("php") && !self.php_version.is_empty() {
            self.versions.insert("php".to_string(), self.php_version.clone());
        }
        self
    }
    pub fn tool_version(&self, tool: &str) -> Option<&str> {
        self.versions.get(tool).map(|s| s.as_str())
    }
}
```
  - In `load`, wrap the parsed config: change `Ok(toml::from_str(&text)?)` to `Ok(toml::from_str::<Config>(&text)?.normalize())` and the not-found arm to `Ok(Config::default().normalize())`.

- [ ] **Step 4: Run** — `cargo test -p laralux-core config` → PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean (the new field is `#[serde(default)]`, existing `Config{...}` literals in tests still compile because they use `..Default::default()` or full init — if any full struct literal of `Config` exists outside `Default`, add `versions: BTreeMap::new()`; grep `Config {` to confirm none break).

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): Config.versions map + legacy php_version migration"
```

---

### Task 3: `apply_versions` + `probe_version`

**Files:** Modify `core/src/layout.rs`, `core/src/lib.rs`.

**Interfaces produced:** `layout::apply_versions(paths, &Config) -> Vec<String>`; `layout::probe_version(program: &Path, args: &[&str]) -> Option<String>`.

- [ ] **Step 1: Write the failing tests** — add to `core/src/layout.rs` tests:

```rust
    #[test]
    fn apply_versions_materializes_present_and_warns_missing() {
        let paths = root();
        std::fs::create_dir_all(paths.version_dir("php", "8.3.31")).unwrap();
        let mut cfg = crate::config::Config::default();
        cfg.versions.insert("php".into(), "8.3.31".into());
        cfg.versions.insert("nginx".into(), "1.31.2".into()); // dir missing
        let warnings = apply_versions(&paths, &cfg);
        assert_eq!(std::fs::read_link(paths.current_link("php")).unwrap(), std::path::Path::new("8.3.31"));
        assert!(warnings.iter().any(|w| w.contains("nginx")));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn probe_version_extracts_semver() {
        // /bin/echo prints the arg; probe extracts the first semver token.
        assert_eq!(probe_version(std::path::Path::new("/bin/echo"), &["v1.2.3 extra"]), Some("1.2.3".to_string()));
        assert_eq!(probe_version(std::path::Path::new("/bin/echo"), &["no version here"]), None);
    }
```

- [ ] **Step 2: Run** — `cargo test -p laralux-core layout` → FAIL.

- [ ] **Step 3: Implement** — in `core/src/layout.rs` add (and `use crate::config::Config;` + `use std::path::Path;`):

```rust
/// Materialize `current` symlinks from config. Returns a warning per tool whose
/// configured version dir is missing. Best-effort: never aborts.
pub fn apply_versions(paths: &LaraluxPaths, config: &Config) -> Vec<String> {
    let mut warnings = Vec::new();
    for (tool, version) in &config.versions {
        if paths.version_dir(tool, version).is_dir() {
            if let Err(e) = set_current(paths, tool, version) {
                warnings.push(format!("{tool}: set_current failed: {e}"));
            }
        } else {
            warnings.push(format!("{tool}: version {version} not installed"));
        }
    }
    warnings
}

/// Run `program args`, capture stdout+stderr, return the first `N.N` or `N.N.N`
/// token found. Used to name mailpit/composer dirs by their real version.
pub fn probe_version(program: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    extract_version(&text)
}

/// First `\d+\.\d+(\.\d+)?` token in `s` (no regex dep — hand-scan).
fn extract_version(s: &str) -> Option<String> {
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || (bytes[i] == '.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())) {
                if bytes[i] == '.' { dots += 1; }
                i += 1;
            }
            if dots >= 1 {
                return Some(bytes[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    None
}
```

- [ ] **Step 4: Re-export** — in `core/src/lib.rs` extend the layout re-export to `pub use layout::{managed_bin_dirs, set_current, installed_versions, apply_versions, probe_version};`.

- [ ] **Step 5: Run** — `cargo test -p laralux-core layout` → PASS; build both crates clean.

- [ ] **Step 6: Commit**

```bash
git add core/src/layout.rs core/src/lib.rs
git commit -m "feat(core): apply_versions (config->symlinks) + probe_version"
```

---

### Task 4: Switch resolvers to `managed_bin_dirs`

**Files:** Modify `core/src/orchestrator.rs`, `core/src/bin.rs` (`ensure_nginx_bind_cap`), `core/src/setup.rs` (nginx/mkcert resolves).

**Interfaces:** consumes `layout::managed_bin_dirs`. No new public API. Pure rewiring — behavior preserved at the unit level (orchestrator tests use `FakeSpawner`).

- [ ] **Step 1: Modify `Orchestrator::do_start`** — change the program resolution line from:

```rust
            spec.program = crate::bin::resolve_or_name(&spec.program, &[self.paths.bin()]);
```
to:
```rust
            spec.program = crate::bin::resolve_or_name(&spec.program, &crate::layout::managed_bin_dirs(&self.paths));
```

- [ ] **Step 2: Modify `bin::ensure_nginx_bind_cap`** — change:

```rust
    if let Some(nginx) = resolve_bin("nginx", &[paths.bin()]) {
```
to:
```rust
    if let Some(nginx) = resolve_bin("nginx", &crate::layout::managed_bin_dirs(paths)) {
```

- [ ] **Step 3: Modify `setup.rs`** — the nginx setcap resolve at the bottom of `run_setup`:

```rust
    if let Some(nginx) = resolve_bin("nginx", &crate::layout::managed_bin_dirs(paths)) {
```
(replacing `&[paths.bin()]`). Leave other `resolve_bin` calls that legitimately scan system PATH unchanged.

- [ ] **Step 4: Run** — `cargo test -p laralux-core` → PASS (orchestrator/bin tests don't depend on real bin resolution); `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

- [ ] **Step 5: Commit**

```bash
git add core/src/orchestrator.rs core/src/bin.rs core/src/setup.rs
git commit -m "refactor(core): resolve managed binaries via bin/*/current dirs"
```

---

### Task 5: `php_static` installs into the layout (returns full version)

**Files:** Modify `core/src/php_static.rs`; update callers `core/src/setup.rs`, `core/src/php_cli.rs`, `src-tauri/src/commands.rs`.

**Interfaces produced:**
- `php_static::latest_patch(version, arch, sapi, json) -> Option<(String, String)>` (full version, url)
- `install_php_static(paths, requested, downloader, runner) -> Result<String, PhpStaticError>` (returns the full installed version, e.g. `"8.4.22"`; installs `bin/php/<full>/php-fpm` and `bin/php/<full>/php`; sets `current`)
- `install_php_cli(paths, requested, downloader, runner) -> Result<String, PhpStaticError>` (installs `bin/php/<full>/php`; returns full version)

- [ ] **Step 1: Replace `latest_patch_url` with `latest_patch`** — in `php_static.rs`, change the function to return both the full version and the URL:

```rust
pub fn latest_patch(version: &str, arch: &str, sapi: &str, listing_json: &str) -> Option<(String, String)> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(listing_json).ok()?;
    let prefix = format!("php-{version}.");
    let suffix = format!("-{sapi}-linux-{arch}.tar.gz");
    let mut best: Option<(u32, String)> = None;
    for e in &entries {
        let name = match e.get("name").and_then(|n| n.as_str()) { Some(n) => n, None => continue };
        if name.starts_with(&prefix) && name.ends_with(&suffix) {
            let mid = &name[prefix.len()..name.len() - suffix.len()];
            if let Ok(patch) = mid.parse::<u32>() {
                if best.as_ref().map_or(true, |(b, _)| patch > *b) {
                    best = Some((patch, name.to_string()));
                }
            }
        }
    }
    best.map(|(patch, name)| (format!("{version}.{patch}"), format!("{STATIC_PHP_BASE}/{name}")))
}
```
Update the existing `latest_patch_url_*` tests to call `latest_patch` and assert the tuple, e.g.:
```rust
    #[test]
    fn latest_patch_picks_highest_patch_for_arch_and_sapi() {
        let (ver, url) = latest_patch("8.4", "x86_64", "fpm", SAMPLE).unwrap();
        assert_eq!(ver, "8.4.22");
        assert_eq!(url, format!("{STATIC_PHP_BASE}/php-8.4.22-fpm-linux-x86_64.tar.gz"));
        assert!(latest_patch("7.4", "x86_64", "fpm", SAMPLE).is_none());
    }
```

- [ ] **Step 2: Rewrite `download_static_php` to take a destination dir + resolve the version**, and `install_php_static`/`install_php_cli` to install into `bin/php/<full>/` and `set_current`:

```rust
/// Download one SAPI tarball for `version`, extract `member`, install it as
/// `<dest_dir>/<dest_name>` (0755). Returns the full resolved version.
fn download_static_php(
    paths: &LaraluxPaths, version: &str, arch: &str, sapi: &str, member: &str,
    dest_dir: &std::path::Path, dest_name: &str, listing_json: &str,
    downloader: &dyn Downloader, runner: &dyn CommandRunner,
) -> Result<String, PhpStaticError> {
    let (full, url) = latest_patch(version, arch, sapi, listing_json)
        .ok_or_else(|| PhpStaticError::Unavailable(version.to_string()))?;
    let tarball = paths.tmp().join(format!("php-{full}-{sapi}.tar.gz"));
    downloader.fetch(&url, &tarball).map_err(|e| PhpStaticError::Download(e.to_string()))?;
    std::fs::create_dir_all(dest_dir)?;
    runner.run("tar", &[
        "-xzf".to_string(), tarball.display().to_string(),
        "-C".to_string(), paths.tmp().display().to_string(), member.to_string(),
    ], None).map_err(|e| PhpStaticError::Extract(e.to_string()))?;
    let extracted = paths.tmp().join(member);
    let dest = dest_dir.join(dest_name);
    std::fs::rename(&extracted, &dest).or_else(|_| {
        std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(full)
}

pub fn install_php_static(
    paths: &LaraluxPaths, requested: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner,
) -> Result<String, PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    let json = fetch_index(paths, downloader)?;
    // Resolve the full version once (from the fpm entry) so both SAPIs share a dir.
    let (full, _) = latest_patch(requested, arch, "fpm", &json)
        .ok_or_else(|| PhpStaticError::Unavailable(requested.to_string()))?;
    let dir = paths.version_dir("php", &full);
    download_static_php(paths, requested, arch, "fpm", "php-fpm", &dir, "php-fpm", &json, downloader, runner)?;
    download_static_php(paths, requested, arch, "cli", "php", &dir, "php", &json, downloader, runner)?;
    crate::layout::set_current(paths, "php", &full)?;
    Ok(full)
}

pub fn install_php_cli(
    paths: &LaraluxPaths, requested: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner,
) -> Result<String, PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    let json = fetch_index(paths, downloader)?;
    let (full, _) = latest_patch(requested, arch, "cli", &json)
        .ok_or_else(|| PhpStaticError::Unavailable(requested.to_string()))?;
    let dir = paths.version_dir("php", &full);
    download_static_php(paths, requested, arch, "cli", "php", &dir, "php", &json, downloader, runner)?;
    crate::layout::set_current(paths, "php", &full)?;
    Ok(full)
}
```
Update the `install_php_static_installs_fpm_and_cli` test assertions to the new layout:
```rust
        let full = install_php_static(&paths, "8.4", &dl, &runner).unwrap();
        assert_eq!(full, "8.4.22");
        assert!(paths.version_dir("php", "8.4.22").join("php-fpm").is_file());
        assert!(paths.version_dir("php", "8.4.22").join("php").is_file());
        assert_eq!(std::fs::read_link(paths.current_link("php")).unwrap(), std::path::Path::new("8.4.22"));
```

- [ ] **Step 3: Update callers** so the crate compiles with the new `Result<String>`:
  - `core/src/php_cli.rs` `ensure_active_php_cli`: change the existence check + install (full rewrite in Task 7; for now make it compile) — replace its body's `install_php_cli(...)?;` with `let _ = install_php_cli(paths, version, downloader, runner)?;` (it already discards). It still calls `set_active_php` which Task 7 rewrites; leave as-is here.
  - `core/src/setup.rs` `run_setup` PHP block: `install_php_static` now returns the version. Replace the `match crate::php_static::install_php_static(...) { Ok(()) => match detect_php_fpm_version... }` block with:
```rust
    if missing.contains(&Component::Php) {
        match crate::php_static::install_php_static(paths, crate::php_versions::DEFAULT_PHP_VERSION, downloader, runner) {
            Ok(full) => {
                report.php_version = Some(full.clone());
                let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
                cfg.versions.insert("php".to_string(), full.clone());
                cfg.php_version = full.clone();
                if let Err(e) = cfg.save(&paths.config_file()) {
                    report.errors.push(format!("persist php version: {e}"));
                }
            }
            Err(e) => report.errors.push(format!("install php (static): {e}")),
        }
    }
```
  (This drops the `detect_php_fpm_version` + `set_active_php` calls here; `install_php_static` already `set_current`s, and config now records the full version. The `set_active_php` reconciliation lives in Task 7's switch flow.)
  - `src-tauri/src/commands.rs` `install_php_version` (~line 403): it calls `install_php_static(...)` then likely returns versions. Change the call to bind the version and ignore or use it: `let _full = install_php_static(&state.paths, &version, &CurlDownloader, &RealCommandRunner).map_err(|e| e.to_string())?;` Keep the rest of that command as-is for now (Task 6/8 finish the php commands).

- [ ] **Step 4: Run** — `cargo test -p laralux-core php_static` and `cargo test -p laralux-core` → PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

- [ ] **Step 5: Commit**

```bash
git add core/src/php_static.rs core/src/setup.rs core/src/php_cli.rs src-tauri/src/commands.rs
git commit -m "feat(core): php_static installs into bin/php/<version>/ and sets current"
```

---

### Task 6: PHP version catalog from the layout

**Files:** Modify `core/src/php_versions.rs`, `core/src/bin.rs` (remove the php-fpm name scanners), `core/src/setup.rs` (`detect`), `src-tauri/src/commands.rs`, `core/src/lib.rs`.

**Interfaces produced:** `php_versions(paths, active)` lists installed **minor** versions derived from `installed_versions(paths,"php")`; `bin::detect_php_fpm_version`/`list_php_fpm_versions` are removed.

- [ ] **Step 1: Write the failing test** — in `core/src/php_versions.rs` tests, add a helper-level test that minor extraction works:

```rust
    #[test]
    fn installed_minors_dedupes_patches() {
        // 8.3.31 and 8.3.40 both count as installed minor "8.3"
        let minors = installed_minors(&["8.3.31".to_string(), "8.3.40".to_string(), "8.4.10".to_string()]);
        assert_eq!(minors, vec!["8.3".to_string(), "8.4".to_string()]);
    }
```

- [ ] **Step 2: Run** — `cargo test -p laralux-core php_versions` → FAIL (`installed_minors` missing).

- [ ] **Step 3: Implement** — in `core/src/php_versions.rs`:
  - Replace `use crate::bin::list_php_fpm_versions;` with `use crate::layout::installed_versions;`.
  - Add the minor-derivation helper:
```rust
/// Reduce full patch versions to sorted unique major.minor strings.
pub fn installed_minors(full_versions: &[String]) -> Vec<String> {
    let mut minors: Vec<String> = Vec::new();
    for v in full_versions {
        let mut it = v.split('.');
        if let (Some(maj), Some(min)) = (it.next(), it.next()) {
            let m = format!("{maj}.{min}");
            if !minors.contains(&m) {
                minors.push(m);
            }
        }
    }
    minors.sort_by_key(|v| vkey(v));
    minors
}
```
  - Change `php_versions` to read the layout and reduce the active full version to its minor:
```rust
pub fn php_versions(paths: &LaraluxPaths, active: &str) -> Vec<PhpVersionInfo> {
    let full = installed_versions(paths, "php");
    let installed = installed_minors(&full);
    // `active` may be a full version ("8.3.31") or a minor ("8.3"); compare on minor.
    let active_minor = installed_minors(std::slice::from_ref(&active.to_string()))
        .into_iter().next().unwrap_or_else(|| active.to_string());
    php_versions_from(&installed, &active_minor)
}
```

- [ ] **Step 4: Remove the obsolete php-fpm name scanners** — in `core/src/bin.rs`, delete `parse_php_version`, `detect_php_fpm_version_in`, `detect_php_fpm_version`, `list_php_fpm_versions_in`, `list_php_fpm_versions`, and their unit tests (`detects_highest_php_fpm_version`, `no_php_fpm_returns_none`, `lists_all_php_fpm_versions_sorted`, `lists_empty_when_none`). In `core/src/lib.rs`, remove `list_php_fpm_versions` from the `pub use bin::{...}` re-export.

- [ ] **Step 5: Update the remaining callers**:
  - `core/src/setup.rs` `detect`: change the PHP arm from `Component::Php => crate::bin::detect_php_fpm_version(&[paths.bin()]).is_some(),` to `Component::Php => crate::bin::resolve_bin("php-fpm", &crate::layout::managed_bin_dirs(paths)).is_some(),`.
  - `src-tauri/src/commands.rs`: remove `list_php_fpm_versions` from the `laralux_core` import list. The `set_php_version` guard at ~line 419 (`if !list_php_fpm_versions(&[state.paths.bin()]).contains(&version)`) changes to check installed minors:
```rust
        let installed = laralux_core::php_versions(&state.paths, &config.php_version);
        if !installed.iter().any(|p| p.version == version && p.installed) {
            // fall through to install (existing behavior)
        }
```
  (Keep the existing install-then-switch logic; only the "is it installed" check changes. If the existing code structure differs, preserve its intent: treat `version` as a minor, installed iff `php_versions` marks it installed.)

- [ ] **Step 6: Run** — `cargo test -p laralux-core` → PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

- [ ] **Step 7: Commit**

```bash
git add core/src/php_versions.rs core/src/bin.rs core/src/setup.rs core/src/lib.rs src-tauri/src/commands.rs
git commit -m "feat(core): PHP version catalog from bin/php/* layout; drop name-scanners"
```

---

### Task 7: php-fpm service, version switch, composer wrapper, shell PATH

**Files:** Modify `core/src/service/php_fpm.rs`, `core/src/orchestrator.rs`, `core/src/php_cli.rs`, `core/src/shell_env.rs`; update `src-tauri/src/commands.rs`.

**Interfaces produced:** `PhpFpmService::command` runs `php-fpm` (resolved via `bin/php/current`); `Orchestrator::replace_php_version(version)` = `set_current("php",version)` + restart; `set_active_php` = `layout::set_current`; composer wrapper + shell PATH use `bin/php/current` + `bin/composer/current`.

- [ ] **Step 1: php-fpm program name** — in `core/src/service/php_fpm.rs` `command`, change:
```rust
        SpawnSpec::new(format!("php-fpm{}", self.version))
```
to:
```rust
        SpawnSpec::new("php-fpm")
```
(The `version` field stays for the config path `etc/php/<version>/php-fpm.conf`; resolution now goes through `bin/php/current/php-fpm`.)

- [ ] **Step 2: `replace_php_version`** — in `core/src/orchestrator.rs`, rewrite so it repoints `current` and restarts (no service struct swap, since the program name is constant):
```rust
    pub fn replace_php_version(&mut self, version: &str) -> Result<bool, ServiceError> {
        let was_running = self.state(ServiceKind::PhpFpm) == ServiceState::Running;
        if was_running {
            let _ = self.stop(ServiceKind::PhpFpm);
        }
        crate::layout::set_current(&self.paths, "php", version)
            .map_err(|e| ServiceError::Config(format!("set php current: {e}")))?;
        if was_running {
            self.start(ServiceKind::PhpFpm)?;
        }
        Ok(was_running)
    }
```
Update the two orchestrator tests that call `replace_php_version("8.3")`: they must first create `bin/php/8.3/php-fpm` + a registered `PhpFpmService` so `set_current` has a target (or assert on the symlink). Adapt minimally so they pass.

- [ ] **Step 3: `set_active_php`** — in `core/src/php_cli.rs`, replace the body of `set_active_php` with a delegate, and fix `ensure_active_php_cli` to the layout:
```rust
pub fn set_active_php(paths: &LaraluxPaths, version: &str) -> std::io::Result<()> {
    crate::layout::set_current(paths, "php", version)
}

pub fn ensure_active_php_cli(
    paths: &LaraluxPaths, version: &str, downloader: &dyn Downloader, runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    if !paths.version_dir("php", version).join("php").is_file() {
        let _ = install_php_cli(paths, version, downloader, runner)?;
    }
    set_active_php(paths, version).map_err(PhpStaticError::Io)?;
    Ok(())
}
```
Update `set_active_php_points_php_to_versioned_binary` and `ensure_active_php_cli_symlinks_without_download_when_present` tests to the layout: create `bin/php/8.4.10/php`, call with the full version, assert `current → 8.4.10`.
(`ensure_active_php_cli` callers in `commands.rs` pass `config.php_version`; once Task 6 records full versions, that's a full version — fine. The CLI-sync path may pass a minor; `install_php_cli` resolves the latest patch and `set_current`s the full version. To keep the existence check meaningful for a minor, guard on `installed_versions(paths,"php").iter().any(|v| v.starts_with(version))` — but YAGNI: the simple `version_dir(version)` check above plus the install fallback is correct because `install_php_cli` is idempotent.)

- [ ] **Step 4: composer wrapper** — in `core/src/php_cli.rs` `install_composer`, write into the layout and point the wrapper at absolute `$HOME` paths. First read the version, then place files:
```rust
pub fn install_composer(paths: &LaraluxPaths, downloader: &dyn Downloader) -> std::io::Result<()> {
    // Download to tmp, read its version, then place into bin/composer/<version>/.
    let tmp_phar = paths.tmp().join("composer.phar");
    std::fs::create_dir_all(paths.tmp())?;
    downloader.fetch(COMPOSER_URL, &tmp_phar)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let php = paths.current_link("php").join("php");
    let version = crate::layout::probe_version(&php, &[tmp_phar.to_string_lossy().as_ref(), "--version"])
        .unwrap_or_else(|| COMPOSER_FALLBACK_VERSION.to_string());
    let dir = paths.version_dir("composer", &version);
    std::fs::create_dir_all(&dir)?;
    let phar = dir.join("composer.phar");
    std::fs::rename(&tmp_phar, &phar).or_else(|_| {
        std::fs::copy(&tmp_phar, &phar).map(|_| ()).and_then(|_| std::fs::remove_file(&tmp_phar))
    })?;
    let wrapper = dir.join("composer");
    std::fs::write(&wrapper,
        "#!/bin/sh\nexec \"$HOME/laralux/bin/php/current/php\" \"$HOME/laralux/bin/composer/current/composer.phar\" \"$@\"\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
    }
    crate::layout::set_current(paths, "composer", &version)?;
    Ok(())
}
```
Add near the top of `php_cli.rs`: `pub const COMPOSER_FALLBACK_VERSION: &str = "2.8.9";`. Update the `install_composer_writes_phar_and_wrapper` test: it must seed `bin/php/current/php` (any executable; the `probe_version` will fail and fall back to the const, which is fine) and assert `bin/composer/<COMPOSER_FALLBACK_VERSION>/composer.phar` + wrapper exist and the wrapper contains `$HOME/laralux/bin/php/current/php`. (Because the FakeDownloader writes dummy bytes and there's no real php, `probe_version` returns `None` → fallback const — deterministic.)

- [ ] **Step 5: shell PATH** — in `core/src/shell_env.rs`, change the exported PATH line(s) so the managed block prepends the two CLI tool dirs. Find the block string that contains `$HOME/laralux/bin` and replace that path with `$HOME/laralux/bin/php/current:$HOME/laralux/bin/composer/current`. Update the shell_env tests that assert the block content to expect the new paths.

- [ ] **Step 6: commands.rs** — `set_php_version` (~line 427) already calls `orch.replace_php_version(&version)` then persists config; ensure it also records the version into `config.versions`:
```rust
        config.versions.insert("php".to_string(), version.clone());
        config.php_version = version.clone();
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
```
(Place this next to the existing `config.php_version = ...; config.save(...)` — keep one save.)

- [ ] **Step 7: Run** — `cargo test -p laralux-core` → PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

- [ ] **Step 8: Commit**

```bash
git add core/src/service/php_fpm.rs core/src/orchestrator.rs core/src/php_cli.rs core/src/shell_env.rs src-tauri/src/commands.rs
git commit -m "feat(core): php-fpm/composer/shell use bin/<tool>/current; version switch repoints symlink"
```

---

### Task 8: mailpit + coredns install into the layout

**Files:** Modify `core/src/setup.rs` (mailpit block), `core/src/coredns.rs` (`ensure_coredns`).

**Interfaces produced:** mailpit installs into `bin/mailpit/<version>/mailpit` + `current`; `ensure_coredns` installs into `bin/coredns/<COREDNS_VERSION>/coredns` + `current`.

- [ ] **Step 1: coredns into the layout** — in `core/src/coredns.rs` `ensure_coredns`, change the destination from `paths.bin().join("coredns")` to the versioned dir and set current. Replace the `dest`/early-return/extract/rename tail with:
```rust
    let dir = paths.version_dir("coredns", COREDNS_VERSION);
    let dest = dir.join("coredns");
    if coredns_installed(&dest) {
        let _ = crate::layout::set_current(paths, "coredns", COREDNS_VERSION);
        return Ok(());
    }
    let arch = coredns_arch().ok_or_else(|| CorednsError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.tmp())?;
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_file(&dest);
    let tgz = paths.tmp().join("coredns.tgz");
    downloader.fetch(&coredns_url(COREDNS_VERSION, arch), &tgz).map_err(|e| CorednsError::Download(e.to_string()))?;
    let extract_dir = paths.tmp().join("coredns-extract");
    std::fs::create_dir_all(&extract_dir)?;
    runner.run("tar", &["-xzf".into(), tgz.display().to_string(), "-C".into(), extract_dir.display().to_string(), "coredns".into()], None)
        .map_err(|e| CorednsError::Extract(e.to_string()))?;
    let extracted = extract_dir.join("coredns");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&extracted, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&extracted, &dest).or_else(|_| {
        std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
    })?;
    crate::layout::set_current(paths, "coredns", COREDNS_VERSION)?;
    Ok(())
```
(Keep the `coredns_installed` helper and `CorednsError`. The orchestrator already resolves `coredns` via `managed_bin_dirs` from Task 4.)

- [ ] **Step 2: mailpit into the layout** — in `core/src/setup.rs`, replace the mailpit extract block (the `tar -xzf … -C paths.bin() mailpit` and surrounding) with: extract into tmp, read version, move into `bin/mailpit/<ver>/mailpit`, set current. Concretely, inside the `Ok(())` arm after `downloader.fetch(MAILPIT_URL, &tarball)`:
```rust
                report.mailpit_fetched = true;
                let extract_dir = paths.tmp().join("mailpit-extract");
                let _ = std::fs::create_dir_all(&extract_dir);
                let out = std::process::Command::new("tar")
                    .arg("-xzf").arg(&tarball).arg("-C").arg(&extract_dir).arg("mailpit").output();
                match out {
                    Ok(o) if o.status.success() => {
                        let extracted = extract_dir.join("mailpit");
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&extracted, std::fs::Permissions::from_mode(0o755));
                        }
                        let ver = crate::layout::probe_version(&extracted, &["version"])
                            .unwrap_or_else(|| MAILPIT_FALLBACK_VERSION.to_string());
                        let dir = paths.version_dir("mailpit", &ver);
                        let _ = std::fs::create_dir_all(&dir);
                        let dest = dir.join("mailpit");
                        let moved = std::fs::rename(&extracted, &dest).or_else(|_| {
                            std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
                        });
                        match moved {
                            Ok(()) => {
                                let _ = crate::layout::set_current(paths, "mailpit", &ver);
                                let mut cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
                                cfg.versions.insert("mailpit".to_string(), ver);
                                let _ = cfg.save(&paths.config_file());
                            }
                            Err(e) => report.errors.push(format!("install mailpit: {e}")),
                        }
                    }
                    Ok(o) => report.errors.push(format!("tar extract mailpit failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
                    Err(e) => report.errors.push(format!("tar spawn: {e}")),
                }
```
Add near the top of `setup.rs`: `pub const MAILPIT_FALLBACK_VERSION: &str = "1.20.0";`.

- [ ] **Step 3: Run** — `cargo test -p laralux-core` → PASS (the coredns unit test only covers pure fns; the layout change is verified live). `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

- [ ] **Step 4: Commit**

```bash
git add core/src/coredns.rs core/src/setup.rs
git commit -m "feat(core): coredns + mailpit install into bin/<tool>/<version>/"
```

---

### Task 9: `run_setup` records versions + applies symlinks; composer ordering

**Files:** Modify `core/src/setup.rs`.

**Interfaces:** `run_setup` installs composer via `install_composer` (after PHP), records `versions` for coredns/composer, and calls `apply_versions` at the end.

- [ ] **Step 1: Install composer in run_setup (after PHP), record coredns version** — in `core/src/setup.rs` `run_setup`, after the PHP-static block and before mailpit, add a composer install for the missing component and record coredns once installed elsewhere. Add, where the apt block used to cover composer:
```rust
    if missing.contains(&Component::Composer) {
        if let Err(e) = crate::php_cli::install_composer(paths, downloader) {
            report.errors.push(format!("install composer: {e}"));
        } else {
            report.composer_fetched = true;
        }
    }
```
(`SetupReport` gains `composer_fetched: bool` — add the field + initialize it `false`, mirroring `mailpit_fetched`.)

- [ ] **Step 2: Apply symlinks at the end of run_setup** — just before `report` is returned, reconcile all `current` symlinks from the freshly-written config:
```rust
    let cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
    for w in crate::layout::apply_versions(paths, &cfg) {
        report.errors.push(format!("apply versions: {w}"));
    }
```

- [ ] **Step 3: Empty the apt packages for the now-downloaded tools** — in `apt_packages_for`, change `Component::Composer` to return `Vec::new()` (composer is downloaded now). Leave nginx/redis/mkcert on apt (Spec 1 removes them) and mariadb (Spec 2). Update the `apt_packages_for_php_is_empty` neighbour test region if a composer-specific assertion exists.

- [ ] **Step 4: Run** — `cargo test -p laralux-core` → PASS; `cargo build -p laralux-desktop && cargo build -p laraluxctl` clean.

- [ ] **Step 5: Manual verification (live)** — delete the old flat `~/laralux/bin` contents, run the desktop app's setup, and confirm: `~/laralux/bin/php/<ver>/{php,php-fpm}`, `bin/php/current → <ver>`, `bin/composer/<ver>/composer.phar`+wrapper, `bin/mailpit/<ver>/mailpit`, `bin/coredns/1.14.4/coredns`, each with a `current` symlink; the stack starts; `php -v` in a new terminal (with shell integration on) shows the active version; switching PHP repoints `bin/php/current`. (nginx/redis still come from apt until Spec 1.)

- [ ] **Step 6: Commit**

```bash
git add core/src/setup.rs
git commit -m "feat(core): run_setup downloads composer, records versions, applies current symlinks"
```

---

## Self-Review

**1. Spec coverage:** Layout helpers + paths (T1, §3.1); Config.versions + migration (T2, §3.2); apply_versions + probe_version (T3, §3.4/3.5); resolver wiring (T4, §3.3); php_static into layout (T5, §3.4); PHP catalog from layout + drop scanners (T6, §3.3); php-fpm/switch/composer/shell (T7, §3.6); mailpit+coredns into layout (T8, §3.4); run_setup records versions + applies symlinks + composer ordering (T9, §3.7). All §3 sections covered. nginx/redis/mkcert installers + apt removal are explicitly Spec 1 (not here).

**2. Placeholder scan:** No "TBD"/"handle errors"/"similar to". Fallback consts are concrete (`COMPOSER_FALLBACK_VERSION="2.8.9"`, `MAILPIT_FALLBACK_VERSION="1.20.0"`) — bump to the current release at implementation time if newer, but a value is given.

**3. Type consistency:** `latest_patch` returns `(String,String)` (T5) and is the only producer used by `install_php_static`/`install_php_cli` (T5); `install_php_static`→`Result<String>` consumed by setup/commands (T5/T6); `set_current`/`managed_bin_dirs`/`installed_versions`/`apply_versions`/`probe_version` defined in T1/T3 and consumed in T5–T9; `Config.versions`/`tool_version` (T2) used in T5/T7/T8/T9; `installed_minors` (T6) consumed by `php_versions` (T6). `replace_php_version` keeps its `(&str)->Result<bool>` signature (T7). Consistent.
