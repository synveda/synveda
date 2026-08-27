//! Durable, tenant-isolated Knowledge conflict evidence (CPR-37, ADR-0096).
//!
//! These rows are candidate evidence below authorisation, just like Knowledge
//! search candidates. The gateway must decide every exact Knowledge member (or
//! the source capture candidate) before disclosing a set or its cardinality.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::knowledge::{
    ConflictClassification, ConflictMember, ConflictResolutionKind, ConflictSet, ConflictSetStatus,
};
use synveda_types::{
    CaptureCandidateId, ConflictMemberId, ConflictSetId, Error, KnowledgeItemId,
    KnowledgeRevisionId, ProjectId, ProposalId, Result, ScopeId, TenantId,
};

/// Maximum members retained in one deterministic set.
pub const MAX_CONFLICT_MEMBERS: usize = 32;

/// One exact current Knowledge match to retain beside a challenger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedRevision {
    /// Stable item.
    pub item_id: KnowledgeItemId,
    /// Exact immutable head compared.
    pub revision_id: KnowledgeRevisionId,
    /// Proposed relationship class.
    pub classification: ConflictClassification,
    /// Integer similarity.
    pub similarity_permille: i32,
    /// Stable bounded reason.
    pub reason_code: String,
}

/// Complete immutable evidence for a new set.
pub struct NewConflictSet<'a> {
    /// Stable set id.
    pub id: ConflictSetId,
    /// Tenant.
    pub tenant_id: TenantId,
    /// Governing scope.
    pub scope_id: ScopeId,
    /// Optional project.
    pub project_id: Option<ProjectId>,
    /// Dominant classification.
    pub classification: ConflictClassification,
    /// Knowledge-backed challenger item.
    pub challenger_item_id: Option<KnowledgeItemId>,
    /// Knowledge-backed challenger revision.
    pub challenger_revision_id: Option<KnowledgeRevisionId>,
    /// Capture-backed challenger.
    pub capture_candidate_id: Option<CaptureCandidateId>,
    /// Visible exact current matches.
    pub matches: &'a [MatchedRevision],
    /// Actor retaining the evidence.
    pub created_by: &'a str,
}

/// Keyset for newest-first set listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictCursor {
    /// Last considered update instant.
    pub updated_at: DateTime<Utc>,
    /// Stable tie breaker.
    pub id: ConflictSetId,
}

#[derive(sqlx::FromRow)]
struct SetRow {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    scope_id: uuid::Uuid,
    project_id: Option<uuid::Uuid>,
    classification: String,
    status: String,
    revision: i64,
    capture_candidate_id: Option<uuid::Uuid>,
    resolution_change_id: Option<uuid::Uuid>,
    resolution: Option<String>,
    created_by: String,
    resolved_by: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
}

impl TryFrom<SetRow> for ConflictSet {
    type Error = Error;

