//! Skill quality vocabulary (SKIL-3, ADR-0053).
//!
//! The half of a skill's score a *person* supplies, and the one thing a
//! policy pack gets to say about the whole of it. The automated rubric
//! lives in `synveda-ingest` beside SKIL-2's scanner; what is here is
//! what a reviewer answers, what a pack configures and what an audit
//! payload names — which is why it sits in the crate everything depends
//! on.
//!
//! # The two halves are never averaged
//!
//! ADR-0053 decision 1. A bundle carries an automated score out of 100
//! *and*, separately, a checklist that is either absent or answered.
//! Summing them would let each hide the other: a well-formatted bundle
//! nobody reviewed would score the same as one a reviewer worked
//! through, and a reviewer's "these instructions are wrong" would become
//! fifteen points rather than the thing it is. A human's judgement
//! averaged into a number is a human's judgement laundered into
//! arithmetic.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// One question on the reviewer's checklist (ADR-0053 decision 6).
///
/// Every item is deliberately something **no machine can answer**. The
/// rubric next door decides whether a bundle's references resolve and
/// whether its manifest is within budget; these decide whether the thing
/// it describes is true, wanted here, and known to work — which is the
/// whole of what a second pair of eyes is for.
///
/// The list is short and is not tenant-configurable in this feature: a
/// checklist a tenant can extend is one whose stored answers stop being
/// comparable across two scopes, and nothing yet needs that.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum ChecklistItem {
    /// The procedure it describes is right for this organisation.
    #[default]
    InstructionsCorrect,
    /// It belongs at this scope, rather than nearer or further up.
    ScopeAppropriate,
    /// It does not repeat a skill already published on this chain.
    NotDuplicate,
    /// The tools and APIs it assumes exist for the people who will be
    /// served it.
    DependenciesAvailable,
    /// Somebody ran it.
    Tested,
}

impl ChecklistItem {
    /// Every item, in the order a reviewer is asked them.
    pub const ALL: [ChecklistItem; 5] = [
        ChecklistItem::InstructionsCorrect,
        ChecklistItem::ScopeAppropriate,
        ChecklistItem::NotDuplicate,
        ChecklistItem::DependenciesAvailable,
        ChecklistItem::Tested,
    ];

    /// Stable wire name, identical to the serde form and to what the CLI
    /// accepts on the command line.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ChecklistItem::InstructionsCorrect => "instructions-correct",
            ChecklistItem::ScopeAppropriate => "scope-appropriate",
            ChecklistItem::NotDuplicate => "not-duplicate",
            ChecklistItem::DependenciesAvailable => "dependencies-available",
            ChecklistItem::Tested => "tested",
        }
    }

    /// The question itself, as a reviewer reads it.
    #[must_use]
    pub const fn prompt(&self) -> &'static str {
        match self {
            ChecklistItem::InstructionsCorrect => {
                "are the instructions correct for this organisation?"
            }
            ChecklistItem::ScopeAppropriate => "does it belong at this scope?",
            ChecklistItem::NotDuplicate => "is it free of a skill already published above it?",
            ChecklistItem::DependenciesAvailable => {
                "do the tools and APIs it assumes exist for the people who will get it?"
            }
            ChecklistItem::Tested => "has somebody run it?",
        }
    }
}

impl fmt::Display for ChecklistItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChecklistItem {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ChecklistItem::ALL
            .into_iter()
            .find(|item| item.as_str() == s)
            .ok_or_else(|| {
                let known: Vec<&str> = ChecklistItem::ALL
                    .iter()
                    .map(ChecklistItem::as_str)
                    .collect();
                Error::Invalid {
                    message: format!("unknown checklist item: {s:?} (expected one of {known:?})"),
                }
            })
    }
}

/// A reviewer's answer to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChecklistVerdict {
    /// Checked, and fine.
    Yes,
    /// Checked, and **not** fine. A concern: a publication over one of
    /// these needs an override (ADR-0053 decision 7), which is what makes
    /// answering the checklist mean something rather than fill a form.
    No,
    /// The question does not apply to this bundle.
    #[serde(rename = "n/a")]
    Na,
}

