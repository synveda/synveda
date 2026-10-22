//! Content-free pgvector patch advisory for migration commands (OPS-11,
//! ADR-0153).
//!
//! Admission is decided by the catalogue fingerprint. This only tells the
//! operator whether the installed package or the retained catalogue still
//! needs the patched transition, and never runs on a request path.

use sqlx::PgPool;
use synveda_types::{Error, Result};

/// The first native release carrying the CVE-2026-103484 fix.
pub const PATCHED_VECTOR_VERSION: &str = "0.8.7";
const PATCHED: (u32, u32, u32) = (0, 8, 7);
const LEGACY_EXCEPTION_EXPIRES: &str = "2027-01-31";
const TRANSITION_GUIDE: &str = "docs/DEPLOYMENT_CONTRACT.md#retained-pgvector-transition";

/// The version recorded by the `vector` catalogue and the default version of
/// the package installed on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorVersions {
    /// `pg_extension.extversion`.
    pub catalogue: String,
    /// `pg_available_extensions.default_version`, from the installed control file.
    pub package: Option<String>,
}

/// Reads both versions. A missing extension is refused by the admission
/// proof, so it is not reported here.
pub async fn read(pool: &PgPool) -> Result<Option<VectorVersions>> {
    let row = sqlx::query!(
        r#"select extension.extversion as "catalogue!",
                  available.default_version as "package"
             from pg_catalog.pg_extension as extension
             left join pg_catalog.pg_available_extensions as available
               on available.name = extension.extname
            where extension.extname = 'vector'"#
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| Error::Storage {
        message: format!("read the vector extension versions: {error}"),
    })?;
    Ok(row.map(|row| VectorVersions {
        catalogue: row.catalogue,
        package: row.package,
    }))
}

/// One operator advisory, or `None` once the package and the catalogue are
/// both patched.
pub fn advisory(versions: &VectorVersions) -> Option<String> {
    let package = versions.package.as_deref().unwrap_or("unknown");
    if !is_patched(package) {
        return Some(format!(
            "warning: the server's installed pgvector package reports {package}; versions before {PATCHED_VECTOR_VERSION} are affected by CVE-2026-103484 unless a verified vendor backport is installed. Install {PATCHED_VECTOR_VERSION} and restart PostgreSQL; see {TRANSITION_GUIDE}"
        ));
    }
    if !is_patched(&versions.catalogue) {
        return Some(format!(
            "warning: the vector catalogue uses the legacy {} profile, admitted until {LEGACY_EXCEPTION_EXPIRES}. The extension owner should run ALTER EXTENSION vector UPDATE TO '{PATCHED_VECTOR_VERSION}'; see {TRANSITION_GUIDE}",
            versions.catalogue
        ));
    }
    None
}

fn is_patched(version: &str) -> bool {
    let mut parts = version.split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) => {
            (major, minor, patch) >= PATCHED
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(catalogue: &str, package: Option<&str>) -> VectorVersions {
        VectorVersions {
            catalogue: catalogue.to_owned(),
            package: package.map(str::to_owned),
        }
    }

    #[test]
    fn a_patched_package_and_catalogue_need_no_advisory() {
        assert_eq!(advisory(&versions("0.8.7", Some("0.8.7"))), None);
        assert_eq!(advisory(&versions("0.8.7", Some("0.9.0"))), None);
    }

    #[test]
    fn an_affected_or_unknown_package_is_reported_first() {
        for package in [Some("0.8.6"), Some("0.8.2"), Some("0.8.7-beta"), None] {
            let message = advisory(&versions("0.8.6", package)).expect("advisory");
            assert!(message.contains("CVE-2026-103484"), "{message}");
            assert!(message.contains("restart PostgreSQL"), "{message}");
        }
    }

    #[test]
    fn a_retained_legacy_catalogue_names_the_owner_step() {
        let message = advisory(&versions("0.8.6", Some("0.8.7"))).expect("advisory");
        assert!(message.contains("legacy 0.8.6 profile"), "{message}");
        assert!(
            message.contains("ALTER EXTENSION vector UPDATE TO '0.8.7'"),
            "{message}"
        );
        assert!(message.contains("2027-01-31"), "{message}");
    }
}
