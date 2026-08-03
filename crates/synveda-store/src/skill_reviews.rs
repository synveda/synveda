//! The reviewer's checklist (SKIL-3, ADR-0053; migration 0033).
//!
//! One table, and its whole design is its **key**. A checklist is stored
//! against a digest of the bundle's own object addresses, so:
//!
//! - an edit beneath a review produces a bundle for which no checklist is
//!   found, rather than one carrying answers about content nobody read;
//! - nothing invalidates anything, because there is nothing to invalidate;
//! - the old answers stay attached to the bytes they were true of, which
//!   is what makes the trail readable backwards.
//!
//! That is ADR-0032 decision 6's "approvals bind bytes" applied to the one
//! review artefact that had no address check of its own — and it is why
//! this module has no `stale` column, no `updated_at` comparison, and no
//! function that goes looking for reviews to expire.
//!
//! The *other* half of a skill's score — the automated rubric — is not
//! here and is not in any table a decision reads. It is a pure function of
//! the bytes, recomputed wherever it renders (ADR-0053 decisions 2 and 3);
//! [`crate::skills`] carries a cache of it on the draft row for the
//! registry listing alone.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Checklist, Error, IdentityId, Result, ScopeId, SkillName, TenantId};

/// A stored checklist: one reviewer's answers about one bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReview {
    /// The scope the bundle was headed for.
    pub scope_id: ScopeId,
    /// The bundle's name.
    pub skill_name: SkillName,
    /// The digest of exactly the bytes these answers are about.
    pub bundle_digest: [u8; 32],
    /// The answers, and whatever the reviewer wanted to say.
    pub checklist: Checklist,
    /// Which rubric was rendered beside them when they answered.
    pub rubric_version: u32,
    /// When.
    pub reviewed_at: DateTime<Utc>,
    /// Who.
    pub reviewed_by: IdentityId,
}

/// What [`record`] writes.
pub struct NewReview<'a> {
    /// The scope the bundle is headed for.
    pub scope_id: ScopeId,
    /// The bundle's name.
    pub skill_name: &'a SkillName,
    /// The digest of exactly the bytes being answered about.
    pub bundle_digest: [u8; 32],
    /// The answers.
    pub checklist: &'a Checklist,
    /// The rubric rendered beside the reviewer.
    pub rubric_version: u32,
    /// Who is answering.
    pub reviewer: IdentityId,
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant.
        if db.code().as_deref() == Some("23503") {
            return Error::Invalid {
                message: format!("a skill review must name a tenant this connection holds: {db}"),
            };
        }
        // 23514 check_violation: a digest that is not 32 bytes, an empty
        // answers object, an over-long note.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (ADR-0009).
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// The stored shape, mapped on the way out.
struct ReviewRow {
    scope_id: uuid::Uuid,
    skill_name: String,
    bundle_digest: Vec<u8>,
    answers: serde_json::Value,
    note: Option<String>,
    rubric_version: i32,
    reviewed_at: DateTime<Utc>,
    reviewed_by: uuid::Uuid,
}

impl TryFrom<ReviewRow> for StoredReview {
    type Error = Error;

    fn try_from(row: ReviewRow) -> Result<Self> {
        let bundle_digest =
            <[u8; 32]>::try_from(row.bundle_digest.as_slice()).map_err(|_| Error::Internal {
                message: format!(
                    "skill review for {:?} has a bundle digest that is not 32 bytes",
                    row.skill_name
                ),
            })?;
        // The column's CHECK guarantees the *shape*; the vocabulary is
        // this crate's to parse, and a value outside it means code and
        // schema have drifted (the role_bindings discipline, ADR-0015).
        let answers = serde_json::from_value(row.answers).map_err(|err| Error::Internal {
            message: format!(
                "skill review for {:?} holds answers this build cannot read: {err}",
                row.skill_name
            ),
        })?;
        Ok(StoredReview {
            scope_id: ScopeId::from_uuid(row.scope_id),
            skill_name: row.skill_name.parse()?,
            bundle_digest,
            checklist: Checklist {
                answers,
                note: row.note,
            },
            rubric_version: row.rubric_version.max(0).unsigned_abs(),
            reviewed_at: row.reviewed_at,
            reviewed_by: IdentityId::from_uuid(row.reviewed_by),
        })
    }
}