impl ChecklistVerdict {
    /// Every verdict.
    pub const ALL: [ChecklistVerdict; 3] = [
        ChecklistVerdict::Yes,
        ChecklistVerdict::No,
        ChecklistVerdict::Na,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ChecklistVerdict::Yes => "yes",
            ChecklistVerdict::No => "no",
            ChecklistVerdict::Na => "n/a",
        }
    }
}

impl fmt::Display for ChecklistVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChecklistVerdict {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ChecklistVerdict::ALL
            .into_iter()
            .find(|verdict| verdict.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown checklist verdict: {s:?} (expected yes, no or n/a)"),
            })
    }
}

/// The longest a checklist note may be. Long enough for a paragraph of
/// reasoning, short enough that nobody pastes a transcript into the audit
/// chain.
pub const MAX_CHECKLIST_NOTE_CHARS: usize = 2_000;

/// A completed checklist: one reviewer's answers about one bundle.
///
/// **What it is bound to is the bundle's bytes**, not a proposal and not a
/// skill name (ADR-0053 decision 4) — the digest is the storage key, and
/// an edit therefore produces a bundle for which no checklist exists
/// rather than one carrying answers about content nobody reviewed. That
/// is ADR-0032 decision 6's "approvals bind bytes" applied to the one
/// review artefact that had no address check of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checklist {
    /// Item → verdict. Absent items are unanswered.
    pub answers: BTreeMap<ChecklistItem, ChecklistVerdict>,
    /// Anything the reviewer wanted to say, in prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Checklist {
    /// Every item has a verdict.
    ///
    /// A partially answered checklist does not satisfy a pack that
    /// requires one: "I answered three of five" is not a review, and the
    /// two nobody answered are exactly the two a hurried reviewer skipped.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        ChecklistItem::ALL
            .iter()
            .all(|item| self.answers.contains_key(item))
    }

    /// The items answered [`No`](ChecklistVerdict::No), in item order.
    #[must_use]
    pub fn concerns(&self) -> Vec<ChecklistItem> {
        self.answers
            .iter()
            .filter(|(_, verdict)| **verdict == ChecklistVerdict::No)
            .map(|(item, _)| *item)
            .collect()
    }

    /// Refuses a checklist a reviewer could not have meant.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for an empty answer set or an over-long note.
    pub fn validate(&self) -> crate::Result<()> {
        if self.answers.is_empty() {
            return Err(Error::Invalid {
                message: "a checklist with no answers records nothing; answer at least one item"
                    .to_owned(),
            });
        }
        if let Some(note) = &self.note {
            let chars = note.chars().count();
            if chars > MAX_CHECKLIST_NOTE_CHARS {
                return Err(Error::Invalid {
                    message: format!(
                        "checklist note is {chars} characters; the bound is \
                         {MAX_CHECKLIST_NOTE_CHARS}"
                    ),
                });
            }
        }
        Ok(())
    }
}

/// Why a publication is below the bar (ADR-0053 decision 7).
///
/// Three reasons rather than one boolean, because a refusal that does not
/// say which bar was missed is one the publisher cannot act on: the
/// remedy for a low score is an edit, for a missing checklist a reviewer,
/// and for a concern a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum QualityShortfall {
    /// The automated rubric scored below the pack's threshold.
    BelowThreshold {
        /// What it scored.
        score: u8,
        /// What the pack asks for.
        min_score: u8,
    },
    /// The pack requires a checklist and none is bound to these bytes —
    /// either nobody answered one, or somebody answered one and the
    /// bundle has changed since.
    ChecklistMissing,
    /// A checklist exists but does not answer every item.
    ChecklistIncomplete {
        /// The items still unanswered, in item order.
        unanswered: Vec<ChecklistItem>,
    },
    /// A reviewer answered `no` to something.
    ChecklistConcerns {
        /// Which items, in item order.
        items: Vec<ChecklistItem>,
    },
}

