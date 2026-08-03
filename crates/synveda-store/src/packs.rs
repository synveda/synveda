//! Context-pack drafts and the chunk mapping (PRMT-2, ADR-0050).
//!
//! Two halves, and they are different kinds of thing.
//!
//! The **draft** half is migration 0029's shape with one extra level:
//! `context_packs` is a bundle's identity and `context_pack_documents` is
//! what is in it, one row per `(tenant, scope, pack, document)`. Neither is
//! a version history — every write also puts a content-addressed object,
//! and the versions a channel has served are its first-parent line
//! (ADR-0050 decision 1).
//!
//! The **chunk mapping** half is the one thing the read path needs and
//! `records` cannot say: which document a pinned row was cut from, and at
//! which address. A pack's published content composes as ordinary pinned
//! records (decision 2), so retrieval, tiering, recall and both exemptions
//! are inherited — but *whether* a chunk is published is decided by
//! comparing its `document_hash` against what the scope's
//! `context-pack/published` tree names (decision 3), and this is the table
//! that holds the left-hand side of that comparison.
//!
//! This module stores; it decides nothing. `ContextPackWrite` at the scope
//! is the seam above it, `ContextPackRead` is the seam the read path
//! crosses, the object address is computed by
//! `synveda_vedaflow::ContextPackAsset` before a call gets here, and
//! whether a draft may cross the trust boundary is the approval matrix's
//! arithmetic.
//!
//! Tenant-scoped (forced RLS, ADR-0009): reach these tables inside
//! [`crate::rls::begin_tenant_tx`].

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{
    ContextPackName, DocumentName, Error, IdentityId, RecordId, Result, ScopeId, Sensitivity,
    TenantId,
};

/// A pack's draft row as stored — the bundle's identity, without its
/// documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPack {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// Its name — the identifier a scope's override is expressed in.
    pub name: ContextPackName,
    /// One line, read in a listing and at review.
    pub description: String,
    /// When it was first authored.
    pub created_at: DateTime<Utc>,
    /// Who first authored it.
    pub created_by: IdentityId,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// Who last changed it.
    pub updated_by: IdentityId,
}

/// One document of a pack as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDocument {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// The bundle it belongs to.
    pub pack_name: ContextPackName,
    /// Its name within the bundle.
    pub document_name: DocumentName,
    /// One line, and what the index tier renders (ADR-0050 decision 10).
    pub title: String,
    /// Its classification. Never `restricted` — the column's CHECK says so,
    /// because nothing in the product can mint that tier for an authored
    /// asset (decision 12).
    pub sensitivity: Sensitivity,
    /// The address of exactly these bytes, so a caller can compare a draft
    /// against what a channel published without re-hashing it.
    pub object_hash: [u8; 32],
    /// How many chunks it cut into.
    pub chunks: u32,
    /// When it was first authored.
    pub created_at: DateTime<Utc>,
    /// Who first authored it.
    pub created_by: IdentityId,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// Who last changed it.
    pub updated_by: IdentityId,
}

/// One chunk's mapping: the record, and the document version it was cut
/// from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackChunk {
    /// The `records` row that holds this chunk's text.
    pub record_id: RecordId,
    /// The scope that authored the document.
    pub scope_id: ScopeId,
    /// The bundle.
    pub pack_name: ContextPackName,
    /// The document.
    pub document_name: DocumentName,
    /// The document's title, as the index tier renders it (decision 10).
    pub title: String,
    /// The document address this chunk was cut from — the left-hand side of
    /// decision 3's comparison.
    pub document_hash: [u8; 32],
    /// Its position in the document, from zero.
    pub ordinal: u32,
    /// The nearest enclosing heading, when the document had one.
    pub heading: Option<String>,
}

/// A pack draft write, as the caller describes it.
#[derive(Debug, Clone)]
pub struct NewPack<'a> {
    /// Where it is authored.
    pub scope_id: ScopeId,
    /// Its name.
    pub name: &'a ContextPackName,
    /// One line.
    pub description: &'a str,
    /// Who is authoring.
    pub author: IdentityId,
}

/// A document write, as the caller describes it.
#[derive(Debug, Clone)]
pub struct NewDocument<'a> {
    /// Where it is authored.
    pub scope_id: ScopeId,
    /// Its bundle, which must already exist.
    pub pack_name: &'a ContextPackName,
    /// Its name.
    pub document_name: &'a DocumentName,
    /// One line.
    pub title: &'a str,
    /// Its classification.
    pub sensitivity: Sensitivity,
    /// The address of the object the caller has already written.
    pub object_hash: [u8; 32],
    /// How many chunks it cut into.
    pub chunks: u32,
    /// Who is authoring.
    pub author: IdentityId,
}

