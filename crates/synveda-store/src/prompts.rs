//! Prompt drafts — the authoring state of the registry (PRMT-1, ADR-0049).
//!
//! One row per `(tenant, scope, name)`, holding the working copy an author
//! edits. It is **not** a version history: every write also puts a
//! content-addressed object, and the versions a channel has served are its
//! first-parent line (ADR-0049 decision 1). What lives here is the one
//! thing VedaFlow deliberately cannot express — a document that can be
//! replaced (ADR-0032 decision 2).
//!
//! This module stores; it decides nothing. `PromptWrite` at the scope is
//! the seam above it, the object address is computed by
//! `synveda_vedaflow::PromptAsset` before a call gets here, and whether a
//! draft may cross the trust boundary is the approval matrix's arithmetic.
//!
//! Tenant-scoped (forced RLS, ADR-0009): reach this table inside
//! [`crate::rls::begin_tenant_tx`].

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{
    Error, IdentityId, PromptName, PromptTemplate, PromptVariable, Result, ScopeId, Sensitivity,
    TenantId,
};

/// A prompt's draft as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPrompt {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// The template, its schema, its name and its description.
    pub template: PromptTemplate,
    /// Its classification. Never `restricted` — the column's CHECK says so,
    /// because nothing in the product can mint that tier for an authored
    /// asset (ADR-0049 decision 5).
    pub sensitivity: Sensitivity,
    /// The address of exactly these bytes, so a caller can compare a draft
    /// against what a channel published without re-hashing it.
    pub object_hash: [u8; 32],
    /// When it was first authored.
    pub created_at: DateTime<Utc>,
    /// Who first authored it.
    pub created_by: IdentityId,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// Who last changed it.
    pub updated_by: IdentityId,
}

/// A draft write, as the caller describes it.
#[derive(Debug, Clone)]
pub struct NewPrompt<'a> {
    /// Where it is authored.
    pub scope_id: ScopeId,
    /// The template, already validated (`PromptTemplate::validate`) and
    /// with its variables sorted by name.
    pub template: &'a PromptTemplate,
    /// Its classification.
    pub sensitivity: Sensitivity,
    /// The address of the object the caller has already written.
    pub object_hash: [u8; 32],
    /// Who is authoring.
    pub author: IdentityId,
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant, or — the one that
        // matters — an object address whose bytes were never stored.
        if db.code().as_deref() == Some("23503") {
            return Error::Invalid {
                message: "a prompt draft must name an object this tenant holds".to_owned(),
            };
        }
        // 23514 check_violation: a name, template or tier the column
        // refuses. `restricted` lands here, which is the structural half of
        // ADR-0049 decision 5.
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

/// The stored shape, mapped into [`StoredPrompt`] on the way out.
struct PromptRow {
    scope_id: uuid::Uuid,
    name: String,
    description: String,
    template: String,
    variables: serde_json::Value,
    sensitivity: String,
    object_hash: Vec<u8>,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

impl TryFrom<PromptRow> for StoredPrompt {
    type Error = Error;

