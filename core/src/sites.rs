use crate::paths::LaragonPaths;
use std::path::PathBuf;

/// A project under `www/` exposed at `<name>.<tld>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub name: String,
    pub root: PathBuf,
    pub hostname: String,
}

impl Site {
    /// Laravel-style: serve `public/` if present, else the project dir.
    pub fn document_root(&self) -> PathBuf {
        let public = self.root.join("public");
        if public.is_dir() {
            public
        } else {
            self.root.clone()
        }
    }
}

/// Discover sites in `www/`: immediate subdirectories, skipping hidden ones.
pub fn scan_sites(paths: &LaragonPaths, tld: &str) -> std::io::Result<Vec<Site>> {
    let www = paths.www();
    let entries = match std::fs::read_dir(&www) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut sites = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        sites.push(Site {
            hostname: format!("{name}.{tld}"),
            root: entry.path(),
            name,
        });
    }
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sites)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_root() -> std::path::PathBuf {
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("lara-sites-{}-{}-{}", std::process::id(), line!(), counter))
    }

    #[test]
    fn scans_only_dirs_builds_hostnames_sorted() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("beta")).unwrap();
        std::fs::create_dir_all(www.join("alpha")).unwrap();
        std::fs::create_dir_all(www.join(".hidden")).unwrap();
        std::fs::write(www.join("index.php"), "x").unwrap();

        let paths = LaragonPaths::new(root.clone());
        let sites = scan_sites(&paths, "dev").unwrap();

        let names: Vec<&str> = sites.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]); // sorted, no file, no hidden
        assert_eq!(sites[0].hostname, "alpha.dev");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn document_root_prefers_public_subdir() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("laravelapp").join("public")).unwrap();
        std::fs::create_dir_all(www.join("plain")).unwrap();

        let paths = LaragonPaths::new(root.clone());
        let sites = scan_sites(&paths, "dev").unwrap();
        let by = |n: &str| sites.iter().find(|s| s.name == n).unwrap().clone();

        assert_eq!(by("laravelapp").document_root(), www.join("laravelapp").join("public"));
        assert_eq!(by("plain").document_root(), www.join("plain"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_www_returns_empty() {
        let paths = LaragonPaths::new(temp_root());
        assert!(scan_sites(&paths, "dev").unwrap().is_empty());
    }
}
