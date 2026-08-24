//! Persistence for stable Knowledge aggregates (CPR-15, ADR-0080).
//!
//! This module is deliberately below the application boundary. It creates no
//! route, CLI command or adapter path and it performs no policy decision. The
//! governed command layer added by CPR-16 must create/evaluate a VedaFlow
//! change and decide through the PDP before calling these primitives.
//!
//! Every mutation here must run inside the caller's transaction. That is
//! load-bearing for the initial circular fact (the stable head points at its
//! first immutable revision), the deferred "every revision has a source"
//! constraint and the later VedaFlow/audit write that must commit or vanish
//! with Knowledge.
//!
//! Nothing in this module imports, reads or writes [`crate::records`]. The old
//! aggregate remains temporarily on its own runtime path until CPR-17 deletes
//! it; there is no bridge or dual write.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use synveda_types::json::canonicalise;
use synveda_types::knowledge::{
    KnowledgeItem, KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeRelation,
    KnowledgeRelationType, KnowledgeRevision, KnowledgeRevisionContent, KnowledgeSource,
    KnowledgeSourceType, normalise_knowledge_tags, validate_content_hash,
    validate_knowledge_principal, validate_knowledge_relation, validate_knowledge_revision_content,
    validate_knowledge_source,
};
use synveda_types::{
    Error, KnowledgeItemId, KnowledgeRelationId, KnowledgeRevisionId, KnowledgeSourceId, ProjectId,
    Result, ScopeId, SessionEventId, TenantId,
};
use uuid::Uuid;

/// Counter for low-level Knowledge persistence changes.
pub const KNOWLEDGE_MUTATIONS_TOTAL: &str = "synveda_knowledge_mutations_total";

/// What [`create_source`] needs.
#[derive(Debug, Clone)]
pub struct NewKnowledgeSource {
    /// Stable source id.
    pub id: KnowledgeSourceId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Scope governing disclosure of the source descriptor.
    pub scope_id: ScopeId,
    /// Source family.
    pub source_type: KnowledgeSourceType,
    /// Real event id for a session-event source.
    pub session_event_id: Option<SessionEventId>,
    /// Bounded logical locator for located source types.
    pub locator: Option<String>,
    /// External source revision, when known.
    pub source_revision: Option<String>,
    /// Source-content digest, when known.
    pub content_hash: Option<String>,
    /// Forward-compatible descriptor metadata.
    pub metadata: Value,
    /// Actor registering the source.
    pub created_by: Option<String>,
}

/// Stable aggregate-head fields supplied when an item is first created.
#[derive(Debug, Clone)]
pub struct NewKnowledgeItem {
    /// Stable item id.
    pub id: KnowledgeItemId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Governing scope.
    pub scope_id: ScopeId,
    /// Associated project, if any.
    pub project_id: Option<ProjectId>,
    /// Owning principal, if any.
    pub owner_principal_id: Option<String>,
    /// Knowledge type.
    pub knowledge_type: synveda_types::knowledge::KnowledgeType,
    /// Creation origin.
    pub origin: KnowledgeOrigin,
    /// Actor creating the item.
    pub created_by: Option<String>,
}

/// What a new immutable content revision needs.
#[derive(Debug, Clone)]
pub struct NewKnowledgeRevision {
    /// Fresh immutable revision id.
    pub id: KnowledgeRevisionId,
    /// Semantic content.
    pub content: KnowledgeRevisionContent,
    /// Actor authoring the revision.
    pub created_by: Option<String>,
}

/// What [`add_relation`] needs.
#[derive(Debug, Clone)]
pub struct NewKnowledgeRelation {
    /// Stable relation id.
    pub id: KnowledgeRelationId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Item making the relation claim.
    pub source_item_id: KnowledgeItemId,
    /// Item the relation is about.
    pub target_item_id: KnowledgeItemId,
    /// Exact source-item revision asserting the relation.
    pub asserting_revision_id: KnowledgeRevisionId,
    /// Relation vocabulary.
    pub relation_type: KnowledgeRelationType,
    /// Forward-compatible relation metadata.
    pub metadata: Value,
    /// Actor adding the relation.
    pub created_by: Option<String>,
}

/// One aggregate head joined to the immutable revision it selected during a
/// transaction-time interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeSnapshot {
    /// Stable aggregate state.
    pub item: KnowledgeItem,
    /// Exact content selected by that state.
    pub revision: KnowledgeRevision,
    /// End of the aggregate-state transaction interval; `None` is current.
    pub transaction_to: Option<DateTime<Utc>>,
}

