//! The convention half of JIT provisioning's mapping rules (AUTH-2,
//! ADR-0013 decision 3): parsing `synveda-{dept}-{team}` group names into
//! candidate (department, team) slug pairs. Pure logic only — validating
//! candidates against the actual hierarchy is the store's job, and the
//! gateway orchestrates the two (seed §8: identity never touches storage).

/// The group-name prefix the convention binds, matched case-insensitively.
pub const CONVENTION_PREFIX: &str = "synveda-";

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
}
