use crate::bin::resolve_bin;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("no terminal emulator found")]
    NoTerminal,
    #[error("failed to launch terminal: {0}")]
    Spawn(String),
}

const TERMINAL_CANDIDATES: [&str; 9] = [
    "x-terminal-emulator",
    "gnome-terminal",
    "ptyxis",
    "konsole",
    "xfce4-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "xterm",
];

/// Working-directory arguments for a known terminal emulator (by basename).
/// Unknown emulators get no args and rely on the spawned process's cwd.
pub fn terminal_argv(emulator: &str, dir: &Path) -> Vec<String> {
    let d = dir.display().to_string();
    match emulator {
        // `--standalone` starts a fresh instance so ptyxis does not also restore
        // the previous session's tab (which otherwise opens a stray extra tab).
        "ptyxis" => vec![
            "--standalone".to_string(),
            "--new-window".to_string(),
            format!("--working-directory={d}"),
        ],
        "gnome-terminal" | "xfce4-terminal" | "tilix" => vec![format!("--working-directory={d}")],
        "konsole" => vec!["--workdir".to_string(), d],
        "kitty" => vec!["--directory".to_string(), d],
        "alacritty" => vec!["--working-directory".to_string(), d],
        "wezterm" => vec!["start".to_string(), "--cwd".to_string(), d],
        _ => Vec::new(),
    }
}

/// Pick the default terminal emulator: $TERMINAL, then the known candidates.
/// The result is canonicalized so `x-terminal-emulator` resolves to the real
/// binary (e.g. ptyxis) — its basename then selects the right flags.
pub fn detect_terminal() -> Option<PathBuf> {
    let mut found: Option<PathBuf> = None;
    if let Ok(t) = std::env::var("TERMINAL") {
        if !t.is_empty() {
            found = resolve_bin(&t, &[]);
        }
    }
    if found.is_none() {
        for c in TERMINAL_CANDIDATES {
            if let Some(p) = resolve_bin(c, &[]) {
                found = Some(p);
                break;
            }
        }
    }
    found.map(|p| std::fs::canonicalize(&p).unwrap_or(p))
}

/// Launch the default terminal emulator in `dir`, detached.
pub fn open_terminal(dir: &Path) -> Result<(), TerminalError> {
    let emu = detect_terminal().ok_or(TerminalError::NoTerminal)?;
    let base = emu
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let args = terminal_argv(&base, dir);
    std::process::Command::new(&emu)
        .args(&args)
        .current_dir(dir)
        .spawn()
        .map_err(|e| TerminalError::Spawn(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn argv_per_emulator() {
        let d = Path::new("/srv/app");
        assert_eq!(
            terminal_argv("ptyxis", d),
            vec![
                "--standalone".to_string(),
                "--new-window".to_string(),
                "--working-directory=/srv/app".to_string()
            ]
        );
        assert_eq!(
            terminal_argv("gnome-terminal", d),
            vec!["--working-directory=/srv/app".to_string()]
        );
        assert_eq!(
            terminal_argv("konsole", d),
            vec!["--workdir".to_string(), "/srv/app".to_string()]
        );
        assert_eq!(
            terminal_argv("kitty", d),
            vec!["--directory".to_string(), "/srv/app".to_string()]
        );
        assert_eq!(
            terminal_argv("wezterm", d),
            vec!["start".to_string(), "--cwd".to_string(), "/srv/app".to_string()]
        );
        assert!(terminal_argv("xterm", d).is_empty());
    }
}
