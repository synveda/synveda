//! Auto-promotion vocabulary (FLOW-4, ADR-0033).
//!
//! A promotion rule is a *trigger*, never an authority. Everything it
//! causes is an ordinary FLOW-3 proposal that the ADR-0032 matrix then
//! judges exactly as it judges a human's, which is why these rules ride
//! the policy pack (ADR-0033 decision 6) instead of getting the governed
//! -asset treatment curator files needed: changing who must approve is a
//! change to authority, changing what gets proposed is a change to what
//! lands in a queue.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{AssetKind, Channel, RecordClass, RecordId, Sensitivity};

/// The most rules one pack may carry. A sweep evaluates every rule
/// against every candidate scope, so an unbounded list is an unbounded
/// sweep; a pack needing more than this is describing a workflow, not a
/// threshold.
pub const MAX_PROMOTION_RULES: usize = 32;

/// The longest a rule name may be. Names travel into evidence and into
/// the audit payload, where a reviewer reads them.
pub const MAX_RULE_NAME: usize = 64;

/// A pack's promotion rules (ADR-0033 decision 6).
///
/// Rides the loaded pack beside [`crate::RedactionConfig`],
/// [`crate::CompositionConfig`], and [`crate::ApprovalMatrix`], and
/// resolves at the scope whose channel would move. Unlike the approval
/// matrix — where "unconfigured" still resolves to the invariant floor —
/// an absent config here means *nothing auto-promotes*. Silence is the
/// safe reading for a trigger and the unsafe one for a requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PromotionConfig {
    /// The rules, evaluated independently; each fires its own batch.
    #[serde(default)]
    pub rules: Vec<PromotionRule>,
}

impl PromotionConfig {
    /// The empty config: no rules, nothing auto-promotes. The fail-safe
    /// for a pack that configures nothing and for one whose stored JSON
    /// does not parse.
    pub const EMPTY: PromotionConfig = PromotionConfig { rules: Vec::new() };

    /// Whether any rule could fire under this config.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Refuses a config that cannot do what it says, at install rather
    /// than at sweep time (the ADR-0032 discipline: an unsatisfiable
    /// matrix is refused when it is written, not discovered when it is
    /// needed).
    pub fn validate(&self) -> crate::Result<()> {
        if self.rules.len() > MAX_PROMOTION_RULES {
            return Err(crate::Error::Invalid {
                message: format!(
                    "{} promotion rules exceeds the {MAX_PROMOTION_RULES} a pack may carry",
                    self.rules.len()
                ),
            });
        }
        let mut seen = BTreeSet::new();
        for (index, rule) in self.rules.iter().enumerate() {
            rule.validate(index)?;
            if !seen.insert(rule.name.as_str()) {
                return Err(crate::Error::Invalid {
                    message: format!(
                        "promotion rule {index}: the name {:?} is already used; \
                         a rule's name is how a reviewer tells two batches apart",
                        rule.name
                    ),
                });
            }
        }
        Ok(())
    }
}

