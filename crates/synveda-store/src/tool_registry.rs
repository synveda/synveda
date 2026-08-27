//! Trusted MCP server catalogue persistence (CPR-25, ADR-0086; migration
//! 0053).
//!
//! Versions, capability snapshots and test runs are immutable. A version's
//! trust state is projected from its one VedaFlow proposal, while a project
//! binding always stores an exact approved version id.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{
    CapabilitySnapshotId, Error, IdentityId, NormalizedCapabilities, ProjectId, ProposalId, Result,
    ScopeId, TenantId, ToolBindingId, ToolBindingState, ToolCommand, ToolMutationOutcome,
    ToolMutationResult, ToolServerDescriptor, ToolServerId, ToolServerVersionId, ToolTestHarness,
    ToolTestOutcome, ToolTestRunId, ToolVersionState,
};

/// One stable catalogue entry.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredToolServer {
    /// Stable id.
    pub id: ToolServerId,
    /// Governing scope.
    pub governing_scope_id: ScopeId,
    /// Tenant-unique display name.
    pub name: String,
    /// Current approved exact version, absent while the first version awaits review.
    pub current_version_id: Option<ToolServerVersionId>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Creator.
    pub created_by: IdentityId,
    /// Last approved-pointer transition.
    pub updated_at: DateTime<Utc>,
    /// Actor moving that pointer.
    pub updated_by: IdentityId,
}

/// One immutable version plus its immutable discovery snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredToolVersion {
    /// Immutable version id.
    pub id: ToolServerVersionId,
    /// Stable server id.
    pub server_id: ToolServerId,
    /// VedaFlow change that is the trust state.
    pub proposal_id: ProposalId,
    /// Monotonic ordinal.
    pub ordinal: u64,
    /// Digest over descriptor and normalised capabilities.
    pub digest: [u8; 32],
    /// Pinned MCP protocol version.
    pub protocol_version: String,
    /// Credential-free descriptor.
    pub descriptor: ToolServerDescriptor,
    /// Trust state projected from VedaFlow.
    pub state: ToolVersionState,
    /// Immutable snapshot id.
    pub snapshot_id: CapabilitySnapshotId,
    /// Raw discovery evidence.
    pub raw_capabilities: Value,
    /// Canonical comparison projection.
    pub normalized_capabilities: NormalizedCapabilities,
    /// Snapshot digest.
    pub capability_digest: [u8; 32],
    /// Discovery instant.
    pub discovered_at: DateTime<Utc>,
    /// Trusted reporting actor.
    pub discovered_by: IdentityId,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Creator.
    pub created_by: IdentityId,
}

/// One revisioned exact-version project binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredToolBinding {
    /// Stable binding id.
    pub id: ToolBindingId,
    /// Target project.
    pub project_id: ProjectId,
    /// Project scope derived through the project row.
    pub scope_id: ScopeId,
    /// Bound stable server.
    pub server_id: ToolServerId,
    /// Exact immutable approved version.
    pub version_id: ToolServerVersionId,
    /// Activation/removal state.
    pub state: ToolBindingState,
    /// Optimistic concurrency revision.
    pub revision: u64,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Creator.
    pub created_by: IdentityId,
    /// Last update instant.
    pub updated_at: DateTime<Utc>,
    /// Last updater.
    pub updated_by: IdentityId,
}

/// One immutable read-only test result.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredToolTestRun {
    /// Run id.
    pub id: ToolTestRunId,
    /// Exact version tested.
    pub version_id: ToolServerVersionId,
    /// Trusted harness class.
    pub harness: ToolTestHarness,
    /// Exact reporter version.
    pub harness_version: String,
    /// Terminal outcome.
    pub outcome: ToolTestOutcome,
    /// Closed read-only method set actually attempted.
    pub methods: Vec<String>,
    /// Elapsed milliseconds when measured.
    pub latency_ms: Option<u64>,
    /// Bounded, credential-free evidence.
    pub evidence: Value,
    /// Server receipt instant.
    pub created_at: DateTime<Utc>,
    /// Reporting actor.
    pub created_by: IdentityId,
}

