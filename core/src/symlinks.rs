use crate::paths::LaraluxPaths;
use crate::privileged::Privileged;
use crate::tools::{cli_path, cli_paths, info, ManagedTool};
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

/// `/usr/local/bin/<cli>` for the tool's PRIMARY CLI (the one whose presence
/// gates linking), if any.
pub fn system_link_path(tool: ManagedTool) -> Option<PathBuf> {
    info(tool).cli_binary().map(|b| std::path::Path::new(SYSTEM_BIN_DIR).join(b))
}

/// `/usr/local/bin/<cli>` for EVERY CLI the tool ships (e.g. node → node/npm/npx).
pub fn system_link_paths(tool: ManagedTool) -> Vec<PathBuf> {
    info(tool)
        .cli_binaries
        .iter()
        .map(|b| std::path::Path::new(SYSTEM_BIN_DIR).join(b))
        .collect()
}

/// Symlink every CLI the tool ships into `/usr/local/bin`. Gated on the primary
/// CLI existing (`NotInstalled` otherwise); secondary CLIs that happen to be
/// absent are skipped rather than failing the whole link.
pub fn link_tool(paths: &LaraluxPaths, tool: ManagedTool, privileged: &dyn Privileged) -> Result<(), SymlinkError> {
    let primary = cli_path(tool, paths).ok_or(SymlinkError::NoCli)?;
    if !primary.exists() {
        return Err(SymlinkError::NotInstalled);
    }
    for (name, src) in cli_paths(tool, paths) {
        if !src.exists() {
            continue;
        }
        let dst = std::path::Path::new(SYSTEM_BIN_DIR).join(name);
        privileged.create_symlink(&src, &dst).map_err(|e| SymlinkError::Priv(e.to_string()))?;
    }
    Ok(())
}

/// Remove every `/usr/local/bin` symlink the tool owns.
pub fn unlink_tool(tool: ManagedTool, privileged: &dyn Privileged) -> Result<(), SymlinkError> {
    let dsts = system_link_paths(tool);
    if dsts.is_empty() {
        return Err(SymlinkError::NoCli);
    }
    for dst in dsts {
        privileged.remove_symlink(&dst).map_err(|e| SymlinkError::Priv(e.to_string()))?;
    }
    Ok(())
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
    fn link_tool_links_all_node_clis() {
        let root = std::env::temp_dir().join(format!("lara-symlink-node-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        let cur = paths.bin().join("node").join("current");
        std::fs::create_dir_all(&cur).unwrap();
        for b in ["node", "npm", "npx"] {
            std::fs::write(cur.join(b), b"x").unwrap();
        }
        let p = FakePrivileged::new();
        link_tool(&paths, ManagedTool::Node, &p).unwrap();
        let created = p.symlinks_created();
        let created = created.lock().unwrap();
        let dsts: Vec<&str> = created.iter().map(|(_, d)| d.as_str()).collect();
        assert!(dsts.contains(&"/usr/local/bin/node"));
        assert!(dsts.contains(&"/usr/local/bin/npm"));
        assert!(dsts.contains(&"/usr/local/bin/npx"));
        assert_eq!(created.len(), 3);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn link_tool_skips_absent_secondary_clis() {
        // Only `node` present (no npm/npx) — link still succeeds with just node.
        let root = std::env::temp_dir().join(format!("lara-symlink-node2-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        let cur = paths.bin().join("node").join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(cur.join("node"), b"x").unwrap();
        let p = FakePrivileged::new();
        link_tool(&paths, ManagedTool::Node, &p).unwrap();
        let created = p.symlinks_created();
        assert_eq!(created.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unlink_tool_removes_all_node_clis() {
        let p = FakePrivileged::new();
        unlink_tool(ManagedTool::Node, &p).unwrap();
        let removed = p.symlinks_removed();
        assert_eq!(removed.lock().unwrap().len(), 3);
        std::fs::remove_dir_all(std::env::temp_dir().join("nonexistent-noop")).ok();
    }

    #[test]
    fn link_tool_errors_when_not_installed() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-symlink2-{}", std::process::id())));
        let p = FakePrivileged::new();
        assert!(matches!(link_tool(&paths, ManagedTool::Php, &p), Err(SymlinkError::NotInstalled)));
    }
}