/// One rule: a match plus thresholds (ADR-0033 decision 6).
///
/// A rule selects material that is *already at the scope whose channel
/// would move* — FLOW-4 is same-scope (ADR-0033 decision 8) — and its
/// thresholds are read against the usage projection, never against the
/// record's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionRule {
    /// Stable name; appears in every proposal's evidence and in the
    /// audit payload, so renaming one is a visible change.
    pub name: String,
    /// Which asset type. Only [`AssetKind::Memory`] has a usage signal
    /// today; the field exists because the approval matrix already
    /// resolves per asset and PRMT-1/SKIL-1 bring the rest.
    #[serde(default = "memory_asset")]
    pub asset: AssetKind,
    /// Which record classes qualify. Empty means any class.
    #[serde(default)]
    pub classes: Vec<RecordClass>,
    /// The most sensitive material this rule will raise. Queue hygiene,
    /// not authority: anything it proposes still faces the whole matrix,
    /// and `restricted` still takes compliance and two approvers.
    pub max_sensitivity: Sensitivity,
    /// Total recalls across all members, at or above which a record
    /// qualifies.
    pub min_recalls: u32,
    /// Distinct members who recalled it. This is the only knob that
    /// distinguishes personal promotion from shared promotion
    /// (ADR-0033 decision 7): `1` promotes a user's own well-used
    /// memory to their own published channel, which under bank mode is
    /// the difference between a record existing and a record counting.
    pub min_distinct_members: u32,
    /// How long a record must have existed before a rule may raise it.
    /// Zero means no floor.
    #[serde(default)]
    pub min_age_hours: u32,
    /// A record whose last recall is older than this is stale and is not
    /// raised. `None` means recency is not required.
    #[serde(default)]
    pub recency_hours: Option<u32>,
    /// Which channel the proposal targets. `published` is the only
    /// legal value — `derived` is written by the pipeline and needs no
    /// review to reach, and `staged` has no writer at all (ADR-0032
    /// decision 2) — and stating it keeps the rule self-describing for
    /// FLOW-5, which changes a rule's target *scope* and not this.
    #[serde(default = "published_channel")]
    pub target_channel: Channel,
}

fn memory_asset() -> AssetKind {
    AssetKind::Memory
}

fn published_channel() -> Channel {
    Channel::Published
}

impl PromotionRule {
    /// Whether a record of this class and sensitivity is in scope for
    /// this rule. The sensitivity test is a ceiling, so a rule capped at
    /// `internal` never raises `confidential` material.
    #[must_use]
    pub fn matches(&self, class: RecordClass, sensitivity: Sensitivity) -> bool {
        (self.classes.is_empty() || self.classes.contains(&class))
            && sensitivity <= self.max_sensitivity
    }

    /// Whether usage clears every threshold. `age_hours` and
    /// `hours_since_last_recall` are computed by the caller against one
    /// sweep instant, so every rule in a sweep judges the same clock.
    #[must_use]
    pub fn fires(&self, usage: &UsageFacts, age_hours: u32, hours_since_last_recall: u32) -> bool {
        usage.recalls >= u64::from(self.min_recalls)
            && usage.distinct_members >= u64::from(self.min_distinct_members)
            && age_hours >= self.min_age_hours
            && self
                .recency_hours
                .is_none_or(|window| hours_since_last_recall <= window)
    }

    fn validate(&self, index: usize) -> crate::Result<()> {
        if self.name.is_empty() || self.name.len() > MAX_RULE_NAME {
            return Err(crate::Error::Invalid {
                message: format!(
                    "promotion rule {index}: the name must be 1..={MAX_RULE_NAME} characters"
                ),
            });
        }
        if self.asset != AssetKind::Memory {
            return Err(crate::Error::Invalid {
                message: format!(
                    "promotion rule {index}: {} has no usage signal to threshold on — \
                     only memories compose into a context block today, so such a rule \
                     could never fire (SKIL-1 and PRMT-1 bring the rest)",
                    self.asset.as_str()
                ),
            });
        }
        if self.target_channel != Channel::Published {
            return Err(crate::Error::Invalid {
                message: format!(
                    "promotion rule {index}: {} is not a promotion target; \
                     derived is written by the pipeline and staged has no writer",
                    self.target_channel.as_str()
                ),
            });
        }
        if self.min_recalls == 0 {
            return Err(crate::Error::Invalid {
                message: format!(
                    "promotion rule {index}: 0 recalls asks nothing — every record \
                     that ever existed would qualify"
                ),
            });
        }
        if self.min_distinct_members == 0 {
            return Err(crate::Error::Invalid {
                message: format!(
                    "promotion rule {index}: 0 distinct members asks nothing; \
                     1 is the rule that promotes a member's own well-used material"
                ),
            });
        }
        if u64::from(self.min_distinct_members) > u64::from(self.min_recalls) {
            return Err(crate::Error::Invalid {
                message: format!(
                    "promotion rule {index}: {} distinct members cannot each recall \
                     at least once inside {} total recalls",
                    self.min_distinct_members, self.min_recalls
                ),
            });
        }
        if self.recency_hours == Some(0) {
            return Err(crate::Error::Invalid {
                message: format!(
                    "promotion rule {index}: a 0-hour recency window can only be \
                     satisfied by a recall in the same instant as the sweep; omit \
                     the field to require no recency"
                ),
            });
        }
        // Positional rather than a set: `RecordClass` is a six-variant
        // vocabulary, so the quadratic scan is six comparisons and it
        // costs the type no ordering it has no other use for.
        for (position, class) in self.classes.iter().enumerate() {
            if self.classes[..position].contains(class) {
                return Err(crate::Error::Invalid {
                    message: format!(
                        "promotion rule {index}: class {} appears twice",
                        class.as_str()
                    ),
                });
            }
        }
        Ok(())
    }
}