/// Writes a checklist: records one, or replaces the one already recorded
/// for exactly these bytes.
///
/// **A replace is an ordinary act and is not a loss.** The row is
/// last-writer-wins because a checklist is a statement about the bundle
/// rather than a vote — two reviewers disagreeing is what the approval
/// matrix is for — and every submission chains `skill.checklist.recorded`,
/// so the durable record of who said what is the audit chain rather than
/// this table (ADR-0053 decision 10).
///
/// # Errors
///
/// [`Error::Invalid`] for an unknown tenant or a value a CHECK refuses;
/// [`Error::Storage`] otherwise.
#[tracing::instrument(
    name = "store.skill_reviews.record",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %new.scope_id, skill.name = %new.skill_name),
    err(Display)
)]
pub async fn record<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewReview<'_>,
) -> Result<StoredReview> {
    let answers = serde_json::to_value(&new.checklist.answers).map_err(|err| Error::Internal {
        message: format!("a checklist could not be serialised: {err}"),
    })?;
    let row = sqlx::query_as!(
        ReviewRow,
        r#"insert into skill_reviews
               (tenant_id, scope_id, skill_name, bundle_digest, answers, note,
                rubric_version, reviewed_by)
           values ($1, $2, $3, $4, $5, $6, $7, $8)
           on conflict (tenant_id, scope_id, skill_name, bundle_digest) do update
               set answers        = excluded.answers,
                   note           = excluded.note,
                   rubric_version = excluded.rubric_version,
                   reviewed_at    = now(),
                   reviewed_by    = excluded.reviewed_by
           returning scope_id, skill_name, bundle_digest, answers, note, rubric_version,
                     reviewed_at, reviewed_by"#,
        tenant.as_uuid(),
        new.scope_id.as_uuid(),
        new.skill_name.as_str(),
        &new.bundle_digest[..],
        answers,
        new.checklist.note.as_deref(),
        i32::try_from(new.rubric_version).unwrap_or(i32::MAX),
        new.reviewer.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredReview::try_from(row)
}

/// The checklist recorded for exactly these bytes, or `None`.
///
/// `None` is the answer to both "nobody has reviewed this" and "somebody
/// reviewed it and the bundle has changed since", and those two being the
/// same answer is the design rather than a limitation of it: from the
/// publication's point of view they are the same fact.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skill_reviews.for_bundle",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id, skill.name = %name),
    err(Display)
)]
pub async fn for_bundle<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
    name: &SkillName,
    bundle_digest: &[u8; 32],
) -> Result<Option<StoredReview>> {
    let row = sqlx::query_as!(
        ReviewRow,
        r#"select scope_id, skill_name, bundle_digest, answers, note, rubric_version,
                  reviewed_at, reviewed_by
           from skill_reviews
           where tenant_id = $1 and scope_id = $2 and skill_name = $3 and bundle_digest = $4"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
        name.as_str(),
        &bundle_digest[..],
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredReview::try_from).transpose()
}

/// Every checklist recorded for one skill at one scope, newest first — the
/// history of what reviewers have said about its successive bundles.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skill_reviews.history",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id, skill.name = %name),
    err(Display)
)]
pub async fn history<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
    name: &SkillName,
    limit: i64,
) -> Result<Vec<StoredReview>> {
    let rows = sqlx::query_as!(
        ReviewRow,
        r#"select scope_id, skill_name, bundle_digest, answers, note, rubric_version,
                  reviewed_at, reviewed_by
           from skill_reviews
           where tenant_id = $1 and scope_id = $2 and skill_name = $3
           order by reviewed_at desc
           limit $4"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
        name.as_str(),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredReview::try_from).collect()
}

// ── The override ────────────────────────────────────────────────────────

/// A recorded decision to publish a bundle the quality gate refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOverride {
    /// The scope the bundle is drafted at.
    pub scope_id: ScopeId,
    /// The bundle's name.
    pub skill_name: SkillName,
    /// The digest of exactly the bytes it was granted over.
    pub bundle_digest: [u8; 32],
    /// Why.
    pub reason: String,
    /// What the rubric said when it was granted.
    pub score: u8,
    /// Which rubric said it.
    pub rubric_version: u32,
    /// When.
    pub granted_at: DateTime<Utc>,
    /// Who.
    pub granted_by: IdentityId,
}

