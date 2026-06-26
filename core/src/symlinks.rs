use crate::paths::LaraluxPaths;
use crate::privileged::Privileged;
use crate::tools::{cli_path, info, ManagedTool};
use std::path::PathBuf;

pub const SYSTEM_BIN_DIR: &str = "/usr/local/bin";

#[derive(Debug, thiserror::Error)]
pub enum SymlinkError {
    #[error("tool has no terminal CLI to link")]
    NoCli,
    #[error("tool is not installed yet")]
    NotInstalled,
    #[error("privileged op failed: {0}")]
    Priv(String),
}

pub fn system_link_path(tool: ManagedTool) -> Option<PathBuf> {
    info(tool).cli_binary.map(|b| std::path::Path::new(SYSTEM_BIN_DIR).join(b))
}

pub fn link_tool(paths: &LaraluxPaths, tool: ManagedTool, privileged: &dyn Privileged) -> Result<(), SymlinkError> {
    let src = cli_path(tool, paths).ok_or(SymlinkError::NoCli)?;
    if !src.exists() {
        return Err(SymlinkError::NotInstalled);
    }
    let dst = system_link_path(tool).ok_or(SymlinkError::NoCli)?;
    privileged.create_symlink(&src, &dst).map_err(|e| SymlinkError::Priv(e.to_string()))
}

pub fn unlink_tool(tool: ManagedTool, privileged: &dyn Privileged) -> Result<(), SymlinkError> {
    let dst = system_link_path(tool).ok_or(SymlinkError::NoCli)?;
    privileged.remove_symlink(&dst).map_err(|e| SymlinkError::Priv(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privileged::FakePrivileged;

    #[test]
    fn system_link_path_per_tool() {
        assert_eq!(system_link_path(ManagedTool::Php), Some(PathBuf::from("/usr/local/bin/php")));
        assert_eq!(system_link_path(ManagedTool::Redis), Some(PathBuf::from("/usr/local/bin/redis-cli")));
        assert_eq!(system_link_path(ManagedTool::Mailpit), None);
    }

    #[test]
    fn link_tool_calls_create_symlink_with_resolved_src_and_dst() {
        let root = std::env::temp_dir().join(format!("lara-symlink-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        // Seed an installed php cli at bin/php/current/php.
        let cur = paths.bin().join("php").join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(cur.join("php"), b"x").unwrap();
        let p = FakePrivileged::new();
        link_tool(&paths, ManagedTool::Php, &p).unwrap();
        let created = p.symlinks_created();
        let created = created.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].0, cur.join("php").display().to_string());
        assert_eq!(created[0].1, "/usr/local/bin/php");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn link_tool_errors_when_not_installed() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-symlink2-{}", std::process::id())));
        let p = FakePrivileged::new();
        assert!(matches!(link_tool(&paths, ManagedTool::Php, &p), Err(SymlinkError::NotInstalled)));
    }
}
