use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

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

use crate::paths::LaragonPaths;
use crate::setup::Downloader;

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
}
