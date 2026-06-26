use crate::layout::installed_versions;
use crate::paths::LaraluxPaths;
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
    // Newest first (e.g. 8.5 above 8.0), matching the nginx catalog ordering.
    versions.sort_by(|a, b| vkey(b).cmp(&vkey(a)));
    versions
        .into_iter()
        .map(|v| PhpVersionInfo {
            installed: installed.contains(&v),
            active: v == active,
            version: v,
        })
        .collect()
}

/// Reduce full patch versions to sorted unique major.minor strings.
pub fn installed_minors(full_versions: &[String]) -> Vec<String> {
    let mut minors: Vec<String> = Vec::new();
    for v in full_versions {
        let mut it = v.split('.');
        if let (Some(maj), Some(min)) = (it.next(), it.next()) {
            let m = format!("{maj}.{min}");
            if !minors.contains(&m) {
                minors.push(m);
            }
        }
    }
    minors.sort_by_key(|v| vkey(v));
    minors
}

/// Version catalog using the live filesystem layout (bin/php/*/).
pub fn php_versions(paths: &LaraluxPaths, active: &str) -> Vec<PhpVersionInfo> {
    let full = installed_versions(paths, "php");
    let installed = installed_minors(&full);
    // `active` may be a full version ("8.3.31") or a minor ("8.3"); compare on minor.
    let active_minor = installed_minors(std::slice::from_ref(&active.to_string()))
        .into_iter().next().unwrap_or_else(|| active.to_string());
    php_versions_from(&installed, &active_minor)
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
        // sorted descending (newest first)
        let vers: Vec<String> = infos.iter().map(|i| i.version.clone()).collect();
        let mut sorted = vers.clone();
        sorted.sort_by_key(|v| {
            let mut it = v.split('.');
            (it.next().unwrap().parse::<u32>().unwrap(), it.next().unwrap().parse::<u32>().unwrap())
        });
        sorted.reverse();
        assert_eq!(vers, sorted);
        assert_eq!(vers.first().map(String::as_str), Some("8.5"));
    }

    #[test]
    fn php_versions_includes_unknown_installed() {
        let infos = php_versions_from(&["8.9".to_string()], "8.4");
        assert!(infos.iter().any(|i| i.version == "8.9" && i.installed));
    }

    #[test]
    fn installed_minors_dedupes_patches() {
        // 8.3.31 and 8.3.40 both count as installed minor "8.3"
        let minors = installed_minors(&["8.3.31".to_string(), "8.3.40".to_string(), "8.4.10".to_string()]);
        assert_eq!(minors, vec!["8.3".to_string(), "8.4".to_string()]);
    }

}
