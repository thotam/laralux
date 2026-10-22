//! Version catalogs built from each tool's upstream release feed.

use serde::{Deserialize, Serialize};

pub const PATCHES_PER_LINE: usize = 3;
/// Lines shown when the source does not say which ones are supported.
pub const DEFAULT_LINES: usize = 4;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Feed {
    pub versions: Vec<String>,
    /// Release lines the source marks as supported; `None` means "newest N".
    pub allowed_lines: Option<Vec<String>>,
}

pub enum Lines {
    Latest(usize),
    Allowed(Vec<String>),
}

/// Only plain numeric versions (`2.10.3`, `8.4`): drops RCs, tags and hashes.
pub fn is_stable(v: &str) -> bool {
    !v.is_empty() && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

pub fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn line_major(v: &str) -> String {
    v.split('.').next().unwrap_or("").to_string()
}

pub fn line_minor(v: &str) -> String {
    v.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// Newest `patches` versions of each selected line, newest first.
pub fn select(versions: &[String], line_of: fn(&str) -> String, lines: &Lines, patches: usize) -> Vec<String> {
    let mut stable: Vec<&String> = versions.iter().filter(|v| is_stable(v)).collect();
    stable.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    stable.dedup();

    let mut line_keys: Vec<String> = Vec::new();
    for v in &stable {
        let l = line_of(v);
        if !line_keys.contains(&l) {
            line_keys.push(l);
        }
    }
    let chosen: Vec<String> = match lines {
        Lines::Latest(n) => line_keys.into_iter().take(*n).collect(),
        Lines::Allowed(allowed) => line_keys.into_iter().filter(|l| allowed.contains(l)).collect(),
    };

    chosen
        .iter()
        .flat_map(|l| stable.iter().filter(move |v| &line_of(v) == l).take(patches).map(|v| v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn select_keeps_three_patches_of_four_newest_lines() {
        let vs = s(&["1.31.6", "1.31.5", "1.31.4", "1.31.3", "1.30.5", "1.29.8", "1.28.3", "1.27.5"]);
        assert_eq!(
            select(&vs, line_minor, &Lines::Latest(4), 3),
            s(&["1.31.6", "1.31.5", "1.31.4", "1.30.5", "1.29.8", "1.28.3"])
        );
    }

    #[test]
    fn select_allowed_lines_and_drops_prerelease_and_dupes() {
        let vs = s(&["13.1.0-rc", "12.3.3", "12.3.3", "11.8.6", "10.5.29", "abc123"]);
        let lines = Lines::Allowed(s(&["12.3", "11.8"]));
        assert_eq!(select(&vs, line_minor, &lines, 3), s(&["12.3.3", "11.8.6"]));
    }

    #[test]
    fn select_sorts_numerically_not_lexically() {
        assert_eq!(select(&s(&["9.0.0", "10.0.0"]), line_major, &Lines::Latest(4), 3), s(&["10.0.0", "9.0.0"]));
    }

    #[test]
    fn is_stable_rejects_non_numeric_parts() {
        assert!(is_stable("2.10.3") && is_stable("8.4"));
        assert!(!is_stable("2.12.0-RC1") && !is_stable("v1.4.4") && !is_stable("") && !is_stable("1..2"));
    }
}
