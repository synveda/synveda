//! Lapse vocabulary (seed §6, AUTHZ-4, ADR-0037).
//!
//! A lapse is a **time-boxed, dual-approved, reasoned grant of one action
//! from one scope to another** — seed §6's "allow team X to read team Y's
//! records for 30 days — reason: joint incident review", and the mechanism
//! it names as the thing that lets one product serve both an SMB and a
//! bank.
//!
//! # What this module decides and what it does not
//!
//! It describes terms and bounds them. It never authorises: whether a
//! standing grant reaches a decision is [`crate::PackConfig`]'s ceiling and
//! a Cedar permit conditioned on the PDP's own resolution (ADR-0037
//! decisions 7 and 9). And it holds no clock of its own — [`Lapse`] is
//! asked about an instant the caller supplies, so the one query that reads
//! these rows is the one place expiry is decided (decision 4).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, IdentityId, LapseId, ProposalId, ScopeId, Sensitivity, TenantId};

/// The longest a lapse's reason may be. The reason is mandatory, travels
/// into the audit chain, and is read by two approvers and later by an
/// auditor; a paragraph is a reason, a document is an attachment.
pub const MAX_LAPSE_REASON: usize = 512;

/// The longest window any pack may grant, whatever it configures: 90 days.
///
/// The product maximum, not a default. A pack asking for more is refused at
/// install *and* clamped at resolution — the [`crate::ApprovalMatrix`]
/// floor's discipline, where an invariant is applied rather than trusted to
/// have been checked (ADR-0032 decision 4).
pub const PRODUCT_MAX_DURATION_SECS: u32 = 90 * 24 * 60 * 60;

/// The window `regulated-strict` grants: 30 days, seed §6's own example.
pub const STRICT_MAX_DURATION_SECS: u32 = 30 * 24 * 60 * 60;

/// What a lapse may relax.
///
/// A closed vocabulary, and deliberately a small one. Every variant must be
/// an action whose seam a standing grant can widen *and* whose widening is
/// something two stewards can reason about in advance; widening the admin
/// plane on a timer is a different product, and the CLI's break-glass
/// already covers the case it would serve (ADR-0037 decision 2).
///
/// There is no `Default`: what a lapse relaxes is the whole question it
/// exists to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LapseAction {
    /// Compose memories attached to the target scope — the seam seed §6's
    /// example names, and the only one with a standing grant today.
    ///
    /// The wire name is `synveda_policy::Action::MemoryRead`'s, because the
    /// two are the same action seen from two crates and a trail that
    /// spelled them differently would make an auditor reconcile them.
    #[serde(rename = "memory.read")]
    MemoryRead,
}

impl LapseAction {
    /// Every lapsable action.
    pub const ALL: [LapseAction; 1] = [LapseAction::MemoryRead];

    /// Stable wire name, identical to the serde form, to the stored column,
    /// and to the policy crate's name for the same action.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            LapseAction::MemoryRead => "memory.read",
        }
    }
}

