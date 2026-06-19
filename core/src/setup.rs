use crate::bin::resolve_bin;
use crate::paths::LaragonPaths;
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum Component {
    Nginx,
    Php,
    Mariadb,
    Redis,
    Mkcert,
    Mailpit,
}

impl Component {
    pub const ALL: [Component; 6] = [
        Component::Nginx,
        Component::Php,
        Component::Mariadb,
        Component::Redis,
        Component::Mkcert,
        Component::Mailpit,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Component::Nginx => "nginx",
            Component::Php => "php-fpm",
            Component::Mariadb => "mariadb",
            Component::Redis => "redis",
            Component::Mkcert => "mkcert",
            Component::Mailpit => "mailpit",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComponentStatus {
    pub component: Component,
    pub present: bool,
}

/// The binary that, if resolvable, means the component is installed.
fn detect_binary(component: Component, php_version: &str) -> String {
    match component {
        Component::Nginx => "nginx".to_string(),
        Component::Php => format!("php-fpm{php_version}"),
        Component::Mariadb => "mariadbd".to_string(),
        Component::Redis => "redis-server".to_string(),
        Component::Mkcert => "mkcert".to_string(),
        Component::Mailpit => "mailpit".to_string(),
    }
}

/// Detect presence of every component. Mailpit also searches `~/laragon/bin`.
pub fn detect(paths: &LaragonPaths, php_version: &str) -> Vec<ComponentStatus> {
    Component::ALL
        .iter()
        .map(|&component| {
            let name = detect_binary(component, php_version);
            let present = resolve_bin(&name, &[paths.bin()]).is_some();
            ComponentStatus { component, present }
        })
        .collect()
}

/// The apt packages that install a component (empty for mailpit, which is downloaded).
pub fn apt_packages_for(component: Component, php_version: &str) -> Vec<String> {
    match component {
        Component::Nginx => vec!["nginx".to_string()],
        Component::Php => vec![
            format!("php{php_version}-fpm"),
            format!("php{php_version}-cli"),
            format!("php{php_version}-mysql"),
            format!("php{php_version}-curl"),
            format!("php{php_version}-mbstring"),
            format!("php{php_version}-xml"),
        ],
        Component::Mariadb => vec!["mariadb-server".to_string()],
        Component::Redis => vec!["redis-server".to_string()],
        Component::Mkcert => vec!["mkcert".to_string(), "libnss3-tools".to_string()],
        Component::Mailpit => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaragonPaths;

    #[test]
    fn php_packages_are_versioned() {
        let pkgs = apt_packages_for(Component::Php, "8.4");
        assert!(pkgs.contains(&"php8.4-fpm".to_string()));
        assert!(pkgs.contains(&"php8.4-mysql".to_string()));
    }

    #[test]
    fn mailpit_has_no_apt_packages() {
        assert!(apt_packages_for(Component::Mailpit, "8.4").is_empty());
    }

    #[test]
    fn mkcert_includes_nss_tools() {
        let pkgs = apt_packages_for(Component::Mkcert, "8.4");
        assert!(pkgs.contains(&"mkcert".to_string()));
        assert!(pkgs.contains(&"libnss3-tools".to_string()));
    }

    #[test]
    fn detect_reports_all_components() {
        let paths = LaragonPaths::new(std::env::temp_dir().join(format!("lara-detect-{}", std::process::id())));
        let statuses = detect(&paths, "8.4");
        assert_eq!(statuses.len(), 6);
        // A bogus root means mailpit (only in ~/laragon/bin or PATH) is absent here.
        let mailpit = statuses.iter().find(|s| s.component == Component::Mailpit).unwrap();
        assert!(!mailpit.present);
    }
}
