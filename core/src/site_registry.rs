use crate::scaffold::validate_site_name;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
