//! Retention, disposal & staleness vocabulary (MEM-6, ADR-0040).
//!
//! Horizons are whole days, and zero means *keep* — the absence of a
//! horizon, never a horizon of zero length (ADR-0040 decision 4). Days
//! rather than seconds because a retention schedule is written in days by
//! the people who sign it, and an integer keeps every pack config `Eq` and
//! `Hash`, which [`crate::PackConfig`] and the PDP's effective-pack type
//! both need.
//!
//! Nothing here is stamped on a record. A record's fate is a function of
//! facts it already carries — class, kind, valid time — and the pack in
//! force at its scope *now*, which is what makes "a retention policy
//! change re-evaluates existing records" structural rather than a backfill
//! (ADR-0040 decision 1).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, RecordClass, RecordKind};

/// How much of the feature runs at scopes a pack governs (ADR-0040
/// decision 13).
///
/// Two values, because a pack that wants scoring without expiry sets a
/// half-life and no horizons, and a pack that wants expiry without scoring
/// sets horizons and no half-life. A third mode would only name a
/// combination the numbers already express.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum RetentionMode {
    /// Nothing expires, nothing is disposed of, and nothing decays: the
    /// product exactly as it behaved before MEM-6.
    Off,
    /// Horizons are read on the read path and acted on by the sweep, and
    /// staleness scores composition. The default — with, in every embedded
    /// pack, no record horizon set, so the machinery is on and the
    /// schedule is the org's to name (ADR-0040 decision 13).
    #[default]
    Enforce,
}

impl RetentionMode {
    /// All modes, least to most active.
    pub const ALL: [RetentionMode; 2] = [RetentionMode::Off, RetentionMode::Enforce];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            RetentionMode::Off => "off",
            RetentionMode::Enforce => "enforce",
        }
    }

    /// Whether horizons are read and acted on under this mode.
    #[must_use]
    pub const fn enforces(&self) -> bool {
        matches!(self, RetentionMode::Enforce)
    }
}

impl fmt::Display for RetentionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RetentionMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RetentionMode::ALL
            .into_iter()
            .find(|mode| mode.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown retention mode: {s:?}"),
            })
    }
}

/// The longest horizon a pack may ask for, in days — a hundred years.
/// Not a policy statement: a bound that catches a schedule written in
/// seconds or milliseconds by mistake, which would otherwise read as
/// "keep forever" and be indistinguishable from the default.
pub const MAX_RETENTION_DAYS: u32 = 36_500;

/// The shortest staging horizon a pack may ask for, in days (ADR-0040
/// decision 7). Disposing of a staging row frees its
/// `(tenant_id, idempotency_key)`, so this floor is what MEM-1's
/// first-writer-wins admission gate is worth in the worst configuration a
/// pack can express: one day, against adapters that retry in seconds and a
/// pipeline whose lag SLO is 60s.
pub const MIN_STAGING_DAYS: u32 = 1;

/// A retention horizon per record class, in days; zero means keep
/// (ADR-0040 decision 4).
///
/// A fixed struct rather than a map over [`RecordClass`]: the vocabulary
/// is closed at six (seed §4.2), so every class is answered, a pack cannot
/// name a class that does not exist, and the config stays `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ClassTtl {
    /// Days a `fact` is retained.
    pub fact: u32,
    /// Days a `decision` is retained.
    pub decision: u32,
    /// Days a `preference` is retained.
    pub preference: u32,
    /// Days a `procedure` is retained.
    pub procedure: u32,
    /// Days an `entity` is retained.
    pub entity: u32,
    /// Days an `episode` is retained.
    pub episode: u32,
}

impl ClassTtl {
    /// Every class kept indefinitely — the product default, and what every
    /// embedded pack carries (ADR-0040 decision 13).
    pub const KEEP: ClassTtl = ClassTtl {
        fact: 0,
        decision: 0,
        preference: 0,
        procedure: 0,
        entity: 0,
        episode: 0,
    };

