//! Bounded governed inbound source descriptors.

use serde::{Deserialize, Serialize};

use crate::{Error, RepositoryId, Result};

/// One immutable Configuration entry. Deployment fixes actual transport custody.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSourceDescriptor {
    /// Stable source identity; cannot be reassigned to a different repository.
    pub source_id: String,
    /// Actual repository attached to the governing project.
    pub repository_id: RepositoryId,
    /// Exact deployment allowlist target.
    pub read_target: String,
    /// Fully qualified branch, resolved once per complete observation.
    pub git_ref: String,
    /// Sorted exact Markdown paths; no globs or recursive traversal.
    pub allowed_paths: Vec<String>,
    /// Disconnect stops refresh without deleting approved Knowledge.
    pub enabled: bool,
}

/// Validate the bounded identifier used by descriptors and deployment targets.
pub fn validate_source_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 48
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(invalid());
    }
    Ok(())
}

/// Validate exact inert Git paths before any transport access.
pub fn validate_source_paths(paths: &[String]) -> Result<()> {
    if paths.is_empty() || paths.len() > 32 || paths.windows(2).any(|p| p[0] >= p[1]) {
        return Err(invalid());
    }
    for path in paths {
        let parts: Vec<_> = path.split('/').collect();
        if path.len() > 512
            || !path.ends_with(".md")
            || parts.len() > 16
            || parts
                .iter()
                .any(|p| p.is_empty() || *p == "." || *p == ".." || *p == ".git")
            || path
                .chars()
                .any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '[' | ']'))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Validate the closed branch-reference subset without invoking Git.
pub fn validate_source_ref(reference: &str) -> Result<()> {
    let branch = reference.strip_prefix("refs/heads/").ok_or_else(invalid)?;
    if branch.is_empty()
        || reference.len() > 200
        || branch.contains("..")
        || branch.contains("@{")
        || branch.ends_with('.')
        || branch
            .split('/')
            .any(|p| p.is_empty() || p.starts_with('.') || p.ends_with(".lock"))
        || branch
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !matches!(b, b'/' | b'-' | b'_' | b'.'))
    {
        return Err(invalid());
    }
    Ok(())
}

impl GitSourceDescriptor {
    /// Validate every field before Configuration publication or transport.
    pub fn validate(&self) -> Result<()> {
        validate_source_id(&self.source_id)?;
        validate_source_id(&self.read_target)?;
        validate_source_ref(&self.git_ref)?;
        validate_source_paths(&self.allowed_paths)
    }
}

fn invalid() -> Error {
    Error::Invalid { message: "Git source requires bounded canonical IDs, an exact branch and 1..=32 sorted inert Markdown paths".to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_bounds_reject_path_ref_and_identifier_ambiguity() {
        assert!(validate_source_paths(&["procedures/retries.md".into()]).is_ok());
        for path in [
            "../secret.md",
            "/secret.md",
            "a\\b.md",
            "*.md",
            ".git/config.md",
            "a//b.md",
        ] {
            assert!(validate_source_paths(&[path.into()]).is_err(), "{path}");
        }
        for reference in [
            "main",
            "refs/heads/../main",
            "refs/heads/a.lock",
            "refs/heads/a@{1}",
        ] {
            assert!(validate_source_ref(reference).is_err(), "{reference}");
        }
        assert!(validate_source_ref("refs/heads/release/1.10").is_ok());
        assert!(validate_source_paths(&vec!["index.md".into(); 33]).is_err());
        assert!(validate_source_id("caller-argument").is_ok());
        assert!(validate_source_id("--argument").is_err());
    }
}
