//! Immutable governed runtime configuration (CPR-30, ADR-0089).
//!
//! The functions that mutate aggregate heads and bindings are intentionally
//! command-shaped and are called only by the gateway's typed VedaFlow effect.
//! Runtime readers resolve an enabled binding nearest-scope-first and obtain
//! the exact immutable document that drove the request.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::types::Json;
use synveda_types::configuration::{
    ConfigurationArtifact, ConfigurationBinding, ConfigurationCommand, ConfigurationDocument,
    ConfigurationTemplate, ConfigurationVersion, EffectiveConfiguration,
    validate_configuration_name,
};
use synveda_types::{
    ConfigurationArtifactId, ConfigurationBindingId, ConfigurationVersionId, Error,
    PolicyAssignment, ProposalId, Result, ScopeId, TenantId,
};

/// One persisted typed VedaFlow command and its eventual result.
#[derive(Debug, Clone)]
pub struct ConfigurationChange {
    /// VedaFlow change id.
    pub proposal_id: ProposalId,
    /// Complete immutable command payload.
    pub command: ConfigurationCommand,
    /// Canonical payload hash bound into the reviewed manifest.
    pub payload_hash: String,
    /// Resulting stable aggregate.
    pub resulting_artifact_id: Option<ConfigurationArtifactId>,
    /// Resulting immutable version.
    pub resulting_version_id: Option<ConfigurationVersionId>,
    /// Resulting binding.
    pub resulting_binding_id: Option<ConfigurationBindingId>,
    /// Resulting binding revision.
    pub resulting_binding_revision: Option<u64>,
    /// Effect completion instant.
    pub applied_at: Option<DateTime<Utc>>,
}

/// Result of applying one typed command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppliedConfiguration {
    /// Stable aggregate.
    pub artifact_id: Option<ConfigurationArtifactId>,
    /// Immutable version.
    pub version_id: Option<ConfigurationVersionId>,
    /// Stable binding.
    pub binding_id: Option<ConfigurationBindingId>,
    /// Binding revision.
    pub binding_revision: Option<u64>,
}

/// Keyset cursor for aggregate listings.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactCursor {
    /// Last considered creation time.
    pub created_at: DateTime<Utc>,
    /// Last considered id.
    pub id: ConfigurationArtifactId,
}