    /// The horizon for `class`, in days; zero means keep.
    #[must_use]
    pub const fn days(&self, class: RecordClass) -> u32 {
        match class {
            RecordClass::Fact => self.fact,
            RecordClass::Decision => self.decision,
            RecordClass::Preference => self.preference,
            RecordClass::Procedure => self.procedure,
            RecordClass::Entity => self.entity,
            RecordClass::Episode => self.episode,
        }
    }

    /// The same horizon with its class, for every class — the form the
    /// read path's per-scope predicate and the CLI's rendering both want.
    #[must_use]
    pub fn all(&self) -> [(RecordClass, u32); 6] {
        RecordClass::ALL.map(|class| (class, self.days(class)))
    }

    /// Whether any class has a horizon at all.
    #[must_use]
    pub fn any(&self) -> bool {
        RecordClass::ALL.iter().any(|class| self.days(*class) > 0)
    }

    /// Refuses a horizon past [`MAX_RETENTION_DAYS`], naming the class.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming the class that is out of range.
    pub fn validate(&self) -> Result<(), Error> {
        for (class, days) in self.all() {
            if days > MAX_RETENTION_DAYS {
                return Err(Error::Invalid {
                    message: format!(
                        "retention for {class} is {days} days; the maximum is {MAX_RETENTION_DAYS}"
                    ),
                });
            }
        }
        Ok(())
    }
}

/// A pack's retention, disposal and staleness configuration (ADR-0040).
///
/// Rides the loaded pack beside [`crate::DedupConfig`] and
/// [`crate::CompositionConfig`] and resolves the same way: the read path
/// asks for the effective pack at each planned scope and gets the horizons
/// that scope serves under; the sweep asks at the scope a record lives at
/// and gets the horizons that scope keeps under (ADR-0040 decision 10).
///
/// This config never grants and never widens: a horizon only ever removes
/// material, and a half-life only ever reorders within a gradient
/// position. That is why [`Default`] — the product config, which expires
/// nothing — is also the fail-safe for a stored pack configuring nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    /// How much of the feature runs.
    pub mode: RetentionMode,
    /// Per-class record horizons, measured from `valid_from` — the one
    /// stamp that moves only when the fact does (ADR-0040 decision 3).
    pub ttl: ClassTtl,
    /// Days a closed version is kept before it is destroyed, measured
    /// from the instant its transaction period closed. Zero means keep:
    /// no embedded pack sets one, because destruction is irreversible and
    /// must be a decision somebody took (ADR-0040 decision 13).
    pub destroy_after_days: u32,
    /// Days an observe staging row and its quarantine marker are kept
    /// before disposal (ADR-0040 decision 7). Never zero: this plane holds
    /// whole payloads and its disposal was promised by ADR-0020/0021.
    pub staging_days: u32,
    /// The staleness half-life in days: the age at which a record's
    /// freshness score halves. Zero disables scoring entirely.
    pub staleness_half_life_days: u32,
}

impl RetentionConfig {
    /// The product config: the machinery on, no record horizon, no
    /// destruction, staging disposed at 30 days, a 180-day half-life.
    ///
    /// The two numbers that are not zero are the two that cannot destroy
    /// anything a tenant expected to keep: staging is pre-extraction
    /// provenance whose disposal ADR-0020 already promised, and a
    /// half-life only reorders.
    pub const DEFAULT: RetentionConfig = RetentionConfig {
        mode: RetentionMode::Enforce,
        ttl: ClassTtl::KEEP,
        destroy_after_days: 0,
        staging_days: 30,
        staleness_half_life_days: 180,
    };

    /// `regulated-strict`'s config: the same, with the staging plane
    /// disposed of at a week and a 90-day half-life. Seed §6's "retention
    /// enforced" in the one place a schedule can be a product default
    /// without destroying something nobody asked to lose (ADR-0040
    /// decision 13).
    pub const STRICT: RetentionConfig = RetentionConfig {
        mode: RetentionMode::Enforce,
        ttl: ClassTtl::KEEP,
        destroy_after_days: 0,
        staging_days: 7,
        staleness_half_life_days: 90,
    };

