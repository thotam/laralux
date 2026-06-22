# Laragon Linux — New Site / Quick App Creation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "New site" action that scaffolds a Blank / Laravel / WordPress project in `~/laragon/www/`, optionally auto-creates a matching MySQL database, then makes it reachable at `https://<name>.dev`.

**Architecture:** A new pure-logic `core::scaffold` module creates the project; external tools (composer, tar, mariadb client) run behind a `CommandRunner` trait, and the WordPress tarball is fetched via the existing `Downloader` trait — so all logic is unit-tested with fakes (no network/tools). A Tauri `create_site` command wires the real runner/downloader, then reuses `sync_sites` + nginx reload. The frontend enables the existing "New site" button and adds a creation modal.

**Tech Stack:** Rust (reuses `laragon_core`: `LaragonPaths`, `setup::Downloader`/`CurlDownloader`, `sync_sites`, orchestrator), Tauri 2 + vanilla JS frontend; live tools: composer, curl, tar, mariadb client.

## Global Constraints

- Templates (serde enum `SiteTemplate`): `Blank`, `Laravel`, `Wordpress` (exact variant names; the frontend MUST send `"Wordpress"`, not `"WordPress"`).
- Site name rule (also the DB name rule): regex `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`, length 1–63. Reject otherwise and reject if `www/<name>` already exists.
- DB defaults (project dev default): user `root`, empty password, host `127.0.0.1`. DB name = site name, backtick-quoted.
- WordPress tarball URL: `https://wordpress.org/latest.tar.gz`. WP salts are generated locally (offline) — never fetched.
- Laravel requires `composer`; add `Composer` to the setup wizard (apt package `composer`, detect binary `composer`).
- After create, the GUI command runs `sync_sites` (vhost+cert+`/etc/hosts`, via `PkexecPrivileged`) and reloads nginx if it is Running.
- `core` keeps zero Tauri deps. No `alert()` in the frontend — use the existing toast system. Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD for all `core` changes (Tasks 1–6). IPC + frontend (Tasks 7–8) are build + manual smoke (GUI glue over already-tested core).

---

### Task 1: scaffold module — SiteTemplate, ScaffoldError, validate_site_name

**Files:**
- Create: `core/src/scaffold.rs`
- Modify: `core/src/lib.rs` (add `pub mod scaffold;` + re-exports)

**Interfaces:**
- Produces:
  - `enum SiteTemplate { Blank, Laravel, Wordpress }` (derives `Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug`).
  - `enum ScaffoldError` (thiserror): `InvalidName(String)`, `AlreadyExists(String)`, `ToolMissing(String)`, `Download(String)`, `Command(String)`, `Db(String)`, `Io(#[from] std::io::Error)`.
  - `fn validate_site_name(name: &str) -> Result<(), ScaffoldError>`.

- [ ] **Step 1: Add module + re-exports to lib.rs**

In `core/src/lib.rs` add (with the other `pub mod` lines and re-exports):
```rust
pub mod scaffold;
pub use scaffold::{SiteTemplate, ScaffoldError};
```

- [ ] **Step 2: Write the failing test**

