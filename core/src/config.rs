use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicesConfig {
    pub nginx: bool,
    pub php: bool,
    pub mariadb: bool,
    pub redis: bool,
    pub mailpit: bool,
}

impl Default for ServicesConfig {
    fn default() -> Self {
        Self { nginx: true, php: true, mariadb: true, redis: true, mailpit: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_tld")]
    pub tld: String,
    #[serde(default = "default_php")]
    pub php_version: String,
    #[serde(default)]
    pub services: ServicesConfig,
    #[serde(default)]
    pub versions: BTreeMap<String, String>,
    #[serde(default)]
    pub symlinks: BTreeSet<String>,
    #[serde(default)]
    pub php_ini: crate::php_ini::PhpIniSettings,
}

fn default_tld() -> String {
    "dev".to_string()
}
fn default_php() -> String {
    "8.4".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self { tld: default_tld(), php_version: default_php(), services: ServicesConfig::default(), versions: BTreeMap::new(), symlinks: BTreeSet::new(), php_ini: crate::php_ini::PhpIniSettings::default() }
    }
}

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

    /// Load the config, always `normalize()`-ing the result (parsed or default),
    /// so a loaded config's `versions` always reflects the legacy `php_version`.
    /// Note: a loaded missing-file config therefore differs from a bare
    /// `Config::default()` (its `versions["php"]` is populated) — compare via
    /// `tool_version`, not `== Config::default()`.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str::<Config>(&text)?.normalize()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default().normalize()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_dev_tld_and_php84() {
        let c = Config::default();
        assert_eq!(c.tld, "dev");
        assert_eq!(c.php_version, "8.4");
        assert!(c.services.nginx && c.services.php && c.services.mariadb);
    }

    #[test]
    fn load_missing_file_returns_normalized_default() {
        let c = Config::load(std::path::Path::new("/no/such/laralux.toml")).unwrap();
        // load applies normalize(), so compare against normalized default
        assert_eq!(c, Config::default().normalize());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let tmp = std::env::temp_dir().join(format!("lara-cfg-{}.toml", std::process::id()));
        let mut c = Config::default();
        c.tld = "test".into();
        c.php_version = "8.3".into();
        // normalize before save so versions map is populated, matching what load returns
        let c = c.normalize();
        c.save(&tmp).unwrap();
        let back = Config::load(&tmp).unwrap();
        assert_eq!(c, back);
        std::fs::remove_file(&tmp).ok();
    }

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

    #[test]
    fn php_ini_defaults_and_roundtrips() {
        let mut c = Config::default();
        assert_eq!(c.php_ini.memory_limit, "256M");
        c.php_ini.memory_limit = "512M".into();
        c.php_ini.opcache_enable = false;
        let dir = std::env::temp_dir().join(format!("lara-cfg-phpini-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("laralux.toml");
        c.save(&f).unwrap();
        let loaded = Config::load(&f).unwrap();
        assert_eq!(loaded.php_ini.memory_limit, "512M");
        assert!(!loaded.php_ini.opcache_enable);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn old_config_without_php_ini_loads_defaults() {
        let dir = std::env::temp_dir().join(format!("lara-cfg-oldphpini-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("laralux.toml");
        std::fs::write(&f, "tld = \"dev\"\n").unwrap();
        let loaded = Config::load(&f).unwrap();
        assert_eq!(loaded.php_ini, crate::php_ini::PhpIniSettings::default());
        std::fs::remove_dir_all(&dir).ok();
    }
}
