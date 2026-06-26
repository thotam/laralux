# Versioned Tool Manager (Foundation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a uniform tool registry, a generalized version-switch, a `/usr/local/bin` symlink manager, generic Tauri commands, and a per-app Setup modal — so each app in Setup can pick its version and toggle a system-wide CLI symlink; PHP version management moves from Settings to Setup; the old Terminal-integration feature is removed.

**Architecture:** A new `core/src/tools.rs` registry describes each managed tool (`cli_binary`, `service_kind`, `available_versions`, `install_version`). The orchestrator's `replace_php_version` is generalized to `replace_version(kind, tool, version)`. A new `core/src/symlinks.rs` + two `Privileged` methods create/remove absolute symlinks under `/usr/local/bin` pointing at `bin/<tool>/current/<cli>`. Generic Tauri commands drive a per-app modal in `dist/app.js`. PHP is the only multi-version tool now; other tools expose their installed version.

**Tech Stack:** Rust (Cargo workspace: `core` = laragon-core with ZERO Tauri deps, `src-tauri` = laragon-desktop, `laragonctl`), Tauri 2 (events + commands), vanilla JS (`dist/app.js`).

## Global Constraints

- `core` (laragon-core) keeps ZERO Tauri dependencies — `tools.rs`/`symlinks.rs` use only `std` + existing core modules + `thiserror`. (verbatim from project rule)
- Commits MUST NOT contain a `Co-Authored-By` trailer.
- Symlink target dir is exactly `/usr/local/bin` (no other location this sub-project).
- CLI binary mapping (exact): Php→`php`, Composer→`composer`, Mariadb→`mariadb`, Mkcert→`mkcert`, Redis→`redis-cli`, Nginx→`nginx`, Mailpit→`None` (no symlink toggle).
- `bin/<tool>/current/<cli>` directory keys (exact): `php`, `nginx`, `mariadb`, `redis`, `mailpit`, `mkcert`, `composer`.
- Only PHP supports installing/switching multiple versions in this sub-project; other tools show their single installed version.
- Build green after every task: `cargo build -p laragon-desktop && cargo build -p laragonctl`. Tests: `cargo test -p laragon-core`.

---

### Task 1: Tool registry skeleton (`core/src/tools.rs`)

**Files:**
- Create: `core/src/tools.rs`
- Modify: `core/src/lib.rs` (add `pub mod tools;` after `pub mod orphans;`)

**Interfaces:**
- Consumes: `crate::service::ServiceKind` (variants `Nginx, PhpFpm, Mariadb, Redis, Mailpit`), `crate::paths::LaragonPaths` (`bin()`).
- Produces: `enum ManagedTool { Php, Nginx, Mariadb, Redis, Mailpit, Mkcert, Composer }`, `ManagedTool::ALL: [ManagedTool; 7]`, `struct ToolInfo { key: &'static str, display: &'static str, cli_binary: Option<&'static str>, service_kind: Option<ServiceKind> }`, `fn info(ManagedTool) -> ToolInfo`, `fn key(ManagedTool) -> &'static str`, `fn from_key(&str) -> Option<ManagedTool>`, `fn cli_path(ManagedTool, &LaragonPaths) -> Option<PathBuf>`.

- [ ] **Step 1: Write the failing test**

Add to `core/src/tools.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::service::ServiceKind;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedTool { Php, Nginx, Mariadb, Redis, Mailpit, Mkcert, Composer }

impl ManagedTool {
    pub const ALL: [ManagedTool; 7] = [
        ManagedTool::Php, ManagedTool::Nginx, ManagedTool::Mariadb, ManagedTool::Redis,
        ManagedTool::Mailpit, ManagedTool::Mkcert, ManagedTool::Composer,
    ];
}

pub struct ToolInfo {
    pub key: &'static str,
    pub display: &'static str,
    pub cli_binary: Option<&'static str>,
    pub service_kind: Option<ServiceKind>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_binary_mapping_and_mailpit_has_none() {
        assert_eq!(info(ManagedTool::Php).cli_binary, Some("php"));
        assert_eq!(info(ManagedTool::Composer).cli_binary, Some("composer"));
        assert_eq!(info(ManagedTool::Mariadb).cli_binary, Some("mariadb"));
        assert_eq!(info(ManagedTool::Mkcert).cli_binary, Some("mkcert"));
        assert_eq!(info(ManagedTool::Redis).cli_binary, Some("redis-cli"));
        assert_eq!(info(ManagedTool::Nginx).cli_binary, Some("nginx"));
        assert_eq!(info(ManagedTool::Mailpit).cli_binary, None);
    }

    #[test]
    fn key_roundtrips_through_from_key() {
        for t in ManagedTool::ALL {
            assert_eq!(from_key(key(t)), Some(t));
        }
        assert_eq!(from_key("nope"), None);
    }

    #[test]
    fn cli_path_is_under_current_and_none_for_mailpit() {
        let p = LaragonPaths::new("/tmp/lara".into());
        assert_eq!(cli_path(ManagedTool::Php, &p), Some(PathBuf::from("/tmp/lara/bin/php/current/php")));
        assert_eq!(cli_path(ManagedTool::Redis, &p), Some(PathBuf::from("/tmp/lara/bin/redis/current/redis-cli")));
        assert_eq!(cli_path(ManagedTool::Mailpit, &p), None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core tools:: 2>&1 | tail -20`
Expected: FAIL — `cannot find function 'info'` / `from_key` / `key` / `cli_path` not found, and `core/src/tools.rs` not yet a module (after Step 3 module wiring it compiles to the function errors).

- [ ] **Step 3: Add the module to lib.rs and implement the functions**

In `core/src/lib.rs`, add after the line `pub mod orphans;`:

```rust
pub mod tools;
```

In `core/src/tools.rs`, add (above the `#[cfg(test)]` block):