/// What the usage projection says about one record (ADR-0033 decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageFacts {
    /// Total recalls across every member.
    pub recalls: u64,
    /// How many distinct subjects recalled it.
    pub distinct_members: u64,
}

/// Why a rule fired, frozen on the proposal at open time (ADR-0033
/// decision 12).
///
/// The `from_seq`/`to_seq` range is the point (ADR-0033 decision 4): a
/// reviewer who does not believe a count can verify it against
/// hash-chained audit rows rather than trust a table this subsystem
/// wrote, and an auditor can do the same after the projection has been
/// rebuilt twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionEvidence {
    /// The rule that fired.
    pub rule: String,
    /// The pack that carried it, and the version in force when it fired.
    pub pack_name: String,
    /// Pack version, so a rule that has since been edited is still
    /// readable as it stood.
    pub pack_version: i64,
    /// Which audit actions were counted as recalls. Recorded because
    /// the set grows: CTX-5's explicit recall is a stronger signal than
    /// composition, and a proposal opened before it must not read as
    /// though it counted one (ADR-0033 decision 5).
    pub actions: Vec<String>,
    /// The chain range the counts were folded from, inclusive. `from_seq`
    /// is 1 for a projection never rebuilt from a later point.
    pub from_seq: i64,
    /// The last folded seq, inclusive.
    pub to_seq: i64,
    /// Per member, what the projection said when the rule fired.
    pub members: Vec<MemberEvidence>,
}

impl PromotionEvidence {
    /// The one-line summary a commit message carries (ADR-0033 decision
    /// 12): the human rendering of the same fact the structure holds.
    #[must_use]
    pub fn summary(&self) -> String {
        let records = self.members.len();
        let recalls: u64 = self.members.iter().map(|member| member.recalls).sum();
        let members = self
            .members
            .iter()
            .map(|member| member.distinct_members)
            .max()
            .unwrap_or(0);
        format!(
            "auto-promotion ({}): {records} record{}, {recalls} recall{} by up to \
             {members} member{}",
            self.rule,
            plural(records as u64),
            plural(recalls),
            plural(members),
        )
    }
}

/// What the projection said about one member of the batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberEvidence {
    /// The record raised.
    pub record_id: RecordId,
    /// Total recalls across every member.
    pub recalls: u64,
    /// Distinct subjects who recalled it.
    pub distinct_members: u64,
    /// When it was first recalled.
    pub first_recall_at: DateTime<Utc>,
    /// When it was last recalled.
    pub last_recall_at: DateTime<Utc>,
}

