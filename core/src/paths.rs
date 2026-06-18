//! Path management utilities (to be implemented in future tasks).

use std::path::{Path, PathBuf};

/// Resolves the `~/laragon/` directory layout.
#[derive(Clone, Debug)]
pub struct LaragonPaths {
    root: PathBuf,
}

impl LaragonPaths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// `$HOME/laragon`, falling back to `./laragon` if `$HOME` is unset.
    pub fn default_root() -> PathBuf {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join("laragon"),
            None => PathBuf::from("laragon"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn www(&self) -> PathBuf {
        self.root.join("www")
    }
    pub fn etc(&self) -> PathBuf {
        self.root.join("etc")
    }
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn log(&self) -> PathBuf {
        self.root.join("log")
    }
    pub fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }
    pub fn ssl(&self) -> PathBuf {
        self.root.join("ssl")
    }
    pub fn etc_for(&self, sub: &str) -> PathBuf {
        self.etc().join(sub)
    }
    pub fn config_file(&self) -> PathBuf {
        self.root.join("laragon.toml")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [self.www(), self.etc(), self.data(), self.log(), self.tmp(), self.ssl()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_subpaths_under_root() {
        let p = LaragonPaths::new("/tmp/lara".into());
        assert_eq!(p.root(), std::path::Path::new("/tmp/lara"));
        assert_eq!(p.www(), std::path::Path::new("/tmp/lara/www"));
        assert_eq!(p.etc(), std::path::Path::new("/tmp/lara/etc"));
        assert_eq!(p.etc_for("nginx"), std::path::Path::new("/tmp/lara/etc/nginx"));
        assert_eq!(p.config_file(), std::path::Path::new("/tmp/lara/laragon.toml"));
    }

    #[test]
    fn ensure_dirs_creates_layout() {
        let tmp = std::env::temp_dir().join(format!("lara-test-{}", std::process::id()));
        let p = LaragonPaths::new(tmp.clone());
        p.ensure_dirs().unwrap();
        for sub in ["www", "etc", "data", "log", "tmp", "ssl"] {
            assert!(tmp.join(sub).is_dir(), "missing {sub}");
        }
        std::fs::remove_dir_all(&tmp).ok();
    }
}