```rust
pub fn info(tool: ManagedTool) -> ToolInfo {
    use ManagedTool::*;
    match tool {
        Php => ToolInfo { key: "php", display: "PHP", cli_binary: Some("php"), service_kind: Some(ServiceKind::PhpFpm) },
        Nginx => ToolInfo { key: "nginx", display: "Nginx", cli_binary: Some("nginx"), service_kind: Some(ServiceKind::Nginx) },
        Mariadb => ToolInfo { key: "mariadb", display: "MariaDB", cli_binary: Some("mariadb"), service_kind: Some(ServiceKind::Mariadb) },
        Redis => ToolInfo { key: "redis", display: "Redis", cli_binary: Some("redis-cli"), service_kind: Some(ServiceKind::Redis) },
        Mailpit => ToolInfo { key: "mailpit", display: "Mailpit", cli_binary: None, service_kind: Some(ServiceKind::Mailpit) },
        Mkcert => ToolInfo { key: "mkcert", display: "mkcert", cli_binary: Some("mkcert"), service_kind: None },
        Composer => ToolInfo { key: "composer", display: "Composer", cli_binary: Some("composer"), service_kind: None },
    }
}

pub fn key(tool: ManagedTool) -> &'static str {
    info(tool).key
}

pub fn from_key(k: &str) -> Option<ManagedTool> {
    ManagedTool::ALL.into_iter().find(|t| key(*t) == k)
}

/// Absolute path to the tool's terminal CLI under `bin/<key>/current/<cli>`, if it has one.
pub fn cli_path(tool: ManagedTool, paths: &LaragonPaths) -> Option<PathBuf> {
    info(tool)
        .cli_binary
        .map(|b| paths.bin().join(key(tool)).join("current").join(b))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core tools:: 2>&1 | tail -20`
Expected: PASS — 3 tests in `tools::tests`.

- [ ] **Step 5: Commit**

```bash
git add core/src/tools.rs core/src/lib.rs
git commit -m "feat(core): tool registry skeleton (ManagedTool, info, cli_path)"
```

---

### Task 2: Version catalog + install dispatch (`core/src/tools.rs`)

**Files:**
- Modify: `core/src/tools.rs`
- Modify: `core/src/lib.rs` (re-export `ToolVersion`, `ToolError`)

**Interfaces:**
- Consumes: `crate::php_versions::php_versions(paths, active) -> Vec<PhpVersionInfo{version,installed,active}>`, `crate::layout::installed_versions(paths, tool) -> Vec<String>`, `crate::config::Config` (`load`, `php_version`, `versions: BTreeMap<String,String>`), `crate::php_static::install_php_static(paths, requested, downloader, runner, sink) -> Result<String, PhpStaticError>`, `crate::setup::Downloader`, `crate::scaffold::CommandRunner`, `crate::progress::ProgressSink`.
- Produces: `struct ToolVersion { version: String, installed: bool, active: bool }` (Serialize), `enum ToolError`, `fn available_versions(ManagedTool, &LaragonPaths) -> Vec<ToolVersion>`, `fn install_version(ManagedTool, &LaragonPaths, &str, &dyn Downloader, &dyn CommandRunner, &dyn ProgressSink) -> Result<String, ToolError>`.

- [ ] **Step 1: Write the failing test**

Add these tests inside the existing `#[cfg(test)] mod tests` in `core/src/tools.rs`:

```rust
    #[test]
    fn php_available_versions_lists_known_set() {
        let root = std::env::temp_dir().join(format!("lara-tools-php-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let vs = available_versions(ManagedTool::Php, &paths);
        // KNOWN_PHP_VERSIONS has 6 entries (8.0..8.5); none installed on a fresh root.
        assert_eq!(vs.len(), 6);
        assert!(vs.iter().all(|v| !v.installed));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn single_version_tool_lists_installed_only() {
        let root = std::env::temp_dir().join(format!("lara-tools-ng-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        // Seed an installed nginx version dir.
        std::fs::create_dir_all(paths.version_dir("nginx", "1.31.2")).unwrap();
        let vs = available_versions(ManagedTool::Nginx, &paths);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].version, "1.31.2");
        assert!(vs[0].installed);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_version_unsupported_for_non_php() {
        let paths = LaragonPaths::new("/tmp/lara".into());
        let err = install_version(
            ManagedTool::Nginx, &paths, "1.31.2",
            &crate::setup::FakeDownloader::new(), &crate::scaffold::FakeCommandRunner::new(),
            &crate::progress::NullProgress,
        );
        assert!(matches!(err, Err(ToolError::Unsupported)));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core tools:: 2>&1 | tail -20`
Expected: FAIL — `available_versions`, `install_version`, `ToolError`, `ToolVersion` not found.

- [ ] **Step 3: Implement the catalog + dispatch**

Add to `core/src/tools.rs` (above the test module):

```rust
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolVersion {
    pub version: String,
    pub installed: bool,
    pub active: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("installing additional versions is not supported for this tool yet")]
    Unsupported,
    #[error("install failed: {0}")]
    Install(String),
}

/// Versions selectable for a tool. PHP exposes the known catalog (∪ installed);
/// every other tool exposes only its installed version(s) (single, for now).
pub fn available_versions(tool: ManagedTool, paths: &LaragonPaths) -> Vec<ToolVersion> {
    let cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
    match tool {
        ManagedTool::Php => crate::php_versions::php_versions(paths, &cfg.php_version)
            .into_iter()
            .map(|p| ToolVersion { version: p.version, installed: p.installed, active: p.active })
            .collect(),
        other => {
            let k = key(other);
            let active = cfg.versions.get(k).cloned().unwrap_or_default();
            crate::layout::installed_versions(paths, k)
                .into_iter()
                .map(|v| ToolVersion { active: v == active, installed: true, version: v })
                .collect()
        }
    }
}

/// Install a specific version. Only PHP supports installing extra versions in this
/// sub-project; other tools are installed (single-version) via the bulk Setup run.
pub fn install_version(
    tool: ManagedTool,
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, ToolError> {
    match tool {
        ManagedTool::Php => crate::php_static::install_php_static(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        _ => Err(ToolError::Unsupported),
    }
}
```