Create `core/src/scaffold.rs` with the test first:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        assert!(validate_site_name("blog").is_ok());
        assert!(validate_site_name("shop-api").is_ok());
        assert!(validate_site_name("a1").is_ok());
    }

    #[test]
    fn rejects_invalid_names() {
        for bad in ["", "Blog", "a b", "-x", "x-", "a_b", "café"] {
            assert!(validate_site_name(bad).is_err(), "should reject {bad:?}");
        }
        let long = "a".repeat(64);
        assert!(validate_site_name(&long).is_err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laragon-core scaffold`
Expected: FAIL — `cannot find function validate_site_name`.

- [ ] **Step 4: Write minimal implementation**

Prepend to `core/src/scaffold.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiteTemplate {
    Blank,
    Laravel,
    Wordpress,
}

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("invalid site name: {0}")]
    InvalidName(String),
    #[error("site already exists: {0}")]
    AlreadyExists(String),
    #[error("required tool not installed: {0}")]
    ToolMissing(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("command failed: {0}")]
    Command(String),
    #[error("database error: {0}")]
    Db(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// A valid DNS label: lowercase alnum and hyphens, not starting/ending with a
/// hyphen, length 1–63.
pub fn validate_site_name(name: &str) -> Result<(), ScaffoldError> {
    let ok = (1..=63).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(ScaffoldError::InvalidName(name.to_string()))
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p laragon-core scaffold`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/scaffold.rs core/src/lib.rs
git commit -m "feat(core): add scaffold module (SiteTemplate, validate_site_name)"
```

---

### Task 2: Pure content generators

**Files:**
- Modify: `core/src/scaffold.rs`

**Interfaces:**
- Produces:
  - `fn blank_index(site_name: &str) -> String`
  - `fn wp_salts() -> String`
  - `fn wp_config(db_name: &str, db_user: &str, db_pass: &str, db_host: &str, salts: &str) -> String`
  - `fn create_database_sql(name: &str) -> String`
  - `fn laravel_create_argv(target_dir: &str) -> Vec<String>`
  - `const WORDPRESS_URL: &str`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/scaffold.rs`:
```rust
    #[test]
    fn blank_index_has_app_and_php_info() {
        let s = blank_index("blog");
        assert!(s.contains("<?php"));
        assert!(s.contains("blog"));
        assert!(s.contains("phpversion("));
        assert!(s.contains("Laragon Linux"));
        assert!(s.contains("phpinfo")); // ?phpinfo toggle
    }

    #[test]
    fn create_database_sql_is_backtick_quoted() {
        assert_eq!(create_database_sql("shop-api"), "CREATE DATABASE IF NOT EXISTS `shop-api`");
    }

    #[test]
    fn laravel_argv_is_create_project() {
        assert_eq!(
            laravel_create_argv("/x/www/blog"),
            vec!["create-project".to_string(), "laravel/laravel".to_string(), "/x/www/blog".to_string()]
        );
    }

    #[test]
    fn wp_salts_are_eight_random_lines() {
        let a = wp_salts();
        let lines: Vec<&str> = a.lines().filter(|l| l.contains("define(")).collect();
        assert_eq!(lines.len(), 8);
        for key in ["AUTH_KEY", "SECURE_AUTH_KEY", "LOGGED_IN_KEY", "NONCE_KEY",
                    "AUTH_SALT", "SECURE_AUTH_SALT", "LOGGED_IN_SALT", "NONCE_SALT"] {
            assert!(a.contains(key), "missing {key}");
        }
        assert_ne!(a, wp_salts(), "salts must be random per call");
    }

    #[test]
    fn wp_config_embeds_db_and_salts() {
        let cfg = wp_config("blog", "root", "", "127.0.0.1", "/*SALTS*/");
        assert!(cfg.contains("define( 'DB_NAME', 'blog' )"));
        assert!(cfg.contains("define( 'DB_USER', 'root' )"));
        assert!(cfg.contains("define( 'DB_HOST', '127.0.0.1' )"));
        assert!(cfg.contains("/*SALTS*/"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core scaffold`
Expected: FAIL — `cannot find function blank_index`.

- [ ] **Step 3: Write minimal implementation**

Add to `core/src/scaffold.rs` (above the test module):
```rust
pub const WORDPRESS_URL: &str = "https://wordpress.org/latest.tar.gz";

/// A self-contained welcome page (no external assets) showing app + PHP info.
pub fn blank_index(site_name: &str) -> String {
    format!(
        r#"<?php
if (isset($_GET['phpinfo'])) {{ phpinfo(); exit; }}
$exts = ['pdo_mysql','redis','curl','mbstring','gd'];
?><!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} — Laragon Linux</title>
<style>
  body{{font-family:system-ui,sans-serif;margin:0;background:#0f1115;color:#e6e8ee;}}
  .wrap{{max-width:680px;margin:8vh auto;padding:0 24px;}}
  h1{{font-size:1.6rem;margin:0 0 4px;}} .muted{{color:#9aa3b2;}}
  .card{{background:#171a21;border:1px solid #262b36;border-radius:12px;padding:20px;margin-top:20px;}}
  .row{{display:flex;justify-content:space-between;padding:6px 0;border-bottom:1px solid #20242e;}}
  .row:last-child{{border-bottom:0;}} code{{color:#7ee2b8;}}
  .ok{{color:#34d399;}} .no{{color:#f87171;}}
  a{{color:#60a5fa;}}
</style></head><body><div class="wrap">
  <h1>🚀 {name}</h1>
  <div class="muted">powered by <strong>Laragon Linux</strong> · <code><?= $_SERVER['HTTP_HOST'] ?? '{name}.dev' ?></code></div>
  <div class="card">
    <div class="row"><span>PHP version</span><code><?= phpversion() ?></code></div>
    <div class="row"><span>SAPI</span><code><?= PHP_SAPI ?></code></div>
    <div class="row"><span>Server</span><code><?= $_SERVER['SERVER_SOFTWARE'] ?? '?' ?></code></div>
    <div class="row"><span>Document root</span><code><?= $_SERVER['DOCUMENT_ROOT'] ?? '?' ?></code></div>
    <div class="row"><span>HTTPS</span><code><?= !empty($_SERVER['HTTPS']) ? 'on' : 'off' ?></code></div>
  </div>
  <div class="card">
    <?php foreach ($exts as $e): $on = extension_loaded($e); ?>
      <div class="row"><span><?= $e ?></span><span class="<?= $on?'ok':'no' ?>"><?= $on?'✓ loaded':'✗ missing' ?></span></div>
    <?php endforeach; ?>
  </div>
  <p class="muted" style="margin-top:20px">
    <a href="http://localhost:8025" target="_blank">Mailpit inbox</a> ·
    <a href="?phpinfo">View full phpinfo</a>
  </p>
</div></body></html>
"#,
        name = site_name
    )
}

/// 8 WordPress keys/salts, random 64-char values generated locally (offline).
pub fn wp_salts() -> String {
    const KEYS: [&str; 8] = [
        "AUTH_KEY", "SECURE_AUTH_KEY", "LOGGED_IN_KEY", "NONCE_KEY",
        "AUTH_SALT", "SECURE_AUTH_SALT", "LOGGED_IN_SALT", "NONCE_SALT",
    ];
    const CHARSET: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-_[]{}";
    let mut rnd = [0u8; 8 * 64];
    // Linux: read randomness from /dev/urandom (offline, no crate).
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut rnd))
        .is_err()
    {
        // Fallback: time/pid-seeded fill (still varies per call).
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ (std::process::id() as u128);
        for (i, b) in rnd.iter_mut().enumerate() {
            *b = ((seed.rotate_left(i as u32 % 97) >> ((i % 13) * 4)) & 0xff) as u8;
        }
    }
    let mut out = String::new();
    for (k, key) in KEYS.iter().enumerate() {
        let val: String =
            (0..64).map(|i| CHARSET[rnd[k * 64 + i] as usize % CHARSET.len()] as char).collect();
        out.push_str(&format!("define( '{key}', '{val}' );\n"));
    }
    out
}

pub fn wp_config(db_name: &str, db_user: &str, db_pass: &str, db_host: &str, salts: &str) -> String {
    format!(
        "<?php\n\
         define( 'DB_NAME', '{db_name}' );\n\
         define( 'DB_USER', '{db_user}' );\n\
         define( 'DB_PASSWORD', '{db_pass}' );\n\
         define( 'DB_HOST', '{db_host}' );\n\
         define( 'DB_CHARSET', 'utf8mb4' );\n\
         define( 'DB_COLLATE', '' );\n\
         \n{salts}\n\
         $table_prefix = 'wp_';\n\
         define( 'WP_DEBUG', false );\n\
         if ( ! defined( 'ABSPATH' ) ) {{ define( 'ABSPATH', __DIR__ . '/' ); }}\n\
         require_once ABSPATH . 'wp-settings.php';\n"
    )
}

pub fn create_database_sql(name: &str) -> String {
    format!("CREATE DATABASE IF NOT EXISTS `{name}`")
}

pub fn laravel_create_argv(target_dir: &str) -> Vec<String> {
    vec![
        "create-project".to_string(),
        "laravel/laravel".to_string(),
        target_dir.to_string(),
    ]
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core scaffold`
Expected: PASS — generators tests green.

- [ ] **Step 5: Commit**

```bash
git add core/src/scaffold.rs
git commit -m "feat(core): add scaffold content generators (index/wp-config/salts/sql)"
```

---

### Task 3: CommandRunner seam

**Files:**
- Modify: `core/src/scaffold.rs`
- Modify: `core/src/lib.rs` (re-export `CommandRunner`, `RealCommandRunner`)

**Interfaces:**
- Produces:
  - `trait CommandRunner: Send + Sync { fn run(&self, program: &str, args: &[String], cwd: Option<&std::path::Path>) -> Result<(), ScaffoldError>; }`
  - `struct RealCommandRunner;` (std::process; non-zero exit → `ScaffoldError::Command` incl captured stderr; spawn error → `Command`).
  - `struct FakeCommandRunner` (NOT `#[cfg(test)]`-gated; records `(program, Vec<args>, cwd:Option<String>)`; `fail: bool` to simulate failure; `calls()` accessor).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:
```rust
    #[test]
    fn fake_runner_records_calls() {
        let r = FakeCommandRunner::new();
        let calls = r.calls();
        r.run("composer", &["create-project".into()], Some(std::path::Path::new("/x"))).unwrap();
        let c = calls.lock().unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0, "composer");
        assert_eq!(c[0].1, vec!["create-project".to_string()]);
        assert_eq!(c[0].2.as_deref(), Some("/x"));
    }

    #[test]
    fn fake_runner_can_fail() {
        let r = FakeCommandRunner::failing();
        assert!(r.run("composer", &[], None).is_err());
    }

    #[test]
    fn real_runner_runs_true_and_errors_on_false() {
        let r = RealCommandRunner;
        assert!(r.run("true", &[], None).is_ok());
        assert!(r.run("false", &[], None).is_err());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core scaffold`
Expected: FAIL — `cannot find type FakeCommandRunner`.

- [ ] **Step 3: Write minimal implementation**

Add to `core/src/scaffold.rs`:
```rust
use std::path::Path;
use std::sync::{Arc, Mutex};

pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<(), ScaffoldError>;
}

pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<(), ScaffoldError> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let output = cmd
            .output()
            .map_err(|e| ScaffoldError::Command(format!("spawn {program}: {e}")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(ScaffoldError::Command(format!(
                "{program} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
}

#[derive(Clone, Default)]
pub struct FakeCommandRunner {
    calls: Arc<Mutex<Vec<(String, Vec<String>, Option<String>)>>>,
    fail: bool,
}

impl FakeCommandRunner {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn failing() -> Self {
        Self { fail: true, ..Self::default() }
    }
    pub fn calls(&self) -> Arc<Mutex<Vec<(String, Vec<String>, Option<String>)>>> {
        self.calls.clone()
    }
}

impl CommandRunner for FakeCommandRunner {
    fn run(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<(), ScaffoldError> {
        self.calls.lock().unwrap().push((
            program.to_string(),
            args.to_vec(),
            cwd.map(|p| p.display().to_string()),
        ));
        if self.fail {
            Err(ScaffoldError::Command(format!("fake failure: {program}")))
        } else {
            Ok(())
        }
    }
}
```

Add to `core/src/lib.rs` re-exports:
```rust
pub use scaffold::{CommandRunner, RealCommandRunner};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core scaffold`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/scaffold.rs core/src/lib.rs
git commit -m "feat(core): add CommandRunner seam (real + fake)"
```

---

### Task 4: create_site orchestrator + CreateReport

**Files:**
- Modify: `core/src/scaffold.rs`
- Modify: `core/src/lib.rs` (re-export `create_site`, `CreateReport`)

**Interfaces:**
- Consumes: `LaragonPaths`, `setup::Downloader`, the generators + `CommandRunner` from Tasks 2–3.
- Produces:
  - `struct CreateReport { site_name: String, hostname: String, template: SiteTemplate, database_created: bool, warnings: Vec<String> }` (serde `Serialize, Clone, Debug`).
  - `fn create_site(paths: &LaragonPaths, name: &str, tld: &str, template: SiteTemplate, mariadb_running: bool, runner: &dyn CommandRunner, downloader: &dyn crate::setup::Downloader) -> Result<CreateReport, ScaffoldError>`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:
```rust
    use crate::paths::LaragonPaths;
    use crate::setup::FakeDownloader;

    fn root() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static C: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!("lara-newsite-{}-{}", std::process::id(), C.fetch_add(1, Ordering::SeqCst)))
    }

    #[test]
    fn blank_creates_index_and_optionally_db() {
        let p = LaragonPaths::new(root());
        p.ensure_dirs().unwrap();
        let runner = FakeCommandRunner::new();
        let calls = runner.calls();
        let dl = FakeDownloader::new();

        let rep = create_site(&p, "blog", "dev", SiteTemplate::Blank, true, &runner, &dl).unwrap();
        assert_eq!(rep.hostname, "blog.dev");
        assert!(p.www().join("blog").join("index.php").is_file());
        // auto-db issued because mariadb_running = true
        let c = calls.lock().unwrap();
        assert!(c.iter().any(|(prog, args, _)| prog == "mariadb"
            && args.iter().any(|a| a.contains("CREATE DATABASE IF NOT EXISTS `blog`"))));
        assert!(rep.database_created);
        std::fs::remove_dir_all(p.root()).ok();
    }

    #[test]
    fn rejects_existing_site() {
        let p = LaragonPaths::new(root());
        p.ensure_dirs().unwrap();
        std::fs::create_dir_all(p.www().join("dup")).unwrap();
        let r = create_site(&p, "dup", "dev", SiteTemplate::Blank, false, &FakeCommandRunner::new(), &FakeDownloader::new());
        assert!(matches!(r, Err(ScaffoldError::AlreadyExists(_))));
        std::fs::remove_dir_all(p.root()).ok();
    }

    #[test]
    fn laravel_runs_composer() {
        let p = LaragonPaths::new(root());
        p.ensure_dirs().unwrap();
        let runner = FakeCommandRunner::new();
        let calls = runner.calls();
        create_site(&p, "app", "dev", SiteTemplate::Laravel, false, &runner, &FakeDownloader::new()).unwrap();
        let c = calls.lock().unwrap();
        assert!(c.iter().any(|(prog, args, _)| prog == "composer"
            && args == &vec!["create-project".to_string(), "laravel/laravel".to_string(),
                             p.www().join("app").display().to_string()]));
        std::fs::remove_dir_all(p.root()).ok();
    }

    #[test]
    fn wordpress_downloads_extracts_and_writes_config() {
        let p = LaragonPaths::new(root());
        p.ensure_dirs().unwrap();
        let runner = FakeCommandRunner::new();
        let calls = runner.calls();
        let dl = FakeDownloader::new();
        let urls = dl.requested();
        create_site(&p, "wp", "dev", SiteTemplate::Wordpress, true, &runner, &dl).unwrap();
        assert!(urls.lock().unwrap().iter().any(|u| u.contains("wordpress.org")));
        assert!(calls.lock().unwrap().iter().any(|(prog, _, _)| prog == "tar"));
        assert!(p.www().join("wp").join("wp-config.php").is_file());
        std::fs::remove_dir_all(p.root()).ok();
    }

    #[test]
    fn rolls_back_dir_on_failure() {
        let p = LaragonPaths::new(root());
        p.ensure_dirs().unwrap();
        let runner = FakeCommandRunner::failing(); // composer will "fail"
        let r = create_site(&p, "boom", "dev", SiteTemplate::Laravel, false, &runner, &FakeDownloader::new());
        assert!(r.is_err());
        assert!(!p.www().join("boom").exists(), "partial dir must be rolled back");
        std::fs::remove_dir_all(p.root()).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core scaffold`
Expected: FAIL — `cannot find function create_site`.

- [ ] **Step 3: Write minimal implementation**

Add to `core/src/scaffold.rs`:
```rust
use crate::paths::LaragonPaths;
use crate::setup::Downloader;
use serde::Serialize as _;

#[derive(serde::Serialize, Clone, Debug)]
pub struct CreateReport {
    pub site_name: String,
    pub hostname: String,
    pub template: SiteTemplate,
    pub database_created: bool,
    pub warnings: Vec<String>,
}

fn auto_create_db(
    name: &str,
    mariadb_running: bool,
    runner: &dyn CommandRunner,
    warnings: &mut Vec<String>,
    required: bool,
) -> bool {
    if !mariadb_running {
        if required {
            warnings.push(format!("MariaDB is not running — start it, then create database `{name}`"));
        } else {
            warnings.push("MariaDB is not running — skipped database creation".to_string());
        }
        return false;
    }
    let sql = create_database_sql(name);
    let args = vec![
        "-h".to_string(), "127.0.0.1".to_string(),
        "-u".to_string(), "root".to_string(),
        "-e".to_string(), sql,
    ];
    match runner.run("mariadb", &args, None) {
        Ok(()) => true,
        Err(e) => {
            warnings.push(format!("database creation failed: {e}"));
            false
        }
    }
}

pub fn create_site(
    paths: &LaragonPaths,
    name: &str,
    tld: &str,
    template: SiteTemplate,
    mariadb_running: bool,
    runner: &dyn CommandRunner,
    downloader: &dyn Downloader,
) -> Result<CreateReport, ScaffoldError> {
    validate_site_name(name)?;
    let dir = paths.www().join(name);
    if dir.exists() {
        return Err(ScaffoldError::AlreadyExists(name.to_string()));
    }

    let mut warnings = Vec::new();
    let result = build_template(paths, &dir, name, template, runner, downloader);
    if let Err(e) = result {
        // Roll back any partially-created directory.
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }

    let required_db = matches!(template, SiteTemplate::Wordpress);
    let database_created = auto_create_db(name, mariadb_running, runner, &mut warnings, required_db);

    Ok(CreateReport {
        site_name: name.to_string(),
        hostname: format!("{name}.{tld}"),
        template,
        database_created,
        warnings,
    })
}

fn build_template(
    paths: &LaragonPaths,
    dir: &std::path::Path,
    name: &str,
    template: SiteTemplate,
    runner: &dyn CommandRunner,
    downloader: &dyn Downloader,
) -> Result<(), ScaffoldError> {
    match template {
        SiteTemplate::Blank => {
            std::fs::create_dir_all(dir)?;
            std::fs::write(dir.join("index.php"), blank_index(name))?;
        }
        SiteTemplate::Laravel => {
            // composer creates the dir itself; run from www/.
            let argv = laravel_create_argv(&dir.display().to_string());
            runner.run("composer", &argv, Some(&paths.www()))?;
            // Best-effort .env DB wiring (only if composer produced an .env).
            let env_path = dir.join(".env");
            if let Ok(env) = std::fs::read_to_string(&env_path) {
                let edited = env
                    .lines()
                    .map(|l| {
                        if l.starts_with("DB_DATABASE=") { format!("DB_DATABASE={name}") }
                        else if l.starts_with("DB_USERNAME=") { "DB_USERNAME=root".to_string() }
                        else if l.starts_with("DB_PASSWORD=") { "DB_PASSWORD=".to_string() }
                        else if l.starts_with("DB_HOST=") { "DB_HOST=127.0.0.1".to_string() }
                        else { l.to_string() }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = std::fs::write(&env_path, edited);
            }
        }
        SiteTemplate::Wordpress => {
            std::fs::create_dir_all(dir)?;
            let tarball = paths.tmp().join(format!("wordpress-{name}.tar.gz"));
            downloader
                .fetch(WORDPRESS_URL, &tarball)
                .map_err(|e| ScaffoldError::Download(e.to_string()))?;
            runner.run(
                "tar",
                &[
                    "-xzf".to_string(),
                    tarball.display().to_string(),
                    "-C".to_string(),
                    dir.display().to_string(),
                    "--strip-components=1".to_string(),
                ],
                None,
            )?;
            let cfg = wp_config(name, "root", "", "127.0.0.1", &wp_salts());
            std::fs::write(dir.join("wp-config.php"), cfg)?;
        }
    }
    Ok(())
}
```

Add to `core/src/lib.rs` re-exports:
```rust
pub use scaffold::{create_site, CreateReport};
```

(Note: remove the unused `use serde::Serialize as _;` if the compiler flags it — it is only there should a derive need it; the `#[derive(serde::Serialize)]` is fully-qualified so the import is unnecessary. Delete that line if it warns.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p laragon-core scaffold`
Expected: PASS — all scaffold tests (Blank/Laravel/WordPress/rollback/exists).

- [ ] **Step 5: Run the full core suite + commit**

Run: `cargo test -p laragon-core`
Expected: PASS — everything green.

```bash
git add core/src/scaffold.rs core/src/lib.rs
git commit -m "feat(core): add create_site orchestrator with auto-db and rollback"
```

---

### Task 5: Add `composer` to the setup wizard

**Files:**
- Modify: `core/src/setup.rs`

**Interfaces:**
- Produces: `Component::Composer` participates in `ALL`, `label`, `detect_binary`, `apt_packages_for`, and (via the existing `other =>` arm) detection by binary `composer`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/setup.rs`:
```rust
    #[test]
    fn composer_is_a_component_with_apt_package() {
        assert!(Component::ALL.contains(&Component::Composer));
        assert_eq!(apt_packages_for(Component::Composer), vec!["composer".to_string()]);
        assert_eq!(Component::Composer.label(), "composer");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core setup::tests::composer_is_a_component_with_apt_package`
Expected: FAIL — `no variant named Composer`.

- [ ] **Step 3: Implement**

In `core/src/setup.rs`:
- Add `Composer` to the `enum Component { ... }`.
- Add `Component::Composer,` to `pub const ALL` and bump its length annotation to `[Component; 7]`.
- In `label()`: add arm `Component::Composer => "composer",`.
- In `detect_binary()`: add arm `Component::Composer => "composer".to_string(),`.
- In `apt_packages_for()`: add arm `Component::Composer => vec!["composer".to_string()],`.

(Detection needs no special arm — the `other =>` branch in `detect` already does `resolve_bin(detect_binary(other), ...)`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laragon-core setup`
Expected: PASS — the new test plus existing setup tests (the `detect_reports_all_components` length check now expects 7 — update that assertion from `== 6` to `== 7` if present).

- [ ] **Step 5: Commit**

```bash
git add core/src/setup.rs
git commit -m "feat(core): add composer to the setup wizard components"
```

---

### Task 6: Re-export check + full build

**Files:**
- Modify: `core/src/lib.rs` (verify all re-exports present)

**Interfaces:**
- Produces: `laragon_core` re-exports `SiteTemplate`, `ScaffoldError`, `CommandRunner`, `RealCommandRunner`, `create_site`, `CreateReport` (added across Tasks 1–4). This task just verifies the crate compiles cleanly and the public surface is complete for the GUI.

- [ ] **Step 1: Confirm re-exports exist**

Ensure `core/src/lib.rs` contains:
```rust
pub use scaffold::{
    create_site, CommandRunner, CreateReport, RealCommandRunner, ScaffoldError, SiteTemplate,
};
```
(Consolidate the lines added in earlier tasks into this single `pub use` if convenient.)

- [ ] **Step 2: Build + test the whole core**

Run: `cargo test -p laragon-core`
Expected: PASS — no warnings about unused/missing items.

- [ ] **Step 3: Commit (only if lib.rs changed)**

```bash
git add core/src/lib.rs
git commit -m "chore(core): consolidate scaffold re-exports"
```

---

### Task 7: Tauri `create_site` command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs` (register command)

**Interfaces:**
- Consumes: `laragon_core::{create_site, CreateReport, SiteTemplate, RealCommandRunner, CurlDownloader, Config, PkexecPrivileged, MkcertIssuer, sync_sites, ServiceKind}`, `PhpFpmService`.
- Produces: `create_site({ name, template }) -> Result<CreateReport, String>`.

- [ ] **Step 1: Add imports**

In `src-tauri/src/commands.rs`, extend the `use laragon_core::{...}` groups to add `create_site, CreateReport, SiteTemplate, RealCommandRunner`. (`CurlDownloader`, `MkcertIssuer`, `PkexecPrivileged`, `sync_sites`, `Config`, `ServiceKind` are already imported; `PhpFpmService` too.)

- [ ] **Step 2: Add the command**

Append to `src-tauri/src/commands.rs`:
```rust
#[tauri::command]
pub fn create_site(
    state: tauri::State<AppState>,
    name: String,
    template: SiteTemplate,
) -> Result<CreateReport, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();

    // Read whether MariaDB is currently running (brief lock).
    let mariadb_running = {
        let orch = state.orch.lock().map_err(|_| "internal lock poisoned".to_string())?;
        orch.state(ServiceKind::Mariadb) == laragon_core::ServiceState::Running
    };

    // Scaffold (slow; no orchestrator lock held).
    let report = laragon_core::create_site(
        &state.paths,
        &name,
        &config.tld,
        template,
        mariadb_running,
        &RealCommandRunner,
        &CurlDownloader,
    )
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
        let mut orch = state.orch.lock().map_err(|_| "internal lock poisoned".to_string())?;
        if orch.state(ServiceKind::Nginx) == laragon_core::ServiceState::Running {
            let _ = orch.stop(ServiceKind::Nginx);
            let _ = orch.start(ServiceKind::Nginx);
        }
    }

    Ok(report)
}
```
(If `ServiceState` isn't already imported, reference it fully-qualified as above via `laragon_core::ServiceState`, or add it to the imports.)

- [ ] **Step 3: Register the command**

In `src-tauri/src/main.rs`, add `commands::create_site,` to the `tauri::generate_handler![ ... ]` list.

- [ ] **Step 4: Build**

Run: `cargo build -p laragon-desktop`
Expected: PASS — compiles.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): add create_site command (scaffold + sync + nginx reload)"
```

---

### Task 8: Frontend — New Site modal

**Files:**
- Modify: `dist/app.js`
- Modify: `dist/styles.css` (modal styles, reuse existing tokens)
- (No new files; follow the existing render-from-state + `toast()` pattern in `app.js`.)

**Interfaces:**
- Consumes (via `invoke`): `create_site({ name, template })` → `CreateReport`. `template` MUST be exactly `"Blank" | "Laravel" | "Wordpress"`.

- [ ] **Step 1: Enable the New site buttons**

In `dist/app.js`, the two `<button class="btn-newsite" disabled ...>` markers (in `sitesView()` header and empty-state) must become enabled buttons that open the modal — remove `disabled` and the `title="Coming soon"`, and add a click hook (the app uses event delegation / inline data attributes — match the existing approach, e.g. a `data-action="new-site"` handled in the central click handler).

- [ ] **Step 2: Add modal state + render**

Add to the app `state`: `modal: null` and `newSite: { name: "", template: "Blank", busy: false, error: "" }`. Add a `newSiteModal()` render function returning the modal markup (overlay + card) with: a heading "New site", a text input bound to `state.newSite.name`, a template selector (three options with values exactly `Blank` / `Laravel` / `Wordpress`), a live preview line `→ https://<name>.dev`, an inline error slot, and `Cancel` / `Create` buttons. Render it from the main render when `state.modal === "newsite"`. Match the redesign's card/border/spacing tokens already in `styles.css`.

- [ ] **Step 3: Validate + submit**

Add the same name rule as the backend:
```js
const SITE_NAME_RE = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;
function validName(n) { return n.length >= 1 && n.length <= 63 && SITE_NAME_RE.test(n); }
```
The Create button is disabled unless `validName(state.newSite.name)`. On submit:
```js
async function submitNewSite() {
  const { name, template } = state.newSite;
  if (!validName(name)) { state.newSite.error = "Use lowercase letters, digits, hyphens"; render(); return; }
  state.newSite.busy = true; state.newSite.error = ""; render();
  try {
    const rep = await invoke("create_site", { name, template });
    const extra = rep.database_created ? " · database created" : "";
    const warn = rep.warnings && rep.warnings.length ? { details: rep.warnings } : {};
    toast({ type: "success", title: "Created " + rep.site_name, msg: "https://" + rep.hostname + extra, ...warn });
    state.modal = null;
    state.newSite = { name: "", template: "Blank", busy: false, error: "" };
    applySites(await invoke("list_sites"));
  } catch (e) {
    state.newSite.error = String(e);
    toast({ type: "error", title: "Create failed", msg: String(e) });
  } finally {
    state.newSite.busy = false; render();
  }
}
```
(Use the app's actual state-apply/render helper names — e.g. if sites are applied via an `apply*` function or by re-`refresh()`, match that. If there is no `applySites`, call `await refresh()` instead.)

- [ ] **Step 4: Accessibility + busy state**

While `state.newSite.busy`: disable inputs and the Create button, show a spinner + "Creating… (this can take a minute)". Modal: focus the name input on open, close on `Esc` and on overlay click, trap focus within the card, and ensure `:focus-visible` rings (the redesign already defines them) apply to the new controls.

- [ ] **Step 5: Build + manual smoke (human)**

Run: `cargo build -p laragon-desktop` (must compile; frontend is static).
Then `cargo run -p laragon-desktop` → Sites view → "New site" → create a **Blank** site `demo2` → expect a success toast and `demo2` appears with `https://demo2.dev` (a pkexec prompt may appear the first time to update `/etc/hosts`); opening it shows the welcome page. (Laravel/WordPress need composer/network and are slower — validate at least Blank live; note the others as deferred-to-user verification.)

- [ ] **Step 6: Commit**

```bash
git add dist/app.js dist/styles.css
git commit -m "feat(desktop): add New Site modal (create Blank/Laravel/WordPress)"
```

---

## Self-Review

**1. Spec coverage:**
- Templates Blank/Laravel/WordPress (spec §3.1, §5) → Tasks 1, 2, 4 ✓
- `validate_site_name` rule (spec §3.1, Global Constraints) → Task 1 ✓
- Blank welcome `index.php` with app + PHP info + `?phpinfo` (spec §4) → Task 2 `blank_index` ✓
- WordPress download/extract/wp-config + local salts (spec §5.3) → Tasks 2, 4 ✓
- Laravel composer + `.env` DB wiring (spec §5.2) → Task 4 ✓
- Auto-DB, root/no-pass, backtick-quoted, required-for-WP warning (spec §5.4) → Task 4 `auto_create_db` ✓
- CommandRunner + reuse Downloader (spec §3.1) → Task 3 ✓
- Rollback on failure (spec §6) → Task 4 ✓
- composer in setup (spec §3.3) → Task 5 ✓
- IPC create_site → sync + nginx reload (spec §3.2) → Task 7 ✓
- Modal + toasts + a11y, no alert() (spec §3.4) → Task 8 ✓
- TDD for core (spec §7) → Tasks 1–5 ✓
- **Deferred (spec §8):** site registry / add existing folder, reverse proxy, streamed progress, custom templates — correctly out of scope.

**2. Placeholder scan:** No TBD/"handle errors" left; core steps have complete code. Task 8 is GUI glue with concrete snippets + "match existing app.js pattern" (the file is a 584-line render-from-state module the implementer reads); the JS apply/render helper names are flagged to match the real ones.

**3. Type consistency:** `SiteTemplate{Blank,Laravel,Wordpress}` variants identical across core enum (Task 1), serde wire value, IPC param (Task 7), and frontend values (Task 8 — exactly `"Wordpress"`). `create_site(paths,name,tld,template,mariadb_running,runner,downloader)` signature identical in Task 4 def and Task 7 call. `CreateReport` fields (`site_name`, `hostname`, `template`, `database_created`, `warnings`) consistent between Task 4 and the Task 8 toast usage. `CommandRunner::run(program,args,cwd)` and `FakeCommandRunner::{new,failing,calls}` consistent across Tasks 3–4. `Component::Composer` added to every match arm (Task 5). Re-exports consolidated (Task 6).
