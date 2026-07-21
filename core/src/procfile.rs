use std::path::Path;

/// One declared process from a Procfile line `name: command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcEntry {
    pub name: String,
    pub command: String,
}

/// Parse Foreman-style Procfile text. Lenient: blank lines and `#` comments are
/// ignored, malformed lines are skipped (never aborts), `name` must match
/// `[A-Za-z0-9_-]+`, and a duplicate name keeps the first occurrence.
pub fn parse_procfile(text: &str) -> Vec<ProcEntry> {
    let mut out: Vec<ProcEntry> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((raw_name, raw_cmd)) = trimmed.split_once(':') else {
            continue;
        };
        let name = raw_name.trim();
        let command = raw_cmd.trim();
        if name.is_empty() || command.is_empty() {
            continue;
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        if out.iter().any(|e| e.name == name) {
            continue;
        }
        out.push(ProcEntry { name: name.to_string(), command: command.to_string() });
    }
    out
}

/// Read `<site_root>/Procfile`. `None` if the file is absent; `Some(entries)`
/// (possibly empty) otherwise.
pub fn read_procfile(site_root: &Path) -> Option<Vec<ProcEntry>> {
    // An empty root means "no folder" (e.g. a proxy site without a project
    // folder). Without this guard the join yields the relative path `Procfile`,
    // reading whatever happens to sit in the process's working directory.
    if site_root.as_os_str().is_empty() {
        return None;
    }
    match std::fs::read_to_string(site_root.join("Procfile")) {
        Ok(text) => Some(parse_procfile(&text)),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_commands_comments_blanks() {
        let text = "\n# a comment\nweb: php artisan serve\n\nqueue:  php artisan queue:work --tries=3\n";
        let e = parse_procfile(text);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], ProcEntry { name: "web".into(), command: "php artisan serve".into() });
        // command keeps a colon after the first split; surrounding spaces trimmed.
        assert_eq!(e[1].name, "queue");
        assert_eq!(e[1].command, "php artisan queue:work --tries=3");
    }

    #[test]
    fn skips_malformed_and_duplicate_lines() {
        let text = "no colon here\nbad name!: cmd\nok: first\nok: second\n: empty name\nempty_cmd:   \n";
        let e = parse_procfile(text);
        // only `ok: first` survives (dup `ok` skipped, others malformed/empty)
        assert_eq!(e.len(), 1);
        assert_eq!(e[0], ProcEntry { name: "ok".into(), command: "first".into() });
    }

    #[test]
    fn empty_root_reads_no_procfile() {
        // A proxy site without a project folder carries an empty root. Note that
        // `Path::new("").join("Procfile")` is the RELATIVE path `Procfile`, so
        // without a guard this would read whatever file happens to sit in the
        // process's working directory.
        assert_eq!(Path::new("").join("Procfile"), Path::new("Procfile"));
        assert!(read_procfile(Path::new("")).is_none());
    }

    #[test]
    fn read_procfile_absent_is_none() {
        let dir = std::env::temp_dir().join(format!("lara-proc-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_procfile(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_procfile_present_parses() {
        let dir = std::env::temp_dir().join(format!("lara-proc-yes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Procfile"), b"worker: sleep 1\n").unwrap();
        let e = read_procfile(&dir).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "worker");
        std::fs::remove_dir_all(&dir).ok();
    }
}
