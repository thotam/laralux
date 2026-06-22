use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiteTemplate {
    Blank,
    Laravel,
    Wordpress,
}

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("invalid site name: {0}")]
    InvalidName(String),
    #[error("site already exists: {0}")]
    AlreadyExists(String),
    #[error("required tool not installed: {0}")]
    ToolMissing(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("command failed: {0}")]
    Command(String),
    #[error("database error: {0}")]
    Db(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// A valid DNS label: lowercase alnum and hyphens, not starting/ending with a
/// hyphen, length 1–63.
pub fn validate_site_name(name: &str) -> Result<(), ScaffoldError> {
    let ok = (1..=63).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(ScaffoldError::InvalidName(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        assert!(validate_site_name("blog").is_ok());
        assert!(validate_site_name("shop-api").is_ok());
        assert!(validate_site_name("a1").is_ok());
    }

    #[test]
    fn rejects_invalid_names() {
        for bad in ["", "Blog", "a b", "-x", "x-", "a_b", "café"] {
            assert!(validate_site_name(bad).is_err(), "should reject {bad:?}");
        }
        let long = "a".repeat(64);
        assert!(validate_site_name(&long).is_err());
    }
}