struct ItemRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    project_id: Option<Uuid>,
    owner_principal_id: Option<String>,
    knowledge_type: String,
    origin: String,
    lifecycle_state: String,
    current_revision_id: Uuid,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tx_from: DateTime<Utc>,
}

impl TryFrom<ItemRow> for KnowledgeItem {
    type Error = Error;

    fn try_from(row: ItemRow) -> Result<Self> {
        Ok(Self {
            id: KnowledgeItemId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            project_id: row.project_id.map(ProjectId::from_uuid),
            owner_principal_id: row.owner_principal_id,
            knowledge_type: stored(&row.knowledge_type)?,
            origin: stored(&row.origin)?,
            lifecycle_state: stored(&row.lifecycle_state)?,
            current_revision_id: KnowledgeRevisionId::from_uuid(row.current_revision_id),
            created_by: row.created_by,
            updated_by: row.updated_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            transaction_from: row.tx_from,
        })
    }
}

struct RevisionRow {
    id: Uuid,
    tenant_id: Uuid,
    knowledge_item_id: Uuid,
    revision_number: i64,
    title: String,
    body_markdown: String,
    summary: String,
    tags: Vec<String>,
    sensitivity: String,
    confidence_permille: i32,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    stale_after: Option<DateTime<Utc>>,
    verification_metadata: Value,
    content_hash: String,
    metadata: Value,
    created_by: Option<String>,
    transaction_time: DateTime<Utc>,
}

impl TryFrom<RevisionRow> for KnowledgeRevision {
    type Error = Error;

    fn try_from(row: RevisionRow) -> Result<Self> {
        Ok(Self {
            id: KnowledgeRevisionId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            knowledge_item_id: KnowledgeItemId::from_uuid(row.knowledge_item_id),
            revision_number: row.revision_number,
            content: KnowledgeRevisionContent {
                title: row.title,
                body_markdown: row.body_markdown,
                summary: row.summary,
                tags: row.tags,
                sensitivity: stored(&row.sensitivity)?,
                confidence_permille: row.confidence_permille,
                valid_from: row.valid_from,
                valid_to: row.valid_to,
                stale_after: row.stale_after,
                verification_metadata: row.verification_metadata,
                metadata: row.metadata,
            },
            content_hash: row.content_hash,
            created_by: row.created_by,
            transaction_time: row.transaction_time,
        })
    }
}

struct SourceRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    source_type: String,
    session_event_id: Option<Uuid>,
    locator: Option<String>,
    source_revision: Option<String>,
    content_hash: Option<String>,
    metadata: Value,
    created_by: Option<String>,
    created_at: DateTime<Utc>,
}

impl TryFrom<SourceRow> for KnowledgeSource {
    type Error = Error;

    fn try_from(row: SourceRow) -> Result<Self> {
        Ok(Self {
            id: KnowledgeSourceId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            source_type: stored(&row.source_type)?,
            session_event_id: row.session_event_id.map(SessionEventId::from_uuid),
            locator: row.locator,
            source_revision: row.source_revision,
            content_hash: row.content_hash,
            metadata: row.metadata,
            created_by: row.created_by,
            created_at: row.created_at,
        })
    }
}

struct RelationRow {
    id: Uuid,
    tenant_id: Uuid,
    source_item_id: Uuid,
    target_item_id: Uuid,
    asserting_revision_id: Uuid,
    relation_type: String,
    metadata: Value,
    created_by: Option<String>,
    created_at: DateTime<Utc>,
}

impl TryFrom<RelationRow> for KnowledgeRelation {
    type Error = Error;

    fn try_from(row: RelationRow) -> Result<Self> {
        Ok(Self {
            id: KnowledgeRelationId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            source_item_id: KnowledgeItemId::from_uuid(row.source_item_id),
            target_item_id: KnowledgeItemId::from_uuid(row.target_item_id),
            asserting_revision_id: KnowledgeRevisionId::from_uuid(row.asserting_revision_id),
            relation_type: stored(&row.relation_type)?,
            metadata: row.metadata,
            created_by: row.created_by,
            created_at: row.created_at,
        })
    }
}

