//! Stable Agent Skill aggregates, immutable versions and scope bindings
//! (CPR-23, ADR-0085; migration 0052).
//!
//! Bundle bytes remain in the VedaFlow object store. This module projects
//! approved versions and their active bindings; it never authors a mutable
//! draft and never reads a skill channel.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use synveda_types::{
    Error, IdentityId, ProposalId, Result, ScopeId, Sensitivity, SessionId, SkillBindingId,
    SkillCommand, SkillFilePath, SkillId, SkillMutationOutcome, SkillMutationResult, SkillName,
    SkillProvenance, SkillSourceKind, SkillTestHarness, SkillTestOutcome, SkillTestRunId,
    SkillUsageEventId, SkillUsageEvidence, SkillUsageStage, SkillVersionFileRef, SkillVersionId,
    TenantId,
};

/// One stable catalogue aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSkill {
    /// Stable id.
    pub id: SkillId,
    /// Governing scope.
    pub governing_scope_id: ScopeId,
    /// Tenant-unique Agent Skills name.
    pub name: SkillName,
    /// Current approved immutable version.
    pub current_version_id: SkillVersionId,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Creator.
    pub created_by: IdentityId,
    /// Last current-pointer transition.
    pub updated_at: DateTime<Utc>,
    /// Actor who advanced the pointer.
    pub updated_by: IdentityId,
}

/// One immutable version.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredVersion {
    /// Immutable version id.
    pub id: SkillVersionId,
    /// Stable aggregate id.
    pub skill_id: SkillId,
    /// Monotonic ordinal inside the aggregate.
    pub ordinal: u64,
    /// Digest over ordered path/object-address pairs.
    pub bundle_digest: [u8; 32],
    /// Bundle sensitivity.
    pub sensitivity: Sensitivity,
    /// Parsed manifest projection.
    pub manifest: Value,
    /// Source kind.
    pub source_kind: SkillSourceKind,
    /// Retained provenance.
    pub provenance: SkillProvenance,
    /// Retained content-free scan report.
    pub scan_report: Value,
    /// Scanner ruleset.
    pub scan_ruleset_version: u32,
    /// Automated quality score.
    pub quality_score: u8,
    /// Rubric version.
    pub rubric_version: u32,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Creator.
    pub created_by: IdentityId,
}

/// One immutable version file reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredVersionFile {
    /// Version.
    pub version_id: SkillVersionId,
    /// Bundle-relative path.
    pub path: SkillFilePath,
    /// VedaFlow object address.
    pub object_hash: [u8; 32],
    /// UTF-8 character count.
    pub chars: u32,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// One revisioned binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBinding {
    /// Stable binding id.
    pub id: SkillBindingId,
    /// Target project/principal scope.
    pub scope_id: ScopeId,
    /// Bound stable skill.
    pub skill_id: SkillId,
    /// Exact pin, absent when following current.
    pub pinned_version_id: Option<SkillVersionId>,
    /// Active switch.
    pub enabled: bool,
    /// Optimistic concurrency revision.
    pub revision: u64,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Creator.
    pub created_by: IdentityId,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
    /// Last updater.
    pub updated_by: IdentityId,
}

/// One enabled binding resolved to its exact immutable version.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedBinding {
    /// Binding.
    pub binding: StoredBinding,
    /// Stable skill name.
    pub name: SkillName,
    /// Exact resolved version.
    pub version: StoredVersion,
    /// `SKILL.md` object address.
    pub manifest_object_hash: [u8; 32],
}

/// One append-only usage event.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredUsageEvent {
    /// Event id.
    pub id: SkillUsageEventId,
    /// Binding.
    pub binding_id: SkillBindingId,
    /// Exact version.
    pub version_id: SkillVersionId,
    /// Session when present.
    pub session_id: Option<SessionId>,
    /// Principal observed/reported.
    pub principal_id: IdentityId,
    /// Client idempotency key.
    pub client_event_id: String,
    /// Lifecycle stage.
    pub stage: SkillUsageStage,
    /// Evidence authority.
    pub evidence: SkillUsageEvidence,
    /// Loaded/requested resource when applicable.
    pub resource_path: Option<SkillFilePath>,
    /// Bounded content-free metadata.
    pub metadata: Value,
    /// Client occurrence instant.
    pub occurred_at: DateTime<Utc>,
    /// Server receipt instant.
    pub received_at: DateTime<Utc>,
}

/// One immutable controlled-harness test run.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredTestRun {
    /// Run id.
    pub id: SkillTestRunId,
    /// Exact version.
    pub version_id: SkillVersionId,
    /// Harness class.
    pub harness: SkillTestHarness,
    /// Exact harness implementation/version.
    pub harness_version: String,
    /// Terminal outcome.
    pub outcome: SkillTestOutcome,
    /// Scanner ruleset.
    pub scan_ruleset_version: u32,
    /// Quality rubric.
    pub rubric_version: u32,
    /// Content-free evidence.
    pub evidence: Value,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Actor requesting/reporting it.
    pub created_by: IdentityId,
}