    /// Nothing runs: the pre-MEM-6 product.
    pub const OFF: RetentionConfig = RetentionConfig {
        mode: RetentionMode::Off,
        ttl: ClassTtl::KEEP,
        destroy_after_days: 0,
        staging_days: 30,
        staleness_half_life_days: 0,
    };

    /// Refuses a config that could never do anything sensible: a horizon
    /// past [`MAX_RETENTION_DAYS`], or a staging horizon under
    /// [`MIN_STAGING_DAYS`] — which would trade MEM-1's idempotency
    /// guarantee for nothing (ADR-0040 decision 7). Called at apply time,
    /// and again when a stored row is read.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming the field that is out of range.
    pub fn validate(&self) -> Result<(), Error> {
        self.ttl.validate()?;
        for (label, days) in [
            ("destroy_after_days", self.destroy_after_days),
            ("staging_days", self.staging_days),
            ("staleness_half_life_days", self.staleness_half_life_days),
        ] {
            if days > MAX_RETENTION_DAYS {
                return Err(Error::Invalid {
                    message: format!("{label} is {days}; the maximum is {MAX_RETENTION_DAYS} days"),
                });
            }
        }
        if self.staging_days < MIN_STAGING_DAYS {
            return Err(Error::Invalid {
                message: format!(
                    "staging_days is {}; the minimum is {MIN_STAGING_DAYS} — disposal frees an \
                     idempotency key, and a shorter horizon would spend MEM-1's admission \
                     guarantee for nothing",
                    self.staging_days
                ),
            });
        }
        Ok(())
    }

    /// The instant at or before which a record of `class` is past this
    /// pack's horizon, given the instant the question is asked at.
    /// `None` when the class is kept — there is no cutoff, not a cutoff
    /// at the beginning of time.
    ///
    /// This is the whole of the read path's retention predicate and the
    /// whole of the sweep's selection: a record is due exactly when its
    /// `valid_from` is at or before the cutoff, and pinned records are
    /// never asked (ADR-0040 decision 8).
    #[must_use]
    pub fn cutoff(&self, class: RecordClass, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if !self.mode.enforces() {
            return None;
        }
        let days = self.ttl.days(class);
        if days == 0 {
            return None;
        }
        at.checked_sub_signed(Duration::days(i64::from(days)))
    }

    /// The instant at or before which a closed version is destroyed.
    /// `None` when this pack destroys nothing.
    #[must_use]
    pub fn destroy_cutoff(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if !self.mode.enforces() || self.destroy_after_days == 0 {
            return None;
        }
        at.checked_sub_signed(Duration::days(i64::from(self.destroy_after_days)))
    }

    /// The instant at or before which a staging row is disposed of.
    /// `None` when this pack is off.
    #[must_use]
    pub fn staging_cutoff(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if !self.mode.enforces() {
            return None;
        }
        at.checked_sub_signed(Duration::days(i64::from(self.staging_days)))
    }

    /// The staleness score of material last asserted at `asserted_at`, as
    /// composed at `at`: `0.5 ^ (age / half_life)`, clamped to `0..=1`.
    ///
    /// `1.0` is fresh. Pinned material never reaches this function
    /// (seed §4.2: it cannot be decayed), a zero half-life scores
    /// everything `1.0`, and material asserted in the future — a client's
    /// clock ahead of the server's — scores `1.0` rather than above it.
    #[must_use]
    pub fn staleness(&self, asserted_at: DateTime<Utc>, at: DateTime<Utc>) -> f64 {
        if !self.mode.enforces() || self.staleness_half_life_days == 0 {
            return 1.0;
        }
        let age_days = (at - asserted_at).num_seconds() as f64 / 86_400.0;
        if age_days <= 0.0 {
            return 1.0;
        }
        let score = 0.5_f64.powf(age_days / f64::from(self.staleness_half_life_days));
        score.clamp(0.0, 1.0)
    }