/// A chunk mapping write, as the caller describes it.
#[derive(Debug, Clone)]
pub struct NewChunk<'a> {
    /// The record the caller has already inserted, with its embedding.
    pub record_id: RecordId,
    /// Where the document was authored.
    pub scope_id: ScopeId,
    /// Its bundle.
    pub pack_name: &'a ContextPackName,
    /// Its document.
    pub document_name: &'a DocumentName,
    /// Its document's title.
    pub title: &'a str,
    /// The document address this chunk was cut from.
    pub document_hash: [u8; 32],
    /// Its position in the document.
    pub ordinal: u32,
    /// The nearest enclosing heading.
    pub heading: Option<&'a str>,
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant, a pack row that was
        // never written, a record that does not exist, or — the one that
        // matters — an object address whose bytes were never stored.
        if db.code().as_deref() == Some("23503") {
            return Error::Invalid {
                message: format!(
                    "a context pack row must name a tenant, pack, record and object \
                     this tenant holds: {db}"
                ),
            };
        }
        // 23514 check_violation: a name, title or tier the column refuses.
        // `restricted` lands here, which is the structural half of ADR-0050
        // decision 12.
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

/// The stored pack shape, mapped on the way out.
struct PackRow {
    scope_id: uuid::Uuid,
    name: String,
    description: String,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

impl TryFrom<PackRow> for StoredPack {
    type Error = Error;

