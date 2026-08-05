//! The convention half of JIT provisioning's mapping rules (AUTH-2,
//! ADR-0013 decision 3): parsing `synveda-{dept}-{team}` group names into
//! candidate (department, team) slug pairs, and the personal-scope slug
//! every placement mints (users at login, service identities at
//! registration — AUTH-3, ADR-0018). Pure logic only — validating
//! candidates against the actual hierarchy is the store's job, and the
//! gateway orchestrates the two (seed §8: identity never touches storage).

use synveda_types::IdentityId;

/// The group-name prefix the convention binds, matched case-insensitively.
pub const CONVENTION_PREFIX: &str = "synveda-";

/// The admin convention group (AUTHZ-3, ADR-0015 decision 6): a subject
/// whose IdP groups contain this name (case-insensitively) gets a
/// tenant-wide `org-admin` role binding upserted at every login
/// completion — the zero-config bootstrap that makes a fresh tenant
/// governable. It has no valid `{dept}-{team}` split, so it can never
/// collide with placement mapping.
pub const ADMIN_GROUP: &str = "synveda-admins";

/// Whether `groups` names the admin convention group.
#[must_use]
pub fn contains_admin_group(groups: &[String]) -> bool {
    groups
        .iter()
        .any(|group| group.eq_ignore_ascii_case(ADMIN_GROUP))
}

/// A candidate split of a convention-shaped group name.
///
/// Group names may themselves contain hyphens (`synveda-eng-data-platform`),
/// so one group yields every split whose halves are both valid slugs; the
/// hierarchy decides which candidate is real (ADR-0013 decision 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConventionCandidate {
    /// The department slug half.
    pub department: String,
    /// The team slug half.
    pub team: String,
}

/// Parses one IdP group name into its convention candidates, leftmost
/// split first. Non-convention names (no `synveda-` prefix, or no valid
/// split) yield nothing; matching is case-insensitive because slugs are
/// lowercase by grammar (ADR-0011) while IdP group names may not be.
#[must_use]
pub fn convention_candidates(group: &str) -> Vec<ConventionCandidate> {
    let lowered = group.to_lowercase();
    let Some(rest) = lowered.strip_prefix(CONVENTION_PREFIX) else {
        return Vec::new();
    };
    rest.match_indices('-')
        .filter_map(|(at, _)| {
            let (department, team) = (&rest[..at], &rest[at + 1..]);
            (is_slug(department) && is_slug(team)).then(|| ConventionCandidate {
                department: department.to_owned(),
                team: team.to_owned(),
            })
        })
        .collect()
}

/// A slug for a personal scope node: a readable base (email local part,
/// else the subject) sanitised into the slug grammar, plus an identity-id
/// suffix so siblings never collide. Paths are display-only (ADR-0011).
/// Used by JIT provisioning (AUTH-2) and service-identity registration
/// (AUTH-3), which place their leaves the same way.
///
/// **The suffix is the id's tail, not its head, and that is a correction.**
/// Identifiers here are UUIDv7 (ADR-0005): the first 48 bits are a
/// millisecond timestamp, so the *first* eight hex characters are identical
/// for everything minted in the same ~65-second window — which made this
/// suffix a constant rather than a discriminator. Two people whose email
/// local parts match, placed under one parent inside a minute, collided on
/// `hierarchy_nodes_sibling_slug_unique`; AUTH-4 found it by creating two
/// personal scopes for one person milliseconds apart (a rehire), where JIT
/// had only ever created them seconds apart by different humans.
#[must_use]
pub fn personal_slug(email: Option<&str>, subject: &str, id: IdentityId) -> String {
    let base = email
        .and_then(|address| address.split('@').next())
        .filter(|local| !local.is_empty())
        .unwrap_or(subject);
    let mut readable: String = base
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    readable.truncate(40);
    let readable = readable.trim_matches('-');
    // The tail: UUIDv7's low bits are random, where its high bits are a
    // clock every sibling minted this minute shares.
    let simple = id.as_uuid().simple().to_string();
    let suffix = &simple[simple.len() - 8..];
    if readable.is_empty() {
        format!("u-{suffix}")
    } else {
        format!("{readable}-{suffix}")
    }
}

/// The tenant/hierarchy slug grammar (ADR-0008): `^[a-z0-9][a-z0-9-]{0,62}$`.
fn is_slug(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    text.len() <= 63
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(department: &str, team: &str) -> ConventionCandidate {
        ConventionCandidate {
            department: department.to_owned(),
            team: team.to_owned(),
        }
    }

    #[test]
    fn simple_group_yields_one_candidate() {
        assert_eq!(
            convention_candidates("synveda-eng-platform"),
            [candidate("eng", "platform")]
        );
    }

    #[test]
    fn hyphenated_names_yield_every_valid_split_leftmost_first() {
        assert_eq!(
            convention_candidates("synveda-eng-data-platform"),
            [
                candidate("eng", "data-platform"),
                candidate("eng-data", "platform"),
            ]
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(
            convention_candidates("Synveda-Eng-Platform"),
            [candidate("eng", "platform")]
        );
    }

    #[test]
    fn non_convention_groups_yield_nothing() {
        for group in [
            "everyone",
            "other-eng-platform",
            "synveda-engineering", // no team half
            "synveda--platform",   // empty department half
            "synveda-eng-",        // empty team half
            "synveda-",
            "",
        ] {
            assert_eq!(convention_candidates(group), [], "group {group:?}");
        }
    }

    #[test]
    fn personal_slugs_fit_the_grammar() {
        let id = IdentityId::new();
        let simple = id.as_uuid().simple().to_string();
        let suffix = &simple[simple.len() - 8..];
        let cases = [
            (
                Some("alice@example.test"),
                "sub-1",
                format!("alice-{suffix}"),
            ),
            (None, "Alice Q. User", format!("alice-q--user-{suffix}")),
            (Some("@nolocal"), "--", format!("u-{suffix}")),
            (None, "ûñïçøðé", format!("u-{suffix}")),
        ];
        for (email, subject, want) in cases {
            let slug = personal_slug(email, subject, id);
            assert_eq!(slug, want);
            assert!(
                slug.len() <= 63
                    && slug.chars().next().unwrap().is_ascii_alphanumeric()
                    && slug
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "slug {slug:?} breaks the grammar"
            );
        }
    }

    /// The suffix has to *discriminate*, which is the whole reason it is
    /// there — and until AUTH-4 it did not.
    ///
    /// Identifiers are UUIDv7 (ADR-0005), whose leading bits are a
    /// millisecond clock, so the first eight hex characters are shared by
    /// everything minted in the same ~65-second window. Two people with the
    /// same email local part placed under one parent inside a minute
    /// collided on `hierarchy_nodes_sibling_slug_unique` — and so did one
    /// person given a second personal scope milliseconds after their first,
    /// which is what a rehire is.
    ///
    /// A thousand ids minted back to back is the shape that failure had.
    #[test]
    fn the_suffix_discriminates_between_ids_minted_in_one_instant() {
        let mut slugs = std::collections::HashSet::new();
        for _ in 0..1_000 {
            let slug = personal_slug(Some("alice@example.test"), "sub", IdentityId::new());
            assert!(slugs.insert(slug.clone()), "slug {slug} collided");
        }
    }
}