/// Typed Tool/apply projection.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredToolChange {
    /// VedaFlow proposal id.
    pub proposal_id: ProposalId,
    /// Immutable command.
    pub command: ToolCommand,
    /// Canonical command hash.
    pub payload_hash: String,
    /// Applied server.
    pub resulting_server_id: Option<ToolServerId>,
    /// Applied version.
    pub resulting_version_id: Option<ToolServerVersionId>,
    /// Applied binding.
    pub resulting_binding_id: Option<ToolBindingId>,
    /// Applied binding revision.
    pub resulting_binding_revision: Option<u64>,
    /// Application instant.
    pub applied_at: Option<DateTime<Utc>>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        match db.code().as_deref() {
            Some("23503" | "23505" | "23514" | "P0001") => {
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

/// Decode one lowercase 32-byte hexadecimal digest.
pub fn decode_hex_32(value: &str, what: &str) -> Result<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Invalid {
            message: format!("{what} must be 64 hexadecimal characters"),
        });
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair).expect("hexadecimal is UTF-8");
        decoded[index] = u8::from_str_radix(pair, 16).map_err(|_| Error::Invalid {
            message: format!("{what} is invalid hexadecimal"),
        })?;
    }
    Ok(decoded)
}

/// Encode one 32-byte digest.
#[must_use]
pub fn hex_32(value: &[u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fixed_32(value: Vec<u8>, what: &str) -> Result<[u8; 32]> {
    value.try_into().map_err(|_| Error::Internal {
        message: format!("{what} is not 32 bytes"),
    })
}

fn version_state(value: &str) -> Result<ToolVersionState> {
    value.parse()
}

/// Stage the immutable rows named by a register/version command.
pub async fn stage_version(
    conn: &mut PgConnection,
    tenant: TenantId,
    proposal: ProposalId,
    command: &ToolCommand,
    actor: IdentityId,
) -> Result<()> {
    let (
        server_id,
        version_id,
        snapshot_id,
        scope_id,
        descriptor,
        digest,
        raw,
        normalized,
        ordinal,
    ) = match command {
        ToolCommand::Register {
            server_id,
            version_id,
            snapshot_id,
            governing_scope_id,
            name,
            descriptor,
            digest,
            raw_capabilities,
            normalized_capabilities,
        } => {
            sqlx::query!(
                r#"insert into tool_servers
                       (id, tenant_id, governing_scope_id, name, current_version_id,
                        created_by, updated_by)
                       values ($1, $2, $3, $4, null, $5, $5)"#,
                server_id.as_uuid(),
                tenant.as_uuid(),
                governing_scope_id.as_uuid(),
                name,
                actor.as_uuid(),
            )
            .execute(&mut *conn)
            .await
            .map_err(storage_error)?;
            (
                *server_id,
                *version_id,
                *snapshot_id,
                *governing_scope_id,
                descriptor,
                digest,
                raw_capabilities,
                normalized_capabilities,
                1_i64,
            )
        }
        ToolCommand::StageVersion {
            server_id,
            expected_current_version_id,
            version_id,
            snapshot_id,
            governing_scope_id,
            descriptor,
            digest,
            raw_capabilities,
            normalized_capabilities,
        } => {
            let next = sqlx::query_scalar!(
                r#"select coalesce(max(ordinal), 0) + 1 as "next!"
                         from tool_server_versions
                        where tenant_id = $1 and server_id = $2"#,
                tenant.as_uuid(),
                server_id.as_uuid(),
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(storage_error)?;
            let matches = sqlx::query_scalar!(
                r#"select exists(
                           select 1 from tool_servers
                            where tenant_id = $1 and id = $2
                              and governing_scope_id = $3
                              and current_version_id = $4
                       ) as "matches!""#,
                tenant.as_uuid(),
                server_id.as_uuid(),
                governing_scope_id.as_uuid(),
                expected_current_version_id.as_uuid(),
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(storage_error)?;
            if !matches {
                return Err(Error::Conflict {
                    message: format!(
                        "tool server {server_id} no longer has expected version {expected_current_version_id}"
                    ),
                });
            }
            (
                *server_id,
                *version_id,
                *snapshot_id,
                *governing_scope_id,
                descriptor,
                digest,
                raw_capabilities,
                normalized_capabilities,
                next,
            )
        }
        ToolCommand::Bind { .. } | ToolCommand::SetBinding { .. } => return Ok(()),
    };
    let digest = decode_hex_32(digest, "tool version digest")?;
    let descriptor = serde_json::to_value(descriptor).map_err(|err| Error::Invalid {
        message: format!("encode tool descriptor: {err}"),
    })?;
    sqlx::query!(
        r#"insert into tool_server_versions
           (id, tenant_id, server_id, proposal_id, ordinal, digest,
            protocol_version, descriptor, created_by)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        version_id.as_uuid(),
        tenant.as_uuid(),
        server_id.as_uuid(),
        proposal.as_uuid(),
        ordinal,
        digest.as_slice(),
        synveda_types::MCP_PROTOCOL_VERSION,
        descriptor,
        actor.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    let normalized_value = serde_json::to_value(normalized).map_err(|err| Error::Invalid {
        message: format!("encode normalized tool capabilities: {err}"),
    })?;
    let capability_digest = blake3::hash(normalized_value.to_string().as_bytes());
    sqlx::query!(
        r#"insert into capability_snapshots
           (id, tenant_id, version_id, raw, normalized, digest, discovered_at, discovered_by)
           values ($1, $2, $3, $4, $5, $6, now(), $7)"#,
        snapshot_id.as_uuid(),
        tenant.as_uuid(),
        version_id.as_uuid(),
        raw,
        normalized_value,
        capability_digest.as_bytes().as_slice(),
        actor.as_uuid(),
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    let _ = scope_id;
    Ok(())
}

/// Insert a typed Tool/apply projection.
pub async fn insert_change(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    proposal: ProposalId,
    command: &ToolCommand,
    payload_hash: &str,
) -> Result<()> {
    let payload = serde_json::to_value(command).map_err(|err| Error::Invalid {
        message: format!("encode Tool command: {err}"),
    })?;
    sqlx::query!(
        r#"insert into tool_changes
           (tenant_id, proposal_id, command_kind, payload, payload_hash)
           values ($1, $2, $3, $4, $5)"#,
        tenant.as_uuid(),
        proposal.as_uuid(),
        command.kind(),
        payload,
        payload_hash,
    )
    .execute(executor)
    .await
    .map(|_| ())
    .map_err(storage_error)
}

/// Load a typed change.
pub async fn change(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    proposal: ProposalId,
) -> Result<Option<StoredToolChange>> {
    let row = sqlx::query!(
        r#"select proposal_id, payload, payload_hash, resulting_server_id,
                  resulting_version_id, resulting_binding_id,
                  resulting_binding_revision, applied_at, created_at
             from tool_changes where tenant_id = $1 and proposal_id = $2"#,
        tenant.as_uuid(),
        proposal.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(StoredToolChange {
            proposal_id: row.proposal_id.into(),
            command: serde_json::from_value(row.payload).map_err(|err| Error::Internal {
                message: format!("stored Tool command is invalid: {err}"),
            })?,
            payload_hash: row.payload_hash,
            resulting_server_id: row.resulting_server_id.map(Into::into),
            resulting_version_id: row.resulting_version_id.map(Into::into),
            resulting_binding_id: row.resulting_binding_id.map(Into::into),
            resulting_binding_revision: row.resulting_binding_revision.map(|v| v as u64),
            applied_at: row.applied_at,
            created_at: row.created_at,
        })
    })
    .transpose()
}

/// Record a Tool change's one application result.
pub async fn finish_change(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    proposal: ProposalId,
    result: &ToolMutationResult,
) -> Result<bool> {
    let binding_revision = result.binding_revision.map(|value| value as i64);
    sqlx::query!(
        r#"update tool_changes
              set resulting_server_id = $3, resulting_version_id = $4,
                  resulting_binding_id = $5, resulting_binding_revision = $6,
                  applied_at = now()
            where tenant_id = $1 and proposal_id = $2 and applied_at is null"#,
        tenant.as_uuid(),
        proposal.as_uuid(),
        result.server_id.map(|id| id.as_uuid()),
        result.version_id.map(|id| id.as_uuid()),
        result.binding_id.map(|id| id.as_uuid()),
        binding_revision,
    )
    .execute(executor)
    .await
    .map(|done| done.rows_affected() == 1)
    .map_err(storage_error)
}

/// Advance a stable aggregate to a staged version under an exact precondition.
pub async fn approve_version(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    command: &ToolCommand,
    actor: IdentityId,
) -> Result<bool> {
    match command {
        ToolCommand::Register {
            server_id,
            version_id,
            ..
        } => sqlx::query!(
            r#"update tool_servers set current_version_id = $3, updated_at = now(), updated_by = $4
                where tenant_id = $1 and id = $2 and current_version_id is null"#,
            tenant.as_uuid(),
            server_id.as_uuid(),
            version_id.as_uuid(),
            actor.as_uuid(),
        )
        .execute(executor)
        .await
        .map(|done| done.rows_affected() == 1)
        .map_err(storage_error),
        ToolCommand::StageVersion {
            server_id,
            expected_current_version_id,
            version_id,
            ..
        } => sqlx::query!(
            r#"update tool_servers set current_version_id = $4, updated_at = now(), updated_by = $5
                    where tenant_id = $1 and id = $2 and current_version_id = $3"#,
            tenant.as_uuid(),
            server_id.as_uuid(),
            expected_current_version_id.as_uuid(),
            version_id.as_uuid(),
            actor.as_uuid(),
        )
        .execute(executor)
        .await
        .map(|done| done.rows_affected() == 1)
        .map_err(storage_error),
        ToolCommand::Bind { .. } | ToolCommand::SetBinding { .. } => Ok(false),
    }
}

/// Apply a new exact-version project binding.
pub async fn create_binding(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    command: &ToolCommand,
    actor: IdentityId,
) -> Result<Option<StoredToolBinding>> {
    let ToolCommand::Bind {
        binding_id,
        project_id,
        server_id,
        version_id,
        state,
        ..
    } = command
    else {
        return Ok(None);
    };
    sqlx::query!(
        r#"insert into tool_bindings
           (id, tenant_id, project_id, server_id, version_id, state, created_by, updated_by)
           values ($1, $2, $3, $4, $5, $6, $7, $7)"#,
        binding_id.as_uuid(),
        tenant.as_uuid(),
        project_id.as_uuid(),
        server_id.as_uuid(),
        version_id.as_uuid(),
        state.as_str(),
        actor.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(None)
}

/// Apply an optimistic binding transition.
pub async fn set_binding(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    command: &ToolCommand,
    actor: IdentityId,
) -> Result<bool> {
    let ToolCommand::SetBinding {
        binding_id,
        project_id,
        expected_revision,
        version_id,
        state,
        ..
    } = command
    else {
        return Ok(false);
    };
    let expected = *expected_revision as i64;
    sqlx::query!(
        r#"update tool_bindings
              set version_id = $5, state = $6, revision = revision + 1,
                  updated_at = now(), updated_by = $7
            where tenant_id = $1 and id = $2 and project_id = $3 and revision = $4"#,
        tenant.as_uuid(),
        binding_id.as_uuid(),
        project_id.as_uuid(),
        expected,
        version_id.as_uuid(),
        state.as_str(),
        actor.as_uuid(),
    )
    .execute(executor)
    .await
    .map(|done| done.rows_affected() == 1)
    .map_err(storage_error)
}

/// Load a stable server.
pub async fn server(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    id: ToolServerId,
) -> Result<Option<StoredToolServer>> {
    let row = sqlx::query!(
        r#"select id, governing_scope_id, name, current_version_id,
                  created_at, created_by, updated_at, updated_by
             from tool_servers where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(|row| StoredToolServer {
        id: row.id.into(),
        governing_scope_id: row.governing_scope_id.into(),
        name: row.name,
        current_version_id: row.current_version_id.map(Into::into),
        created_at: row.created_at,
        created_by: row.created_by.into(),
        updated_at: row.updated_at,
        updated_by: row.updated_by.into(),
    }))
}

/// Find a stable server by tenant-unique name.
pub async fn server_by_name(
    conn: &mut PgConnection,
    tenant: TenantId,
    name: &str,
) -> Result<Option<StoredToolServer>> {
    let row = sqlx::query_scalar!(
        "select id from tool_servers where tenant_id = $1 and name = $2",
        tenant.as_uuid(),
        name,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    match row {
        Some(id) => server(&mut *conn, tenant, id.into()).await,
        None => Ok(None),
    }
}

/// List stable catalogue entries in deterministic order.
pub async fn list_servers(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    cursor: Option<ToolServerId>,
    limit: i64,
) -> Result<Vec<StoredToolServer>> {
    let rows = sqlx::query!(
        r#"select id, governing_scope_id, name, current_version_id,
                  created_at, created_by, updated_at, updated_by
             from tool_servers where tenant_id = $1
              and ($2::uuid is null or id > $2)
             order by id limit $3"#,
        tenant.as_uuid(),
        cursor.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| StoredToolServer {
            id: row.id.into(),
            governing_scope_id: row.governing_scope_id.into(),
            name: row.name,
            current_version_id: row.current_version_id.map(Into::into),
            created_at: row.created_at,
            created_by: row.created_by.into(),
            updated_at: row.updated_at,
            updated_by: row.updated_by.into(),
        })
        .collect())
}

fn map_version(row: VersionRow) -> Result<StoredToolVersion> {
    Ok(StoredToolVersion {
        id: row.id.into(),
        server_id: row.server_id.into(),
        proposal_id: row.proposal_id.into(),
        ordinal: row.ordinal as u64,
        digest: fixed_32(row.digest, "tool version digest")?,
        protocol_version: row.protocol_version,
        descriptor: serde_json::from_value(row.descriptor).map_err(|err| Error::Internal {
            message: format!("stored tool descriptor is invalid: {err}"),
        })?,
        state: version_state(&row.state)?,
        snapshot_id: row.snapshot_id.into(),
        raw_capabilities: row.raw_capabilities,
        normalized_capabilities: serde_json::from_value(row.normalized_capabilities).map_err(
            |err| Error::Internal {
                message: format!("stored normalized tool capabilities are invalid: {err}"),
            },
        )?,
        capability_digest: fixed_32(row.capability_digest, "capability snapshot digest")?,
        discovered_at: row.discovered_at,
        discovered_by: row.discovered_by.into(),
        created_at: row.created_at,
        created_by: row.created_by.into(),
    })
}

struct VersionRow {
    id: uuid::Uuid,
    server_id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    ordinal: i64,
    digest: Vec<u8>,
    protocol_version: String,
    descriptor: Value,
    state: String,
    snapshot_id: uuid::Uuid,
    raw_capabilities: Value,
    normalized_capabilities: Value,
    capability_digest: Vec<u8>,
    discovered_at: DateTime<Utc>,
    discovered_by: uuid::Uuid,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
}

/// Load an immutable version and snapshot.
pub async fn version(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    id: ToolServerVersionId,
) -> Result<Option<StoredToolVersion>> {
    let row = sqlx::query_as!(
        VersionRow,
        r#"select version.id, version.server_id, version.proposal_id, version.ordinal,
                  version.digest, version.protocol_version, version.descriptor,
                  case proposal.state when 'open' then 'quarantined'
                       when 'applied' then 'approved' else 'rejected' end as "state!",
                  snapshot.id as snapshot_id, snapshot.raw as raw_capabilities,
                  snapshot.normalized as normalized_capabilities,
                  snapshot.digest as capability_digest, snapshot.discovered_at,
                  snapshot.discovered_by, version.created_at, version.created_by
             from tool_server_versions version
             join vedaflow_proposals proposal
               on proposal.tenant_id = version.tenant_id and proposal.id = version.proposal_id
             join capability_snapshots snapshot
               on snapshot.tenant_id = version.tenant_id and snapshot.version_id = version.id
            where version.tenant_id = $1 and version.id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(map_version).transpose()
}

/// List all immutable versions for a server, newest first.
pub async fn versions(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    server: ToolServerId,
    before_ordinal: Option<u64>,
    limit: i64,
) -> Result<Vec<StoredToolVersion>> {
    let before_ordinal = before_ordinal.map(|value| value as i64);
    let rows = sqlx::query_as!(
        VersionRow,
        r#"select version.id, version.server_id, version.proposal_id, version.ordinal,
                  version.digest, version.protocol_version, version.descriptor,
                  case proposal.state when 'open' then 'quarantined'
                       when 'applied' then 'approved' else 'rejected' end as "state!",
                  snapshot.id as snapshot_id, snapshot.raw as raw_capabilities,
                  snapshot.normalized as normalized_capabilities,
                  snapshot.digest as capability_digest, snapshot.discovered_at,
                  snapshot.discovered_by, version.created_at, version.created_by
             from tool_server_versions version
             join vedaflow_proposals proposal
               on proposal.tenant_id = version.tenant_id and proposal.id = version.proposal_id
             join capability_snapshots snapshot
               on snapshot.tenant_id = version.tenant_id and snapshot.version_id = version.id
            where version.tenant_id = $1 and version.server_id = $2
              and ($3::bigint is null or version.ordinal < $3)
            order by version.ordinal desc limit $4"#,
        tenant.as_uuid(),
        server.as_uuid(),
        before_ordinal,
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(map_version).collect()
}

/// Find an already staged identical version.
pub async fn version_by_digest(
    conn: &mut PgConnection,
    tenant: TenantId,
    server: ToolServerId,
    digest: &[u8; 32],
) -> Result<Option<StoredToolVersion>> {
    let id = sqlx::query_scalar!(
        r#"select id from tool_server_versions
            where tenant_id = $1 and server_id = $2 and digest = $3"#,
        tenant.as_uuid(),
        server.as_uuid(),
        digest.as_slice(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    match id {
        Some(id) => version(&mut *conn, tenant, id.into()).await,
        None => Ok(None),
    }
}

fn map_binding(row: BindingRow) -> Result<StoredToolBinding> {
    Ok(StoredToolBinding {
        id: row.id.into(),
        project_id: row.project_id.into(),
        scope_id: row.scope_id.into(),
        server_id: row.server_id.into(),
        version_id: row.version_id.into(),
        state: row.state.parse()?,
        revision: row.revision as u64,
        created_at: row.created_at,
        created_by: row.created_by.into(),
        updated_at: row.updated_at,
        updated_by: row.updated_by.into(),
    })
}

struct BindingRow {
    id: uuid::Uuid,
    project_id: uuid::Uuid,
    scope_id: uuid::Uuid,
    server_id: uuid::Uuid,
    version_id: uuid::Uuid,
    state: String,
    revision: i64,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

/// Load one binding and its project scope.
pub async fn binding(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    id: ToolBindingId,
) -> Result<Option<StoredToolBinding>> {
    let row = sqlx::query_as!(
        BindingRow,
        r#"select binding.id, binding.project_id, project.scope_id, binding.server_id,
                  binding.version_id, binding.state, binding.revision, binding.created_at,
                  binding.created_by, binding.updated_at, binding.updated_by
             from tool_bindings binding
             join projects project on project.tenant_id = binding.tenant_id
                                  and project.id = binding.project_id
            where binding.tenant_id = $1 and binding.id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(map_binding).transpose()
}

/// List project bindings, optionally including removed history handles.
pub async fn bindings(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    project_id: Option<ProjectId>,
    include_removed: bool,
    cursor: Option<ToolBindingId>,
    limit: i64,
) -> Result<Vec<StoredToolBinding>> {
    let rows = sqlx::query_as!(
        BindingRow,
        r#"select binding.id, binding.project_id, project.scope_id, binding.server_id,
                  binding.version_id, binding.state, binding.revision, binding.created_at,
                  binding.created_by, binding.updated_at, binding.updated_by
             from tool_bindings binding
             join projects project on project.tenant_id = binding.tenant_id
                                  and project.id = binding.project_id
            where binding.tenant_id = $1
              and ($2::uuid is null or binding.project_id = $2)
              and ($3 or binding.state <> 'removed')
              and ($4::uuid is null or binding.id < $4)
            order by binding.id desc limit $5"#,
        tenant.as_uuid(),
        project_id.map(|id| id.as_uuid()),
        include_removed,
        cursor.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(map_binding).collect()
}

/// Insert immutable read-only connection-test evidence.
#[allow(clippy::too_many_arguments)]
pub async fn insert_test_run(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    id: ToolTestRunId,
    version_id: ToolServerVersionId,
    harness: ToolTestHarness,
    harness_version: &str,
    outcome: ToolTestOutcome,
    methods: &[String],
    latency_ms: Option<u64>,
    evidence: &Value,
    actor: IdentityId,
) -> Result<()> {
    let latency = latency_ms.map(|value| value as i64);
    sqlx::query!(
        r#"insert into tool_test_runs
           (id, tenant_id, version_id, harness, harness_version, outcome,
            methods, latency_ms, evidence, created_by)
           values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        id.as_uuid(),
        tenant.as_uuid(),
        version_id.as_uuid(),
        harness.as_str(),
        harness_version,
        outcome.as_str(),
        methods,
        latency,
        evidence,
        actor.as_uuid(),
    )
    .execute(executor)
    .await
    .map(|_| ())
    .map_err(storage_error)
}

/// List immutable test evidence for an exact version.
pub async fn test_runs(
    executor: impl PgExecutor<'_>,
    tenant: TenantId,
    version_id: ToolServerVersionId,
    cursor: Option<ToolTestRunId>,
    limit: i64,
) -> Result<Vec<StoredToolTestRun>> {
    let rows = sqlx::query!(
        r#"select id, version_id, harness, harness_version, outcome, methods,
                  latency_ms, evidence, created_at, created_by
             from tool_test_runs
            where tenant_id = $1 and version_id = $2
              and ($3::uuid is null or id < $3)
            order by id desc limit $4"#,
        tenant.as_uuid(),
        version_id.as_uuid(),
        cursor.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(StoredToolTestRun {
                id: row.id.into(),
                version_id: row.version_id.into(),
                harness: row.harness.parse()?,
                harness_version: row.harness_version,
                outcome: row.outcome.parse()?,
                methods: row.methods,
                latency_ms: row.latency_ms.map(|value| value as u64),
                evidence: row.evidence,
                created_at: row.created_at,
                created_by: row.created_by.into(),
            })
        })
        .collect()
}

/// Render a change projection with the workflow outcome supplied by caller.
#[must_use]
pub fn mutation_result(
    change: &StoredToolChange,
    outcome: ToolMutationOutcome,
) -> ToolMutationResult {
    ToolMutationResult {
        change_id: change.proposal_id,
        outcome,
        server_id: change
            .resulting_server_id
            .or_else(|| change.command.server_id()),
        version_id: change
            .resulting_version_id
            .or(Some(change.command.version_id())),
        binding_id: change
            .resulting_binding_id
            .or_else(|| change.command.binding_id()),
        binding_revision: change.resulting_binding_revision,
    }
}
