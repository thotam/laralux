use crate::bin::list_php_fpm_versions;
use crate::paths::LaragonPaths;
use serde::Serialize;

pub const KNOWN_PHP_VERSIONS: [&str; 6] = ["8.0", "8.1", "8.2", "8.3", "8.4", "8.5"];

pub const DEFAULT_PHP_VERSION: &str = "8.5";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhpVersionInfo {
    pub version: String,
    pub installed: bool,
    pub active: bool,
}

fn vkey(v: &str) -> (u32, u32) {
    let mut it = v.split('.');
    let maj = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let min = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (maj, min)
}

/// Build the version catalog from a known list unioned with installed versions.
pub fn php_versions_from(installed: &[String], active: &str) -> Vec<PhpVersionInfo> {
    let mut versions: Vec<String> = KNOWN_PHP_VERSIONS.iter().map(|s| s.to_string()).collect();
    for v in installed {
        if !versions.contains(v) {
            versions.push(v.clone());
        }
    }
    versions.sort_by_key(|v| vkey(v));
    versions
        .into_iter()
        .map(|v| PhpVersionInfo {
            installed: installed.contains(&v),
            active: v == active,
            version: v,
        })
        .collect()
}

/// Version catalog using the live filesystem (PATH + ~/laragon/bin + system dirs).
pub fn php_versions(paths: &LaragonPaths, active: &str) -> Vec<PhpVersionInfo> {
    php_versions_from(&list_php_fpm_versions(&[paths.bin()]), active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn php_versions_marks_installed_and_active() {
        let infos = php_versions_from(&["8.2".to_string(), "8.4".to_string()], "8.4");
        // every known version present
        for v in KNOWN_PHP_VERSIONS {
            assert!(infos.iter().any(|i| i.version == v), "missing {v}");
        }
        let by = |v: &str| infos.iter().find(|i| i.version == v).unwrap().clone();
        assert!(by("8.4").installed && by("8.4").active);
        assert!(by("8.2").installed && !by("8.2").active);
        assert!(!by("8.3").installed && !by("8.3").active);
        // sorted ascending
        let vers: Vec<String> = infos.iter().map(|i| i.version.clone()).collect();
        let mut sorted = vers.clone();
        sorted.sort_by_key(|v| {
            let mut it = v.split('.');
            (it.next().unwrap().parse::<u32>().unwrap(), it.next().unwrap().parse::<u32>().unwrap())
        });
        assert_eq!(vers, sorted);
    }

    #[test]
    fn php_versions_includes_unknown_installed() {
        let infos = php_versions_from(&["8.9".to_string()], "8.4");
        assert!(infos.iter().any(|i| i.version == "8.9" && i.installed));
    }

}
