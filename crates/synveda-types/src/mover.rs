//! What happens to a person's own memory when the directory moves them
//! (AUTH-4, ADR-0059 decision 10).
//!
//! Rides the loaded pack beside [`crate::RetentionConfig`] and resolves at
//! the scope the person is moving **away from** — authority over material
//! belongs where the material is.
//!
//! ## Why this is a policy question at all
//!
//! The instinct is that carrying somebody's notes into a more open
//! department discloses them. It does not: every embedded pack excludes
//! user-kind scopes from every content-role grant (ADR-0015 decision 4's
//! privacy floor), and the base layer's explicit principal-scope forbid means
//! a personal scope is readable by its owner and by nobody else no matter
//! where in the tree it hangs.
//!
//! What a move actually changes is the **retention regime**. Nothing is
//! stamped on a record (ADR-0040 decision 3): horizons resolve from the
//! effective pack at the record's own scope, on each sweep. So moving a
//! personal node from a department that keeps material for seven years
//! into one that keeps it for ninety days is a bulk disposal that nobody
//! approved, that no diff shows, and that happens on a background loop's
//! next pass. The hazard of a move is disposal, not disclosure — and that
//! is what this config is about.
//!
//! ## Why it only ever asks when the regime changes
//!
//! A move within one pack's governance re-prices nothing, so nothing is
//! asked and the material follows. The question is put only when the
//! source and destination scopes resolve *different* effective packs.

use serde::{Deserialize, Serialize};

/// What becomes of a mover's personal memory when their placement crosses
/// a policy boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalMemory {
    /// The personal scope moves with the person, and its records take on
    /// the destination's horizons, tiers and budgets from the next read
    /// and the next sweep.
    Follows,
    /// The personal scope is sealed in place — retention-held and
    /// unreadable, exactly as a leaver's is (ADR-0059 decision 8) — and
    /// the person gets a fresh personal scope under their new parent.
    SealsAndRestarts,
}

/// A pack's mover configuration.
///
/// Like [`crate::RetentionConfig`], this never grants and never widens:
/// both options leave the material readable by exactly the people who
/// could read it before, and the choice is only about which regime governs
/// it afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoverConfig {
    /// What happens to the mover's own scope on a cross-pack move.
    pub personal_memory: PersonalMemory,
}

impl MoverConfig {
    /// `regulated-strict`'s configuration, and the fail-safe for a stored
    /// pack that configures nothing.
    ///
    /// The default is the sealing one against the instinct that the
    /// friendlier default is better, and the argument is ADR-0040
    /// decision 13's own sentence: *a pack that configures nothing must
    /// not start destroying memory*. Of the two options only one of them
    /// can, because only one of them hands material to a schedule nobody
    /// wrote it under.
    ///
    /// This is deliberately **not** ADR-0053's fail-safe, which runs the
    /// other way for the quality gate ("a pack that has said nothing has
    /// not asked for a gate"). Nothing here refuses anything: the move
    /// always succeeds either way. What varies is only whether material
    /// crosses a regime boundary, and the unconfigured answer is the one
    /// that cannot lose it.
    pub const STRICT: MoverConfig = MoverConfig {
        personal_memory: PersonalMemory::SealsAndRestarts,
    };

    /// `standard` and `open-collaboration`'s configuration: the material
    /// follows the person.
    ///
    /// Safe under those packs for a reason they state themselves — both
    /// carry [`crate::RetentionConfig::DEFAULT`], whose record horizons
    /// are all unset, so there is no schedule for the material to be
    /// handed to. A tenant that sets a horizon at one department and not
    /// another has made this a live question, and can then set this field
    /// deliberately.
    pub const FOLLOWS: MoverConfig = MoverConfig {
        personal_memory: PersonalMemory::Follows,
    };

    /// Whether a cross-pack move seals the scope it leaves.
    #[must_use]
    pub const fn seals_on_move(&self) -> bool {
        matches!(self.personal_memory, PersonalMemory::SealsAndRestarts)
    }
}

impl Default for MoverConfig {
    fn default() -> Self {
        MoverConfig::STRICT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unconfigured_default_is_the_one_that_cannot_lose_material() {
        // The fail-safe argued in `STRICT`'s docs, asserted rather than
        // described: a stored pack that says nothing about movers must not
        // hand somebody's memory to a schedule it was not written under.
        assert_eq!(MoverConfig::default(), MoverConfig::STRICT);
        assert!(MoverConfig::default().seals_on_move());
    }

    #[test]
    fn the_config_round_trips_through_the_pack_encoding() {
        for config in [MoverConfig::STRICT, MoverConfig::FOLLOWS] {
            let json = serde_json::to_string(&config).expect("serialise");
            let back: MoverConfig = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(config, back);
        }
        // The snake_case wire form a stored pack's JSON is authored in.
        assert_eq!(
            serde_json::to_string(&MoverConfig::STRICT).expect("serialise"),
            r#"{"personal_memory":"seals_and_restarts"}"#
        );
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        // `deny_unknown_fields`, for the reason every other pack config
        // carries it: a typo in a stored pack must fail at apply time, not
        // resolve to a default nobody chose.
        let err = serde_json::from_str::<MoverConfig>(
            r#"{"personal_memory":"follows","personal_prompts":"follows"}"#,
        );
        assert!(err.is_err(), "unknown field must be refused");
    }
}