In `core/src/lib.rs`, add after the `pub mod tools;` line a re-export block near the other `pub use`s:

```rust
pub use tools::{available_versions, install_version, ManagedTool, ToolError, ToolInfo, ToolVersion};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core tools:: 2>&1 | tail -20`
Expected: PASS — 6 tests in `tools::tests`.

- [ ] **Step 5: Commit**

```bash
git add core/src/tools.rs core/src/lib.rs
git commit -m "feat(core): tool version catalog + PHP install dispatch"
```

---

### Task 3: Generalize orchestrator version switch

**Files:**
- Modify: `core/src/orchestrator.rs` (`replace_php_version` and add `replace_version`)

**Interfaces:**
- Consumes: `crate::orphans::reap`, `crate::layout::{resolve_installed_version, set_current}`, `self.tracked_pids()`, `ServiceKind`.
- Produces: `Orchestrator::replace_version(&mut self, kind: ServiceKind, tool: &str, version: &str) -> Result<bool, ServiceError>`. `replace_php_version` becomes a thin wrapper.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `core/src/orchestrator.rs`:

```rust
    #[test]
    fn replace_version_restarts_running_service() {
        let tmp = std::env::temp_dir().join(format!("lara-orch-rv-{}", std::process::id()));
        let paths = LaragonPaths::new(tmp.clone());
        std::fs::create_dir_all(paths.version_dir("nginx", "1.31.2")).unwrap();
        crate::layout::set_current(&paths, "nginx", "1.31.2").unwrap();
        let spawner = crate::process::FakeSpawner::new();
        let mut orch = Orchestrator::new(
            paths,
            vec![Box::new(Dummy { kind: ServiceKind::Nginx, name: "nginx" })],
            Box::new(spawner),
        );
        orch.start(ServiceKind::Nginx).unwrap();
        let was = orch.replace_version(ServiceKind::Nginx, "nginx", "1.31.2").unwrap();
        assert!(was);
        assert_eq!(orch.state(ServiceKind::Nginx), ServiceState::Running);
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core orchestrator::tests::replace_version 2>&1 | tail -20`
Expected: FAIL — no method named `replace_version`.

- [ ] **Step 3: Implement `replace_version` and rewrite the wrapper**

In `core/src/orchestrator.rs`, replace the entire `replace_php_version` function body with:

```rust
    /// Swap the active version of a tool. Stops the service `kind` if running,
    /// reaps that tool's orphans, repoints `bin/<tool>/current`, and restarts iff
    /// it had been running. Returns whether the service had been running.
    pub fn replace_version(
        &mut self,
        kind: ServiceKind,
        tool: &str,
        version: &str,
    ) -> Result<bool, ServiceError> {
        let was_running = self.state(kind) == ServiceState::Running;
        if was_running {
            let _ = self.stop(kind);
        }
        let _ = crate::orphans::reap(&self.paths.bin().join(tool), &self.tracked_pids());
        let full = crate::layout::resolve_installed_version(&self.paths, tool, version)
            .unwrap_or_else(|| version.to_string());
        crate::layout::set_current(&self.paths, tool, &full)
            .map_err(|e| ServiceError::Config(format!("set {tool} current: {e}")))?;
        if was_running {
            self.start(kind)?;
        }
        Ok(was_running)
    }

    /// Swap the active php-fpm version (thin wrapper over `replace_version`).
    pub fn replace_php_version(&mut self, version: &str) -> Result<bool, ServiceError> {
        self.replace_version(ServiceKind::PhpFpm, "php", version)
    }
```

(Delete the old `replace_php_version` implementation that previously held the stop/reap/set_current/start body — it is now inside `replace_version`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laragon-core orchestrator:: 2>&1 | tail -20`
Expected: PASS — including the existing `replace_php_version_restarts_when_running`, `replace_php_version_does_not_start_when_stopped`, and the new `replace_version_restarts_running_service`.

- [ ] **Step 5: Commit**

```bash
git add core/src/orchestrator.rs
git commit -m "feat(core): generalize replace_php_version into replace_version(kind,tool,version)"
```

---

### Task 4: Privileged symlink operations

**Files:**
- Modify: `core/src/privileged.rs` (trait + free argv builders + `SudoPrivileged`/`PkexecPrivileged`/`FakePrivileged` + tests)

**Interfaces:**
- Consumes: existing `run_escalated`, `PrivError`, `std::path::Path`.
- Produces: trait methods `Privileged::create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError>` and `Privileged::remove_symlink(&self, dst: &Path) -> Result<(), PrivError>`; free fns `ln_symlink_argv(src,dst) -> Vec<String>`, `rm_argv(dst) -> Vec<String>`; `FakePrivileged::symlinks_created() -> Arc<Mutex<Vec<(String,String)>>>`, `FakePrivileged::symlinks_removed() -> Arc<Mutex<Vec<String>>>`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `core/src/privileged.rs`:

```rust
    #[test]
    fn symlink_argv_builders_are_correct() {
        assert_eq!(
            ln_symlink_argv(Path::new("/home/u/laragon/bin/php/current/php"), Path::new("/usr/local/bin/php")),
            vec!["ln".to_string(), "-sfn".to_string(),
                 "/home/u/laragon/bin/php/current/php".to_string(), "/usr/local/bin/php".to_string()]
        );
        assert_eq!(
            rm_argv(Path::new("/usr/local/bin/php")),
            vec!["rm".to_string(), "-f".to_string(), "/usr/local/bin/php".to_string()]
        );
    }

    #[test]
    fn fake_records_symlink_create_and_remove() {
        let p = FakePrivileged::new();
        p.create_symlink(Path::new("/src/php"), Path::new("/usr/local/bin/php")).unwrap();
        p.remove_symlink(Path::new("/usr/local/bin/php")).unwrap();
        assert_eq!(p.symlinks_created().lock().unwrap().as_slice(),
            &[("/src/php".to_string(), "/usr/local/bin/php".to_string())]);
        assert_eq!(p.symlinks_removed().lock().unwrap().as_slice(),
            &["/usr/local/bin/php".to_string()]);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core privileged:: 2>&1 | tail -20`
Expected: FAIL — `ln_symlink_argv` / `rm_argv` / `create_symlink` / `symlinks_created` not found.

- [ ] **Step 3: Implement trait methods, argv builders, and impls**

In `core/src/privileged.rs`, add the two methods to the `pub trait Privileged` block (after `remove_resolved_dropin`):

```rust
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError>;
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError>;
```

Add these free helpers near the other `*_argv` builders:

```rust
fn ln_symlink_argv(src: &Path, dst: &Path) -> Vec<String> {
    vec!["ln".to_string(), "-sfn".to_string(), src.display().to_string(), dst.display().to_string()]
}

fn rm_argv(dst: &Path) -> Vec<String> {
    vec!["rm".to_string(), "-f".to_string(), dst.display().to_string()]
}
```

In `impl Privileged for SudoPrivileged` add:

```rust
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &ln_symlink_argv(src, dst))
    }
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError> {
        run_escalated("sudo", &rm_argv(dst))
    }