    fn try_from(row: PromptRow) -> Result<Self> {
        // Every column's CHECK mirrors a vocabulary this crate can parse, so
        // a value outside one means code and schema have drifted. Say so
        // rather than shrug — the role_bindings discipline (ADR-0015).
        let variables: Vec<PromptVariable> =
            serde_json::from_value(row.variables).map_err(|err| Error::Internal {
                message: format!(
                    "prompt {:?} has an unreadable variable schema: {err}",
                    row.name
                ),
            })?;
        let object_hash: [u8; 32] = row.object_hash.try_into().map_err(|_| Error::Internal {
            message: format!(
                "prompt {:?} has an object address that is not 32 bytes",
                row.name
            ),
        })?;
        Ok(StoredPrompt {
            scope_id: ScopeId::from_uuid(row.scope_id),
            template: PromptTemplate {
                name: row.name.parse()?,
                description: row.description,
                template: row.template,
                variables,
            },
            sensitivity: row.sensitivity.parse()?,
            object_hash,
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    }
}

/// Writes a draft: creates it, or replaces the content of the one that is
/// there.
///
/// An overwrite is the authoring act, not a conflict — a draft is the
/// document you change your mind in. What cannot change is its identity:
/// migration 0029's trigger refuses a moved scope or a renamed prompt, so
/// this statement's `on conflict` can only ever rewrite content.
#[tracing::instrument(
    name = "store.prompts.upsert",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %new.scope_id, prompt.name = %new.template.name),
    err(Display)
)]
pub async fn upsert<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    new: &NewPrompt<'_>,
) -> Result<StoredPrompt> {
    let variables =
        serde_json::to_value(&new.template.variables).map_err(|err| Error::Internal {
            message: format!("a variable schema failed to serialise: {err}"),
        })?;
    let row = sqlx::query_as!(
        PromptRow,
        r#"insert into prompts
               (tenant_id, scope_id, name, description, template, variables,
                sensitivity, object_hash, created_by, updated_by)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
           on conflict (tenant_id, scope_id, name) do update
               set description = excluded.description,
                   template    = excluded.template,
                   variables   = excluded.variables,
                   sensitivity = excluded.sensitivity,
                   object_hash = excluded.object_hash,
                   updated_at  = now(),
                   updated_by  = excluded.updated_by
           returning scope_id, name, description, template, variables,
                     sensitivity, object_hash, created_at, created_by,
                     updated_at, updated_by"#,
        tenant.as_uuid(),
        new.scope_id.as_uuid(),
        new.template.name.as_str(),
        new.template.description,
        new.template.template,
        variables,
        new.sensitivity.as_str(),
        &new.object_hash[..],
        new.author.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredPrompt::try_from(row)
}

/// One draft, or `None` when that scope has never authored that name.
#[tracing::instrument(
    name = "store.prompts.read",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, prompt.name = %name),
    err(Display)
)]
pub async fn read<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope: ScopeId,
    name: &PromptName,
) -> Result<Option<StoredPrompt>> {
    let row = sqlx::query_as!(
        PromptRow,
        r#"select scope_id, name, description, template, variables, sensitivity,
                  object_hash, created_at, created_by, updated_at, updated_by
           from prompts
           where tenant_id = $1 and scope_id = $2 and name = $3"#,
        tenant.as_uuid(),
        scope.as_uuid(),
        name.as_str(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredPrompt::try_from).transpose()
}

/// Every draft at one scope, in name order.
#[tracing::instrument(
    name = "store.prompts.list",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, prompts = tracing::field::Empty),
    err(Display)
)]
pub async fn list<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope: ScopeId,
) -> Result<Vec<StoredPrompt>> {
    let rows = sqlx::query_as!(
        PromptRow,
        r#"select scope_id, name, description, template, variables, sensitivity,
                  object_hash, created_at, created_by, updated_at, updated_by
           from prompts
           where tenant_id = $1 and scope_id = $2
           order by name"#,
        tenant.as_uuid(),
        scope.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("prompts", rows.len());
    rows.into_iter().map(StoredPrompt::try_from).collect()
}

/// The drafts at `scope` for `names`, in name order — the proposal path's
/// read, which needs several at once and must be able to say which of them
/// is missing.
#[tracing::instrument(
    name = "store.prompts.read_many",
    skip_all,
    fields(tenant.id = %tenant, scope.id = %scope, names.count = names.len()),
    err(Display)
)]
pub async fn read_many<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    scope: ScopeId,
    names: &[PromptName],
) -> Result<Vec<StoredPrompt>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: Vec<String> = names.iter().map(ToString::to_string).collect();
    let rows = sqlx::query_as!(
        PromptRow,
        r#"select scope_id, name, description, template, variables, sensitivity,
                  object_hash, created_at, created_by, updated_at, updated_by
           from prompts
           where tenant_id = $1 and scope_id = $2 and name = any($3)
           order by name"#,
        tenant.as_uuid(),
        scope.as_uuid(),
        &wanted,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredPrompt::try_from).collect()
}
