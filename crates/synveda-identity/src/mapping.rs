//! The one IdP-group convention this product reads (AUTHZ-3, ADR-0015
//! decision 6; re-cut onto grants by CPR-7, ADR-0074 decision 4): the
//! `synveda-admins` group, which may claim the tenant's insert-only initial
//! administrator marker on the first qualifying login. Later members receive
//! no automatic grant; Synveda grants remain authoritative.
//!
//! It is deliberately the *only* one. Until CPR-7 this module also parsed
//! `synveda-{dept}-{team}` group names into candidate `(department, team)`
//! placements and minted a personal-scope slug for each of them, because
//! an identity's authority came from where the hierarchy put it. Placement
//! is identity now: an identity's scope is its own `principal`-shaped
//! scope, minted by `synveda_store::scopes::ensure_principal_scope` and
//! slugged by `synveda_store::scopes::principal_slug`, and belonging is
//! directory groups plus grants. So the convention, its candidate parser
//! and its slug helper are deleted rather than re-expressed — there is no
//! placement left for them to decide (ADR-0074 decision 3).
//!
//! Pure logic only: what a group *means* is decided here, and everything
//! it touches in storage is the gateway's to orchestrate (seed §8 —
//! identity never touches storage).

/// The admin convention group (AUTHZ-3, ADR-0015 decision 6; CPR-7,
/// ADR-0074 decision 4): a subject whose IdP groups contain this name
/// (case-insensitively) may claim the one-time tenant-root `administrator`
/// bootstrap when the insert-only marker is still open. Later members must
/// receive authority through normal governed Synveda grants.
pub const ADMIN_GROUP: &str = "synveda-admins";

/// Whether `groups` names the admin convention group.
#[must_use]
pub fn contains_admin_group(groups: &[String]) -> bool {
    groups
        .iter()
        .any(|group| group.eq_ignore_ascii_case(ADMIN_GROUP))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Case-insensitively, because IdP group names are not lowercase by
    /// grammar and an admin who typed `Synveda-Admins` into Entra has
    /// named the same group.
    #[test]
    fn the_admin_group_is_matched_case_insensitively() {
        for group in ["synveda-admins", "Synveda-Admins", "SYNVEDA-ADMINS"] {
            assert!(
                contains_admin_group(&[group.to_owned()]),
                "group {group:?} should name the admin convention"
            );
        }
    }

    /// And nothing else is. The `synveda-` prefix used to open a whole
    /// placement vocabulary; since CPR-7 it opens exactly one door, so a
    /// group that merely starts with it grants nothing.
    #[test]
    fn no_other_synveda_group_means_anything() {
        for group in [
            "synveda-eng-platform",
            "synveda-engineering",
            "synveda-",
            "admins",
            "/synveda-admins",
            "/parent/synveda-admins",
            "",
        ] {
            assert!(
                !contains_admin_group(&[group.to_owned()]),
                "group {group:?} must not name the admin convention"
            );
        }
    }

    #[test]
    fn an_empty_group_list_names_nothing() {
        assert!(!contains_admin_group(&[]));
    }
}
