use crate::paths::LaragonPaths;
use crate::service::ServiceKind;
use std::path::PathBuf;

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
}
