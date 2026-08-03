//! The skills registry's draft rows (SKIL-1, ADR-0051; migration 0031).
//!
//! Two tables and no third. A context pack needed a chunk mapping because
//! its published content becomes `records`; a skill's content becomes
//! nothing — it is fetched by name and materialised into a client's own
//! skills directory (ADR-0051 decision 9). So this module is the smaller
//! half of `packs`, with one addition and one subtraction:
//!
//! - **added**: [`prune_files`], because a skill is authored *whole*
//!   (decision 17). A file removed from an authoring request is removed from
//!   the draft, since a client loads a bundle whole and a file the author
//!   deleted must not be published back onto a laptop by the next proposal.
//! - **subtracted**: nothing here knows what a chunk is.
//!
//! Bytes live in `vedaflow_objects` and the published set is the channel's;
//! these rows are the working copy (ADR-0051 decision 1).

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{
    Error, IdentityId, Result, ScopeId, Sensitivity, SkillFilePath, SkillName, TenantId,
};

/// A skill's draft row: the bundle's identity and its registry metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSkill {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// Its name — the tree entry prefix, and the directory an install
    /// creates.
    pub name: SkillName,
    /// `SKILL.md`'s frontmatter `description`, denormalised so a listing
    /// reads no objects.
    pub description: String,
    /// Its classification. Per skill rather than per file, because a client
    /// loads a bundle whole (ADR-0051 decision 11).
    pub sensitivity: Sensitivity,
    /// When it was first authored.
    pub created_at: DateTime<Utc>,
    /// Who first authored it.
    pub created_by: IdentityId,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// Who changed it last.
    pub updated_by: IdentityId,
}

/// One file's draft row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// The bundle it belongs to.
    pub skill_name: SkillName,
    /// Its path within the bundle.
    pub path: SkillFilePath,
    /// The address of exactly these bytes.
    pub object_hash: [u8; 32],
    /// When it was first authored.
    pub created_at: DateTime<Utc>,
    /// Who first authored it.
    pub created_by: IdentityId,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// Who changed it last.
    pub updated_by: IdentityId,
}

/// What [`upsert_skill`] writes.
pub struct NewSkill<'a> {
    /// Where it is authored.
    pub scope_id: ScopeId,
    /// Its name.
    pub name: &'a SkillName,
    /// Its `description`, as the frontmatter parse found it.
    pub description: &'a str,
    /// Its tier.
    pub sensitivity: Sensitivity,
    /// Who is authoring.
    pub author: IdentityId,
}

/// What [`upsert_file`] writes.
pub struct NewFile<'a> {
    /// Where it is authored.
    pub scope_id: ScopeId,
    /// The bundle.
    pub skill_name: &'a SkillName,
    /// Its path within the bundle.
    pub path: &'a SkillFilePath,
    /// The address of exactly these bytes — already stored, or the FK
    /// refuses the row.
    pub object_hash: [u8; 32],
    /// Who is authoring.
    pub author: IdentityId,
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant, a skill row that was
        // never written, or — the one that matters — an object address
        // whose bytes were never stored.
        if db.code().as_deref() == Some("23503") {
            return Error::Invalid {
                message: format!(
                    "a skill row must name a tenant, skill and object this tenant holds: {db}"
                ),
            };
        }
        // 23514 check_violation: a name, description or tier the column
        // refuses. `restricted` lands here, which is the structural half of
        // ADR-0051 decision 11.
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

/// The stored skill shape, mapped on the way out.
struct SkillRow {
    scope_id: uuid::Uuid,
    name: String,
    description: String,
    sensitivity: String,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

impl TryFrom<SkillRow> for StoredSkill {
    type Error = Error;