/// What [`grant_override`] writes.
pub struct NewOverride<'a> {
    /// The scope the bundle is drafted at.
    pub scope_id: ScopeId,
    /// The bundle's name.
    pub skill_name: &'a SkillName,
    /// The digest of exactly the bytes being overridden.
    pub bundle_digest: [u8; 32],
    /// Why.
    pub reason: &'a str,
    /// What the rubric said.
    pub score: u8,
    /// Which rubric said it.
    pub rubric_version: u32,
    /// Who is granting.
    pub granter: IdentityId,
}

/// The stored override shape, mapped on the way out.
struct OverrideRow {
    scope_id: uuid::Uuid,
    skill_name: String,
    bundle_digest: Vec<u8>,
    reason: String,
    score: i16,
    rubric_version: i32,
    granted_at: DateTime<Utc>,
    granted_by: uuid::Uuid,
}

impl TryFrom<OverrideRow> for StoredOverride {
    type Error = Error;

    fn try_from(row: OverrideRow) -> Result<Self> {
        let bundle_digest =
            <[u8; 32]>::try_from(row.bundle_digest.as_slice()).map_err(|_| Error::Internal {
                message: format!(
                    "quality override for {:?} has a bundle digest that is not 32 bytes",
                    row.skill_name
                ),
            })?;
        Ok(StoredOverride {
            scope_id: ScopeId::from_uuid(row.scope_id),
            skill_name: row.skill_name.parse()?,
            bundle_digest,
            reason: row.reason,
            score: row.score.clamp(0, i16::from(u8::MAX)) as u8,
            rubric_version: row.rubric_version.max(0).unsigned_abs(),
            granted_at: row.granted_at,
            granted_by: IdentityId::from_uuid(row.granted_by),
        })
    }
}

/// Records an override over one bundle.
///
/// **First writer wins, and the row cannot be rewritten** (migration 0033
/// grants no UPDATE). Re-answering a checklist is an ordinary act; editing
/// the stated reason for shipping something below the bar is not one
/// anybody should have, because that sentence is the whole durable
/// explanation. A conflicting second grant returns the first, which is the
/// honest answer: the override already exists, and it says what it says.
///
/// # Errors
///
/// [`Error::Invalid`] for an unknown tenant or a value a CHECK refuses;
/// [`Error::Storage`] otherwise.
#[tracing::instrument(
    name = "store.skill_reviews.grant_override",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %new.scope_id, skill.name = %new.skill_name),
    err(Display)
)]
pub async fn grant_override<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewOverride<'_>,
) -> Result<StoredOverride> {
    let row = sqlx::query_as!(
        OverrideRow,
        r#"insert into skill_quality_overrides
               (tenant_id, scope_id, skill_name, bundle_digest, reason, score,
                rubric_version, granted_by)
           values ($1, $2, $3, $4, $5, $6, $7, $8)
           on conflict (tenant_id, scope_id, skill_name, bundle_digest) do update
               set reason = skill_quality_overrides.reason
           returning scope_id, skill_name, bundle_digest, reason, score, rubric_version,
                     granted_at, granted_by"#,
        tenant.as_uuid(),
        new.scope_id.as_uuid(),
        new.skill_name.as_str(),
        &new.bundle_digest[..],
        new.reason,
        i16::from(new.score),
        i32::try_from(new.rubric_version).unwrap_or(i32::MAX),
        new.granter.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredOverride::try_from(row)
}

/// The override standing over exactly these bytes, or `None`.
///
/// `None` is also the answer for a bundle somebody overrode and then
/// edited, which is the design: nobody agreed to ship whatever it became.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skill_reviews.override_for",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id, skill.name = %name),
    err(Display)
)]
pub async fn override_for<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
    name: &SkillName,
    bundle_digest: &[u8; 32],
) -> Result<Option<StoredOverride>> {
    let row = sqlx::query_as!(
        OverrideRow,
        r#"select scope_id, skill_name, bundle_digest, reason, score, rubric_version,
                  granted_at, granted_by
           from skill_quality_overrides
           where tenant_id = $1 and scope_id = $2 and skill_name = $3 and bundle_digest = $4"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
        name.as_str(),
        &bundle_digest[..],
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredOverride::try_from).transpose()
}