impl fmt::Display for LapseAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LapseAction {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        LapseAction::ALL
            .into_iter()
            .find(|action| action.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "{s:?} is not an action a lapse may relax; a lapse grants one of [{}], \
                     and widening any other plane on a timer is not what this mechanism is",
                    LapseAction::ALL
                        .iter()
                        .map(LapseAction::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

/// A pack's lapse configuration (ADR-0037 decision 5).
///
/// Rides the loaded pack beside [`crate::RedactionConfig`],
/// [`crate::CompositionConfig`], [`crate::ApprovalMatrix`], and
/// [`crate::PromotionConfig`], and resolves at the scope being opened.
///
/// Like [`crate::CompositionConfig`], it **narrows and never grants**: the
/// ceiling can only shorten a window a matrix already approved, and zero
/// admits no lapse at all. That is what makes "this tenant does not do
/// lapses" a configuration rather than a Cedar exercise, and why `Default`
/// (the strict window) is also the fail-safe for a pack configuring
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LapseConfig {
    /// The longest window a lapse at scopes this pack governs may run for.
    /// Zero admits no lapse at all.
    pub max_duration_secs: u32,
}

impl LapseConfig {
    /// `regulated-strict`'s configuration: 30 days, seed §6's own example.
    ///
    /// Also the fail-safe for a stored pack that configures nothing. The
    /// strictest *functioning* value rather than zero: seed §2.1's strict
    /// default is about narrowing access, and a pack that silently lost the
    /// mechanism would be a feature going missing, not a control holding.
    pub const STRICT: LapseConfig = LapseConfig {
        max_duration_secs: STRICT_MAX_DURATION_SECS,
    };

    /// The relaxed packs' configuration: the product maximum.
    pub const RELAXED: LapseConfig = LapseConfig {
        max_duration_secs: PRODUCT_MAX_DURATION_SECS,
    };

    /// No lapse at any window — the config a pack sets to refuse the
    /// mechanism outright.
    pub const NONE: LapseConfig = LapseConfig {
        max_duration_secs: 0,
    };

    /// The window actually available, product maximum applied.
    ///
    /// Clamped rather than trusted: [`LapseConfig::validate`] refuses an
    /// over-long pack at install, and this makes the bound hold anyway for
    /// a row that reached the engine by some other road.
    #[must_use]
    pub const fn resolved_max_secs(&self) -> u32 {
        if self.max_duration_secs > PRODUCT_MAX_DURATION_SECS {
            PRODUCT_MAX_DURATION_SECS
        } else {
            self.max_duration_secs
        }
    }

    /// Whether any lapse at all may stand under this pack.
    ///
    /// Asked at grant time *and* at decision time (ADR-0037 decision 5): a
    /// pack that admits no lapses admits none on the very next request,
    /// standing grants included.
    #[must_use]
    pub const fn admits_lapses(&self) -> bool {
        self.resolved_max_secs() > 0
    }

    /// Refuses a ceiling above the product maximum, at install rather than
    /// at grant time.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming both windows.
    pub fn validate(&self) -> crate::Result<()> {
        if self.max_duration_secs > PRODUCT_MAX_DURATION_SECS {
            return Err(Error::Invalid {
                message: format!(
                    "lapse ceiling of {} seconds exceeds the product maximum of {} \
                     ({} days); a pack may shorten the window, never lengthen it",
                    self.max_duration_secs,
                    PRODUCT_MAX_DURATION_SECS,
                    PRODUCT_MAX_DURATION_SECS / 86_400
                ),
            });
        }
        Ok(())
    }
}

impl Default for LapseConfig {
    fn default() -> Self {
        LapseConfig::STRICT
    }
}

/// What a lapse asks for: the whole of it (ADR-0037 decision 2).
///
/// These are the bytes of the `AssetKind::Policy` object the proposal's
/// commit names, so they are what the approvals bind. There is deliberately
/// no record-*type* qualifier: the seam a lapse widens decides once per
/// scope with no record in hand, and a stored narrowing that nothing
/// applies is a widening wearing a narrowing's name (decision 6).
///
/// [`LapseTerms::max_sensitivity`] is the exception AUTHZ-5 earned, and it
/// is an exception for a reason rather than a change of mind: a tier is a
/// closed, ordered vocabulary, so the decision seam can be asked about one
/// without holding a record (ADR-0038 decision 1). A class cannot, and
/// stays refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LapseTerms {
    /// Who gets the access: every principal placed at or under this scope.
    /// A single person is expressible as their own personal scope, so this
    /// one shape covers "team X" and "just Dana".
    pub grantee_scope_id: ScopeId,
    /// Whose material is disclosed. Requirements resolve here, and this is
    /// the scope whose published channel the grantee composes.
    pub target_scope_id: ScopeId,
    /// What is relaxed.
    pub action: LapseAction,
    /// The most sensitive material this grant discloses (AUTHZ-5, ADR-0038
    /// decision 6).
    ///
    /// **The approval matrix resolves at this tier**, so declaring
    /// `restricted` is what pulls in the invariant floor — the `compliance`
    /// role and two distinct approvers, under every pack, unauthorable away
    /// (ADR-0032 decision 4). Nobody wrote a rule for that; declaring what
    /// you are disclosing is the rule.
    ///
    /// Defaults to [`Sensitivity::WORKING`] on the wire, which is what every
    /// grant written before this field existed means.
    #[serde(default = "working_tier")]
    pub max_sensitivity: Sensitivity,
    /// How long the grant runs once its effect executes. Seconds, and with
    /// no minimum — which is what lets an acceptance test observe a real
    /// expiry rather than a clock it controls (ADR-0037 decision 4).
    pub duration_secs: u32,
    /// Why. Mandatory, by the feature's own text and seed §6's example.
    pub reason: String,
}

/// The tier a lapse means when it says nothing — see
/// [`LapseTerms::max_sensitivity`].
fn working_tier() -> Sensitivity {
    Sensitivity::WORKING
}

impl LapseTerms {
    /// Whether these terms mean anything and fit inside `config`.
    ///
    /// Structural only. Two refusals that need the scope tree — a
    /// `principal`-shaped target, and a target already on the grantee's own
    /// chain — are the gateway's, because this crate is the root of the graph
    /// and knows no scopes.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming what is wrong and, where there is one, the
    /// bound that was exceeded.
    pub fn validate(&self, config: &LapseConfig) -> crate::Result<()> {
        let reason = self.reason.trim();
        if reason.is_empty() {
            return Err(Error::Invalid {
                message: "a lapse's reason is mandatory: it is what two approvers weigh \
                          and what an auditor reads afterwards"
                    .to_owned(),
            });
        }
        if reason.len() > MAX_LAPSE_REASON {
            return Err(Error::Invalid {
                message: format!(
                    "a lapse's reason is at most {MAX_LAPSE_REASON} characters, not {}",
                    reason.len()
                ),
            });
        }
        if self.grantee_scope_id == self.target_scope_id {
            return Err(Error::Invalid {
                message: "a lapse from a scope to itself grants nothing: its members \
                          already compose it through their own chain"
                    .to_owned(),
            });
        }
        if self.duration_secs == 0 {
            return Err(Error::Invalid {
                message: "a lapse of zero seconds expires before it is granted; \
                          state the window the reason justifies"
                    .to_owned(),
            });
        }
        let max = config.resolved_max_secs();
        if max == 0 {
            return Err(Error::Invalid {
                message: "the pack in force at the target scope admits no lapses \
                          (its lapse ceiling is zero)"
                    .to_owned(),
            });
        }
        if self.duration_secs > max {
            return Err(Error::Invalid {
                message: format!(
                    "a {}-second lapse exceeds the {max}-second ceiling of the pack \
                     in force at the target scope",
                    self.duration_secs
                ),
            });
        }
        Ok(())
    }

    /// The one-line summary the proposal title and the commit message both
    /// carry — the human rendering of the same fact the structure holds.
    ///
    /// The tier is named whenever it is above the working one, because that
    /// is the half of the terms that decides what the matrix asks for: a
    /// reviewer reading a queue should see "up to restricted" before they
    /// open anything.
    #[must_use]
    pub fn summary(&self) -> String {
        let tier = if self.max_sensitivity > Sensitivity::WORKING {
            format!(" up to {}", self.max_sensitivity)
        } else {
            String::new()
        };
        format!(
            "lapse: {} may {} at {}{tier} for {}",
            self.grantee_scope_id,
            self.action,
            self.target_scope_id,
            humanise_secs(self.duration_secs)
        )
    }
}

/// A standing grant: the projection a granted lapse proposal writes, and
/// the row every authorization context reads (ADR-0037 decision 16).
///
/// Typed columns rather than the object's bytes because the read path reads
/// it per request, and parsing an object per decision is not a read path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lapse {
    /// This grant.
    pub id: LapseId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The proposal whose effect created it — where the approvals, the
    /// requirement as resolved, and the reviewed object all live.
    pub proposal_id: ProposalId,
    /// Who gets the access.
    pub grantee_scope_id: ScopeId,
    /// Whose material is disclosed.
    pub target_scope_id: ScopeId,
    /// What is relaxed.
    pub action: LapseAction,
    /// The most sensitive material this grant discloses, carried from the
    /// reviewed terms — and the tier its approval matrix resolved at
    /// (AUTHZ-5, ADR-0038 decision 6).
    pub max_sensitivity: Sensitivity,
    /// Why, carried from the reviewed terms.
    pub reason: String,
    /// When the effect ran — the instant the window starts, never the
    /// instant the proposal opened.
    pub granted_at: DateTime<Utc>,
    /// When the grant ends by itself.
    pub expires_at: DateTime<Utc>,
    /// The identity that ran the effect.
    pub granted_by: IdentityId,
    /// When it was ended early, if it was.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Who ended it early.
    pub revoked_by: Option<IdentityId>,
    /// Why it was ended early — mandatory when it was.
    pub revoke_reason: Option<String>,
    /// When the sweep chained this grant's expiry event. Bookkeeping only:
    /// nothing consults it to decide access (ADR-0037 decision 4).
    pub expiry_recorded_at: Option<DateTime<Utc>>,
}

impl Lapse {
    /// Whether this grant stands at `now`.
    ///
    /// The instant is supplied rather than read, which is what keeps expiry
    /// a property of the decision: the query that loads these rows applies
    /// the same predicate, and this is the in-process restatement of it.
    #[must_use]
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && now < self.expires_at
    }