    fn try_from(row: SkillRow) -> Result<Self> {
        // Every column's CHECK mirrors a vocabulary this crate can parse, so
        // a value outside one means code and schema have drifted. Say so
        // rather than shrug — the role_bindings discipline (ADR-0015).
        Ok(StoredSkill {
            scope_id: ScopeId::from_uuid(row.scope_id),
            name: row.name.parse()?,
            description: row.description,
            sensitivity: row.sensitivity.parse()?,
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    }
}

/// The stored file shape, mapped on the way out.
struct FileRow {
    scope_id: uuid::Uuid,
    skill_name: String,
    path: String,
    object_hash: Vec<u8>,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

impl TryFrom<FileRow> for StoredFile {
    type Error = Error;

    fn try_from(row: FileRow) -> Result<Self> {
        let object_hash =
            <[u8; 32]>::try_from(row.object_hash.as_slice()).map_err(|_| Error::Internal {
                message: format!(
                    "skill file {:?} has an object address that is not 32 bytes",
                    row.path
                ),
            })?;
        Ok(StoredFile {
            scope_id: ScopeId::from_uuid(row.scope_id),
            skill_name: row.skill_name.parse()?,
            path: row.path.parse()?,
            object_hash,
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    }
}

// ── Drafts ──────────────────────────────────────────────────────────────

/// Writes a skill's draft row: creates it, or replaces the metadata of the
/// one that is there.
///
/// An overwrite is the authoring act, not a conflict. What cannot change is
/// its identity: migration 0031's trigger refuses a moved scope or a renamed
/// skill, so this statement's `on conflict` can only ever rewrite the
/// description and the tier.
///
/// # Errors
///
/// [`Error::Invalid`] for an unknown tenant or a refused tier;
/// [`Error::Storage`] otherwise.
#[tracing::instrument(
    name = "store.skills.upsert_skill",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %new.scope_id, skill.name = %new.name),
    err(Display)
)]
pub async fn upsert_skill<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewSkill<'_>,
) -> Result<StoredSkill> {
    let row = sqlx::query_as!(
        SkillRow,
        r#"insert into skills
               (tenant_id, scope_id, name, description, sensitivity, created_by, updated_by)
           values ($1, $2, $3, $4, $5, $6, $6)
           on conflict (tenant_id, scope_id, name) do update
               set description = excluded.description,
                   sensitivity = excluded.sensitivity,
                   updated_at  = now(),
                   updated_by  = excluded.updated_by
           returning scope_id, name, description, sensitivity, created_at, created_by,
                     updated_at, updated_by"#,
        tenant.as_uuid(),
        new.scope_id.as_uuid(),
        new.name.as_str(),
        new.description,
        new.sensitivity.as_str(),
        new.author.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredSkill::try_from(row)
}

/// Writes one bundled file's draft row: creates it, or re-points the one
/// that is there at new bytes.
///
/// # Errors
///
/// [`Error::Invalid`] when the skill row or the object address does not
/// exist; [`Error::Storage`] otherwise.
#[tracing::instrument(
    name = "store.skills.upsert_file",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %new.scope_id, skill.name = %new.skill_name, skill.path = %new.path),
    err(Display)
)]
pub async fn upsert_file<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewFile<'_>,
) -> Result<StoredFile> {
    let row = sqlx::query_as!(
        FileRow,
        r#"insert into skill_files
               (tenant_id, scope_id, skill_name, path, object_hash, created_by, updated_by)
           values ($1, $2, $3, $4, $5, $6, $6)
           on conflict (tenant_id, scope_id, skill_name, path) do update
               set object_hash = excluded.object_hash,
                   updated_at  = now(),
                   updated_by  = excluded.updated_by
           returning scope_id, skill_name, path, object_hash, created_at, created_by,
                     updated_at, updated_by"#,
        tenant.as_uuid(),
        new.scope_id.as_uuid(),
        new.skill_name.as_str(),
        new.path.as_str(),
        &new.object_hash[..],
        new.author.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredFile::try_from(row)
}

/// Removes every draft file of `skill` whose path is not in `keep`,
/// returning how many went (ADR-0051 decision 17).
///
/// **The one delete in the three authored-asset registries**, and the reason
/// is that a client loads a bundle whole: an authoring request *is* the
/// bundle, so a file the author dropped must not survive to be published
/// back onto a laptop. It cannot reach a published version — a tree names
/// object addresses, objects are append-only, and nothing here removes one.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skills.prune_files",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id, skill.name = %name, skill.kept = keep.len()),
    err(Display)
)]
pub async fn prune_files<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
    name: &SkillName,
    keep: &[SkillFilePath],
) -> Result<u64> {
    let kept: Vec<String> = keep.iter().map(|path| path.as_str().to_owned()).collect();
    let done = sqlx::query!(
        "delete from skill_files
         where tenant_id = $1 and scope_id = $2 and skill_name = $3
           and path <> all($4)",
        tenant.as_uuid(),
        scope_id.as_uuid(),
        name.as_str(),
        &kept[..],
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(done.rows_affected())
}

// ── Reads ───────────────────────────────────────────────────────────────

/// One skill's draft row at one scope, or `None`.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skills.skill",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id, skill.name = %name),
    err(Display)
)]
pub async fn skill<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
    name: &SkillName,
) -> Result<Option<StoredSkill>> {
    let row = sqlx::query_as!(
        SkillRow,
        r#"select scope_id, name, description, sensitivity, created_at, created_by,
                  updated_at, updated_by
           from skills
           where tenant_id = $1 and scope_id = $2 and name = $3"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
        name.as_str(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredSkill::try_from).transpose()
}

/// Every skill drafted at one scope, by name.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skills.list_skills",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id),
    err(Display)
)]
pub async fn list_skills<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
) -> Result<Vec<StoredSkill>> {
    let rows = sqlx::query_as!(
        SkillRow,
        r#"select scope_id, name, description, sensitivity, created_at, created_by,
                  updated_at, updated_by
           from skills
           where tenant_id = $1 and scope_id = $2
           order by name"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredSkill::try_from).collect()
}

/// One bundle's draft files, in path order — the order an install writes
/// them in.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skills.files_of",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id, skill.name = %name),
    err(Display)
)]
pub async fn files_of<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
    name: &SkillName,
) -> Result<Vec<StoredFile>> {
    let rows = sqlx::query_as!(
        FileRow,
        r#"select scope_id, skill_name, path, object_hash, created_at, created_by,
                  updated_at, updated_by
           from skill_files
           where tenant_id = $1 and scope_id = $2 and skill_name = $3
           order by path"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
        name.as_str(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredFile::try_from).collect()
}

/// Every drafted file at one scope, across every skill — one read behind a
/// registry listing.
///
/// # Errors
///
/// [`Error::Storage`] on a database failure.
#[tracing::instrument(
    name = "store.skills.list_all_files",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope_id),
    err(Display)
)]
pub async fn list_all_files<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope_id: ScopeId,
) -> Result<Vec<StoredFile>> {
    let rows = sqlx::query_as!(
        FileRow,
        r#"select scope_id, skill_name, path, object_hash, created_at, created_by,
                  updated_at, updated_by
           from skill_files
           where tenant_id = $1 and scope_id = $2
           order by skill_name, path"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredFile::try_from).collect()
}