struct SnapshotRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    project_id: Option<Uuid>,
    owner_principal_id: Option<String>,
    knowledge_type: String,
    origin: String,
    lifecycle_state: String,
    current_revision_id: Uuid,
    item_created_by: Option<String>,
    updated_by: Option<String>,
    item_created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tx_from: DateTime<Utc>,
    tx_to: Option<DateTime<Utc>>,
    revision_number: i64,
    title: String,
    body_markdown: String,
    summary: String,
    tags: Vec<String>,
    sensitivity: String,
    confidence_permille: i32,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    stale_after: Option<DateTime<Utc>>,
    verification_metadata: Value,
    content_hash: String,
    metadata: Value,
    revision_created_by: Option<String>,
    transaction_time: DateTime<Utc>,
}

impl TryFrom<SnapshotRow> for KnowledgeSnapshot {
    type Error = Error;

    fn try_from(row: SnapshotRow) -> Result<Self> {
        let item = ItemRow {
            id: row.id,
            tenant_id: row.tenant_id,
            scope_id: row.scope_id,
            project_id: row.project_id,
            owner_principal_id: row.owner_principal_id,
            knowledge_type: row.knowledge_type,
            origin: row.origin,
            lifecycle_state: row.lifecycle_state,
            current_revision_id: row.current_revision_id,
            created_by: row.item_created_by,
            updated_by: row.updated_by,
            created_at: row.item_created_at,
            updated_at: row.updated_at,
            tx_from: row.tx_from,
        }
        .try_into()?;
        let revision = RevisionRow {
            id: row.current_revision_id,
            tenant_id: row.tenant_id,
            knowledge_item_id: row.id,
            revision_number: row.revision_number,
            title: row.title,
            body_markdown: row.body_markdown,
            summary: row.summary,
            tags: row.tags,
            sensitivity: row.sensitivity,
            confidence_permille: row.confidence_permille,
            valid_from: row.valid_from,
            valid_to: row.valid_to,
            stale_after: row.stale_after,
            verification_metadata: row.verification_metadata,
            content_hash: row.content_hash,
            metadata: row.metadata,
            created_by: row.revision_created_by,
            transaction_time: row.transaction_time,
        }
        .try_into()?;
        Ok(Self {
            item,
            revision,
            transaction_to: row.tx_to,
        })
    }
}

