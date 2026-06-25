use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_tld")]
    pub tld: String,
    #[serde(default = "default_php")]
    pub php_version: String,
    #[serde(default)]
    pub services: ServicesConfig,
    #[serde(default)]
    pub shell_integration: bool,
}

fn default_tld() -> String {
    "dev".to_string()
}
fn default_php() -> String {
    "8.4".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self { tld: default_tld(), php_version: default_php(), services: ServicesConfig::default(), shell_integration: false }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
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
    fn load_missing_file_returns_default() {
        let c = Config::load(std::path::Path::new("/no/such/laragon.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let tmp = std::env::temp_dir().join(format!("lara-cfg-{}.toml", std::process::id()));
        let mut c = Config::default();
        c.tld = "test".into();
        c.php_version = "8.3".into();
        c.save(&tmp).unwrap();
        let back = Config::load(&tmp).unwrap();
        assert_eq!(c, back);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn shell_integration_defaults_false_and_roundtrips() {
        assert!(!Config::default().shell_integration);
        let tmp = std::env::temp_dir().join(format!("lara-cfg-si-{}.toml", std::process::id()));
        let mut c = Config::default();
        c.shell_integration = true;
        c.save(&tmp).unwrap();
        assert!(Config::load(&tmp).unwrap().shell_integration);
        std::fs::remove_file(&tmp).ok();
    }
}