/// Typed Skill/apply projection.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredChange {
    /// VedaFlow proposal id.
    pub proposal_id: ProposalId,
    /// Typed immutable command.
    pub command: SkillCommand,
    /// Canonical command hash.
    pub payload_hash: String,
    /// Applied skill id.
    pub resulting_skill_id: Option<SkillId>,
    /// Applied version id.
    pub resulting_version_id: Option<SkillVersionId>,
    /// Applied binding id.
    pub resulting_binding_id: Option<SkillBindingId>,
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

fn fixed_32(value: Vec<u8>, what: &str) -> Result<[u8; 32]> {
    <[u8; 32]>::try_from(value.as_slice()).map_err(|_| Error::Internal {
        message: format!("{what} is not 32 bytes"),
    })
}

struct SkillRow {
    id: uuid::Uuid,
    governing_scope_id: uuid::Uuid,
    name: String,
    current_version_id: uuid::Uuid,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

impl TryFrom<SkillRow> for StoredSkill {
    type Error = Error;

    fn try_from(row: SkillRow) -> Result<Self> {
        Ok(Self {
            id: SkillId::from_uuid(row.id),
            governing_scope_id: ScopeId::from_uuid(row.governing_scope_id),
            name: row.name.parse()?,
            current_version_id: SkillVersionId::from_uuid(row.current_version_id),
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    }
}

struct VersionRow {
    id: uuid::Uuid,
    skill_id: uuid::Uuid,
    ordinal: i64,
    bundle_digest: Vec<u8>,
    sensitivity: String,
    manifest: Value,
    source_kind: String,
    provenance: Value,
    scan_report: Value,
    scan_ruleset_version: i32,
    quality_score: i16,
    rubric_version: i32,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
}

impl TryFrom<VersionRow> for StoredVersion {
    type Error = Error;

    fn try_from(row: VersionRow) -> Result<Self> {
        let provenance = serde_json::from_value(row.provenance).map_err(|err| Error::Internal {
            message: format!("stored skill provenance is invalid: {err}"),
        })?;
        Ok(Self {
            id: SkillVersionId::from_uuid(row.id),
            skill_id: SkillId::from_uuid(row.skill_id),
            ordinal: u64::try_from(row.ordinal).map_err(|_| Error::Internal {
                message: "stored skill version ordinal is negative".to_owned(),
            })?,
            bundle_digest: fixed_32(row.bundle_digest, "stored skill bundle digest")?,
            sensitivity: row.sensitivity.parse()?,
            manifest: row.manifest,
            source_kind: row.source_kind.parse()?,
            provenance,
            scan_report: row.scan_report,
            scan_ruleset_version: row.scan_ruleset_version.max(0).unsigned_abs(),
            quality_score: row.quality_score.clamp(0, 100) as u8,
            rubric_version: row.rubric_version.max(0).unsigned_abs(),
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
        })
    }
}

struct FileRow {
    version_id: uuid::Uuid,
    path: String,
    object_hash: Vec<u8>,
    chars: i32,
    created_at: DateTime<Utc>,
}

impl TryFrom<FileRow> for StoredVersionFile {
    type Error = Error;

    fn try_from(row: FileRow) -> Result<Self> {
        Ok(Self {
            version_id: SkillVersionId::from_uuid(row.version_id),
            path: row.path.parse()?,
            object_hash: fixed_32(row.object_hash, "stored skill file object address")?,
            chars: row.chars.max(0) as u32,
            created_at: row.created_at,
        })
    }
}

struct BindingRow {
    id: uuid::Uuid,
    scope_id: uuid::Uuid,
    skill_id: uuid::Uuid,
    pinned_version_id: Option<uuid::Uuid>,
    enabled: bool,
    revision: i64,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
    updated_at: DateTime<Utc>,
    updated_by: uuid::Uuid,
}

impl TryFrom<BindingRow> for StoredBinding {
    type Error = Error;

    fn try_from(row: BindingRow) -> Result<Self> {
        Ok(Self {
            id: SkillBindingId::from_uuid(row.id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            skill_id: SkillId::from_uuid(row.skill_id),
            pinned_version_id: row.pinned_version_id.map(SkillVersionId::from_uuid),
            enabled: row.enabled,
            revision: u64::try_from(row.revision).map_err(|_| Error::Internal {
                message: "stored skill binding revision is negative".to_owned(),
            })?,
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
            updated_at: row.updated_at,
            updated_by: IdentityId::from_uuid(row.updated_by),
        })
    }
}

struct ResolvedRow {
    binding_id: uuid::Uuid,
    scope_id: uuid::Uuid,
    skill_id: uuid::Uuid,
    pinned_version_id: Option<uuid::Uuid>,
    enabled: bool,
    binding_revision: i64,
    binding_created_at: DateTime<Utc>,
    binding_created_by: uuid::Uuid,
    binding_updated_at: DateTime<Utc>,
    binding_updated_by: uuid::Uuid,
    name: String,
    version_id: uuid::Uuid,
    ordinal: i64,
    bundle_digest: Vec<u8>,
    sensitivity: String,
    manifest: Value,
    source_kind: String,
    provenance: Value,
    scan_report: Value,
    scan_ruleset_version: i32,
    quality_score: i16,
    rubric_version: i32,
    version_created_at: DateTime<Utc>,
    version_created_by: uuid::Uuid,
    manifest_object_hash: Vec<u8>,
}

impl TryFrom<ResolvedRow> for ResolvedBinding {
    type Error = Error;

    fn try_from(row: ResolvedRow) -> Result<Self> {
        Ok(Self {
            binding: StoredBinding {
                id: SkillBindingId::from_uuid(row.binding_id),
                scope_id: ScopeId::from_uuid(row.scope_id),
                skill_id: SkillId::from_uuid(row.skill_id),
                pinned_version_id: row.pinned_version_id.map(SkillVersionId::from_uuid),
                enabled: row.enabled,
                revision: row.binding_revision.max(0) as u64,
                created_at: row.binding_created_at,
                created_by: IdentityId::from_uuid(row.binding_created_by),
                updated_at: row.binding_updated_at,
                updated_by: IdentityId::from_uuid(row.binding_updated_by),
            },
            name: row.name.parse()?,
            version: StoredVersion::try_from(VersionRow {
                id: row.version_id,
                skill_id: row.skill_id,
                ordinal: row.ordinal,
                bundle_digest: row.bundle_digest,
                sensitivity: row.sensitivity,
                manifest: row.manifest,
                source_kind: row.source_kind,
                provenance: row.provenance,
                scan_report: row.scan_report,
                scan_ruleset_version: row.scan_ruleset_version,
                quality_score: row.quality_score,
                rubric_version: row.rubric_version,
                created_at: row.version_created_at,
                created_by: row.version_created_by,
            })?,
            manifest_object_hash: fixed_32(
                row.manifest_object_hash,
                "resolved skill manifest object address",
            )?,
        })
    }
}

struct UsageRow {
    id: uuid::Uuid,
    binding_id: uuid::Uuid,
    version_id: uuid::Uuid,
    session_id: Option<uuid::Uuid>,
    principal_id: uuid::Uuid,
    client_event_id: String,
    stage: String,
    evidence: String,
    resource_path: Option<String>,
    metadata: Value,
    occurred_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
}

impl TryFrom<UsageRow> for StoredUsageEvent {
    type Error = Error;

    fn try_from(row: UsageRow) -> Result<Self> {
        Ok(Self {
            id: SkillUsageEventId::from_uuid(row.id),
            binding_id: SkillBindingId::from_uuid(row.binding_id),
            version_id: SkillVersionId::from_uuid(row.version_id),
            session_id: row.session_id.map(SessionId::from_uuid),
            principal_id: IdentityId::from_uuid(row.principal_id),
            client_event_id: row.client_event_id,
            stage: row.stage.parse()?,
            evidence: row.evidence.parse()?,
            resource_path: row.resource_path.map(|value| value.parse()).transpose()?,
            metadata: row.metadata,
            occurred_at: row.occurred_at,
            received_at: row.received_at,
        })
    }
}

struct TestRunRow {
    id: uuid::Uuid,
    version_id: uuid::Uuid,
    harness: String,
    harness_version: String,
    outcome: String,
    scan_ruleset_version: i32,
    rubric_version: i32,
    evidence: Value,
    created_at: DateTime<Utc>,
    created_by: uuid::Uuid,
}

impl TryFrom<TestRunRow> for StoredTestRun {
    type Error = Error;

    fn try_from(row: TestRunRow) -> Result<Self> {
        Ok(Self {
            id: SkillTestRunId::from_uuid(row.id),
            version_id: SkillVersionId::from_uuid(row.version_id),
            harness: row.harness.parse()?,
            harness_version: row.harness_version,
            outcome: row.outcome.parse()?,
            scan_ruleset_version: row.scan_ruleset_version.max(0).unsigned_abs(),
            rubric_version: row.rubric_version.max(0).unsigned_abs(),
            evidence: row.evidence,
            created_at: row.created_at,
            created_by: IdentityId::from_uuid(row.created_by),
        })
    }
}

struct ChangeRow {
    proposal_id: uuid::Uuid,
    payload: Value,
    payload_hash: String,
    resulting_skill_id: Option<uuid::Uuid>,
    resulting_version_id: Option<uuid::Uuid>,
    resulting_binding_id: Option<uuid::Uuid>,
    resulting_binding_revision: Option<i64>,
    applied_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl TryFrom<ChangeRow> for StoredChange {
    type Error = Error;

    fn try_from(row: ChangeRow) -> Result<Self> {
        let command = serde_json::from_value(row.payload).map_err(|err| Error::Internal {
            message: format!("stored Skill change payload is invalid: {err}"),
        })?;
        Ok(Self {
            proposal_id: ProposalId::from_uuid(row.proposal_id),
            command,
            payload_hash: row.payload_hash,
            resulting_skill_id: row.resulting_skill_id.map(SkillId::from_uuid),
            resulting_version_id: row.resulting_version_id.map(SkillVersionId::from_uuid),
            resulting_binding_id: row.resulting_binding_id.map(SkillBindingId::from_uuid),
            resulting_binding_revision: row
                .resulting_binding_revision
                .map(|value| value.max(0) as u64),
            applied_at: row.applied_at,
            created_at: row.created_at,
        })
    }
}

/// Read a stable skill by id.
pub async fn by_id<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    id: SkillId,
) -> Result<Option<StoredSkill>> {
    let row = sqlx::query_as!(
        SkillRow,
        r#"select id, governing_scope_id, name, current_version_id, created_at, created_by,
                  updated_at, updated_by
           from skills where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredSkill::try_from).transpose()
}

/// Read a stable skill by tenant-unique name.
pub async fn by_name<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    name: &SkillName,
) -> Result<Option<StoredSkill>> {
    let row = sqlx::query_as!(
        SkillRow,
        r#"select id, governing_scope_id, name, current_version_id, created_at, created_by,
                  updated_at, updated_by
           from skills where tenant_id = $1 and name = $2"#,
        tenant.as_uuid(),
        name.as_str(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredSkill::try_from).transpose()
}

/// Keyset-page stable skills using a reusable connection.
pub async fn list(
    conn: &mut PgConnection,
    tenant: TenantId,
    after: Option<SkillId>,
    limit: i64,
) -> Result<Vec<StoredSkill>> {
    let rows = sqlx::query_as!(
        SkillRow,
        r#"select id, governing_scope_id, name, current_version_id, created_at, created_by,
                  updated_at, updated_by
           from skills
           where tenant_id = $1 and ($2::uuid is null or id > $2)
           order by id limit $3"#,
        tenant.as_uuid(),
        after.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredSkill::try_from).collect()
}

/// Read one immutable version.
pub async fn version<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    id: SkillVersionId,
) -> Result<Option<StoredVersion>> {
    let row = sqlx::query_as!(
        VersionRow,
        r#"select id, skill_id, ordinal, bundle_digest, sensitivity, manifest, source_kind,
                  provenance, scan_report, scan_ruleset_version, quality_score, rubric_version,
                  created_at, created_by
           from skill_versions where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredVersion::try_from).transpose()
}

/// Versions of one skill, newest ordinal first.
pub async fn versions(
    conn: &mut PgConnection,
    tenant: TenantId,
    skill_id: SkillId,
    before_ordinal: Option<u64>,
    limit: i64,
) -> Result<Vec<StoredVersion>> {
    let rows = sqlx::query_as!(
        VersionRow,
        r#"select id, skill_id, ordinal, bundle_digest, sensitivity, manifest, source_kind,
                  provenance, scan_report, scan_ruleset_version, quality_score, rubric_version,
                  created_at, created_by
           from skill_versions
           where tenant_id = $1 and skill_id = $2
             and ($3::bigint is null or ordinal < $3)
           order by ordinal desc limit $4"#,
        tenant.as_uuid(),
        skill_id.as_uuid(),
        before_ordinal.map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
        limit,
    )
    .fetch_all(conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredVersion::try_from).collect()
}

/// Files of one immutable version in path order.
pub async fn files<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    version_id: SkillVersionId,
) -> Result<Vec<StoredVersionFile>> {
    let rows = sqlx::query_as!(
        FileRow,
        r#"select version_id, path, object_hash, chars, created_at
           from skill_version_files
           where tenant_id = $1 and version_id = $2 order by path"#,
        tenant.as_uuid(),
        version_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredVersionFile::try_from).collect()
}

/// One file of one immutable version.
pub async fn file<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    version_id: SkillVersionId,
    path: &SkillFilePath,
) -> Result<Option<StoredVersionFile>> {
    let row = sqlx::query_as!(
        FileRow,
        r#"select version_id, path, object_hash, chars, created_at
           from skill_version_files
           where tenant_id = $1 and version_id = $2 and path = $3"#,
        tenant.as_uuid(),
        version_id.as_uuid(),
        path.as_str(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredVersionFile::try_from).transpose()
}

/// Insert a stable aggregate and its first immutable version.
pub async fn install(
    conn: &mut PgConnection,
    tenant: TenantId,
    command: &SkillCommand,
    actor: IdentityId,
) -> Result<(StoredSkill, StoredVersion)> {
    let SkillCommand::Install {
        skill_id,
        version_id,
        governing_scope_id,
        name,
        sensitivity,
        bundle_digest,
        manifest,
        files,
        provenance,
        scan,
        scan_ruleset_version,
        quality_score,
        rubric_version,
    } = command
    else {
        return Err(Error::Internal {
            message: "skills::install received a non-install command".to_owned(),
        });
    };
    let digest = decode_hex_32(bundle_digest, "skill bundle digest")?;
    let skill_row = sqlx::query_as!(
        SkillRow,
        r#"insert into skills
               (id, tenant_id, governing_scope_id, name, current_version_id, created_by, updated_by)
           values ($1, $2, $3, $4, $5, $6, $6)
           returning id, governing_scope_id, name, current_version_id, created_at, created_by,
                     updated_at, updated_by"#,
        skill_id.as_uuid(),
        tenant.as_uuid(),
        governing_scope_id.as_uuid(),
        name.as_str(),
        version_id.as_uuid(),
        actor.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    let version = insert_version(
        conn,
        tenant,
        *skill_id,
        *version_id,
        1,
        *sensitivity,
        &digest,
        manifest,
        provenance,
        scan,
        *scan_ruleset_version,
        *quality_score,
        *rubric_version,
        files,
        actor,
    )
    .await?;
    Ok((StoredSkill::try_from(skill_row)?, version))
}

/// Insert and advance a new immutable version under an exact precondition.
pub async fn update(
    conn: &mut PgConnection,
    tenant: TenantId,
    command: &SkillCommand,
    actor: IdentityId,
) -> Result<Option<(StoredSkill, StoredVersion)>> {
    let SkillCommand::Update {
        skill_id,
        expected_current_version_id,
        version_id,
        governing_scope_id,
        name,
        sensitivity,
        bundle_digest,
        manifest,
        files,
        provenance,
        scan,
        scan_ruleset_version,
        quality_score,
        rubric_version,
    } = command
    else {
        return Err(Error::Internal {
            message: "skills::update received a non-update command".to_owned(),
        });
    };
    let current = sqlx::query!(
        r#"select current_version_id from skills
           where tenant_id = $1 and id = $2 and governing_scope_id = $3 and name = $4
           for update"#,
        tenant.as_uuid(),
        skill_id.as_uuid(),
        governing_scope_id.as_uuid(),
        name.as_str(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(current) = current else {
        return Ok(None);
    };
    if current.current_version_id != expected_current_version_id.as_uuid() {
        return Ok(None);
    }
    let ordinal = sqlx::query_scalar!(
        r#"select coalesce(max(ordinal), 0) + 1 as "ordinal!"
           from skill_versions where tenant_id = $1 and skill_id = $2"#,
        tenant.as_uuid(),
        skill_id.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    let digest = decode_hex_32(bundle_digest, "skill bundle digest")?;
    let version = insert_version(
        conn,
        tenant,
        *skill_id,
        *version_id,
        ordinal,
        *sensitivity,
        &digest,
        manifest,
        provenance,
        scan,
        *scan_ruleset_version,
        *quality_score,
        *rubric_version,
        files,
        actor,
    )
    .await?;
    let row = sqlx::query_as!(
        SkillRow,
        r#"update skills set current_version_id = $3, updated_at = clock_timestamp(), updated_by = $4
           where tenant_id = $1 and id = $2 and current_version_id = $5
           returning id, governing_scope_id, name, current_version_id, created_at, created_by,
                     updated_at, updated_by"#,
        tenant.as_uuid(),
        skill_id.as_uuid(),
        version_id.as_uuid(),
        actor.as_uuid(),
        expected_current_version_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    row.map(StoredSkill::try_from)
        .transpose()
        .map(|skill| skill.map(|skill| (skill, version)))
}

#[allow(clippy::too_many_arguments)]
async fn insert_version(
    conn: &mut PgConnection,
    tenant: TenantId,
    skill_id: SkillId,
    version_id: SkillVersionId,
    ordinal: i64,
    sensitivity: Sensitivity,
    bundle_digest: &[u8; 32],
    manifest: &Value,
    provenance: &SkillProvenance,
    scan: &Value,
    scan_ruleset_version: u32,
    quality_score: u8,
    rubric_version: u32,
    files: &[SkillVersionFileRef],
    actor: IdentityId,
) -> Result<StoredVersion> {
    let provenance_json = serde_json::to_value(provenance).map_err(|err| Error::Invalid {
        message: format!("encode skill provenance: {err}"),
    })?;
    let row = sqlx::query_as!(
        VersionRow,
        r#"insert into skill_versions
               (id, tenant_id, skill_id, ordinal, bundle_digest, sensitivity, manifest,
                source_kind, provenance, scan_report, scan_ruleset_version, quality_score,
                rubric_version, created_by)
           values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
           returning id, skill_id, ordinal, bundle_digest, sensitivity, manifest, source_kind,
                     provenance, scan_report, scan_ruleset_version, quality_score, rubric_version,
                     created_at, created_by"#,
        version_id.as_uuid(),
        tenant.as_uuid(),
        skill_id.as_uuid(),
        ordinal,
        &bundle_digest[..],
        sensitivity.as_str(),
        manifest,
        provenance.kind.as_str(),
        provenance_json,
        scan,
        i32::try_from(scan_ruleset_version).unwrap_or(i32::MAX),
        i16::from(quality_score),
        i32::try_from(rubric_version).unwrap_or(i32::MAX),
        actor.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    for file in files {
        let object_hash = decode_hex_32(&file.object_hash, "skill file object address")?;
        sqlx::query!(
            r#"insert into skill_version_files (tenant_id, version_id, path, object_hash, chars)
               values ($1,$2,$3,$4,$5)"#,
            tenant.as_uuid(),
            version_id.as_uuid(),
            file.path.as_str(),
            &object_hash[..],
            i32::try_from(file.chars).unwrap_or(i32::MAX),
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;
    }
    StoredVersion::try_from(row)
}

/// Read one binding.
pub async fn binding<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    id: SkillBindingId,
) -> Result<Option<StoredBinding>> {
    let row = sqlx::query_as!(
        BindingRow,
        r#"select id, scope_id, skill_id, pinned_version_id, enabled, revision, created_at,
                  created_by, updated_at, updated_by
           from skill_bindings where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredBinding::try_from).transpose()
}

/// Bind a skill at a project/principal scope.
pub async fn bind(
    conn: &mut PgConnection,
    tenant: TenantId,
    command: &SkillCommand,
    actor: IdentityId,
) -> Result<StoredBinding> {
    let SkillCommand::Bind {
        binding_id,
        skill_id,
        scope_id,
        pinned_version_id,
        enabled,
    } = command
    else {
        return Err(Error::Internal {
            message: "skills::bind received a non-bind command".to_owned(),
        });
    };
    let row = sqlx::query_as!(
        BindingRow,
        r#"insert into skill_bindings
               (id, tenant_id, scope_id, skill_id, pinned_version_id, enabled, created_by, updated_by)
           values ($1,$2,$3,$4,$5,$6,$7,$7)
           returning id, scope_id, skill_id, pinned_version_id, enabled, revision, created_at,
                     created_by, updated_at, updated_by"#,
        binding_id.as_uuid(),
        tenant.as_uuid(),
        scope_id.as_uuid(),
        skill_id.as_uuid(),
        pinned_version_id.map(|id| id.as_uuid()),
        *enabled,
        actor.as_uuid(),
    )
    .fetch_one(conn)
    .await
    .map_err(storage_error)?;
    StoredBinding::try_from(row)
}

/// Apply one exact binding state transition; `None` means stale/missing.
pub async fn set_binding(
    conn: &mut PgConnection,
    tenant: TenantId,
    command: &SkillCommand,
    actor: IdentityId,
) -> Result<Option<StoredBinding>> {
    let SkillCommand::SetBinding {
        binding_id,
        scope_id,
        expected_revision,
        enabled,
        pinned_version_id,
        ..
    } = command
    else {
        return Err(Error::Internal {
            message: "skills::set_binding received another command".to_owned(),
        });
    };
    let row = sqlx::query_as!(
        BindingRow,
        r#"update skill_bindings
           set enabled = $5, pinned_version_id = $6, revision = revision + 1,
               updated_at = clock_timestamp(), updated_by = $7
           where tenant_id = $1 and id = $2 and scope_id = $3 and revision = $4
           returning id, scope_id, skill_id, pinned_version_id, enabled, revision, created_at,
                     created_by, updated_at, updated_by"#,
        tenant.as_uuid(),
        binding_id.as_uuid(),
        scope_id.as_uuid(),
        i64::try_from(*expected_revision).unwrap_or(i64::MAX),
        *enabled,
        pinned_version_id.map(|id| id.as_uuid()),
        actor.as_uuid(),
    )
    .fetch_optional(conn)
    .await
    .map_err(storage_error)?;
    row.map(StoredBinding::try_from).transpose()
}

/// Bindings at one scope, in stable id order.
pub async fn bindings_at(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope_id: ScopeId,
    after: Option<SkillBindingId>,
    limit: i64,
) -> Result<Vec<StoredBinding>> {
    let rows = sqlx::query_as!(
        BindingRow,
        r#"select id, scope_id, skill_id, pinned_version_id, enabled, revision, created_at,
                  created_by, updated_at, updated_by
           from skill_bindings
           where tenant_id = $1 and scope_id = $2 and ($3::uuid is null or id > $3)
           order by id limit $4"#,
        tenant.as_uuid(),
        scope_id.as_uuid(),
        after.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredBinding::try_from).collect()
}

/// Enabled bindings resolved to exact versions for a scope set.
pub async fn resolve_for_scopes(
    conn: &mut PgConnection,
    tenant: TenantId,
    scope_ids: &[ScopeId],
) -> Result<Vec<ResolvedBinding>> {
    let ids: Vec<uuid::Uuid> = scope_ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query_as!(
        ResolvedRow,
        r#"select b.id as binding_id, b.scope_id, b.skill_id, b.pinned_version_id,
                  b.enabled, b.revision as binding_revision,
                  b.created_at as binding_created_at, b.created_by as binding_created_by,
                  b.updated_at as binding_updated_at, b.updated_by as binding_updated_by,
                  s.name, v.id as version_id, v.ordinal, v.bundle_digest, v.sensitivity,
                  v.manifest, v.source_kind, v.provenance, v.scan_report,
                  v.scan_ruleset_version, v.quality_score, v.rubric_version,
                  v.created_at as version_created_at, v.created_by as version_created_by,
                  f.object_hash as manifest_object_hash
           from skill_bindings b
           join skills s on s.tenant_id = b.tenant_id and s.id = b.skill_id
           join skill_versions v on v.tenant_id = s.tenant_id and v.skill_id = s.id
                and v.id = coalesce(b.pinned_version_id, s.current_version_id)
           join skill_version_files f on f.tenant_id = v.tenant_id and f.version_id = v.id
                and f.path = 'SKILL.md'
           where b.tenant_id = $1 and b.scope_id = any($2) and b.enabled
           order by array_position($2::uuid[], b.scope_id), s.name, b.id"#,
        tenant.as_uuid(),
        &ids[..],
    )
    .fetch_all(conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(ResolvedBinding::try_from).collect()
}

/// Insert a typed Skill/apply projection.
pub async fn insert_change<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    proposal_id: ProposalId,
    command: &SkillCommand,
    payload_hash: &str,
) -> Result<StoredChange> {
    let payload = serde_json::to_value(command).map_err(|err| Error::Invalid {
        message: format!("encode Skill command: {err}"),
    })?;
    let row = sqlx::query_as!(
        ChangeRow,
        r#"insert into skill_changes
               (tenant_id, proposal_id, command_kind, payload, payload_hash)
           values ($1,$2,$3,$4,$5)
           returning proposal_id, payload, payload_hash, resulting_skill_id,
                     resulting_version_id, resulting_binding_id,
                     resulting_binding_revision, applied_at, created_at"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
        command.kind(),
        payload,
        payload_hash,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredChange::try_from(row)
}

/// Read a typed Skill/apply projection.
pub async fn change<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    proposal_id: ProposalId,
) -> Result<Option<StoredChange>> {
    let row = sqlx::query_as!(
        ChangeRow,
        r#"select proposal_id, payload, payload_hash, resulting_skill_id,
                  resulting_version_id, resulting_binding_id,
                  resulting_binding_revision, applied_at, created_at
           from skill_changes where tenant_id = $1 and proposal_id = $2"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredChange::try_from).transpose()
}

/// Record one applied result; false means it was already recorded.
pub async fn finish_change<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    proposal_id: ProposalId,
    result: &SkillMutationResult,
) -> Result<bool> {
    let done = sqlx::query!(
        r#"update skill_changes
           set resulting_skill_id = $3, resulting_version_id = $4,
               resulting_binding_id = $5, resulting_binding_revision = $6,
               applied_at = clock_timestamp()
           where tenant_id = $1 and proposal_id = $2 and applied_at is null"#,
        tenant.as_uuid(),
        proposal_id.as_uuid(),
        result.skill_id.map(|id| id.as_uuid()),
        result.version_id.map(|id| id.as_uuid()),
        result.binding_id.map(|id| id.as_uuid()),
        result
            .binding_revision
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(done.rows_affected() == 1)
}

/// Render a change's current result from proposal state supplied by caller.
#[must_use]
pub fn mutation_result(
    change: &StoredChange,
    outcome: SkillMutationOutcome,
) -> SkillMutationResult {
    SkillMutationResult {
        change_id: change.proposal_id,
        outcome,
        skill_id: change
            .resulting_skill_id
            .or_else(|| change.command.skill_id()),
        version_id: change
            .resulting_version_id
            .or_else(|| change.command.version_id()),
        binding_id: change
            .resulting_binding_id
            .or_else(|| change.command.binding_id()),
        binding_revision: change.resulting_binding_revision,
    }
}

/// Insert or replay one exact usage event.
#[allow(clippy::too_many_arguments)]
pub async fn record_usage(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: SkillUsageEventId,
    binding_id: SkillBindingId,
    version_id: SkillVersionId,
    session_id: Option<SessionId>,
    principal_id: IdentityId,
    client_event_id: &str,
    stage: SkillUsageStage,
    evidence: SkillUsageEvidence,
    resource_path: Option<&SkillFilePath>,
    metadata: &Value,
    occurred_at: DateTime<Utc>,
) -> Result<(StoredUsageEvent, bool)> {
    let inserted = sqlx::query_as!(
        UsageRow,
        r#"insert into skill_usage_events
               (id, tenant_id, binding_id, version_id, session_id, principal_id,
                client_event_id, stage, evidence, resource_path, metadata, occurred_at)
           values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           on conflict (tenant_id, binding_id, client_event_id) do nothing
           returning id, binding_id, version_id, session_id, principal_id, client_event_id,
                     stage, evidence, resource_path, metadata, occurred_at, received_at"#,
        id.as_uuid(),
        tenant.as_uuid(),
        binding_id.as_uuid(),
        version_id.as_uuid(),
        session_id.map(|value| value.as_uuid()),
        principal_id.as_uuid(),
        client_event_id,
        stage.as_str(),
        evidence.as_str(),
        resource_path.map(SkillFilePath::as_str),
        metadata,
        occurred_at,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    if let Some(row) = inserted {
        return Ok((StoredUsageEvent::try_from(row)?, true));
    }
    let row = sqlx::query_as!(
        UsageRow,
        r#"select id, binding_id, version_id, session_id, principal_id, client_event_id,
                  stage, evidence, resource_path, metadata, occurred_at, received_at
           from skill_usage_events
           where tenant_id = $1 and binding_id = $2 and client_event_id = $3"#,
        tenant.as_uuid(),
        binding_id.as_uuid(),
        client_event_id,
    )
    .fetch_one(conn)
    .await
    .map_err(storage_error)?;
    Ok((StoredUsageEvent::try_from(row)?, false))
}

/// Page usage for one version, newest first by UUIDv7 id.
pub async fn usage(
    conn: &mut PgConnection,
    tenant: TenantId,
    version_id: SkillVersionId,
    before: Option<SkillUsageEventId>,
    limit: i64,
) -> Result<Vec<StoredUsageEvent>> {
    let rows = sqlx::query_as!(
        UsageRow,
        r#"select id, binding_id, version_id, session_id, principal_id, client_event_id,
                  stage, evidence, resource_path, metadata, occurred_at, received_at
           from skill_usage_events
           where tenant_id = $1 and version_id = $2 and ($3::uuid is null or id < $3)
           order by id desc limit $4"#,
        tenant.as_uuid(),
        version_id.as_uuid(),
        before.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredUsageEvent::try_from).collect()
}

/// Record one immutable controlled test run.
#[allow(clippy::too_many_arguments)]
pub async fn record_test_run<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    id: SkillTestRunId,
    version_id: SkillVersionId,
    harness: SkillTestHarness,
    harness_version: &str,
    outcome: SkillTestOutcome,
    scan_ruleset_version: u32,
    rubric_version: u32,
    evidence: &Value,
    actor: IdentityId,
) -> Result<StoredTestRun> {
    let row = sqlx::query_as!(
        TestRunRow,
        r#"insert into skill_test_runs
               (id, tenant_id, version_id, harness, harness_version, outcome,
                scan_ruleset_version, rubric_version, evidence, created_by)
           values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
           returning id, version_id, harness, harness_version, outcome,
                     scan_ruleset_version, rubric_version, evidence, created_at, created_by"#,
        id.as_uuid(),
        tenant.as_uuid(),
        version_id.as_uuid(),
        harness.as_str(),
        harness_version,
        outcome.as_str(),
        i32::try_from(scan_ruleset_version).unwrap_or(i32::MAX),
        i32::try_from(rubric_version).unwrap_or(i32::MAX),
        evidence,
        actor.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    StoredTestRun::try_from(row)
}

/// Read one controlled-harness run by stable id.
pub async fn test_run<'e, E: PgExecutor<'e>>(
    executor: E,
    tenant: TenantId,
    id: SkillTestRunId,
) -> Result<Option<StoredTestRun>> {
    let row = sqlx::query_as!(
        TestRunRow,
        r#"select id, version_id, harness, harness_version, outcome,
                  scan_ruleset_version, rubric_version, evidence, created_at, created_by
           from skill_test_runs where tenant_id = $1 and id = $2"#,
        tenant.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(StoredTestRun::try_from).transpose()
}

/// Page test runs for one version, newest first by UUIDv7 id.
pub async fn test_runs(
    conn: &mut PgConnection,
    tenant: TenantId,
    version_id: SkillVersionId,
    before: Option<SkillTestRunId>,
    limit: i64,
) -> Result<Vec<StoredTestRun>> {
    let rows = sqlx::query_as!(
        TestRunRow,
        r#"select id, version_id, harness, harness_version, outcome,
                  scan_ruleset_version, rubric_version, evidence, created_at, created_by
           from skill_test_runs
           where tenant_id = $1 and version_id = $2 and ($3::uuid is null or id < $3)
           order by id desc limit $4"#,
        tenant.as_uuid(),
        version_id.as_uuid(),
        before.map(|id| id.as_uuid()),
        limit,
    )
    .fetch_all(conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(StoredTestRun::try_from).collect()
}

/// Decode a 32-byte hexadecimal address.
pub fn decode_hex_32(value: &str, what: &str) -> Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(Error::Invalid {
            message: format!("{what} must be 64 hexadecimal characters"),
        });
    }
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| {
            Error::Invalid {
                message: format!("{what} must be hexadecimal"),
            }
        })?;
    }
    Ok(out)
}

/// Lowercase hexadecimal address.
#[must_use]
pub fn hex_32(value: &[u8; 32]) -> String {
    value
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_addresses_round_trip() {
        let bytes = [0xab; 32];
        assert_eq!(decode_hex_32(&hex_32(&bytes), "address").unwrap(), bytes);
        assert!(decode_hex_32("no", "address").is_err());
    }
}