fn stored<T: std::str::FromStr<Err = Error>>(value: &str) -> Result<T> {
    value.parse().map_err(|err| Error::Internal {
        message: format!("stored value outside Knowledge vocabulary: {err}"),
    })
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        match db.code().as_deref() {
            Some("23505" | "40001") => {
                return Error::Conflict {
                    message: db.to_string(),
                };
            }
            Some("23503" | "23514" | "22001") => {
                return Error::Invalid {
                    message: db.to_string(),
                };
            }
            Some("42501") => return crate::rls::backstop_error(db),
            _ => {}
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

fn canonical_content(content: &KnowledgeRevisionContent) -> Value {
    canonicalise(&serde_json::json!({
        "title": content.title,
        "body_markdown": content.body_markdown,
        "summary": content.summary,
        "tags": content.tags,
        "sensitivity": content.sensitivity,
        "confidence_permille": content.confidence_permille,
        "valid_from": content.valid_from.to_rfc3339_opts(SecondsFormat::Micros, true),
        "valid_to": content.valid_to.map(|value| value.to_rfc3339_opts(SecondsFormat::Micros, true)),
        "stale_after": content.stale_after.map(|value| value.to_rfc3339_opts(SecondsFormat::Micros, true)),
        "verification_metadata": content.verification_metadata,
        "metadata": content.metadata,
    }))
}

/// Computes the canonical BLAKE3-256 digest for semantic revision content.
///
/// Actor, ids and transaction time are deliberately excluded: two separately
/// authored copies of one semantic revision have one content hash.
#[must_use]
pub fn revision_content_hash(content: &KnowledgeRevisionContent) -> String {
    blake3::hash(canonical_content(content).to_string().as_bytes())
        .to_hex()
        .to_string()
}

fn canonical_revision(new: &NewKnowledgeRevision) -> Result<NewKnowledgeRevision> {
    validate_knowledge_principal(new.created_by.as_deref(), "revision author")?;
    let mut canonical = new.clone();
    canonical.content.tags = normalise_knowledge_tags(&canonical.content.tags)?;
    validate_knowledge_revision_content(&canonical.content)?;
    Ok(canonical)
}

fn validate_source_ids(source_ids: &[KnowledgeSourceId]) -> Result<Vec<Uuid>> {
    if source_ids.is_empty() {
        return Err(Error::Invalid {
            message: "a Knowledge revision requires at least one provenance source".to_owned(),
        });
    }
    let mut seen = HashSet::with_capacity(source_ids.len());
    let mut ids = Vec::with_capacity(source_ids.len());
    for source in source_ids {
        if !seen.insert(*source) {
            return Err(Error::Invalid {
                message: format!("Knowledge source {source} is linked more than once"),
            });
        }
        ids.push(source.as_uuid());
    }
    Ok(ids)
}

/// Creates one immutable, independently governed provenance descriptor.
///
/// Must run in the caller's transaction. A session-event source is constrained
/// to a real event in this tenant; other source kinds retain stable logical
/// locators and never copy payload content.
#[tracing::instrument(
    name = "store.knowledge.create_source",
    skip_all,
    fields(tenant.id = %new.tenant_id, knowledge.source.id = %new.id, knowledge.source.type = %new.source_type),
    err(Display)
)]
pub async fn create_source(
    conn: &mut PgConnection,
    new: &NewKnowledgeSource,
) -> Result<KnowledgeSource> {
    validate_knowledge_principal(new.created_by.as_deref(), "source creator")?;
    validate_knowledge_source(
        new.source_type,
        new.session_event_id,
        new.locator.as_deref(),
        new.source_revision.as_deref(),
        new.content_hash.as_deref(),
        &new.metadata,
    )?;
    if let Some(hash) = &new.content_hash {
        validate_content_hash(hash)?;
    }

    let row = sqlx::query_as!(
        SourceRow,
        r#"
        insert into knowledge_sources
            (id, tenant_id, scope_id, source_type, session_event_id, locator,
             source_revision, content_hash, metadata, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        returning id, tenant_id, scope_id, source_type, session_event_id,
                  locator, source_revision, content_hash, metadata,
                  created_by, created_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.scope_id.as_uuid(),
        new.source_type.as_str(),
        new.session_event_id.map(|id| id.as_uuid()) as Option<Uuid>,
        new.locator.as_deref() as Option<&str>,
        new.source_revision.as_deref() as Option<&str>,
        new.content_hash.as_deref() as Option<&str>,
        new.metadata,
        new.created_by.as_deref() as Option<&str>,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        KNOWLEDGE_MUTATIONS_TOTAL,
        "aggregate" => "source",
        "operation" => "create"
    )
    .increment(1);
    row.try_into()
}

async fn insert_revision(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    revision_number: i64,
    new: &NewKnowledgeRevision,
) -> Result<KnowledgeRevision> {
    let new = canonical_revision(new)?;
    let hash = revision_content_hash(&new.content);
    let row = sqlx::query_as!(
        RevisionRow,
        r#"
        insert into knowledge_revisions
            (id, tenant_id, knowledge_item_id, revision_number, title,
             body_markdown, summary, tags, sensitivity, confidence_permille,
             valid_from, valid_to, stale_after, verification_metadata,
             content_hash, metadata, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17)
        returning id, tenant_id, knowledge_item_id, revision_number, title,
                  body_markdown, summary, tags as "tags!: Vec<String>",
                  sensitivity, confidence_permille, valid_from, valid_to,
                  stale_after, verification_metadata, content_hash, metadata,
                  created_by, transaction_time
        "#,
        new.id.as_uuid(),
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        revision_number,
        new.content.title,
        new.content.body_markdown,
        new.content.summary,
        &new.content.tags,
        new.content.sensitivity.as_str(),
        new.content.confidence_permille,
        new.content.valid_from,
        new.content.valid_to,
        new.content.stale_after,
        new.content.verification_metadata,
        hash,
        new.content.metadata,
        new.created_by.as_deref() as Option<&str>,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

async fn link_sources(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    revision_id: KnowledgeRevisionId,
    source_ids: &[KnowledgeSourceId],
) -> Result<()> {
    let source_ids = validate_source_ids(source_ids)?;
    sqlx::query!(
        r#"
        insert into knowledge_revision_sources
            (tenant_id, knowledge_revision_id, knowledge_source_id, ordinal)
        select $1, $2, source_id, ordinal::integer
        from unnest($3::uuid[]) with ordinality as sources(source_id, ordinal)
        "#,
        tenant_id.as_uuid(),
        revision_id.as_uuid(),
        &source_ids,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Creates a stable item and its first immutable revision.
///
/// `source_ids` must name at least one already-created source in the same
/// tenant. The database checks the head/revision cycle and source requirement
/// at transaction commit, so this function must run in a transaction that the
/// caller commits only after its VedaFlow and audit work is also complete.
#[tracing::instrument(
    name = "store.knowledge.create_item",
    skip_all,
    fields(tenant.id = %new.tenant_id, knowledge.item.id = %new.id, knowledge.revision.id = %revision.id),
    err(Display)
)]
pub async fn create_item(
    conn: &mut PgConnection,
    new: &NewKnowledgeItem,
    revision: &NewKnowledgeRevision,
    source_ids: &[KnowledgeSourceId],
) -> Result<KnowledgeSnapshot> {
    validate_knowledge_principal(new.owner_principal_id.as_deref(), "Knowledge owner")?;
    validate_knowledge_principal(new.created_by.as_deref(), "Knowledge creator")?;
    validate_source_ids(source_ids)?;
    let revision = canonical_revision(revision)?;

    let item = sqlx::query_as!(
        ItemRow,
        r#"
        insert into knowledge_items
            (id, tenant_id, scope_id, project_id, owner_principal_id,
             knowledge_type, origin, lifecycle_state, current_revision_id,
             created_by, updated_by)
        values ($1, $2, $3, $4, $5, $6, $7, 'active', $8, $9, $9)
        returning id, tenant_id, scope_id, project_id, owner_principal_id,
                  knowledge_type, origin, lifecycle_state, current_revision_id,
                  created_by, updated_by, created_at, updated_at, tx_from
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.scope_id.as_uuid(),
        new.project_id.map(|id| id.as_uuid()) as Option<Uuid>,
        new.owner_principal_id.as_deref() as Option<&str>,
        new.knowledge_type.as_str(),
        new.origin.as_str(),
        revision.id.as_uuid(),
        new.created_by.as_deref() as Option<&str>,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    let stored_revision = insert_revision(&mut *conn, new.tenant_id, new.id, 1, &revision).await?;
    link_sources(&mut *conn, new.tenant_id, revision.id, source_ids).await?;

    metrics::counter!(
        KNOWLEDGE_MUTATIONS_TOTAL,
        "aggregate" => "item",
        "operation" => "create"
    )
    .increment(1);
    Ok(KnowledgeSnapshot {
        item: item.try_into()?,
        revision: stored_revision,
        transaction_to: None,
    })
}

/// Appends an immutable revision and moves the stable head to it.
///
/// `expected_revision_id` is the low-level stale-write precondition. The
/// governed command layer carries it from the public request and returns a
/// conflict without opening/applying a change when it no longer matches.
/// Returns `None` when the item does not exist in this tenant.
#[tracing::instrument(
    name = "store.knowledge.append_revision",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.item.id = %item_id, knowledge.revision.id = %revision.id),
    err(Display)
)]
pub async fn append_revision(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    expected_revision_id: KnowledgeRevisionId,
    revision: &NewKnowledgeRevision,
    source_ids: &[KnowledgeSourceId],
) -> Result<Option<KnowledgeSnapshot>> {
    validate_source_ids(source_ids)?;
    let revision = canonical_revision(revision)?;
    let head = sqlx::query!(
        r#"
        select item.current_revision_id as "current_revision_id!",
               current.revision_number as "revision_number!"
        from knowledge_items item
        join knowledge_revisions current
          on current.tenant_id = item.tenant_id
         and current.knowledge_item_id = item.id
         and current.id = item.current_revision_id
        where item.tenant_id = $1 and item.id = $2
        for update of item
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(head) = head else { return Ok(None) };
    if head.current_revision_id != expected_revision_id.as_uuid() {
        return Err(Error::Conflict {
            message: format!(
                "Knowledge item {item_id} is at revision {}, expected {expected_revision_id}",
                head.current_revision_id
            ),
        });
    }

    insert_revision(
        &mut *conn,
        tenant_id,
        item_id,
        head.revision_number + 1,
        &revision,
    )
    .await?;
    link_sources(&mut *conn, tenant_id, revision.id, source_ids).await?;
    sqlx::query!(
        r#"
        update knowledge_items
        set current_revision_id = $3, updated_by = $4
        where tenant_id = $1 and id = $2 and current_revision_id = $5
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        revision.id.as_uuid(),
        revision.created_by.as_deref() as Option<&str>,
        expected_revision_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        KNOWLEDGE_MUTATIONS_TOTAL,
        "aggregate" => "item",
        "operation" => "revise"
    )
    .increment(1);
    current(&mut *conn, tenant_id, item_id).await
}

/// Changes only aggregate lifecycle state, preserving content and archiving
/// the prior head state in transaction time.
///
/// This is a persistence primitive, not the lifecycle state machine. CPR-16's
/// governed commands decide which transitions are valid. Returns `None` when
/// the item does not exist in this tenant.
#[tracing::instrument(
    name = "store.knowledge.set_lifecycle",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.item.id = %item_id, knowledge.lifecycle = %lifecycle),
    err(Display)
)]
pub async fn set_lifecycle(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    expected_revision_id: KnowledgeRevisionId,
    lifecycle: KnowledgeLifecycleState,
    updated_by: Option<&str>,
) -> Result<Option<KnowledgeSnapshot>> {
    validate_knowledge_principal(updated_by, "Knowledge updater")?;
    let head = sqlx::query!(
        r#"
        select current_revision_id as "current_revision_id!",
               lifecycle_state as "lifecycle_state!"
        from knowledge_items
        where tenant_id = $1 and id = $2
        for update
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(head) = head else { return Ok(None) };
    if head.current_revision_id != expected_revision_id.as_uuid() {
        return Err(Error::Conflict {
            message: format!(
                "Knowledge item {item_id} is at revision {}, expected {expected_revision_id}",
                head.current_revision_id
            ),
        });
    }
    if head.lifecycle_state == lifecycle.as_str() {
        return Err(Error::Invalid {
            message: format!("Knowledge item {item_id} is already {lifecycle}"),
        });
    }
    sqlx::query!(
        r#"
        update knowledge_items
        set lifecycle_state = $3, updated_by = $4
        where tenant_id = $1 and id = $2 and current_revision_id = $5
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        lifecycle.as_str(),
        updated_by as Option<&str>,
        expected_revision_id.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        KNOWLEDGE_MUTATIONS_TOTAL,
        "aggregate" => "item",
        "operation" => "lifecycle"
    )
    .increment(1);
    current(&mut *conn, tenant_id, item_id).await
}

/// Reads the exact current aggregate projection.
#[tracing::instrument(
    name = "store.knowledge.current",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.item.id = %item_id),
    err(Display)
)]
pub async fn current(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
) -> Result<Option<KnowledgeSnapshot>> {
    let row = sqlx::query_as!(
        SnapshotRow,
        r#"
        select current.id as "id!",
               current.tenant_id as "tenant_id!",
               current.scope_id as "scope_id!",
               current.project_id,
               current.owner_principal_id,
               current.knowledge_type as "knowledge_type!",
               current.origin as "origin!",
               current.lifecycle_state as "lifecycle_state!",
               current.current_revision_id as "current_revision_id!",
               current.created_by as item_created_by,
               current.updated_by,
               current.created_at as "item_created_at!",
               current.updated_at as "updated_at!",
               current.tx_from as "tx_from!",
               null::timestamptz as tx_to,
               current.revision_number as "revision_number!",
               current.title as "title!",
               current.body_markdown as "body_markdown!",
               current.summary as "summary!",
               current.tags as "tags!: Vec<String>",
               current.sensitivity as "sensitivity!",
               current.confidence_permille as "confidence_permille!",
               current.valid_from as "valid_from!",
               current.valid_to,
               current.stale_after,
               current.verification_metadata as "verification_metadata!",
               current.content_hash as "content_hash!",
               current.metadata as "metadata!",
               current.revision_created_by,
               current.transaction_time as "transaction_time!"
        from knowledge_current current
        where current.tenant_id = $1 and current.id = $2
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Reads the aggregate state the database held at `as_known_at`.
///
/// Valid time is not applied here: this answers transaction history. A later
/// query combines the returned revision's valid interval with the caller's
/// explicit valid-time instant.
#[tracing::instrument(
    name = "store.knowledge.as_known_at",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.item.id = %item_id, knowledge.as_known_at = %as_known_at),
    err(Display)
)]
pub async fn as_known_at(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    as_known_at: DateTime<Utc>,
) -> Result<Option<KnowledgeSnapshot>> {
    let row = sqlx::query_as!(
        SnapshotRow,
        r#"
        select head.id as "id!",
               head.tenant_id as "tenant_id!",
               head.scope_id as "scope_id!",
               head.project_id,
               head.owner_principal_id,
               head.knowledge_type as "knowledge_type!",
               head.origin as "origin!",
               head.lifecycle_state as "lifecycle_state!",
               head.current_revision_id as "current_revision_id!",
               head.created_by as item_created_by,
               head.updated_by,
               head.created_at as "item_created_at!",
               head.updated_at as "updated_at!",
               head.tx_from as "tx_from!",
               head.tx_to,
               revision.revision_number as "revision_number!",
               revision.title as "title!",
               revision.body_markdown as "body_markdown!",
               revision.summary as "summary!",
               revision.tags as "tags!: Vec<String>",
               revision.sensitivity as "sensitivity!",
               revision.confidence_permille as "confidence_permille!",
               revision.valid_from as "valid_from!",
               revision.valid_to,
               revision.stale_after,
               revision.verification_metadata as "verification_metadata!",
               revision.content_hash as "content_hash!",
               revision.metadata as "metadata!",
               revision.created_by as revision_created_by,
               revision.transaction_time as "transaction_time!"
        from knowledge_item_versions head
        join knowledge_revisions revision
          on revision.tenant_id = head.tenant_id
         and revision.knowledge_item_id = head.id
         and revision.id = head.current_revision_id
        where head.tenant_id = $1
          and head.id = $2
          and head.tx_from <= $3
          and (head.tx_to is null or $3 < head.tx_to)
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        as_known_at,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists every immutable revision, oldest first.
#[tracing::instrument(
    name = "store.knowledge.revisions",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.item.id = %item_id),
    err(Display)
)]
pub async fn revisions(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
) -> Result<Vec<KnowledgeRevision>> {
    let rows = sqlx::query_as!(
        RevisionRow,
        r#"
        select id, tenant_id, knowledge_item_id, revision_number, title,
               body_markdown, summary, tags as "tags!: Vec<String>",
               sensitivity, confidence_permille, valid_from, valid_to,
               stale_after, verification_metadata, content_hash, metadata,
               created_by, transaction_time
        from knowledge_revisions
        where tenant_id = $1 and knowledge_item_id = $2
        order by revision_number
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Reads one exact immutable revision of one stable item.
///
/// This is a storage primitive, not an authority check. Context inspection
/// uses the retained address and then decides `KnowledgeRead` at this exact
/// revision's sensitivity before exposing any field (CPR-20, ADR-0084).
#[tracing::instrument(
    name = "store.knowledge.revision",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.item.id = %item_id, knowledge.revision.id = %revision_id),
    err(Display)
)]
pub async fn revision(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    revision_id: KnowledgeRevisionId,
) -> Result<Option<KnowledgeRevision>> {
    let row = sqlx::query_as!(
        RevisionRow,
        r#"
        select id, tenant_id, knowledge_item_id, revision_number, title,
               body_markdown, summary, tags as "tags!: Vec<String>",
               sensitivity, confidence_permille, valid_from, valid_to,
               stale_after, verification_metadata, content_hash, metadata,
               created_by, transaction_time
        from knowledge_revisions
        where tenant_id = $1 and knowledge_item_id = $2 and id = $3
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
        revision_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Lists a revision's sources whose own governed scopes the caller was
/// already authorised to inspect.
///
/// This is intentionally not `sources(revision_id)`: item visibility does not
/// imply source visibility. `visible_scope_ids` is the PDP's answer, not a set
/// inferred by this storage layer. An empty set returns an empty list.
#[tracing::instrument(
    name = "store.knowledge.visible_sources",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.revision.id = %revision_id, knowledge.visible_scopes = visible_scope_ids.len()),
    err(Display)
)]
pub async fn visible_sources(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    revision_id: KnowledgeRevisionId,
    visible_scope_ids: &[ScopeId],
) -> Result<Vec<KnowledgeSource>> {
    if visible_scope_ids.is_empty() {
        return Ok(Vec::new());
    }
    let scope_ids: Vec<Uuid> = visible_scope_ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query_as!(
        SourceRow,
        r#"
        select source.id, source.tenant_id, source.scope_id, source.source_type,
               source.session_event_id, source.locator, source.source_revision,
               source.content_hash, source.metadata, source.created_by,
               source.created_at
        from knowledge_revision_sources link
        join knowledge_sources source
          on source.tenant_id = link.tenant_id
         and source.id = link.knowledge_source_id
        where link.tenant_id = $1
          and link.knowledge_revision_id = $2
          and source.scope_id = any($3::uuid[])
        order by link.ordinal
        "#,
        tenant_id.as_uuid(),
        revision_id.as_uuid(),
        &scope_ids,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// Returns the exact ordered source ids attached to a revision.
///
/// This is an internal mutation primitive, not a disclosure surface: the
/// governed command layer uses it to carry provenance into verification and
/// merge revisions inside the same authorised transaction. Public reads must
/// continue to use [`visible_sources`], which applies the independently
/// decided source-scope set.
#[tracing::instrument(
    name = "store.knowledge.revision_source_ids",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.revision.id = %revision_id),
    err(Display)
)]
pub async fn revision_source_ids(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    revision_id: KnowledgeRevisionId,
) -> Result<Vec<KnowledgeSourceId>> {
    let rows = sqlx::query_scalar!(
        r#"
        select knowledge_source_id
        from knowledge_revision_sources
        where tenant_id = $1 and knowledge_revision_id = $2
        order by ordinal
        "#,
        tenant_id.as_uuid(),
        revision_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(KnowledgeSourceId::from_uuid).collect())
}