    /// Whether this grant covers `action` at `resource` — the target scope
    /// itself, never its subtree (ADR-0037 decision 8).
    ///
    /// Scope-level only: this is the containment question the read path's
    /// *plan* asks, where no tier is in hand yet. A decision asks
    /// [`Lapse::grants_at`], which is this plus the tier.
    #[must_use]
    pub fn grants(&self, action: LapseAction, resource: ScopeId) -> bool {
        self.action == action && self.target_scope_id == resource
    }

    /// Whether this grant covers `action` at `resource` for material at
    /// `sensitivity` (AUTHZ-5, ADR-0038 decision 6).
    ///
    /// The declared ceiling is what two approvers consented to, so it bounds
    /// what the grant reaches: a grant written for the working tier does not
    /// quietly become a door to `restricted` because somebody asked.
    #[must_use]
    pub fn grants_at(
        &self,
        action: LapseAction,
        resource: ScopeId,
        sensitivity: Sensitivity,
    ) -> bool {
        self.grants(action, resource) && sensitivity <= self.max_sensitivity
    }

    /// How the grant ended, for an audit payload and a listing.
    #[must_use]
    pub fn outcome_at(&self, now: DateTime<Utc>) -> LapseOutcome {
        if self.revoked_at.is_some() {
            LapseOutcome::Revoked
        } else if now < self.expires_at {
            LapseOutcome::Active
        } else {
            LapseOutcome::Expired
        }
    }
}

