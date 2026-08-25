//! Immutable governed policy relaxations (CPR-31, ADR-0090).
//!
//! Mutations in this module are command-shaped effects invoked only after a
//! typed `Policy/apply` VedaFlow change has satisfied its live approval
//! matrix. Authority ends in the indexed database-time predicate below; the
//! expiry sweep records audit evidence but is not an authorization mechanism.

use std::str::FromStr;

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::PgConnection;
use sqlx::types::Json;
use synveda_types::configuration::EffectiveConfiguration;
use synveda_types::relaxation::{
    CurrentRelaxation, Relaxation, RelaxationAction, RelaxationCommand, RelaxationVersion,
};
use synveda_types::scope::ScopeKind;
use synveda_types::{
    ConfigurationVersionId, Error, IdentityId, IdentityStatus, ProposalId, RelaxationId,
    RelaxationVersionId, Result, ScopeId, Sensitivity, TenantId,
};

/// One persisted typed command and its eventual effect.
#[derive(Debug, Clone)]
pub struct RelaxationChange {
    /// VedaFlow change id.
    pub proposal_id: ProposalId,
    /// Complete command reviewed by VedaFlow.
    pub command: RelaxationCommand,
    /// Canonical command digest.
    pub payload_hash: String,
    /// Stable aggregate after application.
    pub resulting_relaxation_id: Option<RelaxationId>,
    /// New immutable version, when applicable.
    pub resulting_version_id: Option<RelaxationVersionId>,
    /// Aggregate revision after application.
    pub resulting_revision: Option<u64>,
    /// Effect completion time.
    pub applied_at: Option<DateTime<Utc>>,
}

/// Result of applying a typed relaxation command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedRelaxation {
    /// Stable aggregate.
    pub relaxation_id: RelaxationId,
    /// New immutable version for create/revise.
    pub version_id: Option<RelaxationVersionId>,
    /// New aggregate revision.
    pub revision: u64,
}

/// Keyset cursor for aggregate listings.
#[derive(Debug, Clone, Copy)]
pub struct RelaxationCursor {
    /// Last considered update time.
    pub updated_at: DateTime<Utc>,
    /// Last considered id.
    pub id: RelaxationId,
}

fn storage_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error {
        match database.code().as_deref() {
            Some("23503") => {
                return Error::NotFound {
                    entity: "relaxation dependency".to_owned(),
                };
            }
            Some("23505") => {
                return Error::Conflict {
                    message: "relaxation version or change already exists".to_owned(),
                };
            }
            Some("23514") => {
                return Error::Invalid {
                    message: database.to_string(),
                };
            }
            Some("42501") => return crate::rls::backstop_error(database),
            _ => {}
        }
    }
    Error::Storage {
        message: error.to_string(),
    }
}

fn parse_hash(bytes: Vec<u8>) -> Result<String> {
    let hash: [u8; 32] = bytes.try_into().map_err(|value: Vec<u8>| Error::Storage {
        message: format!(
            "relaxation content hash has {} bytes, expected 32",
            value.len()
        ),
    })?;
    Ok(blake3::Hash::from_bytes(hash).to_hex().to_string())
}

struct RelaxationRow {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    governing_scope_id: uuid::Uuid,
    current_version_id: uuid::Uuid,
    revision: i64,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
    revoked_at: Option<DateTime<Utc>>,
    revoked_by: Option<uuid::Uuid>,
    revocation_proposal_id: Option<uuid::Uuid>,
    revocation_reason: Option<String>,
    expiry_recorded_at: Option<DateTime<Utc>>,
}

impl TryFrom<RelaxationRow> for Relaxation {
    type Error = Error;

    fn try_from(row: RelaxationRow) -> Result<Self> {
        Ok(Self {
            id: RelaxationId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            governing_scope_id: ScopeId::from_uuid(row.governing_scope_id),
            current_version_id: RelaxationVersionId::from_uuid(row.current_version_id),
            revision: u64::try_from(row.revision).map_err(|_| Error::Storage {
                message: format!("relaxation revision {} is invalid", row.revision),
            })?,
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
            revoked_at: row.revoked_at,
            revoked_by: row.revoked_by.map(IdentityId::from_uuid),
            revocation_proposal_id: row.revocation_proposal_id.map(ProposalId::from_uuid),
            revocation_reason: row.revocation_reason,
            expiry_recorded_at: row.expiry_recorded_at,
        })
    }
}