/// Adds an immutable relation claim.
#[tracing::instrument(
    name = "store.knowledge.add_relation",
    skip_all,
    fields(tenant.id = %new.tenant_id, knowledge.relation.id = %new.id, knowledge.relation.type = %new.relation_type),
    err(Display)
)]
pub async fn add_relation(
    conn: &mut PgConnection,
    new: &NewKnowledgeRelation,
) -> Result<KnowledgeRelation> {
    validate_knowledge_relation(new.source_item_id, new.target_item_id, &new.metadata)?;
    validate_knowledge_principal(new.created_by.as_deref(), "relation creator")?;
    let row = sqlx::query_as!(
        RelationRow,
        r#"
        insert into knowledge_relations
            (id, tenant_id, source_item_id, target_item_id,
             asserting_revision_id, relation_type, metadata, created_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8)
        returning id, tenant_id, source_item_id, target_item_id,
                  asserting_revision_id, relation_type, metadata,
                  created_by, created_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.source_item_id.as_uuid(),
        new.target_item_id.as_uuid(),
        new.asserting_revision_id.as_uuid(),
        new.relation_type.as_str(),
        new.metadata,
        new.created_by.as_deref() as Option<&str>,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        KNOWLEDGE_MUTATIONS_TOTAL,
        "aggregate" => "relation",
        "operation" => "create"
    )
    .increment(1);
    row.try_into()
}

