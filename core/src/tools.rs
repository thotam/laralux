use crate::paths::LaragonPaths;
use crate::service::ServiceKind;
use std::path::PathBuf;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedTool { Php, Nginx, Mariadb, Redis, Mailpit, Mkcert, Composer }

impl ManagedTool {
    pub const ALL: [ManagedTool; 7] = [
        ManagedTool::Php, ManagedTool::Nginx, ManagedTool::Mariadb, ManagedTool::Redis,
        ManagedTool::Mailpit, ManagedTool::Mkcert, ManagedTool::Composer,
    ];
}

pub struct ToolInfo {
    pub key: &'static str,
    pub display: &'static str,
    pub cli_binary: Option<&'static str>,
    pub service_kind: Option<ServiceKind>,
}

pub fn info(tool: ManagedTool) -> ToolInfo {
    use ManagedTool::*;
    match tool {
        Php => ToolInfo { key: "php", display: "PHP", cli_binary: Some("php"), service_kind: Some(ServiceKind::PhpFpm) },
        Nginx => ToolInfo { key: "nginx", display: "Nginx", cli_binary: Some("nginx"), service_kind: Some(ServiceKind::Nginx) },
        Mariadb => ToolInfo { key: "mariadb", display: "MariaDB", cli_binary: Some("mariadb"), service_kind: Some(ServiceKind::Mariadb) },
        Redis => ToolInfo { key: "redis", display: "Redis", cli_binary: Some("redis-cli"), service_kind: Some(ServiceKind::Redis) },
        Mailpit => ToolInfo { key: "mailpit", display: "Mailpit", cli_binary: None, service_kind: Some(ServiceKind::Mailpit) },
        Mkcert => ToolInfo { key: "mkcert", display: "mkcert", cli_binary: Some("mkcert"), service_kind: None },
        Composer => ToolInfo { key: "composer", display: "Composer", cli_binary: Some("composer"), service_kind: None },
    }
}

pub fn key(tool: ManagedTool) -> &'static str {
    info(tool).key
}

pub fn from_key(k: &str) -> Option<ManagedTool> {
    ManagedTool::ALL.into_iter().find(|t| key(*t) == k)
}

/// Absolute path to the tool's terminal CLI under `bin/<key>/current/<cli>`, if it has one.
pub fn cli_path(tool: ManagedTool, paths: &LaragonPaths) -> Option<PathBuf> {
    info(tool)
        .cli_binary
        .map(|b| paths.bin().join(key(tool)).join("current").join(b))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolVersion {
    pub version: String,
    pub installed: bool,
    pub active: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("installing additional versions is not supported for this tool yet")]
    Unsupported,
    #[error("install failed: {0}")]
    Install(String),
}

/// Versions selectable for a tool. PHP exposes the known catalog (∪ installed);
/// every other tool exposes only its installed version(s) (single, for now).
pub fn available_versions(tool: ManagedTool, paths: &LaragonPaths) -> Vec<ToolVersion> {
    let cfg = crate::config::Config::load(&paths.config_file()).unwrap_or_default();
    match tool {
        ManagedTool::Php => crate::php_versions::php_versions(paths, &cfg.php_version)
            .into_iter()
            .map(|p| ToolVersion { version: p.version, installed: p.installed, active: p.active })
            .collect(),
        other => {
            let k = key(other);
            let active = cfg.versions.get(k).cloned().unwrap_or_default();
            crate::layout::installed_versions(paths, k)
                .into_iter()
                .map(|v| ToolVersion { active: v == active, installed: true, version: v })
                .collect()
        }
    }
}

/// Install a specific version. Only PHP supports installing extra versions in this
/// sub-project; other tools are installed (single-version) via the bulk Setup run.
pub fn install_version(
    tool: ManagedTool,
    paths: &LaragonPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, ToolError> {
    match tool {
        ManagedTool::Php => crate::php_static::install_php_static(paths, version, downloader, runner, sink)
            .map_err(|e| ToolError::Install(e.to_string())),
        _ => Err(ToolError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_binary_mapping_and_mailpit_has_none() {
        assert_eq!(info(ManagedTool::Php).cli_binary, Some("php"));
        assert_eq!(info(ManagedTool::Composer).cli_binary, Some("composer"));
        assert_eq!(info(ManagedTool::Mariadb).cli_binary, Some("mariadb"));
        assert_eq!(info(ManagedTool::Mkcert).cli_binary, Some("mkcert"));
        assert_eq!(info(ManagedTool::Redis).cli_binary, Some("redis-cli"));
        assert_eq!(info(ManagedTool::Nginx).cli_binary, Some("nginx"));
        assert_eq!(info(ManagedTool::Mailpit).cli_binary, None);
    }

    #[test]
    fn key_roundtrips_through_from_key() {
        for t in ManagedTool::ALL {
            assert_eq!(from_key(key(t)), Some(t));
        }
        assert_eq!(from_key("nope"), None);
    }

    #[test]
    fn cli_path_is_under_current_and_none_for_mailpit() {
        let p = LaragonPaths::new("/tmp/lara".into());
        assert_eq!(cli_path(ManagedTool::Php, &p), Some(PathBuf::from("/tmp/lara/bin/php/current/php")));
        assert_eq!(cli_path(ManagedTool::Redis, &p), Some(PathBuf::from("/tmp/lara/bin/redis/current/redis-cli")));
        assert_eq!(cli_path(ManagedTool::Mailpit, &p), None);
    }

    #[test]
    fn php_available_versions_lists_known_set() {
        let root = std::env::temp_dir().join(format!("lara-tools-php-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let vs = available_versions(ManagedTool::Php, &paths);
        // KNOWN_PHP_VERSIONS has 6 entries (8.0..8.5); none installed on a fresh root.
        assert_eq!(vs.len(), 6);
        assert!(vs.iter().all(|v| !v.installed));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn single_version_tool_lists_installed_only() {
        let root = std::env::temp_dir().join(format!("lara-tools-ng-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        // Seed an installed nginx version dir.
        std::fs::create_dir_all(paths.version_dir("nginx", "1.31.2")).unwrap();
        let vs = available_versions(ManagedTool::Nginx, &paths);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].version, "1.31.2");
        assert!(vs[0].installed);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_version_unsupported_for_non_php() {
        let paths = LaragonPaths::new("/tmp/lara".into());
        let err = install_version(
            ManagedTool::Nginx, &paths, "1.31.2",
            &crate::setup::FakeDownloader::new(), &crate::scaffold::FakeCommandRunner::new(),
            &crate::progress::NullProgress,
        );
        assert!(matches!(err, Err(ToolError::Unsupported)));
    }
}