/// How a grant stands, rendered from the row rather than stored on it —
/// the [`crate::ProposalView`] discipline, for the same reason: a stored
/// state would need something to run to stay true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LapseOutcome {
    /// Standing.
    Active,
    /// Ended by reaching `expires_at`.
    Expired,
    /// Ended early by a revocation.
    Revoked,
}

impl LapseOutcome {
    /// Every outcome.
    pub const ALL: [LapseOutcome; 3] = [
        LapseOutcome::Active,
        LapseOutcome::Expired,
        LapseOutcome::Revoked,
    ];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            LapseOutcome::Active => "active",
            LapseOutcome::Expired => "expired",
            LapseOutcome::Revoked => "revoked",
        }
    }
}

impl fmt::Display for LapseOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Renders a window the way a reviewer reads one. Whole days above a day,
/// whole hours above an hour, seconds below — a 30-day lapse and a 2-second
/// acceptance-test lapse both have to read naturally.
fn humanise_secs(secs: u32) -> String {
    let plural = |count: u32, unit: &str| {
        if count == 1 {
            format!("{count} {unit}")
        } else {
            format!("{count} {unit}s")
        }
    };
    match secs {
        secs if secs >= 86_400 && secs % 86_400 == 0 => plural(secs / 86_400, "day"),
        secs if secs >= 3_600 && secs % 3_600 == 0 => plural(secs / 3_600, "hour"),
        secs if secs >= 60 && secs % 60 == 0 => plural(secs / 60, "minute"),
        secs => plural(secs, "second"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
    use uuid::Uuid;

    fn scope(byte: u8) -> ScopeId {
        ScopeId::from_uuid(Uuid::from_bytes([byte; 16]))
    }

    fn terms() -> LapseTerms {
        LapseTerms {
            grantee_scope_id: scope(1),
            target_scope_id: scope(2),
            action: LapseAction::MemoryRead,
            max_sensitivity: Sensitivity::Internal,
            duration_secs: 3_600,
            reason: "joint incident review".to_owned(),
        }
    }

    fn lapse(expires_at: DateTime<Utc>) -> Lapse {
        Lapse {
            id: LapseId::new(),
            tenant_id: TenantId::new(),
            proposal_id: ProposalId::new(),
            grantee_scope_id: scope(1),
            target_scope_id: scope(2),
            action: LapseAction::MemoryRead,
            max_sensitivity: Sensitivity::Internal,
            reason: "joint incident review".to_owned(),
            granted_at: expires_at - TimeDelta::seconds(3_600),
            expires_at,
            granted_by: IdentityId::new(),
            revoked_at: None,
            revoked_by: None,
            revoke_reason: None,
            expiry_recorded_at: None,
        }
    }

    #[test]
    fn wire_names_round_trip_and_match_the_policy_crate() {
        for action in LapseAction::ALL {
            assert_eq!(action.to_string().parse::<LapseAction>().unwrap(), action);
            assert_eq!(
                serde_json::to_string(&action).unwrap(),
                format!("\"{}\"", action.as_str())
            );
        }
        // The one action's name is the PDP's name for it; a rename on
        // either side has to be a deliberate change on both.
        assert_eq!(LapseAction::MemoryRead.as_str(), "memory.read");
        for outcome in LapseOutcome::ALL {
            assert_eq!(
                serde_json::to_string(&outcome).unwrap(),
                format!("\"{}\"", outcome.as_str())
            );
        }
    }

    /// The vocabulary is closed by name, and the refusal says what the
    /// legal set is rather than leaving someone to read an enum.
    #[test]
    fn an_action_outside_the_vocabulary_is_refused_by_name() {
        let err = "memory.write".parse::<LapseAction>().expect_err("closed");
        let message = err.to_string();
        assert!(message.contains("memory.write"), "{message}");
        assert!(message.contains("memory.read"), "{message}");
        assert!("policy.assign".parse::<LapseAction>().is_err());
        assert!("role.assign".parse::<LapseAction>().is_err());
    }

    #[test]
    fn the_product_maximum_is_clamped_as_well_as_refused() {
        let over = LapseConfig {
            max_duration_secs: PRODUCT_MAX_DURATION_SECS + 1,
        };
        let message = over.validate().expect_err("over the maximum").to_string();
        assert!(message.contains("product maximum"), "{message}");
        assert_eq!(
            over.resolved_max_secs(),
            PRODUCT_MAX_DURATION_SECS,
            "a pack that reached the engine anyway is still bounded"
        );
    }

    /// Zero is a real answer: the pack refuses the mechanism outright.
    #[test]
    fn a_zero_ceiling_admits_nothing_and_is_valid_configuration() {
        LapseConfig::NONE
            .validate()
            .expect("zero is legal to write");
        assert!(!LapseConfig::NONE.admits_lapses());
        assert!(LapseConfig::STRICT.admits_lapses());
        assert!(LapseConfig::RELAXED.admits_lapses());

        let message = terms()
            .validate(&LapseConfig::NONE)
            .expect_err("no lapses here")
            .to_string();
        assert!(message.contains("admits no lapses"), "{message}");
    }

    #[test]
    fn the_fail_safe_for_an_unconfigured_pack_is_the_strict_window() {
        assert_eq!(LapseConfig::default(), LapseConfig::STRICT);
        assert_eq!(LapseConfig::STRICT.max_duration_secs, 30 * 86_400);
        assert_eq!(LapseConfig::RELAXED.max_duration_secs, 90 * 86_400);
    }

    #[test]
    fn a_window_over_the_packs_ceiling_is_refused_naming_both() {
        let long = LapseTerms {
            duration_secs: STRICT_MAX_DURATION_SECS + 1,
            ..terms()
        };
        let message = long
            .validate(&LapseConfig::STRICT)
            .expect_err("over the ceiling")
            .to_string();
        assert!(message.contains("ceiling"), "{message}");
        // The same window is fine under a pack that allows it.
        long.validate(&LapseConfig::RELAXED).expect("under 90 days");
    }

    #[test]
    fn a_reason_is_mandatory_and_whitespace_is_not_one() {
        for blank in ["", "   ", "\n\t "] {
            let message = LapseTerms {
                reason: blank.to_owned(),
                ..terms()
            }
            .validate(&LapseConfig::STRICT)
            .expect_err("blank reason")
            .to_string();
            assert!(message.contains("mandatory"), "{message}");
        }
        let long = LapseTerms {
            reason: "x".repeat(MAX_LAPSE_REASON + 1),
            ..terms()
        };
        assert!(long.validate(&LapseConfig::STRICT).is_err());
    }

    #[test]
    fn a_lapse_that_asks_nothing_is_refused() {
        let self_grant = LapseTerms {
            grantee_scope_id: scope(2),
            ..terms()
        };
        let message = self_grant
            .validate(&LapseConfig::STRICT)
            .expect_err("same scope")
            .to_string();
        assert!(message.contains("grants nothing"), "{message}");

        let instant = LapseTerms {
            duration_secs: 0,
            ..terms()
        };
        assert!(instant.validate(&LapseConfig::STRICT).is_err());
    }

    /// There is no minimum, deliberately: the acceptance test observes a
    /// real expiry rather than a simulated one.
    #[test]
    fn a_two_second_lapse_is_legal() {
        let brief = LapseTerms {
            duration_secs: 2,
            ..terms()
        };
        brief
            .validate(&LapseConfig::STRICT)
            .expect("no minimum window");
    }

    #[test]
    fn activity_is_asked_about_an_instant_never_read_from_a_clock() {
        let now = Utc::now();
        let standing = lapse(now + TimeDelta::seconds(60));
        assert!(standing.is_active_at(now));
        assert!(!standing.is_active_at(now + TimeDelta::seconds(61)));
        // The boundary belongs to the past: at `expires_at` it is over.
        assert!(!standing.is_active_at(standing.expires_at));

        let revoked = Lapse {
            revoked_at: Some(now),
            ..lapse(now + TimeDelta::seconds(60))
        };
        assert!(!revoked.is_active_at(now), "revocation beats the window");
    }

    /// The grant covers the target scope itself and nothing else — the
    /// subtree reaches the reader through what the target published.
    #[test]
    fn a_grant_covers_its_target_and_not_its_neighbours() {
        let now = Utc::now();
        let grant = lapse(now + TimeDelta::seconds(60));
        assert!(grant.grants(LapseAction::MemoryRead, scope(2)));
        assert!(!grant.grants(LapseAction::MemoryRead, scope(3)));
        assert!(
            !grant.grants(LapseAction::MemoryRead, scope(1)),
            "the grantee is not a target"
        );
    }

    #[test]
    fn the_outcome_is_rendered_from_the_row_not_stored_on_it() {
        let now = Utc::now();
        let standing = lapse(now + TimeDelta::seconds(60));
        assert_eq!(standing.outcome_at(now), LapseOutcome::Active);
        assert_eq!(
            standing.outcome_at(now + TimeDelta::seconds(61)),
            LapseOutcome::Expired
        );
        let revoked = Lapse {
            revoked_at: Some(now),
            ..standing
        };
        assert_eq!(revoked.outcome_at(now), LapseOutcome::Revoked);
    }

    #[test]
    fn a_window_reads_naturally_at_both_ends_of_the_range() {
        assert_eq!(humanise_secs(30 * 86_400), "30 days");
        assert_eq!(humanise_secs(86_400), "1 day");
        assert_eq!(humanise_secs(7_200), "2 hours");
        assert_eq!(humanise_secs(60), "1 minute");
        assert_eq!(humanise_secs(2), "2 seconds");
        assert_eq!(humanise_secs(1), "1 second");
        // Not a whole unit: falls to the next one down rather than lying.
        assert_eq!(humanise_secs(90), "90 seconds");
    }

    #[test]
    fn the_summary_names_both_scopes_the_action_and_the_window() {
        let summary = terms().summary();
        assert!(summary.contains(&scope(1).to_string()), "{summary}");
        assert!(summary.contains(&scope(2).to_string()), "{summary}");
        assert!(summary.contains("memory.read"), "{summary}");
        assert!(summary.contains("1 hour"), "{summary}");
    }

    #[test]
    fn terms_round_trip_and_refuse_unknown_fields() {
        let json = serde_json::to_string(&terms()).unwrap();
        assert_eq!(serde_json::from_str::<LapseTerms>(&json).unwrap(), terms());
        // A qualifier this feature deliberately does not enforce must not
        // parse into silence (ADR-0037 decision 6; ADR-0038 decision 17
        // keeps class refused while sensitivity became enforceable).
        let with_qualifier = format!(
            r#"{{"grantee_scope_id":"{}","target_scope_id":"{}","action":"memory.read",
                "duration_secs":60,"reason":"r","classes":["procedure"]}}"#,
            scope(1),
            scope(2)
        );
        let err = serde_json::from_str::<LapseTerms>(&with_qualifier).expect_err("unknown field");
        assert!(err.to_string().contains("classes"), "{err}");
    }

    /// Terms written before the ceiling existed mean the working tier —
    /// which is what they granted, since the read path composed nothing
    /// above `internal` (ADR-0038 decision 6).
    #[test]
    fn terms_without_a_declared_tier_mean_the_working_one() {
        let without = format!(
            r#"{{"grantee_scope_id":"{}","target_scope_id":"{}","action":"memory.read",
                "duration_secs":60,"reason":"joint incident review"}}"#,
            scope(1),
            scope(2)
        );
        let parsed = serde_json::from_str::<LapseTerms>(&without).expect("ceiling is optional");
        assert_eq!(parsed.max_sensitivity, Sensitivity::Internal);
        assert_eq!(Sensitivity::WORKING, Sensitivity::Internal);
    }

    /// The declared ceiling bounds what the grant reaches. Two approvers
    /// consented to a tier, not to a scope.
    #[test]
    fn a_grant_reaches_no_further_up_the_tiers_than_it_declared() {
        let now = Utc::now();
        let working = lapse(now + TimeDelta::seconds(60));
        assert!(working.grants_at(LapseAction::MemoryRead, scope(2), Sensitivity::Public));
        assert!(working.grants_at(LapseAction::MemoryRead, scope(2), Sensitivity::Internal));
        assert!(
            !working.grants_at(LapseAction::MemoryRead, scope(2), Sensitivity::Confidential),
            "a working-tier grant is not a door to confidential material"
        );
        assert!(!working.grants_at(LapseAction::MemoryRead, scope(2), Sensitivity::Restricted));

        let restricted = Lapse {
            max_sensitivity: Sensitivity::Restricted,
            ..lapse(now + TimeDelta::seconds(60))
        };
        for tier in Sensitivity::ALL {
            assert!(
                restricted.grants_at(LapseAction::MemoryRead, scope(2), tier),
                "a restricted-tier grant reaches {tier}"
            );
        }
        // The scope rule is unchanged by the tier: a neighbour is still a
        // neighbour at every tier the grant declares.
        assert!(!restricted.grants_at(LapseAction::MemoryRead, scope(3), Sensitivity::Public));
    }

    #[test]
    fn the_summary_names_a_tier_only_when_it_is_above_the_working_one() {
        assert!(
            !terms().summary().contains("up to"),
            "{}",
            terms().summary()
        );
        let restricted = LapseTerms {
            max_sensitivity: Sensitivity::Restricted,
            ..terms()
        };
        let summary = restricted.summary();
        assert!(summary.contains("up to restricted"), "{summary}");
    }

    #[test]
    fn a_lapse_config_refuses_unknown_fields() {
        assert!(serde_json::from_str::<LapseConfig>(r#"{"max_duration_secs":60}"#).is_ok());
        assert!(serde_json::from_str::<LapseConfig>(r#"{"max_duration_days":1}"#).is_err());
    }
}
