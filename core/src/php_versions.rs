use crate::bin::list_php_fpm_versions;
use crate::paths::LaragonPaths;
use crate::privileged::Privileged;
use serde::Serialize;

pub const KNOWN_PHP_VERSIONS: [&str; 7] = ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"];

#[derive(Debug, thiserror::Error)]
pub enum PhpVersionError {
    #[error("add ondrej PPA failed: {0}")]
    Repo(String),
    #[error("apt install failed: {0}")]
    Apt(String),
    #[error("php {0} is not installed")]
    NotInstalled(String),
}

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

/// The Laragon-parity, version-pinned apt package set for a PHP version.
pub fn apt_packages_for_php(version: &str) -> Vec<String> {
    [
        "fpm", "cli", "curl", "gd", "intl", "imagick", "mbstring", "mysql", "sqlite3", "xml",
        "xsl", "zip", "redis",
    ]
    .iter()
    .map(|ext| format!("php{version}-{ext}"))
    .collect()
}

/// Install a PHP version via the ondrej PPA, then disable its distro fpm unit.
pub fn install_php(version: &str, privileged: &dyn Privileged) -> Result<(), PhpVersionError> {
    privileged
        .add_apt_repository("ppa:ondrej/php")
        .map_err(|e| PhpVersionError::Repo(e.to_string()))?;
    privileged
        .apt_install(&apt_packages_for_php(version))
        .map_err(|e| PhpVersionError::Apt(e.to_string()))?;
    // Best-effort: keep the app in charge of php-fpm; the distro unit is just noise.
    let _ = privileged.disable_system_services(&[format!("php{version}-fpm")]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privileged::FakePrivileged;

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

    #[test]
    fn apt_packages_are_laragon_parity() {
        let pkgs = apt_packages_for_php("8.3");
        assert_eq!(pkgs.len(), 13);
        assert_eq!(pkgs[0], "php8.3-fpm");
        for ext in ["php8.3-gd", "php8.3-imagick", "php8.3-redis", "php8.3-xsl", "php8.3-zip", "php8.3-sqlite3", "php8.3-mysql"] {
            assert!(pkgs.contains(&ext.to_string()), "missing {ext}");
        }
    }

    #[test]
    fn install_php_adds_ppa_installs_and_disables_unit() {
        let p = FakePrivileged::new();
        let repos = p.add_repos();
        let installs = p.apt_installs();
        let disabled = p.disabled_services();
        install_php("8.3", &p).unwrap();
        assert_eq!(repos.lock().unwrap().as_slice(), &["ppa:ondrej/php".to_string()]);
        assert_eq!(installs.lock().unwrap()[0], apt_packages_for_php("8.3"));
        assert_eq!(disabled.lock().unwrap()[0], vec!["php8.3-fpm".to_string()]);
    }

    #[test]
    fn install_php_surfaces_apt_error() {
        let p = FakePrivileged::new();
        p.set_fail_apt(true);
        assert!(matches!(install_php("8.3", &p), Err(PhpVersionError::Apt(_))));
    }
}