/// Lists all visible relation claims touching an item, oldest first.
///
/// The application layer must decide both endpoint items before exposing the
/// returned rows. This tenant-filtered store query is not an authorisation
/// decision.
#[tracing::instrument(
    name = "store.knowledge.relations",
    skip_all,
    fields(tenant.id = %tenant_id, knowledge.item.id = %item_id),
    err(Display)
)]
pub async fn relations(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
) -> Result<Vec<KnowledgeRelation>> {
    let rows = sqlx::query_as!(
        RelationRow,
        r#"
        select id, tenant_id, source_item_id, target_item_id,
               asserting_revision_id, relation_type, metadata,
               created_by, created_at
        from knowledge_relations
        where tenant_id = $1
          and (source_item_id = $2 or target_item_id = $2)
        order by created_at, id
        "#,
        tenant_id.as_uuid(),
        item_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::Sensitivity;

    fn content(metadata: Value) -> KnowledgeRevisionContent {
        KnowledgeRevisionContent {
            title: "Trace propagation".to_owned(),
            body_markdown: "Use `traceparent`.".to_owned(),
            summary: "Use traceparent on public requests.".to_owned(),
            tags: vec!["http".to_owned(), "observability".to_owned()],
            sensitivity: Sensitivity::Internal,
            confidence_permille: 950,
            valid_from: Utc::now(),
            valid_to: None,
            stale_after: None,
            verification_metadata: serde_json::json!({"method": "review"}),
            metadata,
        }
    }

    #[test]
    fn content_hash_is_stable_across_metadata_key_order_and_not_across_content() {
        let a = content(serde_json::json!({"b": 2, "a": {"z": 1, "x": 2}}));
        let mut b = a.clone();
        b.metadata = serde_json::json!({"a": {"x": 2, "z": 1}, "b": 2});
        assert_eq!(revision_content_hash(&a), revision_content_hash(&b));
        assert_eq!(revision_content_hash(&a).len(), 64);

        let mut changed = b;
        changed.summary.push_str(" Everywhere.");
        assert_ne!(revision_content_hash(&a), revision_content_hash(&changed));
    }

    #[test]
    fn duplicate_source_links_are_refused_before_sql() {
        let source = KnowledgeSourceId::new();
        let error = validate_source_ids(&[source, source]).expect_err("duplicate source");
        assert!(error.to_string().contains("more than once"));
    }
}