fn plural(count: u64) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str) -> PromotionRule {
        PromotionRule {
            name: name.to_owned(),
            asset: AssetKind::Memory,
            classes: vec![RecordClass::Procedure],
            max_sensitivity: Sensitivity::Internal,
            min_recalls: 5,
            min_distinct_members: 3,
            min_age_hours: 24,
            recency_hours: Some(720),
            target_channel: Channel::Published,
        }
    }

    fn usage(recalls: u64, distinct_members: u64) -> UsageFacts {
        UsageFacts {
            recalls,
            distinct_members,
        }
    }

    #[test]
    fn empty_config_promotes_nothing() {
        assert!(PromotionConfig::EMPTY.is_empty());
        PromotionConfig::EMPTY.validate().expect("empty is valid");
    }

    #[test]
    fn class_filter_is_a_whitelist_and_empty_means_any() {
        let rule = rule("procedures");
        assert!(rule.matches(RecordClass::Procedure, Sensitivity::Internal));
        assert!(!rule.matches(RecordClass::Episode, Sensitivity::Internal));

        let any = PromotionRule {
            classes: Vec::new(),
            ..rule
        };
        for class in RecordClass::ALL {
            assert!(any.matches(class, Sensitivity::Internal), "{class:?}");
        }
    }

    #[test]
    fn sensitivity_is_a_ceiling_not_a_match() {
        let rule = rule("procedures");
        assert!(rule.matches(RecordClass::Procedure, Sensitivity::Public));
        assert!(rule.matches(RecordClass::Procedure, Sensitivity::Internal));
        assert!(!rule.matches(RecordClass::Procedure, Sensitivity::Confidential));
        assert!(!rule.matches(RecordClass::Procedure, Sensitivity::Restricted));
    }

    #[test]
    fn every_threshold_is_load_bearing() {
        let rule = rule("procedures");
        assert!(rule.fires(&usage(5, 3), 24, 1));
        // One short on each axis in turn.
        assert!(!rule.fires(&usage(4, 3), 24, 1), "recalls");
        assert!(!rule.fires(&usage(5, 2), 24, 1), "distinct members");
        assert!(!rule.fires(&usage(5, 3), 23, 1), "age");
        assert!(!rule.fires(&usage(5, 3), 24, 721), "recency");
    }

    #[test]
    fn absent_recency_window_never_stales() {
        let rule = PromotionRule {
            recency_hours: None,
            ..rule("procedures")
        };
        assert!(rule.fires(&usage(5, 3), 24, u32::MAX));
    }

    #[test]
    fn a_rule_asking_nothing_is_refused() {
        for (field, broken) in [
            (
                "recalls",
                PromotionRule {
                    min_recalls: 0,
                    ..rule("r")
                },
            ),
            (
                "members",
                PromotionRule {
                    min_distinct_members: 0,
                    ..rule("r")
                },
            ),
        ] {
            let config = PromotionConfig {
                rules: vec![broken],
            };
            assert!(config.validate().is_err(), "{field} = 0 must be refused");
        }
    }

    #[test]
    fn members_cannot_outnumber_the_recalls_they_would_each_make() {
        let config = PromotionConfig {
            rules: vec![PromotionRule {
                min_recalls: 2,
                min_distinct_members: 3,
                ..rule("r")
            }],
        };
        let message = config.validate().expect_err("unsatisfiable").to_string();
        assert!(message.contains("distinct members"), "{message}");
    }

    #[test]
    fn a_rule_that_could_never_fire_is_refused_at_install() {
        // No usage signal exists for a non-memory asset, so the rule is
        // not merely unusual — it can never fire.
        let config = PromotionConfig {
            rules: vec![PromotionRule {
                asset: AssetKind::Skill,
                ..rule("skills")
            }],
        };
        let message = config.validate().expect_err("no signal").to_string();
        assert!(message.contains("usage signal"), "{message}");

        // Neither `derived` nor `staged` is a promotion target.
        for channel in [Channel::Derived, Channel::Staged] {
            let config = PromotionConfig {
                rules: vec![PromotionRule {
                    target_channel: channel,
                    ..rule("r")
                }],
            };
            assert!(config.validate().is_err(), "{channel:?}");
        }
    }

    #[test]
    fn a_zero_hour_recency_window_is_refused_but_an_absent_one_is_not() {
        let config = PromotionConfig {
            rules: vec![PromotionRule {
                recency_hours: Some(0),
                ..rule("r")
            }],
        };
        assert!(config.validate().is_err());

        let config = PromotionConfig {
            rules: vec![PromotionRule {
                recency_hours: None,
                ..rule("r")
            }],
        };
        config.validate().expect("no recency requirement is valid");
    }

    #[test]
    fn duplicate_rule_names_and_classes_are_refused() {
        let config = PromotionConfig {
            rules: vec![rule("same"), rule("same")],
        };
        let message = config.validate().expect_err("duplicate name").to_string();
        assert!(message.contains("already used"), "{message}");

        let config = PromotionConfig {
            rules: vec![PromotionRule {
                classes: vec![RecordClass::Procedure, RecordClass::Procedure],
                ..rule("r")
            }],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn too_many_rules_is_a_sweep_that_never_ends() {
        let config = PromotionConfig {
            rules: (0..=MAX_PROMOTION_RULES)
                .map(|index| rule(&format!("rule-{index}")))
                .collect(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn unknown_fields_are_refused() {
        let json = r#"{"rules":[{"name":"r","max_sensitivity":"internal",
                       "min_recalls":5,"min_distinct_members":3,"tomorrow":true}]}"#;
        let err = serde_json::from_str::<PromotionConfig>(json).expect_err("unknown field");
        assert!(err.to_string().contains("tomorrow"), "{err}");
    }

    #[test]
    fn defaults_fill_the_optional_fields() {
        let json = r#"{"rules":[{"name":"r","max_sensitivity":"internal",
                       "min_recalls":5,"min_distinct_members":3}]}"#;
        let config: PromotionConfig = serde_json::from_str(json).expect("parse");
        let rule = &config.rules[0];
        assert_eq!(rule.asset, AssetKind::Memory);
        assert_eq!(rule.target_channel, Channel::Published);
        assert_eq!(rule.min_age_hours, 0);
        assert_eq!(rule.recency_hours, None);
        assert!(rule.classes.is_empty());
        config.validate().expect("defaults are valid");
    }

    #[test]
    fn evidence_summary_reads_as_a_commit_message() {
        let now = Utc::now();
        let evidence = PromotionEvidence {
            rule: "well-used-procedures".to_owned(),
            pack_name: "standard".to_owned(),
            pack_version: 4,
            actions: vec!["context.injected".to_owned()],
            from_seq: 1,
            to_seq: 900,
            members: vec![
                MemberEvidence {
                    record_id: RecordId::new(),
                    recalls: 7,
                    distinct_members: 3,
                    first_recall_at: now,
                    last_recall_at: now,
                },
                MemberEvidence {
                    record_id: RecordId::new(),
                    recalls: 5,
                    distinct_members: 4,
                    first_recall_at: now,
                    last_recall_at: now,
                },
            ],
        };
        assert_eq!(
            evidence.summary(),
            "auto-promotion (well-used-procedures): 2 records, 12 recalls by up to 4 members"
        );
    }

    #[test]
    fn evidence_summary_counts_one_in_the_singular() {
        let now = Utc::now();
        let evidence = PromotionEvidence {
            rule: "r".to_owned(),
            pack_name: "standard".to_owned(),
            pack_version: 1,
            actions: vec!["context.injected".to_owned()],
            from_seq: 1,
            to_seq: 2,
            members: vec![MemberEvidence {
                record_id: RecordId::new(),
                recalls: 1,
                distinct_members: 1,
                first_recall_at: now,
                last_recall_at: now,
            }],
        };
        assert_eq!(
            evidence.summary(),
            "auto-promotion (r): 1 record, 1 recall by up to 1 member"
        );
    }

    #[test]
    fn evidence_round_trips_through_json() {
        let now = Utc::now();
        let evidence = PromotionEvidence {
            rule: "r".to_owned(),
            pack_name: "standard".to_owned(),
            pack_version: 2,
            actions: vec!["context.injected".to_owned()],
            from_seq: 1,
            to_seq: 42,
            members: vec![MemberEvidence {
                record_id: RecordId::new(),
                recalls: 9,
                distinct_members: 3,
                first_recall_at: now,
                last_recall_at: now,
            }],
        };
        let json = serde_json::to_value(&evidence).expect("serialise");
        let parsed: PromotionEvidence = serde_json::from_value(json).expect("parse");
        assert_eq!(parsed, evidence);
    }
}
