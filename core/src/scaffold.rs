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
}