struct VersionRow {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    relaxation_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    ordinal: i64,
    subject_identity_id: uuid::Uuid,
    subject_principal_id: String,
    target_scope_id: uuid::Uuid,
    action: String,
    max_sensitivity: String,
    requested_start_at: DateTime<Utc>,
    requested_end_at: DateTime<Utc>,
    effective_start_at: DateTime<Utc>,
    hard_expires_at: DateTime<Utc>,
    reason: String,
    configuration_version_id: Option<uuid::Uuid>,
    configuration_hash: String,
    content_hash: Vec<u8>,
    creator_id: uuid::Uuid,
    approver_ids: Vec<uuid::Uuid>,
    auto_applied: bool,
    created_at: DateTime<Utc>,
}

impl TryFrom<VersionRow> for RelaxationVersion {
    type Error = Error;

    fn try_from(row: VersionRow) -> Result<Self> {
        let terms = synveda_types::relaxation::RelaxationTerms {
            subject_identity_id: IdentityId::from_uuid(row.subject_identity_id),
            target_scope_id: ScopeId::from_uuid(row.target_scope_id),
            action: RelaxationAction::from_str(&row.action)?,
            max_sensitivity: Sensitivity::from_str(&row.max_sensitivity)?,
            requested_start_at: row.requested_start_at,
            requested_end_at: row.requested_end_at,
            reason: row.reason,
        };
        let content_hash = parse_hash(row.content_hash)?;
        if terms.content_hash()? != content_hash {
            return Err(Error::Storage {
                message: format!("relaxation version {} has an invalid digest", row.id),
            });
        }
        Ok(Self {
            id: RelaxationVersionId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            relaxation_id: RelaxationId::from_uuid(row.relaxation_id),
            ordinal: row.ordinal,
            proposal_id: ProposalId::from_uuid(row.proposal_id),
            terms,
            subject_principal_id: row.subject_principal_id,
            effective_start_at: row.effective_start_at,
            hard_expires_at: row.hard_expires_at,
            configuration_version_id: row
                .configuration_version_id
                .map(ConfigurationVersionId::from_uuid),
            configuration_hash: row.configuration_hash,
            content_hash,
            creator_id: IdentityId::from_uuid(row.creator_id),
            approver_ids: row
                .approver_ids
                .into_iter()
                .map(IdentityId::from_uuid)
                .collect(),
            auto_applied: row.auto_applied,
            created_at: row.created_at,
        })
    }
}