    /// Whether `kind` is subject to this feature at all — the seed §4.2
    /// exemption in the one place every caller reads it from (ADR-0040
    /// decision 8).
    #[must_use]
    pub const fn governs(kind: RecordKind) -> bool {
        matches!(kind, RecordKind::Derived)
    }
}

impl Default for RetentionConfig {
    fn default() -> Self {
        RetentionConfig::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("fixture instant parses")
            .with_timezone(&Utc)
    }

    #[test]
    fn the_product_config_is_valid_and_expires_nothing() {
        let config = RetentionConfig::DEFAULT;
        config.validate().expect("the product config is valid");
        assert_eq!(config, RetentionConfig::default());
        assert!(config.mode.enforces());
        assert!(
            !config.ttl.any(),
            "an upgrade must not start deleting a tenant's memory"
        );
        let now = at("2026-07-26T00:00:00Z");
        for class in RecordClass::ALL {
            assert!(
                config.cutoff(class, now).is_none(),
                "{class} has no product horizon"
            );
        }
        assert!(config.destroy_cutoff(now).is_none());
        assert!(
            config.staging_cutoff(now).is_some(),
            "the staging plane's disposal was promised by ADR-0020"
        );
        RetentionConfig::STRICT
            .validate()
            .expect("the strict config is valid");
        assert!(!RetentionConfig::STRICT.ttl.any());
        // Strict disposes of staging sooner — the one place seed §6's
        // "retention enforced" can be a product default without
        // destroying something a tenant expected to keep.
        const { assert!(RetentionConfig::STRICT.staging_days < RetentionConfig::DEFAULT.staging_days) };
    }

    #[test]
    fn off_is_the_pre_mem6_product() {
        let config = RetentionConfig::OFF;
        config.validate().expect("off is valid");
        let now = at("2026-07-26T00:00:00Z");
        let expiring = RetentionConfig {
            mode: RetentionMode::Off,
            ttl: ClassTtl {
                episode: 1,
                ..ClassTtl::KEEP
            },
            destroy_after_days: 1,
            ..RetentionConfig::DEFAULT
        };
        assert!(
            expiring.cutoff(RecordClass::Episode, now).is_none(),
            "off must ignore horizons rather than merely lack them"
        );
        assert!(expiring.destroy_cutoff(now).is_none());
        assert!(expiring.staging_cutoff(now).is_none());
        assert_eq!(config.staleness(at("2020-01-01T00:00:00Z"), now), 1.0);
    }

    #[test]
    fn modes_round_trip_through_their_wire_names() {
        for mode in RetentionMode::ALL {
            assert_eq!(
                mode.as_str().parse::<RetentionMode>().expect("parses"),
                mode
            );
        }
        assert!("expire".parse::<RetentionMode>().is_err());
    }

    #[test]
    fn a_cutoff_is_the_horizon_subtracted_from_the_instant_asked_at() {
        let now = at("2026-07-26T00:00:00Z");
        let config = RetentionConfig {
            ttl: ClassTtl {
                episode: 30,
                decision: 2_555,
                ..ClassTtl::KEEP
            },
            ..RetentionConfig::DEFAULT
        };
        assert_eq!(
            config.cutoff(RecordClass::Episode, now),
            Some(at("2026-06-26T00:00:00Z"))
        );
        assert_eq!(
            config.cutoff(RecordClass::Decision, now),
            Some(at("2019-07-28T00:00:00Z")),
            "seven years, the other end of a real schedule"
        );
        assert!(
            config.cutoff(RecordClass::Fact, now).is_none(),
            "an unset class is kept, not expired at the beginning of time"
        );
    }

