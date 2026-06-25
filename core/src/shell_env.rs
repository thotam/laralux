use std::path::Path;

pub const SHELL_BEGIN: &str = "# >>> laragon >>>";
pub const SHELL_END: &str = "# <<< laragon <<<";

pub const SHELL_BLOCK: &str =
    "# >>> laragon >>>\nexport PATH=\"$HOME/laragon/bin:$PATH\"\n# <<< laragon <<<\n";

/// `.zshrc` for a zsh login shell, else `.bashrc`.
pub fn rc_filename_for_shell(shell: &str) -> &'static str {
    if shell.trim_end_matches('/').ends_with("zsh") {
        ".zshrc"
    } else {
        ".bashrc"
    }
}

/// Strip the managed block (markers + their contents) from `contents`.
pub fn remove_shell_block(contents: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in contents.lines() {
        let t = line.trim();
        if t == SHELL_BEGIN {
            skipping = true;
            continue;
        }
        if t == SHELL_END {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Return `contents` with the managed block appended exactly once (idempotent).
pub fn apply_shell_block(contents: &str) -> String {
    let mut base = remove_shell_block(contents);
    if !base.is_empty() && !base.ends_with('\n') {
        base.push('\n');
    }
    base.push_str(SHELL_BLOCK);
    base
}

/// Add the managed PATH block to existing rc files; if none exist, create the
/// one matching `$SHELL`.
pub fn enable_shell_path(home: &Path, shell: &str) -> std::io::Result<()> {
    let mut wrote_any = false;
    for rc in [".bashrc", ".zshrc"] {
        let p = home.join(rc);
        if p.exists() {
            let cur = std::fs::read_to_string(&p)?;
            let upd = apply_shell_block(&cur);
            if upd != cur {
                std::fs::write(&p, upd)?;
            }
            wrote_any = true;
        }
    }
    if !wrote_any {
        let p = home.join(rc_filename_for_shell(shell));
        std::fs::write(&p, apply_shell_block(""))?;
    }
    Ok(())
}

/// Remove the managed PATH block from any existing rc files.
pub fn disable_shell_path(home: &Path) -> std::io::Result<()> {
    for rc in [".bashrc", ".zshrc"] {
        let p = home.join(rc);
        if p.exists() {
            let cur = std::fs::read_to_string(&p)?;
            let upd = remove_shell_block(&cur);
            if upd != cur {
                std::fs::write(&p, upd)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_filename_matches_shell() {
        assert_eq!(rc_filename_for_shell("/usr/bin/zsh"), ".zshrc");
        assert_eq!(rc_filename_for_shell("/bin/bash"), ".bashrc");
        assert_eq!(rc_filename_for_shell(""), ".bashrc");
    }

    #[test]
    fn apply_is_idempotent_and_remove_restores() {
        let base = "export EDITOR=vim\n";
        let once = apply_shell_block(base);
        assert!(once.contains("export PATH=\"$HOME/laragon/bin:$PATH\""));
        assert!(once.contains(SHELL_BEGIN) && once.contains(SHELL_END));
        assert!(once.contains("export EDITOR=vim"));
        let twice = apply_shell_block(&once);
        assert_eq!(once, twice, "re-apply is idempotent");
        let removed = remove_shell_block(&once);
        assert!(!removed.contains("laragon"));
        assert!(removed.contains("export EDITOR=vim"));
    }

    #[test]
    fn enable_creates_rc_matching_shell_when_none() {
        let home = std::env::temp_dir().join(format!("lara-rc-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&home).unwrap();
        enable_shell_path(&home, "/bin/bash").unwrap();
        let bashrc = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        assert!(bashrc.contains("$HOME/laragon/bin"));
        assert!(!home.join(".zshrc").exists(), "no zshrc for a bash user");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn enable_updates_existing_then_disable_removes() {
        let home = std::env::temp_dir().join(format!("lara-rc2-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".bashrc"), "export A=1\n").unwrap();
        enable_shell_path(&home, "/bin/bash").unwrap();
        assert!(std::fs::read_to_string(home.join(".bashrc")).unwrap().contains("laragon"));
        disable_shell_path(&home).unwrap();
        let after = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        assert!(!after.contains("laragon"));
        assert!(after.contains("export A=1"));
        std::fs::remove_dir_all(&home).ok();
    }
}