```

In `impl Privileged for PkexecPrivileged` add:

```rust
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &ln_symlink_argv(src, dst))
    }
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError> {
        run_escalated("pkexec", &rm_argv(dst))
    }
```

In `struct FakePrivileged`, add two fields:

```rust
    symlinks_created: Arc<Mutex<Vec<(String, String)>>>,
    symlinks_removed: Arc<Mutex<Vec<String>>>,
```

In `impl FakePrivileged`, add accessors:

```rust
    pub fn symlinks_created(&self) -> Arc<Mutex<Vec<(String, String)>>> {
        self.symlinks_created.clone()
    }
    pub fn symlinks_removed(&self) -> Arc<Mutex<Vec<String>>> {
        self.symlinks_removed.clone()
    }
```

In `impl Privileged for FakePrivileged`, add:

```rust
    fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError> {
        self.symlinks_created.lock().unwrap().push((src.display().to_string(), dst.display().to_string()));
        Ok(())
    }
    fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError> {
        self.symlinks_removed.lock().unwrap().push(dst.display().to_string());
        Ok(())
    }
```

(`FakePrivileged` derives `Default`, so the two new `Arc<Mutex<Vec<_>>>` fields default to empty — no change to `new()` needed.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core privileged:: 2>&1 | tail -20`
Expected: PASS — `symlink_argv_builders_are_correct`, `fake_records_symlink_create_and_remove`, plus existing privileged tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/privileged.rs
git commit -m "feat(core): Privileged create_symlink/remove_symlink (ln -sfn / rm -f)"
```

---

### Task 5: Symlink manager (`core/src/symlinks.rs`)

**Files:**
- Create: `core/src/symlinks.rs`
- Modify: `core/src/lib.rs` (add `pub mod symlinks;` and re-export `link_tool, unlink_tool, system_link_path, SymlinkError`)

**Interfaces:**
- Consumes: `crate::tools::{ManagedTool, info, cli_path}`, `crate::privileged::Privileged`, `crate::paths::LaragonPaths`.
- Produces: `const SYSTEM_BIN_DIR: &str`, `fn system_link_path(ManagedTool) -> Option<PathBuf>`, `fn link_tool(&LaragonPaths, ManagedTool, &dyn Privileged) -> Result<(), SymlinkError>`, `fn unlink_tool(ManagedTool, &dyn Privileged) -> Result<(), SymlinkError>`, `enum SymlinkError { NoCli, NotInstalled, Priv(String) }`.

- [ ] **Step 1: Write the failing test**

Create `core/src/symlinks.rs`:

```rust
use crate::paths::LaragonPaths;
use crate::privileged::Privileged;
use crate::tools::{cli_path, info, ManagedTool};
use std::path::{Path, PathBuf};

pub const SYSTEM_BIN_DIR: &str = "/usr/local/bin";

