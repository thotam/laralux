pub const HOSTS_BEGIN: &str = "# BEGIN laralux";
pub const HOSTS_END: &str = "# END laralux";

/// Render the managed block (markers + one mapping line per hostname).
pub fn render_block(hostnames: &[String]) -> String {
    let mut s = String::new();
    s.push_str(HOSTS_BEGIN);
    s.push('\n');
    for host in hostnames {
        s.push_str(&format!("127.0.0.1 {host}\n"));
    }
    s.push_str(HOSTS_END);
    s.push('\n');
    s
}

/// Strip any existing managed block from `existing`, then append a fresh one.
pub fn apply_block(existing: &str, hostnames: &[String]) -> String {
    // Collect lines that are NOT inside a managed block.
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        if line.trim() == HOSTS_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == HOSTS_END {
            inside = false;
            continue;
        }
        if !inside {
            kept.push(line);
        }
    }

    let mut out = String::new();
    for line in &kept {
        out.push_str(line);
        out.push('\n');
    }
    // Exactly one blank separator only if there is preceding content.
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&render_block(hostnames));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn render_block_lists_each_host() {
        let b = render_block(&hosts(&["app.dev", "blog.dev"]));
        assert!(b.starts_with(HOSTS_BEGIN));
        assert!(b.contains("\n127.0.0.1 app.dev\n"));
        assert!(b.contains("\n127.0.0.1 blog.dev\n"));
        assert!(b.trim_end().ends_with(HOSTS_END));
    }

    #[test]
    fn apply_block_appends_to_clean_file_and_preserves_lines() {
        let existing = "127.0.0.1 localhost\n255.255.255.255 broadcasthost\n";
        let out = apply_block(existing, &hosts(&["app.dev"]));
        assert!(out.contains("127.0.0.1 localhost"));
        assert!(out.contains("broadcasthost"));
        assert!(out.contains("127.0.0.1 app.dev"));
        assert!(out.contains(HOSTS_BEGIN) && out.contains(HOSTS_END));
    }

    #[test]
    fn apply_block_replaces_existing_block_idempotently() {
        let existing = "127.0.0.1 localhost\n";
        let once = apply_block(existing, &hosts(&["app.dev"]));
        let twice = apply_block(&once, &hosts(&["app.dev"]));
        assert_eq!(once, twice); // idempotent
        // and the localhost line still appears exactly once
        assert_eq!(twice.matches("127.0.0.1 localhost").count(), 1);
    }

    #[test]
    fn apply_block_updates_when_hosts_change() {
        let existing = "127.0.0.1 localhost\n";
        let first = apply_block(existing, &hosts(&["app.dev"]));
        let second = apply_block(&first, &hosts(&["app.dev", "blog.dev"]));
        assert!(second.contains("127.0.0.1 blog.dev"));
        assert_eq!(second.matches("127.0.0.1 app.dev").count(), 1);
    }
}