    #[test]
    fn every_class_is_answered_and_none_can_be_invented() {
        let ttl = ClassTtl {
            fact: 1,
            decision: 2,
            preference: 3,
            procedure: 4,
            entity: 5,
            episode: 6,
        };
        let mut seen: Vec<u32> = ttl.all().iter().map(|(_, days)| *days).collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(ttl.all().len(), RecordClass::ALL.len());
        // The closed vocabulary is the point: serde refuses a class the
        // product does not have, rather than silently dropping it.
        let err = serde_json::from_str::<ClassTtl>(r#"{"fact":1,"anecdote":2}"#)
            .expect_err("a class that does not exist is refused");
        assert!(err.to_string().contains("anecdote"));
        // And an omitted class is kept, so a schedule names only what it
        // schedules.
        let partial: ClassTtl =
            serde_json::from_str(r#"{"episode":30}"#).expect("a partial schedule parses");
        assert_eq!(partial.episode, 30);
        assert_eq!(partial.fact, 0);
    }

    #[test]
    fn a_config_outside_its_bounds_is_refused_by_field_name() {
        let centuries = RetentionConfig {
            ttl: ClassTtl {
                fact: MAX_RETENTION_DAYS + 1,
                ..ClassTtl::KEEP
            },
            ..RetentionConfig::DEFAULT
        };
        let err = centuries
            .validate()
            .expect_err("a schedule written in seconds is a mistake, not a policy");
        assert!(err.to_string().contains("fact"), "{err}");

        let hasty = RetentionConfig {
            staging_days: 0,
            ..RetentionConfig::DEFAULT
        };
        let err = hasty
            .validate()
            .expect_err("disposal frees an idempotency key");
        assert!(err.to_string().contains("staging_days"), "{err}");

        let long = RetentionConfig {
            destroy_after_days: MAX_RETENTION_DAYS + 1,
            ..RetentionConfig::DEFAULT
        };
        let err = long.validate().expect_err("past the product bound");
        assert!(err.to_string().contains("destroy_after_days"), "{err}");
    }

    #[test]
    fn staleness_halves_at_the_half_life_and_never_exceeds_one() {
        let now = at("2026-07-26T00:00:00Z");
        let config = RetentionConfig {
            staleness_half_life_days: 90,
            ..RetentionConfig::DEFAULT
        };
        assert_eq!(config.staleness(now, now), 1.0);
        let half = config.staleness(at("2026-04-27T00:00:00Z"), now);
        assert!(
            (half - 0.5).abs() < 0.01,
            "90 days is one half-life: {half}"
        );
        let quarter = config.staleness(at("2026-01-27T00:00:00Z"), now);
        assert!(
            (quarter - 0.25).abs() < 0.01,
            "180 days is two half-lives: {quarter}"
        );
        assert!(
            config.staleness(at("2030-01-01T00:00:00Z"), now) == 1.0,
            "a client's clock ahead of the server's scores fresh, never above fresh"
        );
        let none = RetentionConfig {
            staleness_half_life_days: 0,
            ..RetentionConfig::DEFAULT
        };
        assert_eq!(none.staleness(at("2000-01-01T00:00:00Z"), now), 1.0);
    }

    #[test]
    fn pinned_is_exempt_and_not_by_configuration() {
        assert!(!RetentionConfig::governs(RecordKind::Pinned));
        assert!(RetentionConfig::governs(RecordKind::Derived));
        // There is deliberately no config field that could re-admit it:
        // the exemption is seed §4.2, so it is a function of the kind
        // alone and no pack is consulted.
    }

    #[test]
    fn a_config_round_trips_and_refuses_unknown_fields() {
        let config = RetentionConfig {
            ttl: ClassTtl {
                episode: 30,
                ..ClassTtl::KEEP
            },
            destroy_after_days: 365,
            ..RetentionConfig::DEFAULT
        };
        let json = serde_json::to_string(&config).expect("serialises");
        let back: RetentionConfig = serde_json::from_str(&json).expect("round trips");
        assert_eq!(back, config);
        assert!(
            serde_json::from_str::<RetentionConfig>(
                r#"{"mode":"enforce","ttl":{},"destroy_after_days":0,"staging_days":30,
                    "staleness_half_life_days":180,"forever":true}"#
            )
            .is_err(),
            "a field the product does not have is a typo in someone's schedule"
        );
    }
}