#[derive(Debug, thiserror::Error)]
pub enum SymlinkError {
    #[error("tool has no terminal CLI to link")]
    NoCli,
    #[error("tool is not installed yet")]
    NotInstalled,
    #[error("privileged op failed: {0}")]
    Priv(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privileged::FakePrivileged;

    #[test]
    fn system_link_path_per_tool() {
        assert_eq!(system_link_path(ManagedTool::Php), Some(PathBuf::from("/usr/local/bin/php")));
        assert_eq!(system_link_path(ManagedTool::Redis), Some(PathBuf::from("/usr/local/bin/redis-cli")));
        assert_eq!(system_link_path(ManagedTool::Mailpit), None);
    }

    #[test]
    fn link_tool_calls_create_symlink_with_resolved_src_and_dst() {
        let root = std::env::temp_dir().join(format!("lara-symlink-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        // Seed an installed php cli at bin/php/current/php.
        let cur = paths.bin().join("php").join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(cur.join("php"), b"x").unwrap();
        let p = FakePrivileged::new();
        link_tool(&paths, ManagedTool::Php, &p).unwrap();
        let created = p.symlinks_created();
        let created = created.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].0, cur.join("php").display().to_string());
        assert_eq!(created[0].1, "/usr/local/bin/php");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn link_tool_errors_when_not_installed() {
        let paths = LaragonPaths::new(std::env::temp_dir().join(format!("lara-symlink2-{}", std::process::id())));
        let p = FakePrivileged::new();
        assert!(matches!(link_tool(&paths, ManagedTool::Php, &p), Err(SymlinkError::NotInstalled)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core symlinks:: 2>&1 | tail -20`
Expected: FAIL — module not declared / `system_link_path`, `link_tool` not found.

- [ ] **Step 3: Wire the module and implement**

In `core/src/lib.rs`, add after `pub mod symlinks;` placement (next to `pub mod tools;`):

```rust
pub mod symlinks;
```

and a re-export next to the tools re-export:

```rust
pub use symlinks::{link_tool, system_link_path, unlink_tool, SymlinkError};
```

Add the implementation to `core/src/symlinks.rs` (above the test module):

```rust
pub fn system_link_path(tool: ManagedTool) -> Option<PathBuf> {
    info(tool).cli_binary.map(|b| Path::new(SYSTEM_BIN_DIR).join(b))
}

pub fn link_tool(paths: &LaragonPaths, tool: ManagedTool, privileged: &dyn Privileged) -> Result<(), SymlinkError> {
    let src = cli_path(tool, paths).ok_or(SymlinkError::NoCli)?;
    if !src.exists() {
        return Err(SymlinkError::NotInstalled);
    }
    let dst = system_link_path(tool).ok_or(SymlinkError::NoCli)?;
    privileged.create_symlink(&src, &dst).map_err(|e| SymlinkError::Priv(e.to_string()))
}

pub fn unlink_tool(tool: ManagedTool, privileged: &dyn Privileged) -> Result<(), SymlinkError> {
    let dst = system_link_path(tool).ok_or(SymlinkError::NoCli)?;
    privileged.remove_symlink(&dst).map_err(|e| SymlinkError::Priv(e.to_string()))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core symlinks:: 2>&1 | tail -20`
Expected: PASS — 3 tests in `symlinks::tests`.

- [ ] **Step 5: Commit**

```bash
git add core/src/symlinks.rs core/src/lib.rs
git commit -m "feat(core): /usr/local/bin symlink manager (link_tool/unlink_tool)"
```

---

### Task 6: Config `symlinks` field

**Files:**
- Modify: `core/src/config.rs` (struct, `Default`, tests)

**Interfaces:**
- Produces: `Config.symlinks: std::collections::BTreeSet<String>` (`#[serde(default)]`).

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `core/src/config.rs`:

```rust
    #[test]
    fn symlinks_field_defaults_empty_and_roundtrips() {
        let mut c = Config::default();
        assert!(c.symlinks.is_empty());
        c.symlinks.insert("php".to_string());
        let toml = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&toml).unwrap();
        assert!(back.symlinks.contains("php"));
    }

    #[test]
    fn old_config_without_symlinks_or_with_stale_keys_loads() {
        // Stale `shell_integration` key (removed in a later task) must not break loading;
        // missing `symlinks` defaults to empty.
        let c: Config = toml::from_str("tld = \"dev\"\nphp_version = \"8.4\"\nshell_integration = true\n").unwrap();
        assert!(c.symlinks.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core config:: 2>&1 | tail -20`
Expected: FAIL — no field `symlinks` on `Config`.

- [ ] **Step 3: Add the field**

In `core/src/config.rs`, ensure the import exists at the top (add if missing): `use std::collections::BTreeSet;`

In the `pub struct Config { ... }` block, add after the `versions` field:

```rust
    #[serde(default)]
    pub symlinks: BTreeSet<String>,
```

In the `impl Default for Config` `Self { ... }` literal, add `symlinks: BTreeSet::new(),` to the field list (keep all existing fields, including `shell_integration: false` for now — it is removed in Task 9).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core config:: 2>&1 | tail -20`
Expected: PASS — both new tests plus existing config tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): config.symlinks set of tools linked into /usr/local/bin"
```

---

### Task 7: Desktop generic version commands

**Files:**
- Modify: `src-tauri/src/commands.rs` (add `tool_versions`, `install_tool_version`, `set_tool_version`)
- Modify: `src-tauri/src/main.rs` (register the three commands in `generate_handler!`)

**Interfaces:**
- Consumes: `laragon_core::tools::{from_key, info, available_versions, install_version, key, ManagedTool, ToolVersion}`, `laragon_core::{Config, apply_versions, resolve_installed_version, set_current, ensure_active_php_cli, CurlDownloader, RealCommandRunner, ServiceStatus, ServiceKind}`, existing `AppState`, `TauriProgress`, `lock_err`.
- Produces: Tauri commands `tool_versions(tool: String) -> Result<Vec<ToolVersion>, String>`, `install_tool_version(tool: String, version: String) -> Result<Vec<ToolVersion>, String>`, `set_tool_version(tool: String, version: String) -> Result<Vec<ServiceStatus>, String>`.

- [ ] **Step 1: Add the three commands**

In `src-tauri/src/commands.rs`, add (after `set_php_version`):

```rust
#[tauri::command]
pub fn tool_versions(
    state: tauri::State<AppState>,
    tool: String,
) -> Result<Vec<laragon_core::tools::ToolVersion>, String> {
    let t = laragon_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
    Ok(laragon_core::tools::available_versions(t, &state.paths))
}

#[tauri::command]
pub async fn install_tool_version(
    app: tauri::AppHandle,
    tool: String,
    version: String,
) -> Result<Vec<laragon_core::tools::ToolVersion>, String> {
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<laragon_core::tools::ToolVersion>, String> {
        let state = app.state::<AppState>();
        let t = laragon_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
        let progress = TauriProgress(app_for_progress);
        laragon_core::tools::install_version(t, &state.paths, &version, &CurlDownloader, &RealCommandRunner, &progress)
            .map_err(|e| e.to_string())?;
        // Keep `current` symlinks reconciled to config after an install.
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let _ = laragon_core::apply_versions(&state.paths, &config);
        Ok(laragon_core::tools::available_versions(t, &state.paths))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_tool_version(
    app: tauri::AppHandle,
    tool: String,
    version: String,
) -> Result<Vec<ServiceStatus>, String> {
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ServiceStatus>, String> {
        let state = app.state::<AppState>();
        let t = laragon_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
        let info = laragon_core::tools::info(t);

        let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let full = laragon_core::resolve_installed_version(&state.paths, info.key, &version)
            .unwrap_or_else(|| version.clone());
        config.versions.insert(info.key.to_string(), full.clone());
        if t == laragon_core::tools::ManagedTool::Php {
            config.php_version = full.clone();
        }
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;

        let snapshot = {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            match info.service_kind {
                Some(kind) => { orch.replace_version(kind, info.key, &version).map_err(|e| e.to_string())?; }
                None => { laragon_core::set_current(&state.paths, info.key, &full).map_err(|e| e.to_string())?; }
            }
            orch.snapshot()
        };

        if t == laragon_core::tools::ManagedTool::Php {
            let progress = TauriProgress(app_for_progress);
            let _ = laragon_core::ensure_active_php_cli(&state.paths, &version, &CurlDownloader, &RealCommandRunner, &progress);
        }
        Ok(snapshot)
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 2: Register the commands**

In `src-tauri/src/main.rs`, inside `tauri::generate_handler![ ... ]`, add after `commands::set_php_version,`:

```rust
            commands::tool_versions,
            commands::install_tool_version,
            commands::set_tool_version,
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p laragon-desktop 2>&1 | tail -15`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): generic tool_versions/install_tool_version/set_tool_version commands"
```

---

### Task 8: Desktop symlink commands

**Files:**
- Modify: `src-tauri/src/commands.rs` (add `tool_symlinks`, `set_tool_symlink`)
- Modify: `src-tauri/src/main.rs` (register both)

**Interfaces:**
- Consumes: `laragon_core::{link_tool, unlink_tool, tools::{from_key, key}, Config, PkexecPrivileged}`, `AppState`.
- Produces: Tauri commands `tool_symlinks() -> Result<Vec<String>, String>`, `set_tool_symlink(tool: String, enabled: bool) -> Result<Vec<String>, String>`.

- [ ] **Step 1: Add the two commands**

In `src-tauri/src/commands.rs`, add (after `set_tool_version`):

```rust
#[tauri::command]
pub fn tool_symlinks(state: tauri::State<AppState>) -> Result<Vec<String>, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(config.symlinks.into_iter().collect())
}

#[tauri::command]
pub async fn set_tool_symlink(
    app: tauri::AppHandle,
    tool: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
        let state = app.state::<AppState>();
        let t = laragon_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
        if enabled {
            laragon_core::link_tool(&state.paths, t, &PkexecPrivileged).map_err(|e| e.to_string())?;
        } else {
            laragon_core::unlink_tool(t, &PkexecPrivileged).map_err(|e| e.to_string())?;
        }
        let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let k = laragon_core::tools::key(t).to_string();
        if enabled { config.symlinks.insert(k); } else { config.symlinks.remove(&k); }
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
        Ok(config.symlinks.into_iter().collect())
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 2: Register the commands**

In `src-tauri/src/main.rs`, inside `generate_handler![ ... ]`, add after `commands::set_tool_version,`:

```rust
            commands::tool_symlinks,
            commands::set_tool_symlink,
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p laragon-desktop 2>&1 | tail -15`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): tool_symlinks/set_tool_symlink commands (/usr/local/bin)"
```

---

### Task 9: Remove the Terminal-integration feature

**Files:**
- Delete: `core/src/shell_env.rs`
- Modify: `core/src/lib.rs` (drop `pub mod shell_env;` and the `pub use shell_env::{...}`)
- Modify: `core/src/config.rs` (remove `shell_integration` field + its `Default` entry)
- Modify: `src-tauri/src/commands.rs` (delete `terminal_integration_status`, `set_terminal_integration`; drop unused imports `enable_shell_path`, `disable_shell_path`, `install_composer`)
- Modify: `src-tauri/src/main.rs` (drop the two commands from `generate_handler!`)

**Interfaces:**
- Produces: none (pure removal). After this task `Config` has no `shell_integration`; the desktop exposes no terminal-integration commands.

- [ ] **Step 1: Remove the config field and its test references**

In `core/src/config.rs`: delete the `pub shell_integration: bool,` field from `struct Config`; delete `shell_integration: false,` from the `Default` impl; if `normalize()` or any test references `shell_integration`, delete those lines/asserts. (The `old_config_..._with_stale_keys_loads` test from Task 6 confirms a leftover `shell_integration` key still deserializes, since serde ignores unknown fields.)

- [ ] **Step 2: Remove the core module**

Delete the file `core/src/shell_env.rs`. In `core/src/lib.rs`, delete the line `pub mod shell_env;` and the line `pub use shell_env::{disable_shell_path, enable_shell_path};`.

- [ ] **Step 3: Remove the desktop commands**

In `src-tauri/src/commands.rs`: delete the entire `terminal_integration_status` and `set_terminal_integration` functions. In the top `use laragon_core::{ ... }` imports, remove `enable_shell_path`, `disable_shell_path`, and `install_composer` (now unused in the desktop crate). In `src-tauri/src/main.rs`, delete the lines `commands::terminal_integration_status,` and `commands::set_terminal_integration,` from `generate_handler!`.

- [ ] **Step 4: Build the workspace to verify the removal compiles**

Run: `cargo build -p laragon-desktop && cargo build -p laragonctl && cargo test -p laragon-core 2>&1 | tail -15`
Expected: all `Finished` / tests PASS. If the compiler reports `install_composer`/`enable_shell_path` still used somewhere, that call site was missed — remove or update it (no terminal-integration caller should remain).

- [ ] **Step 5: Commit**

```bash
git add -A core/src src-tauri/src
git commit -m "refactor: remove Terminal-integration feature (superseded by /usr/local/bin symlinks)"
```

---

### Task 10: Setup per-app modal UI + Settings cleanup (`dist/app.js`, `dist/styles.css`)

**Files:**
- Modify: `dist/app.js` (Setup rows clickable, modal component + handlers, remove PHP card + Terminal-integration row from Settings)
- Modify: `dist/styles.css` (modal + version-chip styles)

**Interfaces:**
- Consumes (Tauri commands from Tasks 7–8): `tool_versions({tool})`, `install_tool_version({tool, version})`, `set_tool_version({tool, version})`, `tool_symlinks()`, `set_tool_symlink({tool, enabled})`.
- Produces: a `state.modal` object and a modal renderer; Setup rows that open it.

- [ ] **Step 1: Add modal state and the tool key map**

In `dist/app.js`, in the big `state = { ... }` object (near `setup: {...}`), add:

```javascript
    modal: { open: false, toolKey: null, display: "", cliBinary: null, versions: [], linked: false, busy: false },
    toolSymlinks: [],
```

Add a constant near `DISP_COMP` mapping the Setup component name to the core tool key + its CLI binary (mailpit has no CLI):

```javascript
  const TOOL_KEY = { Nginx: "nginx", Php: "php", Mariadb: "mariadb", Redis: "redis", Mkcert: "mkcert", Mailpit: "mailpit", Composer: "composer" };
  const TOOL_CLI = { nginx: "nginx", php: "php", mariadb: "mariadb", redis: "redis-cli", mkcert: "mkcert", mailpit: null, composer: "composer" };
```

- [ ] **Step 2: Make Setup rows open the modal (render change)**

In `setupView()`, change the per-component row markup so each row is a clickable button carrying its tool key. Replace the row template (the `.map((c) => ... '<div class="setup-item">...')` body) with:

```javascript
      .map((c) => {
        const tag = c.present
          ? '<span class="tag ok">Installed</span>'
          : '<span class="tag warn">Missing</span>';
        const tk = TOOL_KEY[c.component] || "";
        return (
          '<button class="setup-item setup-item-btn" data-action="open-tool" data-tool="' + esc(tk) + '">' +
          '<div class="setup-tile">' + I.setupItem + "</div>" +
          '<span class="nm">' + esc(DISP_COMP[c.component] || c.component) + "</span>" + tag +
          '<span class="chev">' + (I.chevron || "›") + "</span></button>"
        );
      })
```

- [ ] **Step 3: Add the modal renderer**

In `dist/app.js`, add a `toolModal()` function (place near `settingsView()`):

```javascript
  function toolModal() {
    const m = state.modal;
    if (!m.open) return "";
    const verRows = (m.versions || [])
      .map((v) => {
        let right;
        if (m.busy && m.busyVersion === v.version) right = progressRing();
        else if (v.active) right = '<span class="tag ok">Active</span>';
        else if (v.installed) right = '<button class="btn-sm" data-action="use-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Use</button>";
        else right = '<button class="btn-sm" data-action="install-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Install</button>";
        return '<div class="set-row"><div class="grow"><div class="t">' + esc(m.display) + " " + esc(v.version) + '</div><div class="h">' + (v.installed ? "Installed" : "Not installed") + "</div></div>" + right + "</div>";
      })
      .join("") || '<div class="set-row"><div class="h">No versions — run “Install missing” first.</div></div>';

    const anyInstalled = (m.versions || []).some((v) => v.installed);
    const symlinkRow = m.cliBinary
      ? '<div class="modal-divider"></div>' +
        '<div class="set-row"><div class="grow"><div class="t">In terminal (/usr/local/bin)</div>' +
        '<div class="h"><code>' + esc(m.cliBinary) + "</code> available system-wide</div></div>" +
        '<button class="btn-sm" data-action="toggle-tool-symlink"' + (m.busy || !anyInstalled ? " disabled" : "") + ">" +
        (m.linked ? "On" : "Off") + "</button></div>"
      : "";

    return (
      '<div class="modal-backdrop" data-action="close-tool"></div>' +
      '<div class="modal" role="dialog" aria-modal="true">' +
      '<div class="modal-head"><span class="modal-title">' + esc(m.display) + "</span>" +
      '<button class="modal-close" data-action="close-tool" aria-label="Close">' + I.close + "</button></div>" +
      '<div class="modal-body"><div class="modal-sec-label">Versions</div>' + verRows + symlinkRow + "</div>" +
      "</div>"
    );
  }
```

In the top-level `render()` function, append the modal to the rendered HTML (find where the app shell + `toasts()` are concatenated and add `+ toolModal()` to the output string).

- [ ] **Step 4: Add the open/close + action loaders**

In `dist/app.js`, add these functions (near `loadPhpVersions`):

```javascript
  async function openTool(toolKey) {
    const comp = Object.keys(TOOL_KEY).find((k) => TOOL_KEY[k] === toolKey);
    state.modal = {
      open: true, toolKey, display: DISP_COMP[comp] || toolKey, cliBinary: TOOL_CLI[toolKey],
      versions: [], linked: false, busy: false, busyVersion: null,
    };
    render();
    try {
      const [versions, linked] = await Promise.all([
        invoke("tool_versions", { tool: toolKey }),
        invoke("tool_symlinks"),
      ]);
      state.modal.versions = versions;
      state.toolSymlinks = linked;
      state.modal.linked = linked.includes(toolKey);
    } catch (e) {
      toast({ type: "error", title: "Load failed", msg: String(e) });
    }
    render();
  }

  function closeTool() { state.modal.open = false; render(); }

  async function useToolVersion(version) {
    const tk = state.modal.toolKey;
    state.modal.busy = true; state.modal.busyVersion = version; render();
    try {
      await invoke("set_tool_version", { tool: tk, version });
      state.modal.versions = await invoke("tool_versions", { tool: tk });
      toast({ type: "success", title: "Version switched", msg: state.modal.display + " " + version });
    } catch (e) {
      toast({ type: "error", title: "Switch failed", msg: String(e) });
    } finally {
      state.modal.busy = false; state.modal.busyVersion = null; resetDownload(); render();
    }
  }

  async function installToolVersion(version) {
    const tk = state.modal.toolKey;
    state.modal.busy = true; state.modal.busyVersion = version; render();
    try {
      state.modal.versions = await invoke("install_tool_version", { tool: tk, version });
      toast({ type: "success", title: "Installed", msg: state.modal.display + " " + version });
    } catch (e) {
      toast({ type: "error", title: "Install failed", msg: String(e) });
    } finally {
      state.modal.busy = false; state.modal.busyVersion = null; resetDownload(); render();
    }
  }

  async function toggleToolSymlink() {
    const tk = state.modal.toolKey;
    const next = !state.modal.linked;
    state.modal.busy = true; render();
    try {
      state.toolSymlinks = await invoke("set_tool_symlink", { tool: tk, enabled: next });
      state.modal.linked = state.toolSymlinks.includes(tk);
      toast({ type: "success", title: next ? "Linked" : "Unlinked", msg: "/usr/local/bin/" + state.modal.cliBinary });
    } catch (e) {
      toast({ type: "error", title: "Symlink failed", msg: String(e) });
    } finally {
      state.modal.busy = false; render();
    }
  }
```

- [ ] **Step 5: Wire the click handlers**

In the global click handler (the `document.addEventListener("click", ...)` dispatch on `data-action`), add cases:

```javascript
      else if (action === "open-tool") openTool(el.dataset.tool);
      else if (action === "close-tool") closeTool();
      else if (action === "use-tool-version") useToolVersion(el.dataset.version);
      else if (action === "install-tool-version") installToolVersion(el.dataset.version);
      else if (action === "toggle-tool-symlink") toggleToolSymlink();
```

- [ ] **Step 6: Remove the PHP card + Terminal-integration row from Settings**

In `settingsView()`: delete the `phpCard` variable construction and its `+ phpCard` in the return; delete the Terminal-integration `.set-row` (the one with `data-action="toggle-terminal"`). In the view dispatch `if (v === "settings") { loadPhpVersions(); loadTerminalIntegration(); }` change it to do nothing settings-specific (remove both calls). Leave `loadPhpVersions`/`loadTerminalIntegration` functions only if still referenced; otherwise delete them and the `toggle-terminal` click case.

- [ ] **Step 7: Add modal styles**

Append to `dist/styles.css`:

```css
.setup-item-btn { display:flex; align-items:center; gap:10px; width:100%; text-align:left; background:none; border:0; cursor:pointer; padding:10px 12px; border-radius:10px; }
.setup-item-btn:hover { background:var(--row-hover, rgba(127,127,127,.08)); }
.setup-item-btn .chev { margin-left:auto; opacity:.5; }
.modal-backdrop { position:fixed; inset:0; background:rgba(0,0,0,.38); z-index:40; }
.modal { position:fixed; z-index:41; top:50%; left:50%; transform:translate(-50%,-50%); width:min(440px,92vw); max-height:82vh; overflow:auto; background:var(--card,#fff); color:inherit; border-radius:14px; box-shadow:0 18px 60px rgba(0,0,0,.35); }
.modal-head { display:flex; align-items:center; justify-content:space-between; padding:14px 16px; border-bottom:1px solid var(--line,rgba(127,127,127,.18)); }
.modal-title { font-weight:600; font-size:15px; }
.modal-close { background:none; border:0; cursor:pointer; opacity:.6; }
.modal-close:hover { opacity:1; }
.modal-body { padding:8px 16px 16px; }
.modal-sec-label { font-size:11px; text-transform:uppercase; letter-spacing:.05em; opacity:.55; margin:10px 0 4px; }
.modal-divider { height:1px; background:var(--line,rgba(127,127,127,.18)); margin:12px 0 4px; }
```

- [ ] **Step 8: Build the desktop app and manually verify**

Run: `cargo build -p laragon-desktop 2>&1 | tail -5`
Then run `cargo run -p laragon-desktop` and verify manually:
- Setup tab → click "PHP" → modal lists 8.0–8.5 with Active/Use/Install; switching works.
- Click "Nginx" → modal shows the one installed version + a working "In terminal (/usr/local/bin)" toggle (`nginx`).
- Toggle a symlink ON → pkexec prompt → `ls -l /usr/local/bin/php` is a symlink to `~/laragon/bin/php/current/php`; toggle OFF removes it.
- "Mailpit" modal shows versions but NO symlink toggle.
- Settings tab no longer shows the PHP card or Terminal-integration row.

- [ ] **Step 9: Commit**

```bash
git add dist/app.js dist/styles.css
git commit -m "feat(ui): per-app Setup modal (version + /usr/local/bin symlink); move PHP mgmt out of Settings"
```

---

## Self-Review (completed by plan author)

- **Spec coverage:** registry (T1–T2), generalized switch (T3), Privileged symlink ops (T4), symlink manager (T5), config.symlinks (T6), generic version commands (T7), symlink commands (T8), remove terminal integration (T9), Setup modal + Settings cleanup (T10). All §3 components covered; §6 tests folded into T1–T6; §7 out-of-scope respected (install_version is PHP-only).
- **Type consistency:** `ManagedTool`, `ToolInfo.{key,display,cli_binary,service_kind}`, `ToolVersion.{version,installed,active}`, `replace_version(kind,tool,version)`, `link_tool/unlink_tool/system_link_path`, `create_symlink/remove_symlink`, `config.symlinks`, commands `tool_versions/install_tool_version/set_tool_version/tool_symlinks/set_tool_symlink` — names identical across producer and consumer tasks.
- **Placeholders:** none — every code step contains full code.
- **Note:** `I.chevron` and `I.close` are icon strings in `app.js`; T2/T10 fall back to `›` if `I.chevron` is undefined. The implementer should confirm `I.close` exists (it is used by existing toasts) and add a `chevron` glyph if absent.