    fn try_from(row: PackRow) -> Result<Self> {
        Ok(StoredPack {
            scope_id: ScopeId::from_uuid(row.scope_id),
            name: row.name.parse()?,
            description: row.description,
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    }
}

/// The stored document shape, mapped on the way out.
struct DocumentRow {
    scope_id: uuid::Uuid,
    pack_name: String,
    document_name: String,
    title: String,
    sensitivity: String,
    object_hash: Vec<u8>,
    chunks: i32,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

impl TryFrom<DocumentRow> for StoredDocument {
    type Error = Error;

    fn try_from(row: DocumentRow) -> Result<Self> {
        // Every column's CHECK mirrors a vocabulary this crate can parse, so
        // a value outside one means code and schema have drifted. Say so
        // rather than shrug — the role_bindings discipline (ADR-0015).
        let object_hash = address(&row.object_hash, &row.document_name)?;
        Ok(StoredDocument {
            scope_id: ScopeId::from_uuid(row.scope_id),
            pack_name: row.pack_name.parse()?,
            document_name: row.document_name.parse()?,
            title: row.title,
            sensitivity: row.sensitivity.parse()?,
            object_hash,
            chunks: u32::try_from(row.chunks).unwrap_or(0),
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    }
}

/// A 32-byte address out of a `bytea`, or the drift error.
fn address(bytes: &[u8], what: &str) -> Result<[u8; 32]> {
    <[u8; 32]>::try_from(bytes).map_err(|_| Error::Internal {
        message: format!("context pack {what:?} has an object address that is not 32 bytes"),
    })
}

/// The stored chunk shape, mapped on the way out.
struct ChunkRow {
    record_id: uuid::Uuid,
    scope_id: uuid::Uuid,
    pack_name: String,
    document_name: String,
    title: String,
    document_hash: Vec<u8>,
    ordinal: i32,
    heading: Option<String>,
}

impl TryFrom<ChunkRow> for PackChunk {
    type Error = Error;

    fn try_from(row: ChunkRow) -> Result<Self> {
        let document_hash = address(&row.document_hash, &row.document_name)?;
        Ok(PackChunk {
            record_id: RecordId::from_uuid(row.record_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            pack_name: row.pack_name.parse()?,
            document_name: row.document_name.parse()?,
            title: row.title,
            document_hash,
            ordinal: u32::try_from(row.ordinal).unwrap_or(0),
            heading: row.heading,
        })
    }
}

// ── Drafts ──────────────────────────────────────────────────────────────

/// Writes a pack's draft row: creates it, or replaces the description of
/// the one that is there.
///
/// An overwrite is the authoring act, not a conflict. What cannot change is
/// its identity: migration 0030's trigger refuses a moved scope or a
/// renamed pack, so this statement's `on conflict` can only ever rewrite
/// the description.
#[tracing::instrument(
    name = "store.packs.upsert_pack",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %new.scope_id, pack.name = %new.name),
    err(Display)
)]
pub async fn upsert_pack<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewPack<'_>,
) -> Result<StoredPack> {
    let row = sqlx::query_as!(
        PackRow,
        r#"insert into context_packs
               (tenant_id, scope_id, name, description, created_by, updated_by)
           values ($1, $2, $3, $4, $5, $5)
           on conflict (tenant_id, scope_id, name) do update
               set description = excluded.description,
                   updated_at  = now(),
                   updated_by  = excluded.updated_by
           returning scope_id, name, description, created_at, created_by,
                     updated_at, updated_by"#,
        tenant.as_uuid(),
        new.scope_id.as_uuid(),
        new.name.as_str(),
        new.description,
        new.author.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredPack::try_from(row)
}

/// Writes one document of a pack: creates it, or replaces the content of
/// the one that is there.
#[tracing::instrument(
    name = "store.packs.upsert_document",
    skip_all,
    fields(
        tenant.id = %tenant,
        scope.id = %new.scope_id,
        pack.name = %new.pack_name,
        document.name = %new.document_name,
    ),
    err(Display)
)]
pub async fn upsert_document<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewDocument<'_>,
) -> Result<StoredDocument> {
    let row = sqlx::query_as!(
        DocumentRow,
        r#"insert into context_pack_documents
               (tenant_id, scope_id, pack_name, document_name, title, sensitivity,
                object_hash, chunks, created_by, updated_by)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
           on conflict (tenant_id, scope_id, pack_name, document_name) do update
               set title       = excluded.title,
                   sensitivity = excluded.sensitivity,
                   object_hash = excluded.object_hash,
                   chunks      = excluded.chunks,
                   updated_at  = now(),
                   updated_by  = excluded.updated_by
           returning scope_id, pack_name, document_name, title, sensitivity,
                     object_hash, chunks, created_at, created_by, updated_at, updated_by"#,
        tenant.as_uuid(),
        new.scope_id.as_uuid(),
        new.pack_name.as_str(),
        new.document_name.as_str(),
        new.title,
        new.sensitivity.as_str(),
        &new.object_hash[..],
        i32::try_from(new.chunks).unwrap_or(i32::MAX),
        new.author.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredDocument::try_from(row)
}

/// One pack's draft row, or `None` when that scope has never authored that
/// name.
#[tracing::instrument(
    name = "store.packs.read_pack",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, pack.name = %name),
    err(Display)
)]
pub async fn read_pack<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope: ScopeId,
    name: &ContextPackName,
) -> Result<Option<StoredPack>> {
    let row = sqlx::query_as!(
        PackRow,
        r#"select scope_id, name, description, created_at, created_by,
                  updated_at, updated_by
           from context_packs
           where tenant_id = $1 and scope_id = $2 and name = $3"#,
        tenant.as_uuid(),
        scope.as_uuid(),
        name.as_str(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredPack::try_from).transpose()
}

/// Every pack drafted at one scope, in name order.
#[tracing::instrument(
    name = "store.packs.list_packs",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, packs = tracing::field::Empty),
    err(Display)
)]
pub async fn list_packs<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope: ScopeId,
) -> Result<Vec<StoredPack>> {
    let rows = sqlx::query_as!(
        PackRow,
        r#"select scope_id, name, description, created_at, created_by,
                  updated_at, updated_by
           from context_packs
           where tenant_id = $1 and scope_id = $2
           order by name"#,
        tenant.as_uuid(),
        scope.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("packs", rows.len());
    rows.into_iter().map(StoredPack::try_from).collect()
}

/// Every document of one pack, in name order.
#[tracing::instrument(
    name = "store.packs.list_documents",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, pack.name = %pack, documents = tracing::field::Empty),
    err(Display)
)]
pub async fn list_documents<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope: ScopeId,
    pack: &ContextPackName,
) -> Result<Vec<StoredDocument>> {
    let rows = sqlx::query_as!(
        DocumentRow,
        r#"select scope_id, pack_name, document_name, title, sensitivity,
                  object_hash, chunks, created_at, created_by, updated_at, updated_by
           from context_pack_documents
           where tenant_id = $1 and scope_id = $2 and pack_name = $3
           order by document_name"#,
        tenant.as_uuid(),
        scope.as_uuid(),
        pack.as_str(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("documents", rows.len());
    rows.into_iter().map(StoredDocument::try_from).collect()
}

/// Every document of every pack at one scope, in `(pack, document)` order —
/// the listing surface's read, and the proposal path's.
#[tracing::instrument(
    name = "store.packs.list_all_documents",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, documents = tracing::field::Empty),
    err(Display)
)]
pub async fn list_all_documents<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope: ScopeId,
) -> Result<Vec<StoredDocument>> {
    let rows = sqlx::query_as!(
        DocumentRow,
        r#"select scope_id, pack_name, document_name, title, sensitivity,
                  object_hash, chunks, created_at, created_by, updated_at, updated_by
           from context_pack_documents
           where tenant_id = $1 and scope_id = $2
           order by pack_name, document_name"#,
        tenant.as_uuid(),
        scope.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("documents", rows.len());
    rows.into_iter().map(StoredDocument::try_from).collect()
}

// ── The chunk mapping ───────────────────────────────────────────────────

/// Records that `record_id` is chunk `ordinal` of a document version.
///
/// Idempotent on the pair migration 0030 keys it by: re-authoring identical
/// bytes finds the row already there and changes nothing, which is the
/// storage half of "an unchanged document re-embeds nothing" (ADR-0050
/// decision 4). It returns whether the row was new, so the caller can say
/// how much work an author's request actually did.
#[tracing::instrument(
    name = "store.packs.record_chunk",
    skip_all,
    fields(tenant.id = %tenant, record.id = %new.record_id, ordinal = new.ordinal),
    err(Display)
)]
pub async fn record_chunk<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewChunk<'_>,
) -> Result<bool> {
    let inserted = sqlx::query_scalar!(
        r#"insert into context_pack_chunks
               (tenant_id, record_id, scope_id, pack_name, document_name, title,
                document_hash, ordinal, heading)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           on conflict (tenant_id, document_hash, ordinal) do nothing
           returning record_id as "record_id!""#,
        tenant.as_uuid(),
        new.record_id.as_uuid(),
        new.scope_id.as_uuid(),
        new.pack_name.as_str(),
        new.document_name.as_str(),
        new.title,
        &new.document_hash[..],
        i32::try_from(new.ordinal).unwrap_or(i32::MAX),
        new.heading,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(inserted.is_some())
}

/// The chunks already cut from one document version, in ordinal order.
///
/// What makes re-authoring an unchanged document free: the address is the
/// same, so these rows are found and neither embedded nor written again.
#[tracing::instrument(
    name = "store.packs.chunks_of",
    skip_all,
    fields(tenant.id = %tenant, chunks = tracing::field::Empty),
    err(Display)
)]
pub async fn chunks_of<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    document_hash: [u8; 32],
) -> Result<Vec<PackChunk>> {
    let rows = sqlx::query_as!(
        ChunkRow,
        r#"select record_id, scope_id, pack_name, document_name, title,
                  document_hash, ordinal, heading
           from context_pack_chunks
           where tenant_id = $1 and document_hash = $2
           order by ordinal"#,
        tenant.as_uuid(),
        &document_hash[..],
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("chunks", rows.len());
    rows.into_iter().map(PackChunk::try_from).collect()
}

/// The chunks cut from any of `document_hashes`, in `(document, ordinal)`
/// order — **the composition read**.
///
/// The addresses come from what each planned scope's `context-pack/published`
/// tree names, so this returns exactly the chunks that are published *at
/// the version the channel serves* (ADR-0050 decision 3). A document edited
/// since publication has a different address and simply is not asked for,
/// which is how an edit demotes its own chunks rather than riding a
/// published path.
#[tracing::instrument(
    name = "store.packs.published_chunks",
    skip_all,
    fields(tenant.id = %tenant, documents = document_hashes.len(), chunks = tracing::field::Empty),
    err(Display)
)]
pub async fn published_chunks<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    document_hashes: &[[u8; 32]],
) -> Result<Vec<PackChunk>> {
    if document_hashes.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: Vec<Vec<u8>> = document_hashes.iter().map(|hash| hash.to_vec()).collect();
    let rows = sqlx::query_as!(
        ChunkRow,
        r#"select record_id, scope_id, pack_name, document_name, title,
                  document_hash, ordinal, heading
           from context_pack_chunks
           where tenant_id = $1 and document_hash = any($2)
           order by document_hash, ordinal"#,
        tenant.as_uuid(),
        &wanted,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("chunks", rows.len());
    rows.into_iter().map(PackChunk::try_from).collect()
}