    fn try_from(row: SetRow) -> Result<Self> {
        Ok(Self {
            id: ConflictSetId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            project_id: row.project_id.map(ProjectId::from_uuid),
            classification: row.classification.parse()?,
            status: row.status.parse()?,
            revision: row.revision,
            capture_candidate_id: row.capture_candidate_id.map(CaptureCandidateId::from_uuid),
            resolution_change_id: row.resolution_change_id.map(ProposalId::from_uuid),
            resolution: row.resolution.map(|value| value.parse()).transpose()?,
            created_by: row.created_by,
            resolved_by: row.resolved_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            resolved_at: row.resolved_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct MemberRow {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    conflict_set_id: uuid::Uuid,
    role: String,
    knowledge_item_id: Option<uuid::Uuid>,
    knowledge_revision_id: Option<uuid::Uuid>,
    capture_candidate_id: Option<uuid::Uuid>,
    classification: String,
    similarity_permille: i32,
    reason_code: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<MemberRow> for ConflictMember {
    type Error = Error;

    fn try_from(row: MemberRow) -> Result<Self> {
        Ok(Self {
            id: ConflictMemberId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            conflict_set_id: ConflictSetId::from_uuid(row.conflict_set_id),
            role: row.role.parse()?,
            knowledge_item_id: row.knowledge_item_id.map(KnowledgeItemId::from_uuid),
            knowledge_revision_id: row
                .knowledge_revision_id
                .map(KnowledgeRevisionId::from_uuid),
            capture_candidate_id: row.capture_candidate_id.map(CaptureCandidateId::from_uuid),
            classification: row.classification.parse()?,
            similarity_permille: row.similarity_permille,
            reason_code: row.reason_code,
            created_at: row.created_at,
        })
    }
}

fn storage_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error {
        match database.code().as_deref() {
            Some("23505" | "40001") => {
                return Error::Conflict {
                    message: database.to_string(),
                };
            }
            Some("23503" | "23514" | "22001") => {
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

fn validate_new(value: &NewConflictSet<'_>) -> Result<()> {
    let knowledge = value.challenger_item_id.is_some() && value.challenger_revision_id.is_some();
    let capture = value.capture_candidate_id.is_some();
    if knowledge == capture
        || value.challenger_item_id.is_some() != value.challenger_revision_id.is_some()
    {
        return Err(Error::Invalid {
            message: "a conflict set needs exactly one Knowledge or capture challenger".to_owned(),
        });
    }
    if value.matches.is_empty() || value.matches.len() > MAX_CONFLICT_MEMBERS - 1 {
        return Err(Error::Invalid {
            message: format!(
                "a conflict set needs 1..={} exact current matches",
                MAX_CONFLICT_MEMBERS - 1
            ),
        });
    }
    if value.created_by.trim() != value.created_by
        || value.created_by.is_empty()
        || value.created_by.chars().count() > 255
    {
        return Err(Error::Invalid {
            message: "a conflict creator is 1..=255 characters without surrounding whitespace"
                .to_owned(),
        });
    }
    for matched in value.matches {
        if !(0..=1_000).contains(&matched.similarity_permille)
            || matched.reason_code.is_empty()
            || matched.reason_code.len() > 64
            || !matched
                .reason_code
                .chars()
                .enumerate()
                .all(|(index, character)| {
                    (index == 0 && character.is_ascii_lowercase())
                        || (index > 0
                            && (character.is_ascii_lowercase()
                                || character.is_ascii_digit()
                                || character == '_'))
                })
        {
            return Err(Error::Invalid {
                message: "a conflict match has an invalid score or reason code".to_owned(),
            });
        }
    }
    Ok(())
}

/// Insert one set and all immutable members atomically in the caller's tenant
/// transaction.
#[tracing::instrument(
    name = "store.knowledge_conflicts.create",
    skip_all,
    fields(tenant.id = %new.tenant_id, knowledge.conflict.id = %new.id),
    err(Display)
)]
pub async fn create(
    connection: &mut PgConnection,
    new: &NewConflictSet<'_>,
) -> Result<ConflictSet> {
    validate_new(new)?;
    sqlx::query!(
        r#"insert into knowledge_conflict_sets
              (id, tenant_id, scope_id, project_id, classification,
               capture_candidate_id, created_by)
           values ($1, $2, $3, $4, $5, $6, $7)"#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.scope_id.as_uuid(),
        new.project_id.map(|value| value.as_uuid()),
        new.classification.as_str(),
        new.capture_candidate_id.map(|value| value.as_uuid()),
        new.created_by,
    )
    .execute(&mut *connection)
    .await
    .map_err(storage_error)?;

    sqlx::query!(
        r#"insert into knowledge_conflict_members
              (id, tenant_id, conflict_set_id, role, knowledge_item_id,
               knowledge_revision_id, capture_candidate_id, classification,
               similarity_permille, reason_code)
           values ($1, $2, $3, 'challenger', $4, $5, $6, $7, 1000,
                   'proposed_statement')"#,
        ConflictMemberId::new().as_uuid(),
        new.tenant_id.as_uuid(),
        new.id.as_uuid(),
        new.challenger_item_id.map(|value| value.as_uuid()),
        new.challenger_revision_id.map(|value| value.as_uuid()),
        new.capture_candidate_id.map(|value| value.as_uuid()),
        new.classification.as_str(),
    )
    .execute(&mut *connection)
    .await
    .map_err(storage_error)?;

    for matched in new.matches {
        sqlx::query!(
            r#"insert into knowledge_conflict_members
                  (id, tenant_id, conflict_set_id, role, knowledge_item_id,
                   knowledge_revision_id, classification,
                   similarity_permille, reason_code)
               values ($1, $2, $3, 'current', $4, $5, $6, $7, $8)"#,
            ConflictMemberId::new().as_uuid(),
            new.tenant_id.as_uuid(),
            new.id.as_uuid(),
            matched.item_id.as_uuid(),
            matched.revision_id.as_uuid(),
            matched.classification.as_str(),
            matched.similarity_permille,
            matched.reason_code,
        )
        .execute(&mut *connection)
        .await
        .map_err(storage_error)?;
    }
    get(&mut *connection, new.tenant_id, new.id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("new conflict set {} disappeared", new.id),
        })
}

/// Read one tenant-owned set candidate.
pub async fn get(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConflictSetId,
) -> Result<Option<ConflictSet>> {
    let row = sqlx::query_as!(
        SetRow,
        r#"select id, tenant_id, scope_id, project_id, classification, status,
                  revision, capture_candidate_id, resolution_change_id,
                  resolution, created_by, resolved_by, created_at, updated_at,
                  resolved_at
             from knowledge_conflict_sets
            where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Immutable members in challenger-first, then stable-id order.
pub async fn members(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConflictSetId,
) -> Result<Vec<ConflictMember>> {
    let rows = sqlx::query_as!(
        MemberRow,
        r#"select id, tenant_id, conflict_set_id, role, knowledge_item_id,
                  knowledge_revision_id, capture_candidate_id, classification,
                  similarity_permille, reason_code, created_at
             from knowledge_conflict_members
            where tenant_id = $1 and conflict_set_id = $2
            order by (role = 'challenger') desc, id"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Newest-first bounded set candidates. The gateway performs member-level PDP
/// filtering before returning any row or count.
pub async fn list(
    connection: &mut PgConnection,
    tenant: TenantId,
    scope_id: Option<ScopeId>,
    project_id: Option<ProjectId>,
    status: Option<ConflictSetStatus>,
    cursor: Option<ConflictCursor>,
    limit: i64,
) -> Result<Vec<ConflictSet>> {
    let rows = sqlx::query_as!(
        SetRow,
        r#"select id, tenant_id, scope_id, project_id, classification, status,
                  revision, capture_candidate_id, resolution_change_id,
                  resolution, created_by, resolved_by, created_at, updated_at,
                  resolved_at
             from knowledge_conflict_sets
            where tenant_id = $1
              and ($2::uuid is null or scope_id = $2)
              and ($3::uuid is null or project_id = $3)
              and ($4::text is null or status = $4)
              and ($5::timestamptz is null or updated_at < $5
                   or (updated_at = $5 and id < $6))
            order by updated_at desc, id desc
            limit $7"#,
        tenant.as_uuid(),
        scope_id.map(|value| value.as_uuid()),
        project_id.map(|value| value.as_uuid()),
        status.map(ConflictSetStatus::as_str),
        cursor.map(|value| value.updated_at),
        cursor.map(|value| value.id.as_uuid()),
        limit.max(1),
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Attach an open VedaFlow resolution awaiting reviewers.
pub async fn mark_pending(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConflictSetId,
    expected_revision: i64,
    change_id: ProposalId,
    resolution: ConflictResolutionKind,
    actor: &str,
) -> Result<ConflictSet> {
    let result = sqlx::query!(
        r#"update knowledge_conflict_sets
              set status = 'pending_review', revision = revision + 1,
                  resolution_change_id = $4, resolution = $5,
                  resolved_by = $6, updated_at = clock_timestamp()
            where tenant_id = $1 and id = $2 and revision = $3 and status = 'open'"#,
        tenant.as_uuid(),
        id.as_uuid(),
        expected_revision,
        change_id.as_uuid(),
        resolution.as_str(),
        actor,
    )
    .execute(&mut *connection)
    .await
    .map_err(storage_error)?;
    if result.rows_affected() != 1 {
        return Err(Error::Conflict {
            message: format!("conflict set {id} moved before its resolution opened"),
        });
    }
    get(connection, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("conflict set {id}"),
        })
}

/// Complete an immediate or previously reviewed resolution.
pub async fn mark_resolved(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConflictSetId,
    expected_revision: i64,
    change_id: ProposalId,
    resolution: ConflictResolutionKind,
    actor: &str,
) -> Result<ConflictSet> {
    let terminal = if resolution == ConflictResolutionKind::Archive {
        ConflictSetStatus::Dismissed
    } else {
        ConflictSetStatus::Resolved
    };
    let result = sqlx::query!(
        r#"update knowledge_conflict_sets
              set status = $4, revision = revision + 1,
                  resolution_change_id = $5, resolution = $6,
                  resolved_by = $7, resolved_at = clock_timestamp(),
                  updated_at = clock_timestamp()
            where tenant_id = $1 and id = $2
              and ((status = 'open' and revision = $3)
                   or (status = 'pending_review' and revision = $3 + 1
                       and resolution_change_id = $5 and resolution = $6))"#,
        tenant.as_uuid(),
        id.as_uuid(),
        expected_revision,
        terminal.as_str(),
        change_id.as_uuid(),
        resolution.as_str(),
        actor,
    )
    .execute(&mut *connection)
    .await
    .map_err(storage_error)?;
    if result.rows_affected() != 1 {
        return Err(Error::Conflict {
            message: format!("conflict set {id} is no longer at the reviewed revision"),
        });
    }
    get(connection, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("conflict set {id}"),
        })
}

/// Re-open a pending set when its reviewed VedaFlow effect is rejected.
///
/// The rejected proposal remains immutable proposal/audit evidence. Clearing
/// its address here prevents that terminal change from pinning the operational
/// queue and lets a reviewer propose a fresh revision-aware resolution.
pub async fn reopen_after_rejection(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConflictSetId,
    change_id: ProposalId,
) -> Result<ConflictSet> {
    let result = sqlx::query!(
        r#"update knowledge_conflict_sets
              set status = 'open', revision = revision + 1,
                  resolution_change_id = null, resolution = null,
                  resolved_by = null, resolved_at = null,
                  updated_at = clock_timestamp()
            where tenant_id = $1 and id = $2 and status = 'pending_review'
              and resolution_change_id = $3"#,
        tenant.as_uuid(),
        id.as_uuid(),
        change_id.as_uuid(),
    )
    .execute(&mut *connection)
    .await
    .map_err(storage_error)?;
    if result.rows_affected() != 1 {
        return Err(Error::Conflict {
            message: format!("conflict set {id} moved before rejection was recorded"),
        });
    }
    get(connection, tenant, id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("conflict set {id}"),
        })
}