fn storage_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error {
        match database.code().as_deref() {
            Some("23503") => {
                return Error::NotFound {
                    entity: "configuration dependency".to_owned(),
                };
            }
            Some("23505") => {
                return Error::Conflict {
                    message: "configuration name, version, binding or change already exists"
                        .to_owned(),
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
        message: format!("configuration hash has {} bytes, expected 32", value.len()),
    })?;
    Ok(blake3::Hash::from_bytes(hash).to_hex().to_string())
}

fn decode_template(value: Option<String>) -> Result<Option<ConfigurationTemplate>> {
    value.map(|name| name.parse()).transpose()
}

struct ArtifactRow {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    governing_scope_id: uuid::Uuid,
    name: String,
    current_version_id: uuid::Uuid,
    created_at: DateTime<Utc>,
    created_by: String,
    updated_at: DateTime<Utc>,
    updated_by: String,
}

impl From<ArtifactRow> for ConfigurationArtifact {
    fn from(row: ArtifactRow) -> Self {
        Self {
            id: ConfigurationArtifactId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            governing_scope_id: ScopeId::from_uuid(row.governing_scope_id),
            name: row.name,
            current_version_id: ConfigurationVersionId::from_uuid(row.current_version_id),
            created_at: row.created_at,
            created_by: row.created_by,
            updated_at: row.updated_at,
            updated_by: row.updated_by,
        }
    }
}

struct VersionRow {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    artifact_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    ordinal: i64,
    document: Json<ConfigurationDocument>,
    content_hash: Vec<u8>,
    source_template: Option<String>,
    created_at: DateTime<Utc>,
    created_by: String,
}

impl TryFrom<VersionRow> for ConfigurationVersion {
    type Error = Error;

    fn try_from(row: VersionRow) -> Result<Self> {
        row.document.0.validate()?;
        let content_hash = parse_hash(row.content_hash)?;
        if row.document.0.content_hash()? != content_hash {
            return Err(Error::Storage {
                message: format!(
                    "configuration version {} content hash disagrees with document",
                    row.id
                ),
            });
        }
        Ok(Self {
            id: ConfigurationVersionId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            artifact_id: ConfigurationArtifactId::from_uuid(row.artifact_id),
            ordinal: row.ordinal,
            document: row.document.0,
            content_hash,
            source_template: decode_template(row.source_template)?,
            proposal_id: ProposalId::from_uuid(row.proposal_id),
            created_at: row.created_at,
            created_by: row.created_by,
        })
    }
}

struct BindingRow {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    scope_id: uuid::Uuid,
    artifact_id: uuid::Uuid,
    pinned_version_id: Option<uuid::Uuid>,
    enabled: bool,
    revision: i64,
    created_at: DateTime<Utc>,
    created_by: String,
    updated_at: DateTime<Utc>,
    updated_by: String,
}

impl TryFrom<BindingRow> for ConfigurationBinding {
    type Error = Error;

    fn try_from(row: BindingRow) -> Result<Self> {
        Ok(Self {
            id: ConfigurationBindingId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            artifact_id: ConfigurationArtifactId::from_uuid(row.artifact_id),
            pinned_version_id: row.pinned_version_id.map(ConfigurationVersionId::from_uuid),
            enabled: row.enabled,
            revision: u64::try_from(row.revision).map_err(|_| Error::Storage {
                message: format!("configuration binding revision {} is invalid", row.revision),
            })?,
            created_at: row.created_at,
            created_by: row.created_by,
            updated_at: row.updated_at,
            updated_by: row.updated_by,
        })
    }
}

/// Fetch one stable aggregate.
#[tracing::instrument(name = "store.configuration.artifact", skip_all, fields(tenant.id = %tenant, configuration.id = %id), err(Display))]
pub async fn artifact(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConfigurationArtifactId,
) -> Result<Option<ConfigurationArtifact>> {
    sqlx::query_as!(
        ArtifactRow,
        r#"select id, tenant_id, governing_scope_id, name, current_version_id,
                  created_at, created_by, updated_at, updated_by
             from configuration_artifacts
            where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map(|row| row.map(Into::into))
    .map_err(storage_error)
}

/// Fetch one immutable version.
#[tracing::instrument(name = "store.configuration.version", skip_all, fields(tenant.id = %tenant, configuration.version_id = %id), err(Display))]
pub async fn version(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConfigurationVersionId,
) -> Result<Option<ConfigurationVersion>> {
    let row = sqlx::query_as!(
        VersionRow,
        r#"select id, tenant_id, artifact_id, proposal_id, ordinal,
                  document as "document: Json<ConfigurationDocument>", content_hash,
                  source_template, created_at, created_by
             from configuration_versions
            where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Fetch one revisioned binding.
#[tracing::instrument(name = "store.configuration.binding", skip_all, fields(tenant.id = %tenant, configuration.binding_id = %id), err(Display))]
pub async fn binding(
    connection: &mut PgConnection,
    tenant: TenantId,
    id: ConfigurationBindingId,
) -> Result<Option<ConfigurationBinding>> {
    let row = sqlx::query_as!(
        BindingRow,
        r#"select id, tenant_id, scope_id, artifact_id, pinned_version_id,
                  enabled, revision, created_at, created_by, updated_at, updated_by
             from configuration_bindings
            where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// List stable aggregates using descending `(created_at, id)` keyset order.
pub async fn list_artifacts(
    connection: &mut PgConnection,
    tenant: TenantId,
    governing_scope_id: Option<ScopeId>,
    cursor: Option<ArtifactCursor>,
    limit: i64,
) -> Result<Vec<ConfigurationArtifact>> {
    let rows = sqlx::query_as!(
        ArtifactRow,
        r#"select id, tenant_id, governing_scope_id, name, current_version_id,
                  created_at, created_by, updated_at, updated_by
            from configuration_artifacts
            where tenant_id = $1
              and ($2::uuid is null or governing_scope_id = $2)
              and ($3::timestamptz is null
                   or (created_at, id) < ($3, $4::uuid))
            order by created_at desc, id desc
            limit $5"#,
        tenant.as_uuid(),
        governing_scope_id.map(|value| value.as_uuid()),
        cursor.map(|value| value.created_at),
        cursor.map(|value| value.id.as_uuid()),
        limit,
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// List immutable versions newest ordinal first.
pub async fn versions(
    connection: &mut PgConnection,
    tenant: TenantId,
    artifact_id: ConfigurationArtifactId,
    before_ordinal: Option<i64>,
    limit: i64,
) -> Result<Vec<ConfigurationVersion>> {
    let rows = sqlx::query_as!(
        VersionRow,
        r#"select id, tenant_id, artifact_id, proposal_id, ordinal,
                  document as "document: Json<ConfigurationDocument>", content_hash,
                  source_template, created_at, created_by
             from configuration_versions
            where tenant_id = $1 and artifact_id = $2
              and ($3::bigint is null or ordinal < $3)
            order by ordinal desc
            limit $4"#,
        tenant.as_uuid(),
        artifact_id.as_uuid(),
        before_ordinal,
        limit,
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// List bindings, optionally at one exact scope.
pub async fn bindings(
    connection: &mut PgConnection,
    tenant: TenantId,
    scope_id: Option<ScopeId>,
    before: Option<ConfigurationBindingId>,
    limit: i64,
) -> Result<Vec<ConfigurationBinding>> {
    let rows = sqlx::query_as!(
        BindingRow,
        r#"select id, tenant_id, scope_id, artifact_id, pinned_version_id,
                  enabled, revision, created_at, created_by, updated_at, updated_by
             from configuration_bindings
            where tenant_id = $1
              and ($2::uuid is null or scope_id = $2)
              and ($3::uuid is null or id < $3)
            order by id desc
            limit $4"#,
        tenant.as_uuid(),
        scope_id.map(|value| value.as_uuid()),
        before.map(|value| value.as_uuid()),
        limit,
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

struct EffectiveRow {
    binding_id: uuid::Uuid,
    binding_scope_id: uuid::Uuid,
    artifact_id: uuid::Uuid,
    version_id: uuid::Uuid,
    document: Json<ConfigurationDocument>,
    content_hash: Vec<u8>,
}

/// Resolve the first enabled binding in a nearest-first scope chain.
pub async fn effective_for_chain(
    connection: &mut PgConnection,
    tenant: TenantId,
    resource_scope_id: ScopeId,
    nearest_first: &[ScopeId],
) -> Result<EffectiveConfiguration> {
    let ids: Vec<uuid::Uuid> = nearest_first.iter().map(ScopeId::as_uuid).collect();
    let row = sqlx::query_as!(
        EffectiveRow,
        r#"select binding.id as binding_id,
                  binding.scope_id as binding_scope_id,
                  binding.artifact_id,
                  version.id as version_id,
                  version.document as "document: Json<ConfigurationDocument>",
                  version.content_hash
             from configuration_bindings binding
             join configuration_artifacts artifact
               on artifact.tenant_id = binding.tenant_id
              and artifact.id = binding.artifact_id
             join configuration_versions version
               on version.tenant_id = binding.tenant_id
              and version.artifact_id = binding.artifact_id
              and version.id = coalesce(binding.pinned_version_id,
                                        artifact.current_version_id)
            where binding.tenant_id = $1
              and binding.enabled
              and binding.scope_id = any($2)
            order by array_position($2::uuid[], binding.scope_id)
            limit 1"#,
        tenant.as_uuid(),
        &ids,
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;

    if let Some(row) = row {
        row.document.0.validate()?;
        let content_hash = parse_hash(row.content_hash)?;
        if content_hash != row.document.0.content_hash()? {
            return Err(Error::Storage {
                message: format!(
                    "effective configuration version {} has an invalid digest",
                    row.version_id
                ),
            });
        }
        return Ok(EffectiveConfiguration {
            scope_id: resource_scope_id,
            binding_id: Some(ConfigurationBindingId::from_uuid(row.binding_id)),
            binding_scope_id: Some(ScopeId::from_uuid(row.binding_scope_id)),
            artifact_id: Some(ConfigurationArtifactId::from_uuid(row.artifact_id)),
            version_id: Some(ConfigurationVersionId::from_uuid(row.version_id)),
            content_hash,
            document: row.document.0,
        });
    }

    let document = ConfigurationDocument::fail_safe();
    let content_hash = document.content_hash()?;
    Ok(EffectiveConfiguration {
        scope_id: resource_scope_id,
        binding_id: None,
        binding_scope_id: None,
        artifact_id: None,
        version_id: None,
        content_hash,
        document,
    })
}

/// Resolve effective configuration for one owned scope by deriving its
/// nearest-first chain inside the same tenant transaction.
pub async fn effective_at_scope(
    connection: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
) -> Result<EffectiveConfiguration> {
    crate::scopes::get(&mut *connection, tenant, scope_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("scope {scope_id} in tenant {tenant}"),
        })?;
    let mut chain = vec![scope_id];
    chain.extend(
        crate::scopes::ancestors(&mut *connection, tenant, scope_id)
            .await?
            .into_iter()
            .map(|scope| scope.id),
    );
    effective_for_chain(connection, tenant, scope_id, &chain).await
}

/// Produce only the assignment projection the Cedar PDP needs for bindings at
/// the supplied scopes. No row with this shape is persisted.
pub async fn policy_assignments_for_scopes(
    connection: &mut PgConnection,
    tenant: TenantId,
    scope_ids: &[ScopeId],
) -> Result<Vec<PolicyAssignment>> {
    struct Row {
        scope_id: uuid::Uuid,
        pack_name: String,
        updated_at: DateTime<Utc>,
    }
    let ids: Vec<uuid::Uuid> = scope_ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query_as!(
        Row,
        r#"select binding.scope_id,
                  version.document ->> 'policy_pack' as "pack_name!",
                  binding.updated_at
             from configuration_bindings binding
             join configuration_artifacts artifact
               on artifact.tenant_id = binding.tenant_id
              and artifact.id = binding.artifact_id
             join configuration_versions version
               on version.tenant_id = binding.tenant_id
              and version.artifact_id = binding.artifact_id
              and version.id = coalesce(binding.pinned_version_id,
                                        artifact.current_version_id)
            where binding.tenant_id = $1
              and binding.enabled
              and binding.scope_id = any($2)"#,
        tenant.as_uuid(),
        &ids,
    )
    .fetch_all(connection)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| PolicyAssignment {
            tenant_id: tenant,
            scope_id: ScopeId::from_uuid(row.scope_id),
            pack_name: row.pack_name,
            updated_at: row.updated_at,
        })
        .collect())
}

/// Policy selector at the tenant root, used for tenant-resource decisions.
pub async fn tenant_policy_pack(
    connection: &mut PgConnection,
    tenant: TenantId,
) -> Result<Option<String>> {
    sqlx::query_scalar!(
        r#"select version.document ->> 'policy_pack' as "pack_name!"
             from scopes root
             join configuration_bindings binding
               on binding.tenant_id = root.tenant_id
              and binding.scope_id = root.id
              and binding.enabled
             join configuration_artifacts artifact
               on artifact.tenant_id = binding.tenant_id
              and artifact.id = binding.artifact_id
             join configuration_versions version
               on version.tenant_id = binding.tenant_id
              and version.artifact_id = binding.artifact_id
              and version.id = coalesce(binding.pinned_version_id,
                                        artifact.current_version_id)
            where root.tenant_id = $1 and root.kind = 'tenant'"#,
        tenant.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)
}

/// Insert a typed change projection. Proposal state is the only workflow
/// state; this row binds erasable command material to the reviewed digest.
pub async fn insert_change(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
    command: &ConfigurationCommand,
    payload_hash: &str,
) -> Result<()> {
    sqlx::query!(
        r#"insert into configuration_changes
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

/// Read one typed change projection.
pub async fn change(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
) -> Result<Option<ConfigurationChange>> {
    struct Row {
        proposal_id: uuid::Uuid,
        payload: Json<ConfigurationCommand>,
        payload_hash: String,
        resulting_artifact_id: Option<uuid::Uuid>,
        resulting_version_id: Option<uuid::Uuid>,
        resulting_binding_id: Option<uuid::Uuid>,
        resulting_binding_revision: Option<i64>,
        applied_at: Option<DateTime<Utc>>,
    }
    let row = sqlx::query_as!(
        Row,
        r#"select proposal_id,
                  payload as "payload: Json<ConfigurationCommand>", payload_hash,
                  resulting_artifact_id, resulting_version_id,
                  resulting_binding_id, resulting_binding_revision, applied_at
             from configuration_changes
            where tenant_id = $1 and proposal_id = $2"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
    )
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(ConfigurationChange {
            proposal_id: ProposalId::from_uuid(row.proposal_id),
            command: row.payload.0,
            payload_hash: row.payload_hash,
            resulting_artifact_id: row
                .resulting_artifact_id
                .map(ConfigurationArtifactId::from_uuid),
            resulting_version_id: row
                .resulting_version_id
                .map(ConfigurationVersionId::from_uuid),
            resulting_binding_id: row
                .resulting_binding_id
                .map(ConfigurationBindingId::from_uuid),
            resulting_binding_revision: row
                .resulting_binding_revision
                .map(u64::try_from)
                .transpose()
                .map_err(|_| Error::Storage {
                    message: "configuration result has a negative binding revision".to_owned(),
                })?,
            applied_at: row.applied_at,
        })
    })
    .transpose()
}

fn validate_command(command: &ConfigurationCommand) -> Result<()> {
    match command {
        ConfigurationCommand::Create {
            name,
            document,
            content_hash,
            source_template,
            ..
        } => {
            validate_configuration_name(name)?;
            document.validate()?;
            if document.content_hash()? != *content_hash {
                return Err(Error::Invalid {
                    message: "configuration document hash does not match its content".to_owned(),
                });
            }
            if let Some(template) = source_template
                && *document != ConfigurationDocument::template(*template)
            {
                return Err(Error::Invalid {
                    message: "configuration template provenance does not match the exact canonical document"
                        .to_owned(),
                });
            }
        }
        ConfigurationCommand::Publish {
            document,
            content_hash,
            source_template,
            ..
        } => {
            document.validate()?;
            if document.content_hash()? != *content_hash {
                return Err(Error::Invalid {
                    message: "configuration document hash does not match its content".to_owned(),
                });
            }
            if let Some(template) = source_template
                && *document != ConfigurationDocument::template(*template)
            {
                return Err(Error::Invalid {
                    message: "configuration template provenance does not match the exact canonical document"
                        .to_owned(),
                });
            }
        }
        ConfigurationCommand::Bind { .. } => {}
        ConfigurationCommand::SetBinding { reason, .. } => {
            if reason.trim() != reason
                || reason.is_empty()
                || reason.len() > 100
                || reason.chars().any(char::is_control)
            {
                return Err(Error::Invalid {
                    message: "configuration binding reason must contain 1..=100 non-control characters without surrounding whitespace".to_owned(),
                });
            }
        }
    }
    Ok(())
}

/// Execute one validated command inside the caller's transaction.
pub async fn apply(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
    actor: &str,
    command: &ConfigurationCommand,
) -> Result<AppliedConfiguration> {
    validate_command(command)?;
    match command {
        ConfigurationCommand::Create {
            artifact_id,
            version_id,
            governing_scope_id,
            name,
            document,
            content_hash,
            source_template,
        } => {
            sqlx::query!(
                r#"insert into configuration_artifacts
                      (id, tenant_id, governing_scope_id, name,
                       current_version_id, created_by, updated_by)
                   values ($1, $2, $3, $4, $5, $6, $6)"#,
                artifact_id.as_uuid(),
                tenant.as_uuid(),
                governing_scope_id.as_uuid(),
                name,
                version_id.as_uuid(),
                actor,
            )
            .execute(&mut *connection)
            .await
            .map_err(storage_error)?;
            insert_version(
                connection,
                tenant,
                proposal_id,
                *artifact_id,
                *version_id,
                1,
                document,
                content_hash,
                *source_template,
                actor,
            )
            .await?;
            Ok(AppliedConfiguration {
                artifact_id: Some(*artifact_id),
                version_id: Some(*version_id),
                ..AppliedConfiguration::default()
            })
        }
        ConfigurationCommand::Publish {
            artifact_id,
            expected_current_version_id,
            version_id,
            governing_scope_id,
            document,
            content_hash,
            source_template,
        } => {
            let current = artifact(&mut *connection, tenant, *artifact_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("configuration artifact {artifact_id}"),
                })?;
            if current.governing_scope_id != *governing_scope_id {
                return Err(Error::NotFound {
                    entity: format!("configuration artifact {artifact_id}"),
                });
            }
            if current.current_version_id != *expected_current_version_id {
                return Err(Error::Conflict {
                    message: format!(
                        "configuration {artifact_id} current version is {}, expected {expected_current_version_id}",
                        current.current_version_id
                    ),
                });
            }
            let previous = version(&mut *connection, tenant, current.current_version_id)
                .await?
                .ok_or_else(|| Error::Storage {
                    message: format!("configuration {} has no current version", current.id),
                })?;
            let ordinal = previous
                .ordinal
                .checked_add(1)
                .ok_or_else(|| Error::Internal {
                    message: "configuration version ordinal overflow".to_owned(),
                })?;
            insert_version(
                connection,
                tenant,
                proposal_id,
                *artifact_id,
                *version_id,
                ordinal,
                document,
                content_hash,
                *source_template,
                actor,
            )
            .await?;
            let result = sqlx::query!(
                r#"update configuration_artifacts
                      set current_version_id = $4, updated_at = now(), updated_by = $5
                    where tenant_id = $1 and id = $2 and current_version_id = $3"#,
                tenant.as_uuid(),
                artifact_id.as_uuid(),
                expected_current_version_id.as_uuid(),
                version_id.as_uuid(),
                actor,
            )
            .execute(connection)
            .await
            .map_err(storage_error)?;
            if result.rows_affected() != 1 {
                return Err(Error::Conflict {
                    message: format!("configuration {artifact_id} moved during publication"),
                });
            }
            Ok(AppliedConfiguration {
                artifact_id: Some(*artifact_id),
                version_id: Some(*version_id),
                ..AppliedConfiguration::default()
            })
        }
        ConfigurationCommand::Bind {
            binding_id,
            scope_id,
            artifact_id,
            pinned_version_id,
            enabled,
        } => {
            let selected_version =
                ensure_artifact_and_pin(connection, tenant, *artifact_id, *pinned_version_id)
                    .await?;
            let row = sqlx::query_as!(
                BindingRow,
                r#"insert into configuration_bindings
                      (id, tenant_id, scope_id, artifact_id, pinned_version_id,
                       enabled, created_by, updated_by)
                   values ($1, $2, $3, $4, $5, $6, $7, $7)
                   returning id, tenant_id, scope_id, artifact_id,
                             pinned_version_id, enabled, revision,
                             created_at, created_by, updated_at, updated_by"#,
                binding_id.as_uuid(),
                tenant.as_uuid(),
                scope_id.as_uuid(),
                artifact_id.as_uuid(),
                pinned_version_id.map(|value| value.as_uuid()),
                enabled,
                actor,
            )
            .fetch_one(connection)
            .await
            .map_err(storage_error)?;
            let binding: ConfigurationBinding = row.try_into()?;
            Ok(AppliedConfiguration {
                artifact_id: Some(*artifact_id),
                version_id: Some(selected_version),
                binding_id: Some(*binding_id),
                binding_revision: Some(binding.revision),
            })
        }
        ConfigurationCommand::SetBinding {
            binding_id,
            scope_id,
            expected_revision,
            artifact_id,
            pinned_version_id,
            enabled,
            ..
        } => {
            let selected_version =
                ensure_artifact_and_pin(connection, tenant, *artifact_id, *pinned_version_id)
                    .await?;
            let expected = i64::try_from(*expected_revision).map_err(|_| Error::Invalid {
                message: "configuration binding revision exceeds database range".to_owned(),
            })?;
            let row = sqlx::query_as!(
                BindingRow,
                r#"update configuration_bindings
                      set artifact_id = $5, pinned_version_id = $6, enabled = $7,
                          revision = revision + 1, updated_at = now(), updated_by = $8
                    where tenant_id = $1 and id = $2 and scope_id = $3
                      and revision = $4
                   returning id, tenant_id, scope_id, artifact_id,
                             pinned_version_id, enabled, revision,
                             created_at, created_by, updated_at, updated_by"#,
                tenant.as_uuid(),
                binding_id.as_uuid(),
                scope_id.as_uuid(),
                expected,
                artifact_id.as_uuid(),
                pinned_version_id.map(|value| value.as_uuid()),
                enabled,
                actor,
            )
            .fetch_optional(connection)
            .await
            .map_err(storage_error)?
            .ok_or_else(|| Error::Conflict {
                message: format!(
                    "configuration binding {binding_id} is absent, moved or not at revision {expected_revision}"
                ),
            })?;
            let binding: ConfigurationBinding = row.try_into()?;
            Ok(AppliedConfiguration {
                artifact_id: Some(*artifact_id),
                version_id: Some(selected_version),
                binding_id: Some(*binding_id),
                binding_revision: Some(binding.revision),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_version(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
    artifact_id: ConfigurationArtifactId,
    version_id: ConfigurationVersionId,
    ordinal: i64,
    document: &ConfigurationDocument,
    content_hash: &str,
    source_template: Option<ConfigurationTemplate>,
    actor: &str,
) -> Result<()> {
    let hash = blake3::Hash::from_hex(content_hash)
        .map_err(|error| Error::Invalid {
            message: format!("decode configuration hash: {error}"),
        })?
        .as_bytes()
        .to_vec();
    sqlx::query!(
        r#"insert into configuration_versions
              (id, tenant_id, artifact_id, proposal_id, ordinal, document,
               content_hash, source_template, created_by)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        version_id.as_uuid(),
        tenant.as_uuid(),
        artifact_id.as_uuid(),
        proposal_id.as_uuid(),
        ordinal,
        Json(document) as _,
        hash,
        source_template.map(ConfigurationTemplate::as_str),
        actor,
    )
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(storage_error)
}

async fn ensure_artifact_and_pin(
    connection: &mut PgConnection,
    tenant: TenantId,
    artifact_id: ConfigurationArtifactId,
    pinned_version_id: Option<ConfigurationVersionId>,
) -> Result<ConfigurationVersionId> {
    let artifact = artifact(&mut *connection, tenant, artifact_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("configuration artifact {artifact_id}"),
        })?;
    let version_id = pinned_version_id.unwrap_or(artifact.current_version_id);
    {
        let selected = version(&mut *connection, tenant, version_id)
            .await?
            .filter(|version| version.artifact_id == artifact_id)
            .ok_or_else(|| Error::NotFound {
                entity: format!("configuration version {version_id} on {artifact_id}"),
            })?;
        selected.document.validate()?;
    }
    Ok(version_id)
}

/// Record the applied result exactly once.
pub async fn complete_change(
    connection: &mut PgConnection,
    tenant: TenantId,
    proposal_id: ProposalId,
    result: AppliedConfiguration,
) -> Result<()> {
    let revision = result
        .binding_revision
        .map(i64::try_from)
        .transpose()
        .map_err(|_| Error::Invalid {
            message: "configuration binding revision exceeds database range".to_owned(),
        })?;
    let changed = sqlx::query!(
        r#"update configuration_changes
              set resulting_artifact_id = $3,
                  resulting_version_id = $4,
                  resulting_binding_id = $5,
                  resulting_binding_revision = $6,
                  applied_at = now()
            where tenant_id = $1 and proposal_id = $2 and applied_at is null"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
        result.artifact_id.map(|value| value.as_uuid()),
        result.version_id.map(|value| value.as_uuid()),
        result.binding_id.map(|value| value.as_uuid()),
        revision,
    )
    .execute(connection)
    .await
    .map_err(storage_error)?;
    if changed.rows_affected() != 1 {
        return Err(Error::Conflict {
            message: format!("configuration change {proposal_id} was already completed"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_validation_binds_the_document_hash() {
        let document = ConfigurationDocument::fail_safe();
        let command = ConfigurationCommand::Create {
            artifact_id: ConfigurationArtifactId::new(),
            version_id: ConfigurationVersionId::new(),
            governing_scope_id: ScopeId::new(),
            name: "enterprise".to_owned(),
            document: document.clone(),
            content_hash: document.content_hash().unwrap(),
            source_template: Some(ConfigurationTemplate::Enterprise),
        };
        validate_command(&command).unwrap();
        let ConfigurationCommand::Create {
            mut content_hash, ..
        } = command
        else {
            unreachable!()
        };
        let replacement = if content_hash.starts_with('0') {
            "1"
        } else {
            "0"
        };
        content_hash.replace_range(..1, replacement);
        let bad = ConfigurationCommand::Create {
            artifact_id: ConfigurationArtifactId::new(),
            version_id: ConfigurationVersionId::new(),
            governing_scope_id: ScopeId::new(),
            name: "enterprise-2".to_owned(),
            document,
            content_hash,
            source_template: None,
        };
        assert!(validate_command(&bad).is_err());
    }
}