impl QualityShortfall {
    /// One line, as a refusal and an audit payload render it.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            QualityShortfall::BelowThreshold { score, min_score } => {
                format!("the rubric scored {score}/100 and this pack asks for {min_score}")
            }
            QualityShortfall::ChecklistMissing => {
                "no reviewer checklist is recorded for exactly these bytes".to_owned()
            }
            QualityShortfall::ChecklistIncomplete { unanswered } => format!(
                "the checklist leaves {} unanswered",
                unanswered
                    .iter()
                    .map(ChecklistItem::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            QualityShortfall::ChecklistConcerns { items } => format!(
                "a reviewer answered `no` to {}",
                items
                    .iter()
                    .map(ChecklistItem::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// A pack's skill-quality configuration (ADR-0053 decision 9).
///
/// Rides the loaded pack inside the PDP beside
/// [`SkillScanConfig`](crate::SkillScanConfig) and resolves with it, so
/// the bar always comes from exactly the pack that will decide the
/// publication.
///
/// # The fail-safe is no gate, and that is the opposite of its neighbour's
///
/// [`SkillScanConfig`](crate::SkillScanConfig)'s default is the invariant
/// floor, because a pack that says nothing must still not ship a
/// credential stealer. This one's default is **no gate at all**, because
/// quality is not an invariant: a pack that has said nothing about quality
/// has not asked for a quality gate, and a product that began refusing
/// publications on a rubric nobody opted into would be one that broke
/// every tenant on an upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillQualityConfig {
    /// The lowest automated score that publishes without an override.
    /// `0` is no bar — nothing scores below it.
    pub min_score: u8,
    /// Whether a publication needs a checklist bound to exactly the bytes
    /// it is about to publish.
    pub require_checklist: bool,
}

impl SkillQualityConfig {
    /// The strict product config (`regulated-strict`): a real bar and a
    /// mandatory checklist. A bank that ships a skill nobody worked
    /// through is a bank that has not reviewed it.
    pub const STRICT: SkillQualityConfig = SkillQualityConfig {
        min_score: 70,
        require_checklist: true,
    };

    /// The middle product config (`standard`): a bar low enough that an
    /// ordinary well-made bundle clears it, and no mandatory checklist —
    /// an SMB's reviewer is often its author's only colleague.
    pub const MODERATE: SkillQualityConfig = SkillQualityConfig {
        min_score: 50,
        require_checklist: false,
    };

    /// `open-collaboration`, and the fail-safe for an unconfigured stored
    /// pack: no gate. The score still renders everywhere; nothing refuses
    /// on it.
    pub const OPEN: SkillQualityConfig = SkillQualityConfig {
        min_score: 0,
        require_checklist: false,
    };

    /// Whether this config gates anything at all.
    #[must_use]
    pub fn is_open(&self) -> bool {
        *self == SkillQualityConfig::OPEN
    }

    /// Every bar `score` and `checklist` miss, in the order a refusal
    /// names them. Empty means the publication needs no override.
    ///
    /// `checklist` is what is bound to *exactly the bytes being
    /// published* — a checklist answered against an earlier draft is not
    /// this argument's `Some`, it is its `None` (ADR-0053 decision 4).
    #[must_use]
    pub fn shortfalls(&self, score: u8, checklist: Option<&Checklist>) -> Vec<QualityShortfall> {
        let mut missed = Vec::new();
        if score < self.min_score {
            missed.push(QualityShortfall::BelowThreshold {
                score,
                min_score: self.min_score,
            });
        }
        match checklist {
            None => {
                if self.require_checklist {
                    missed.push(QualityShortfall::ChecklistMissing);
                }
            }
            Some(checklist) => {
                if self.require_checklist && !checklist.is_complete() {
                    let unanswered: Vec<ChecklistItem> = ChecklistItem::ALL
                        .into_iter()
                        .filter(|item| !checklist.answers.contains_key(item))
                        .collect();
                    missed.push(QualityShortfall::ChecklistIncomplete { unanswered });
                }
                // A concern refuses under **every** config, configured bar
                // or not: a reviewer who wrote down that the instructions
                // are wrong, followed by a publication nobody remarked on,
                // is the exact failure this feature exists to prevent. A
                // pack may decide whether the checklist is mandatory; it
                // does not get to decide that an answered `no` means
                // nothing.
                let concerns = checklist.concerns();
                if !concerns.is_empty() {
                    missed.push(QualityShortfall::ChecklistConcerns { items: concerns });
                }
            }
        }
        missed
    }
}

impl Default for SkillQualityConfig {
    fn default() -> Self {
        SkillQualityConfig::OPEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered(pairs: &[(ChecklistItem, ChecklistVerdict)]) -> Checklist {
        Checklist {
            answers: pairs.iter().copied().collect(),
            note: None,
        }
    }

    fn all_yes() -> Checklist {
        answered(&ChecklistItem::ALL.map(|item| (item, ChecklistVerdict::Yes)))
    }

    #[test]
    fn wire_names_round_trip() {
        for item in ChecklistItem::ALL {
            assert_eq!(item.as_str().parse::<ChecklistItem>().unwrap(), item);
            assert_eq!(
                serde_json::to_value(item).unwrap(),
                serde_json::json!(item.as_str())
            );
        }
        for verdict in ChecklistVerdict::ALL {
            assert_eq!(
                verdict.as_str().parse::<ChecklistVerdict>().unwrap(),
                verdict
            );
            assert_eq!(
                serde_json::to_value(verdict).unwrap(),
                serde_json::json!(verdict.as_str())
            );
        }
        assert!("shipped-on-a-friday".parse::<ChecklistItem>().is_err());
        assert!("maybe".parse::<ChecklistVerdict>().is_err());
    }

    #[test]
    fn an_unknown_item_names_the_ones_that_exist() {
        // A reviewer mistyping an item on a command line should not have
        // to read the source to find the right spelling.
        let err = "tested-thoroughly".parse::<ChecklistItem>().unwrap_err();
        let message = err.to_string();
        for item in ChecklistItem::ALL {
            assert!(message.contains(item.as_str()), "{message}");
        }
    }

    #[test]
    fn the_open_config_is_the_default_and_gates_nothing() {
        assert_eq!(SkillQualityConfig::default(), SkillQualityConfig::OPEN);
        assert!(SkillQualityConfig::OPEN.is_open());
        // The floor's neighbour refuses on a clean scan's absence; this
        // one refuses on nothing at all, including a bundle that scored
        // zero and was never reviewed.
        assert!(SkillQualityConfig::OPEN.shortfalls(0, None).is_empty());
    }

    #[test]
    fn the_strict_config_wants_a_bar_and_a_complete_checklist() {
        let config = SkillQualityConfig::STRICT;
        assert!(config.shortfalls(85, Some(&all_yes())).is_empty());

        // Below the bar, with the score and the bar both named.
        let low = config.shortfalls(40, Some(&all_yes()));
        assert_eq!(
            low,
            vec![QualityShortfall::BelowThreshold {
                score: 40,
                min_score: 70
            }]
        );

        // No checklist at all.
        assert_eq!(
            config.shortfalls(85, None),
            vec![QualityShortfall::ChecklistMissing]
        );

        // A partial one is not a review: the two nobody answered are the
        // two a hurried reviewer skipped.
        let partial = answered(&[(ChecklistItem::Tested, ChecklistVerdict::Yes)]);
        assert_eq!(
            config.shortfalls(85, Some(&partial)),
            vec![QualityShortfall::ChecklistIncomplete {
                unanswered: vec![
                    ChecklistItem::InstructionsCorrect,
                    ChecklistItem::ScopeAppropriate,
                    ChecklistItem::NotDuplicate,
                    ChecklistItem::DependenciesAvailable,
                ]
            }]
        );
    }

    #[test]
    fn a_concern_refuses_under_every_config_including_the_open_one() {
        // The property that makes answering the checklist mean something:
        // a pack decides whether it is *mandatory*, and no pack decides
        // that a written-down `no` counts for nothing.
        let mut answers = all_yes();
        answers
            .answers
            .insert(ChecklistItem::InstructionsCorrect, ChecklistVerdict::No);
        for config in [
            SkillQualityConfig::STRICT,
            SkillQualityConfig::MODERATE,
            SkillQualityConfig::OPEN,
        ] {
            assert_eq!(
                config.shortfalls(100, Some(&answers)),
                vec![QualityShortfall::ChecklistConcerns {
                    items: vec![ChecklistItem::InstructionsCorrect]
                }],
                "{config:?} let a concern through"
            );
        }
    }

    #[test]
    fn n_a_is_an_answer_and_not_a_concern() {
        let mut answers = all_yes();
        answers
            .answers
            .insert(ChecklistItem::NotDuplicate, ChecklistVerdict::Na);
        assert!(answers.is_complete());
        assert!(answers.concerns().is_empty());
        assert!(
            SkillQualityConfig::STRICT
                .shortfalls(90, Some(&answers))
                .is_empty()
        );
    }

    #[test]
    fn a_bundle_can_miss_more_than_one_bar_at_once() {
        let mut answers = all_yes();
        answers
            .answers
            .insert(ChecklistItem::Tested, ChecklistVerdict::No);
        let missed = SkillQualityConfig::STRICT.shortfalls(30, Some(&answers));
        assert_eq!(missed.len(), 2);
        // Ordered as a refusal names them: what the machine measured
        // first, what a person said second.
        assert!(matches!(missed[0], QualityShortfall::BelowThreshold { .. }));
        assert!(matches!(
            missed[1],
            QualityShortfall::ChecklistConcerns { .. }
        ));
        for shortfall in &missed {
            assert!(!shortfall.describe().is_empty());
        }
    }

    #[test]
    fn the_moderate_config_wants_a_bar_and_no_checklist() {
        let config = SkillQualityConfig::MODERATE;
        assert!(config.shortfalls(50, None).is_empty());
        assert_eq!(
            config.shortfalls(49, None),
            vec![QualityShortfall::BelowThreshold {
                score: 49,
                min_score: 50
            }]
        );
    }

    #[test]
    fn a_checklist_refuses_what_a_reviewer_could_not_have_meant() {
        assert!(all_yes().validate().is_ok());
        assert!(
            Checklist {
                answers: BTreeMap::new(),
                note: None
            }
            .validate()
            .is_err()
        );
        let mut long = all_yes();
        long.note = Some("x".repeat(MAX_CHECKLIST_NOTE_CHARS + 1));
        assert!(long.validate().is_err());
        long.note = Some("x".repeat(MAX_CHECKLIST_NOTE_CHARS));
        assert!(long.validate().is_ok());
    }

    #[test]
    fn unknown_fields_are_refused() {
        assert!(
            serde_json::from_str::<SkillQualityConfig>(
                r#"{"min_score":70,"require_checklist":true}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<SkillQualityConfig>(
                r#"{"min_score":70,"require_checklist":true,"waive":["x"]}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<Checklist>(r#"{"answers":{"tested":"yes"},"extra":1}"#).is_err()
        );
        // The map keys are the wire names, which is what makes a stored
        // row readable by a person and by the next version of this type.
        let parsed: Checklist =
            serde_json::from_str(r#"{"answers":{"tested":"yes","not-duplicate":"n/a"}}"#).unwrap();
        assert_eq!(
            parsed.answers.get(&ChecklistItem::Tested),
            Some(&ChecklistVerdict::Yes)
        );
        assert_eq!(
            parsed.answers.get(&ChecklistItem::NotDuplicate),
            Some(&ChecklistVerdict::Na)
        );
    }
}