/// Fetch one stable aggregate.
pub async fn relaxation(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: RelaxationId,
) -> Result<Option<Relaxation>> {
    let row = sqlx::query_as!(
        RelaxationRow,
        r#"select id, tenant_id, governing_scope_id, current_version_id,
                  revision, created_at, created_by, updated_at, updated_by,
                  revoked_at, revoked_by, revocation_proposal_id,
                  revocation_reason, expiry_recorded_at
             from policy_relaxations
            where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Fetch one immutable version.
pub async fn version(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: RelaxationVersionId,
) -> Result<Option<RelaxationVersion>> {
    let row = sqlx::query_as!(
        VersionRow,
        r#"select id, tenant_id, relaxation_id, proposal_id, ordinal,
                  subject_identity_id, subject_principal_id, target_scope_id,
                  action, max_sensitivity, requested_start_at,
                  requested_end_at, effective_start_at, hard_expires_at,
                  reason, configuration_version_id, configuration_hash,
                  content_hash, creator_id, approver_ids, auto_applied,
                  created_at
             from policy_relaxation_versions
            where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Fetch an aggregate and its current immutable version.
pub async fn current(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: RelaxationId,
) -> Result<Option<CurrentRelaxation>> {
    let Some(relaxation) = relaxation(connection, tenant, id).await? else {
        return Ok(None);
    };
    let version = version(connection, tenant, relaxation.current_version_id)
        .await?
        .ok_or_else(|| Error::Storage {
            message: format!("relaxation {id} has no current version"),
        })?;
    Ok(Some(CurrentRelaxation {
        relaxation,
        version,
    }))
}

/// List stable aggregates newest transition first.
pub async fn list(
    connection: &mut PgConnection,
    tenant: TenantId,
    scope_id: Option<ScopeId>,
    cursor: Option<RelaxationCursor>,
    limit: i64,
) -> Result<Vec<Relaxation>> {
    let rows = sqlx::query_as!(
        RelaxationRow,
        r#"select id, tenant_id, governing_scope_id, current_version_id,
                  revision, created_at, created_by, updated_at, updated_by,
                  revoked_at, revoked_by, revocation_proposal_id,
                  revocation_reason, expiry_recorded_at
             from policy_relaxations
            where tenant_id = $1
              and ($2::uuid is null or governing_scope_id = $2)
              and ($3::timestamptz is null
                   or (updated_at, id) < ($3, $4::uuid))
            order by updated_at desc, id desc
            limit $5"#,
        tenant.as_uuid(),
        scope_id.map(|value| value.as_uuid()),
        cursor.map(|value| value.updated_at),
        cursor.map(|value| value.id.as_uuid()),
        limit,
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// List immutable versions newest first.
pub async fn versions(
    connection: &mut PgConnection,
    tenant: TenantId,
    relaxation_id: RelaxationId,
    before_ordinal: Option<i64>,
    limit: i64,
) -> Result<Vec<RelaxationVersion>> {
    let rows = sqlx::query_as!(
        VersionRow,
        r#"select id, tenant_id, relaxation_id, proposal_id, ordinal,
                  subject_identity_id, subject_principal_id, target_scope_id,
                  action, max_sensitivity, requested_start_at,
                  requested_end_at, effective_start_at, hard_expires_at,
                  reason, configuration_version_id, configuration_hash,
                  content_hash, creator_id, approver_ids, auto_applied,
                  created_at
             from policy_relaxation_versions
            where tenant_id = $1 and relaxation_id = $2
              and ($3::bigint is null or ordinal < $3)
            order by ordinal desc
            limit $4"#,
        tenant.as_uuid(),
        relaxation_id.as_uuid(),
        before_ordinal,
        limit,
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Active rows for exactly the authenticated subject. The database clock is
/// the authority; no sweep must run for a window to end.
pub async fn active_for_subject(
    connection: &mut PgConnection,
    tenant: TenantId,
    subject: &str,
) -> Result<Vec<CurrentRelaxation>> {
    let ids = sqlx::query_scalar!(
        r#"select aggregate.id
             from policy_relaxations aggregate
             join policy_relaxation_versions version
               on version.tenant_id = aggregate.tenant_id
              and version.relaxation_id = aggregate.id
              and version.id = aggregate.current_version_id
            where aggregate.tenant_id = $1
              and version.subject_principal_id = $2
              and aggregate.revoked_at is null
              and version.effective_start_at <= now()
              and version.hard_expires_at > now()
            order by aggregate.id"#,
        tenant.as_uuid(),
        subject,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(storage_error)?;
    let mut rows = Vec::with_capacity(ids.len());
    for id in ids {
        rows.push(
            current(connection, tenant, RelaxationId::from_uuid(id))
                .await?
                .ok_or_else(|| Error::Storage {
                    message: format!("active relaxation {id} disappeared"),
                })?,
        );
    }
    Ok(rows)
}

/// Insert the typed change projection beside its open proposal.
pub async fn insert_change(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
    command: &RelaxationCommand,
    payload_hash: &str,
) -> Result<()> {
    command.validate()?;
    sqlx::query!(
        r#"insert into policy_relaxation_changes
              (tenant_id, proposal_id, command_kind, payload, payload_hash)
           values ($1, $2, $3, $4, $5)"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
        command.kind(),
        Json(command) as _,
        payload_hash,
    )
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(storage_error)
}

/// Read one typed VedaFlow command projection.
pub async fn change(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
) -> Result<Option<RelaxationChange>> {
    struct Row {
        proposal_id: uuid::Uuid,
        payload: Json<RelaxationCommand>,
        payload_hash: String,
        resulting_relaxation_id: Option<uuid::Uuid>,
        resulting_version_id: Option<uuid::Uuid>,
        resulting_revision: Option<i64>,
        applied_at: Option<DateTime<Utc>>,
    }
    let row = sqlx::query_as!(
        Row,
        r#"select proposal_id,
                  payload as "payload: Json<RelaxationCommand>", payload_hash,
                  resulting_relaxation_id, resulting_version_id,
                  resulting_revision, applied_at
             from policy_relaxation_changes
            where tenant_id = $1 and proposal_id = $2"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(RelaxationChange {
            proposal_id: ProposalId::from_uuid(row.proposal_id),
            command: row.payload.0,
            payload_hash: row.payload_hash,
            resulting_relaxation_id: row.resulting_relaxation_id.map(RelaxationId::from_uuid),
            resulting_version_id: row.resulting_version_id.map(RelaxationVersionId::from_uuid),
            resulting_revision: row
                .resulting_revision
                .map(u64::try_from)
                .transpose()
                .map_err(|_| Error::Storage {
                    message: "relaxation result has a negative revision".to_owned(),
                })?,
            applied_at: row.applied_at,
        })
    })
    .transpose()
}

struct NewRelaxationVersion<'a> {
    proposal_id: ProposalId,
    relaxation_id: RelaxationId,
    version_id: RelaxationVersionId,
    ordinal: i64,
    command: &'a RelaxationCommand,
    actor: IdentityId,
    approver_ids: &'a [IdentityId],
    configuration: &'a EffectiveConfiguration,
    applied_at: DateTime<Utc>,
}

async fn insert_version(
    connection: &mut PgConnection,
    tenant: TenantId,
    new: NewRelaxationVersion<'_>,
) -> Result<()> {
    let terms = match new.command {
        RelaxationCommand::Create { terms, .. } | RelaxationCommand::Revise { terms, .. } => terms,
        RelaxationCommand::Revoke { .. } => {
            return Err(Error::Internal {
                message: "a revoke command cannot create a relaxation version".to_owned(),
            });
        }
    };
    let identity = crate::identities::by_id(&mut *connection, tenant, terms.subject_identity_id)
        .await?
        .filter(|identity| identity.status == IdentityStatus::Active)
        .ok_or_else(|| Error::NotFound {
            entity: format!("active identity {}", terms.subject_identity_id),
        })?;
    let subject = identity.subject.ok_or_else(|| Error::Invalid {
        message: "a relaxation subject identity must have an authenticated subject".to_owned(),
    })?;
    let target = crate::scopes::get(&mut *connection, tenant, terms.target_scope_id)
        .await?
        .filter(|scope| scope.kind != ScopeKind::Principal)
        .ok_or_else(|| Error::NotFound {
            entity: format!("non-principal scope {}", terms.target_scope_id),
        })?;
    if target.id != new.configuration.scope_id
        || !new.configuration.document.relaxations.permits(terms.action)
    {
        return Err(Error::Invalid {
            message: "current governed configuration does not permit this relaxation".to_owned(),
        });
    }
    let limit = TimeDelta::seconds(i64::from(
        new.configuration.document.relaxations.maximum_duration_secs,
    ));
    let effective_start_at = terms.requested_start_at.max(new.applied_at);
    let hard_expires_at = terms.requested_end_at.min(new.applied_at + limit);
    if hard_expires_at <= effective_start_at {
        return Err(Error::Conflict {
            message: "the requested relaxation window ended before effect application".to_owned(),
        });
    }
    let content_hash = terms.content_hash()?;
    let content_digest =
        blake3::Hash::from_hex(&content_hash).map_err(|error| Error::Internal {
            message: format!("decode relaxation digest: {error}"),
        })?;
    let approver_uuids: Vec<uuid::Uuid> =
        new.approver_ids.iter().map(IdentityId::as_uuid).collect();
    sqlx::query!(
        r#"insert into policy_relaxation_versions
              (id, tenant_id, relaxation_id, proposal_id, ordinal,
               subject_identity_id, subject_principal_id, target_scope_id,
               action, max_sensitivity, requested_start_at, requested_end_at,
               effective_start_at, hard_expires_at, reason,
               configuration_version_id, configuration_hash, content_hash,
               creator_id, approver_ids, auto_applied, created_at)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                   $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                   $21, $22)"#,
        new.version_id.as_uuid(),
        tenant.as_uuid(),
        new.relaxation_id.as_uuid(),
        new.proposal_id.as_uuid(),
        new.ordinal,
        terms.subject_identity_id.as_uuid(),
        subject,
        terms.target_scope_id.as_uuid(),
        terms.action.as_str(),
        terms.max_sensitivity.as_str(),
        terms.requested_start_at,
        terms.requested_end_at,
        effective_start_at,
        hard_expires_at,
        &terms.reason,
        new.configuration.version_id.map(|value| value.as_uuid()),
        &new.configuration.content_hash,
        content_digest.as_bytes().as_slice(),
        new.actor.as_uuid(),
        &approver_uuids,
        new.approver_ids.is_empty(),
        new.applied_at,
    )
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(storage_error)
}

/// Execute one typed, approved command in the caller's transaction.
pub async fn apply(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
    actor: IdentityId,
    approver_ids: &[IdentityId],
    configuration: &EffectiveConfiguration,
    command: &RelaxationCommand,
) -> Result<AppliedRelaxation> {
    command.validate()?;
    let mut sorted_approvers = approver_ids.to_vec();
    sorted_approvers.sort_unstable();
    sorted_approvers.dedup();
    if sorted_approvers != approver_ids {
        return Err(Error::Invalid {
            message: "relaxation approver ids must be sorted and unique".to_owned(),
        });
    }
    let applied_at = sqlx::query_scalar!(r#"select now() as "now!""#)
        .fetch_one(&mut *connection)
        .await
        .map_err(storage_error)?;
    match command {
        RelaxationCommand::Create {
            relaxation_id,
            version_id,
            terms,
        } => {
            sqlx::query!(
                r#"insert into policy_relaxations
                      (id, tenant_id, governing_scope_id, current_version_id,
                       created_at, created_by, updated_at, updated_by)
                   values ($1, $2, $3, $4, $5, $6, $5, $6)"#,
                relaxation_id.as_uuid(),
                tenant.as_uuid(),
                terms.target_scope_id.as_uuid(),
                version_id.as_uuid(),
                applied_at,
                actor.as_uuid(),
            )
            .execute(&mut *connection)
            .await
            .map_err(storage_error)?;
            insert_version(
                connection,
                tenant,
                NewRelaxationVersion {
                    proposal_id,
                    relaxation_id: *relaxation_id,
                    version_id: *version_id,
                    ordinal: 1,
                    command,
                    actor,
                    approver_ids,
                    configuration,
                    applied_at,
                },
            )
            .await?;
            Ok(AppliedRelaxation {
                relaxation_id: *relaxation_id,
                version_id: Some(*version_id),
                revision: 1,
            })
        }
        RelaxationCommand::Revise {
            relaxation_id,
            expected_current_version_id,
            version_id,
            governing_scope_id,
            ..
        } => {
            let current = current(&mut *connection, tenant, *relaxation_id)
                .await?
                .filter(|current| {
                    current.relaxation.governing_scope_id == *governing_scope_id
                        && current.relaxation.current_version_id
                            == *expected_current_version_id
                        && current.relaxation.revoked_at.is_none()
                })
                .ok_or_else(|| Error::Conflict {
                    message: format!(
                        "relaxation {relaxation_id} is absent, revoked, or no longer has expected version {expected_current_version_id}"
                    ),
                })?;
            let ordinal =
                current
                    .version
                    .ordinal
                    .checked_add(1)
                    .ok_or_else(|| Error::Internal {
                        message: "relaxation version ordinal overflow".to_owned(),
                    })?;
            let revision =
                current
                    .relaxation
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| Error::Internal {
                        message: "relaxation revision overflow".to_owned(),
                    })?;
            insert_version(
                connection,
                tenant,
                NewRelaxationVersion {
                    proposal_id,
                    relaxation_id: *relaxation_id,
                    version_id: *version_id,
                    ordinal,
                    command,
                    actor,
                    approver_ids,
                    configuration,
                    applied_at,
                },
            )
            .await?;
            let result = sqlx::query!(
                r#"update policy_relaxations
                      set current_version_id = $4, revision = $5,
                          updated_at = $6, updated_by = $7,
                          expiry_recorded_at = null
                    where tenant_id = $1 and id = $2
                      and current_version_id = $3 and revoked_at is null"#,
                tenant.as_uuid(),
                relaxation_id.as_uuid(),
                expected_current_version_id.as_uuid(),
                version_id.as_uuid(),
                i64::try_from(revision).map_err(|_| Error::Internal {
                    message: "relaxation revision exceeds database range".to_owned(),
                })?,
                applied_at,
                actor.as_uuid(),
            )
            .execute(connection)
            .await
            .map_err(storage_error)?;
            if result.rows_affected() != 1 {
                return Err(Error::Conflict {
                    message: format!("relaxation {relaxation_id} moved during revision"),
                });
            }
            Ok(AppliedRelaxation {
                relaxation_id: *relaxation_id,
                version_id: Some(*version_id),
                revision,
            })
        }
        RelaxationCommand::Revoke {
            relaxation_id,
            expected_current_version_id,
            governing_scope_id,
            reason,
        } => {
            let current = current(&mut *connection, tenant, *relaxation_id)
                .await?
                .filter(|current| {
                    current.relaxation.governing_scope_id == *governing_scope_id
                        && current.relaxation.current_version_id
                            == *expected_current_version_id
                        && current.relaxation.revoked_at.is_none()
                        && current.version.hard_expires_at > applied_at
                })
                .ok_or_else(|| Error::Conflict {
                    message: format!(
                        "relaxation {relaxation_id} is absent, expired, revoked, or no longer has expected version {expected_current_version_id}"
                    ),
                })?;
            let revision =
                current
                    .relaxation
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| Error::Internal {
                        message: "relaxation revision overflow".to_owned(),
                    })?;
            let result = sqlx::query!(
                r#"update policy_relaxations
                      set revision = $4, updated_at = $5, updated_by = $6,
                          revoked_at = $5, revoked_by = $6,
                          revocation_proposal_id = $7, revocation_reason = $8
                    where tenant_id = $1 and id = $2
                      and current_version_id = $3 and revoked_at is null"#,
                tenant.as_uuid(),
                relaxation_id.as_uuid(),
                expected_current_version_id.as_uuid(),
                i64::try_from(revision).map_err(|_| Error::Internal {
                    message: "relaxation revision exceeds database range".to_owned(),
                })?,
                applied_at,
                actor.as_uuid(),
                proposal_id.as_uuid(),
                reason,
            )
            .execute(connection)
            .await
            .map_err(storage_error)?;
            if result.rows_affected() != 1 {
                return Err(Error::Conflict {
                    message: format!("relaxation {relaxation_id} moved during revocation"),
                });
            }
            Ok(AppliedRelaxation {
                relaxation_id: *relaxation_id,
                version_id: None,
                revision,
            })
        }
    }
}

/// Record an applied result exactly once.
pub async fn complete_change(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
    applied: AppliedRelaxation,
) -> Result<()> {
    let result = sqlx::query!(
        r#"update policy_relaxation_changes
              set resulting_relaxation_id = $3, resulting_version_id = $4,
                  resulting_revision = $5, applied_at = now()
            where tenant_id = $1 and proposal_id = $2 and applied_at is null"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
        applied.relaxation_id.as_uuid(),
        applied.version_id.map(|value| value.as_uuid()),
        i64::try_from(applied.revision).map_err(|_| Error::Internal {
            message: "relaxation revision exceeds database range".to_owned(),
        })?,
    )
    .execute(connection)
    .await
    .map_err(storage_error)?;
    if result.rows_affected() != 1 {
        return Err(Error::Conflict {
            message: format!("relaxation change {proposal_id} already completed or disappeared"),
        });
    }
    Ok(())
}

/// Aggregates whose current version has expired and whose audit bookkeeping
/// is still absent. Expiry itself does not depend on this query.
pub async fn due_for_expiry_event(
    connection: &mut PgConnection,
    tenant: TenantId,
    limit: i64,
) -> Result<Vec<CurrentRelaxation>> {
    let ids = sqlx::query_scalar!(
        r#"select aggregate.id
             from policy_relaxations aggregate
             join policy_relaxation_versions version
               on version.tenant_id = aggregate.tenant_id
              and version.relaxation_id = aggregate.id
              and version.id = aggregate.current_version_id
            where aggregate.tenant_id = $1
              and aggregate.revoked_at is null
              and aggregate.expiry_recorded_at is null
              and version.hard_expires_at <= now()
            order by version.hard_expires_at, aggregate.id
            limit $2"#,
        tenant.as_uuid(),
        limit,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(storage_error)?;
    let mut rows = Vec::with_capacity(ids.len());
    for id in ids {
        rows.push(
            current(connection, tenant, RelaxationId::from_uuid(id))
                .await?
                .ok_or_else(|| Error::Storage {
                    message: format!("expired relaxation {id} disappeared"),
                })?,
        );
    }
    Ok(rows)
}

/// Stamp the content-free expiry event once. This is operational bookkeeping:
/// it does not alter the version, aggregate revision, or authorization
/// predicate.
pub async fn record_expiry(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: RelaxationId,
    expected_revision: u64,
) -> Result<bool> {
    let result = sqlx::query!(
        r#"update policy_relaxations aggregate
              set expiry_recorded_at = now()
             from policy_relaxation_versions version
            where aggregate.tenant_id = $1 and aggregate.id = $2
              and aggregate.revision = $3
              and aggregate.current_version_id = version.id
              and version.tenant_id = aggregate.tenant_id
              and version.relaxation_id = aggregate.id
              and aggregate.revoked_at is null
              and aggregate.expiry_recorded_at is null
              and version.hard_expires_at <= now()"#,
        tenant.as_uuid(),
        id.as_uuid(),
        i64::try_from(expected_revision).map_err(|_| Error::Internal {
            message: "relaxation revision exceeds database range".to_owned(),
        })?,
    )
    .execute(connection)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() == 1)
}
