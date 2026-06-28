use crate::bin::resolve_bin;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum FileManagerError {
    #[error("no file manager found")]
    NoFileManager,
    #[error("failed to launch file manager: {0}")]
    Spawn(String),
}

/// Launchers that open a directory in a GUI file manager. `xdg-open` is preferred
/// (it routes to the user's default); the rest are common fallbacks. All accept
/// the directory as their sole argument.
const FILE_MANAGER_CANDIDATES: [&str; 7] = [
    "xdg-open",
    "nautilus",
    "dolphin",
    "thunar",
    "nemo",
    "pcmanfm",
    "caja",
];

/// First resolvable launcher from the candidate list, or None.
pub fn detect_file_manager() -> Option<PathBuf> {
    for c in FILE_MANAGER_CANDIDATES {
        if let Some(p) = resolve_bin(c, &[]) {
            return Some(p);
        }
    }
    None
}

/// Open `dir` in the default file manager, detached.
pub fn open_folder(dir: &Path) -> Result<(), FileManagerError> {
    let fm = detect_file_manager().ok_or(FileManagerError::NoFileManager)?;
    std::process::Command::new(&fm)
        .arg(dir)
        .spawn()
        .map_err(|e| FileManagerError::Spawn(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_open_is_preferred_and_list_is_well_formed() {
        assert_eq!(FILE_MANAGER_CANDIDATES[0], "xdg-open");
        assert_eq!(FILE_MANAGER_CANDIDATES.len(), 7);
        assert!(FILE_MANAGER_CANDIDATES.iter().all(|c| !c.is_empty()));
        assert!(FILE_MANAGER_CANDIDATES.contains(&"nautilus"));
    }
}
