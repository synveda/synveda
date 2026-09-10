//! Governed, versioned policy-relaxation API (CPR-31, ADR-0090).
//!
//! Every mutation is a typed `Policy/apply` VedaFlow change. The current
//! immutable version is consulted by the Cedar PDP; database time, rather
//! than the expiry sweep, ends authority at the hard boundary.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Acquire, PgConnection};
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::anchors::AnchorSelection;
use synveda_store::{configuration, identities, relaxations as store, rls, scopes};
use synveda_types::json::canonicalise;
use synveda_types::relaxation::{
    CurrentRelaxation, RelaxationAction, RelaxationCommand, RelaxationMutationOutcome,
    RelaxationMutationResult, RelaxationStatus, RelaxationTerms, RelaxationVersion,
};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    ArtifactFamily, ArtifactReference, AssetKind, Error, IdentityId, IdentityStatus,
    ProposalEffect, ProposalId, ProposalState, RelaxationId, RelaxationVersionId, Result, ScopeId,
    Sensitivity, TenantId,
};
use synveda_vedaflow::{self as vedaflow, PolicySnapshot, Signer};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::approvals::{self, Requested};
use crate::audit;
use crate::authz::{self, Authorized, DecisionInput};
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, found, tenant_id};
use crate::workspaces::{ApiErrorBody, subject};

/// Relaxation API operations by operation and `ok|rejected|error`.
pub const RELAXATION_OPERATIONS_TOTAL: &str = "synveda_relaxation_operations_total";
/// Expiry bookkeeping events chained by the system sweep.
pub const RELAXATION_EXPIRIES_TOTAL: &str = "synveda_relaxation_expiries_total";

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const EXPIRY_BATCH: i64 = 256;
const EXPIRY_COMPONENT: &str = "relaxation-expiry";

fn action_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(RelaxationAction::ALL.iter().map(|value| value.as_str()))
}

fn status_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(RelaxationStatus::ALL.iter().map(|value| value.as_str()))
}

fn outcome_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(["applied", "pending_review", "rejected"].into_iter())
}

/// Complete reviewed terms for create and revision requests.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelaxationTermsBody {
    #[schema(value_type = String, format = "uuid")]
    pub subject_identity_id: IdentityId,
    #[schema(value_type = String, schema_with = action_schema)]
    pub action: String,
    #[schema(value_type = String)]
    pub max_sensitivity: Sensitivity,
    pub requested_start_at: DateTime<Utc>,
    pub requested_end_at: DateTime<Utc>,
    pub reason: String,
}

impl RelaxationTermsBody {
    fn into_terms(self, target_scope_id: ScopeId) -> Result<RelaxationTerms> {
        let terms = RelaxationTerms {
            subject_identity_id: self.subject_identity_id,
            target_scope_id,
            action: self.action.parse()?,
            max_sensitivity: self.max_sensitivity,
            requested_start_at: self.requested_start_at,
            requested_end_at: self.requested_end_at,
            reason: self.reason,
        };
        terms.validate()?;
        Ok(terms)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateRelaxationBody {
    #[schema(value_type = String, format = "uuid")]
    pub target_scope_id: ScopeId,
    #[serde(flatten)]
    #[schema(inline)]
    pub terms: RelaxationTermsBody,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviseRelaxationBody {
    #[schema(value_type = String, format = "uuid")]
    pub expected_current_version_id: RelaxationVersionId,
    #[serde(flatten)]
    #[schema(inline)]
    pub terms: RelaxationTermsBody,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RevokeRelaxationBody {
    #[schema(value_type = String, format = "uuid")]
    pub expected_current_version_id: RelaxationVersionId,
    pub reason: String,
}

/// Result shared by create, revise and revoke.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RelaxationMutationView {
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    #[schema(value_type = String, schema_with = outcome_schema)]
    pub outcome: String,
    #[schema(value_type = String, format = "uuid")]
    pub relaxation_id: RelaxationId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub version_id: Option<RelaxationVersionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

impl From<RelaxationMutationResult> for RelaxationMutationView {
    fn from(value: RelaxationMutationResult) -> Self {
        let outcome = match value.outcome {
            RelaxationMutationOutcome::Applied => "applied",
            RelaxationMutationOutcome::PendingReview => "pending_review",
            RelaxationMutationOutcome::Rejected => "rejected",
        };
        Self {
            change_id: value.change_id,
            outcome: outcome.to_owned(),
            relaxation_id: value.relaxation_id,
            version_id: value.version_id,
            revision: value.revision,
        }
    }
}

/// One immutable reviewed version.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RelaxationVersionView {
    #[schema(value_type = String, format = "uuid")]
    pub id: RelaxationVersionId,
    #[schema(value_type = String, format = "uuid")]
    pub relaxation_id: RelaxationId,
    pub ordinal: i64,
    #[schema(value_type = String, format = "uuid")]
    pub change_id: ProposalId,
    #[schema(value_type = String, format = "uuid")]
    pub subject_identity_id: IdentityId,
    pub subject: String,
    #[schema(value_type = String, format = "uuid")]
    pub target_scope_id: ScopeId,
    #[schema(value_type = String, schema_with = action_schema)]
    pub action: String,
    #[schema(value_type = String)]
    pub max_sensitivity: Sensitivity,
    pub requested_start_at: DateTime<Utc>,
    pub requested_end_at: DateTime<Utc>,
    pub effective_start_at: DateTime<Utc>,
    pub hard_expires_at: DateTime<Utc>,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub configuration_version_id: Option<synveda_types::ConfigurationVersionId>,
    pub configuration_hash: String,
    pub content_hash: String,
    #[schema(value_type = String, format = "uuid")]
    pub creator_id: IdentityId,
    #[schema(value_type = Vec<String>, format = "uuid")]
    pub approver_ids: Vec<IdentityId>,
    pub auto_applied: bool,
    pub created_at: DateTime<Utc>,
}

impl From<RelaxationVersion> for RelaxationVersionView {
    fn from(value: RelaxationVersion) -> Self {
        Self {
            id: value.id,
            relaxation_id: value.relaxation_id,
            ordinal: value.ordinal,
            change_id: value.proposal_id,
            subject_identity_id: value.terms.subject_identity_id,
            subject: value.subject_principal_id,
            target_scope_id: value.terms.target_scope_id,
            action: value.terms.action.as_str().to_owned(),
            max_sensitivity: value.terms.max_sensitivity,
            requested_start_at: value.terms.requested_start_at,
            requested_end_at: value.terms.requested_end_at,
            effective_start_at: value.effective_start_at,
            hard_expires_at: value.hard_expires_at,
            reason: value.terms.reason,
            configuration_version_id: value.configuration_version_id,
            configuration_hash: value.configuration_hash,
            content_hash: value.content_hash,
            creator_id: value.creator_id,
            approver_ids: value.approver_ids,
            auto_applied: value.auto_applied,
            created_at: value.created_at,
        }
    }
}

/// Current stable aggregate projection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RelaxationView {
    #[schema(value_type = String, format = "uuid")]
    pub id: RelaxationId,
    #[schema(value_type = String, format = "uuid")]
    pub governing_scope_id: ScopeId,
    #[schema(value_type = String, format = "uuid")]
    pub current_version_id: RelaxationVersionId,
    pub revision: u64,
    #[schema(value_type = String, schema_with = status_schema)]
    pub status: String,
    pub current: RelaxationVersionView,
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    pub created_by: IdentityId,
    pub updated_at: DateTime<Utc>,
    #[schema(value_type = String, format = "uuid")]
    pub updated_by: IdentityId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub revoked_by: Option<IdentityId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub revocation_change_id: Option<ProposalId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_reason: Option<String>,
}

impl RelaxationView {
    fn at(value: CurrentRelaxation, now: DateTime<Utc>) -> Self {
        let status = value
            .relaxation
            .status_at(&value.version, now)
            .as_str()
            .to_owned();
        Self {
            id: value.relaxation.id,
            governing_scope_id: value.relaxation.governing_scope_id,
            current_version_id: value.relaxation.current_version_id,
            revision: value.relaxation.revision,
            status,
            current: value.version.into(),
            created_at: value.relaxation.created_at,
            created_by: value.relaxation.created_by,
            updated_at: value.relaxation.updated_at,
            updated_by: value.relaxation.updated_by,
            revoked_at: value.relaxation.revoked_at,
            revoked_by: value.relaxation.revoked_by,
            revocation_change_id: value.relaxation.revocation_proposal_id,
            revocation_reason: value.relaxation.revocation_reason,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RelaxationListView {
    pub relaxations: Vec<RelaxationView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RelaxationVersionListView {
    pub versions: Vec<RelaxationVersionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListQuery {
    #[serde(default)]
    pub scope_id: Option<ScopeId>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VersionListQuery {
    #[serde(default)]
    pub cursor: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    DEFAULT_LIMIT
}

fn checked_limit(limit: i64) -> Result<i64> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Error::Invalid {
            message: format!("limit must be in 1..={MAX_LIMIT}"),
        });
    }
    Ok(limit)
}

fn parse_status(raw: Option<&str>) -> Result<Option<RelaxationStatus>> {
    raw.map(|raw| {
        RelaxationStatus::ALL
            .into_iter()
            .find(|status| status.as_str() == raw)
            .ok_or_else(|| Error::Invalid {
                message: format!(
                    "unknown relaxation status {raw:?}; supported: scheduled, active, expired, revoked"
                ),
            })
    })
    .transpose()
}

fn encode_cursor(cursor: store::RelaxationCursor) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "relax1|{}|{}",
        cursor.updated_at.to_rfc3339(),
        cursor.id
    ))
}

fn decode_cursor(raw: &str) -> Result<store::RelaxationCursor> {
    let invalid = || Error::Invalid {
        message: "invalid relaxation cursor".to_owned(),
    };
    let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = std::str::from_utf8(&decoded).map_err(|_| invalid())?;
    let mut parts = decoded.split('|');
    if parts.next() != Some("relax1") {
        return Err(invalid());
    }
    let updated_at = parts
        .next()
        .ok_or_else(invalid)?
        .parse::<DateTime<Utc>>()
        .map_err(|_| invalid())?;
    let id = parts
        .next()
        .ok_or_else(invalid)?
        .parse::<RelaxationId>()
        .map_err(|_| invalid())?;
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok(store::RelaxationCursor { updated_at, id })
}

async fn respond<T: IntoResponse>(
    state: &AppState,
    operation: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(
        RELAXATION_OPERATIONS_TOTAL,
        "operation" => operation,
        "outcome" => outcome
    )
    .increment(1);
    crate::response::finish(state, operation, result).await
}

fn identity_of(input: &DecisionInput) -> Result<IdentityId> {
    input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "changing a policy relaxation requires a provisioned identity".to_owned(),
        })
}

async fn scope_for(tx: &mut PgConnection, tenant: TenantId, id: ScopeId) -> Result<Scope> {
    found(scopes::get(&mut *tx, tenant, id).await?, tenant, id)
}

async fn read_authorized(
    state: &AppState,
    tx: &mut PgConnection,
    scope: &Scope,
) -> Result<Authorized> {
    let input = authz::gather(state, tx, Some(scope), AnchorSelection::none(), Vec::new()).await?;
    authz::decide(state, &input, Action::PolicyRead, Resource::Scope(scope.id))
}

async fn audit_read(
    tx: &mut PgConnection,
    tenant: TenantId,
    operation: &'static str,
    resource: Resource,
    allowed: &Authorized,
    details: Value,
) -> Result<()> {
    audit::record(
        tx,
        tenant,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({
            "operation": operation,
            "authz": audit::decision_context(Action::PolicyRead, allowed),
            "details": details,
        }),
    )
    .await
    .map(|_| ())
}

struct CommandAuthorization {
    target: Scope,
    input: DecisionInput,
    write_allowed: Authorized,
    proposal_allowed: Authorized,
}

async fn authorize_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &RelaxationCommand,
) -> Result<CommandAuthorization> {
    command.validate()?;
    let target = scope_for(tx, tenant, command.governing_scope_id()).await?;
    if target.kind == ScopeKind::Principal {
        return Err(Error::Invalid {
            message: "personal principal scopes cannot be relaxed".to_owned(),
        });
    }
    let input = authz::gather(
        state,
        tx,
        Some(&target),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let write_allowed = authz::decide(
        state,
        &input,
        Action::RelaxationWrite,
        Resource::Scope(target.id),
    )?;
    let proposal_allowed = authz::decide(
        state,
        &input,
        Action::ProposalOpen,
        Resource::Scope(target.id),
    )?;

    match command {
        RelaxationCommand::Create { terms, .. } | RelaxationCommand::Revise { terms, .. } => {
            identities::by_id(&mut *tx, tenant, terms.subject_identity_id)
                .await?
                .filter(|identity| {
                    identity.status == IdentityStatus::Active && identity.subject.is_some()
                })
                .ok_or_else(|| Error::NotFound {
                    entity: format!("active identity {}", terms.subject_identity_id),
                })?;
            let effective = configuration::effective_at_scope(&mut *tx, tenant, target.id).await?;
            if !effective.document.relaxations.permits(terms.action) {
                return Err(Error::PolicyDenied {
                    action: Action::RelaxationWrite.as_str().to_owned(),
                    resource: Resource::Scope(target.id).to_string(),
                    reason: "the effective governed Configuration does not permit this relaxation action"
                        .to_owned(),
                });
            }
            if let RelaxationCommand::Revise {
                relaxation_id,
                expected_current_version_id,
                ..
            } = command
            {
                store::current(&mut *tx, tenant, *relaxation_id)
                    .await?
                    .filter(|current| {
                        current.relaxation.governing_scope_id == target.id
                            && current.relaxation.current_version_id
                                == *expected_current_version_id
                            && current.relaxation.revoked_at.is_none()
                    })
                    .ok_or_else(|| Error::Conflict {
                        message: format!(
                            "relaxation {relaxation_id} is absent, revoked, or no longer has expected version {expected_current_version_id}"
                        ),
                    })?;
            }
        }
        RelaxationCommand::Revoke {
            relaxation_id,
            expected_current_version_id,
            ..
        } => {
            store::current(&mut *tx, tenant, *relaxation_id)
                .await?
                .filter(|current| {
                    current.relaxation.governing_scope_id == target.id
                        && current.relaxation.current_version_id == *expected_current_version_id
                        && current.relaxation.revoked_at.is_none()
                })
                .ok_or_else(|| Error::Conflict {
                    message: format!(
                        "relaxation {relaxation_id} is absent, revoked, or no longer has expected version {expected_current_version_id}"
                    ),
                })?;
        }
    }
    Ok(CommandAuthorization {
        target,
        input,
        write_allowed,
        proposal_allowed,
    })
}

fn command_payload_hash(command: &RelaxationCommand) -> Result<String> {
    let value = canonicalise(
        &serde_json::to_value(command).map_err(|error| Error::Invalid {
            message: format!("encode relaxation command: {error}"),
        })?,
    );
    Ok(blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string())
}

fn relaxation_artifact_reference(
    command: &RelaxationCommand,
    payload_hash: &str,
) -> Result<ArtifactReference> {
    match command {
        RelaxationCommand::Create {
            relaxation_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::PolicyRelaxation,
            relaxation_id.to_string(),
            command.kind(),
            version_id.to_string(),
            None,
        ),
        RelaxationCommand::Revise {
            relaxation_id,
            expected_current_version_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::PolicyRelaxation,
            relaxation_id.to_string(),
            command.kind(),
            version_id.to_string(),
            Some(expected_current_version_id.to_string()),
        ),
        RelaxationCommand::Revoke {
            relaxation_id,
            expected_current_version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::PolicyRelaxation,
            relaxation_id.to_string(),
            command.kind(),
            payload_hash.to_owned(),
            Some(expected_current_version_id.to_string()),
        ),
    }
}

async fn open_command(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    command: &RelaxationCommand,
    authorization: &CommandAuthorization,
    claim: &Claim,
) -> Result<RelaxationMutationResult> {
    let actor = identity_of(&authorization.input)?;
    let payload_hash = command_payload_hash(command)?;
    let artifact_reference = relaxation_artifact_reference(command, &payload_hash)?;
    let manifest = canonicalise(&json!({
        "command": command.kind(),
        "payload_hash": payload_hash,
        "relaxation_id": command.relaxation_id(),
        "version_id": command.version_id(),
    }));
    let bytes = serde_json::to_vec(&manifest).map_err(|error| Error::Internal {
        message: format!("encode relaxation change manifest: {error}"),
    })?;
    let object = vedaflow::put_object(tx, tenant, AssetKind::Policy, &bytes).await?;
    let snapshot = PolicySnapshot::new(
        authorization.proposal_allowed.decision.pack_name.clone(),
        authorization.proposal_allowed.decision.pack_version,
    );
    let proposal = vedaflow::proposals::open(
        tx,
        tenant,
        &vedaflow::NewProposal {
            target_scope: authorization.target.id,
            source_scope: authorization.target.id,
            asset: AssetKind::Policy,
            effect: ProposalEffect::Apply,
            members: &[("command".to_owned(), object.hash)],
            artifact_references: &[artifact_reference],
            sensitivity: command_sensitivity(command),
            title: &format!("{} policy relaxation", command.kind()),
            proposer: actor,
            proposer_subject: &authorization.input.principal.subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;
    store::insert_change(tx, tenant, proposal.id, command, &payload_hash).await?;
    let entries = ["command".to_owned()];
    let requirement = approvals::resolve(
        state,
        tx,
        tenant,
        &authorization.input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Policy,
            sensitivity: command_sensitivity(command),
            entries: &entries,
        },
    )
    .await?;
    let outstanding = requirement.outstanding(&[]);
    audit::record(
        tx,
        tenant,
        AuditAction::RelaxationChangeOpened,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": proposal.id,
            "command": command.kind(),
            "payload_hash": payload_hash,
            "manifest_hash": object.hash.to_hex(),
            "artifact_references": &proposal.artifact_references,
            "relaxation_id": command.relaxation_id(),
            "version_id": command.version_id(),
            "authz": audit::decision_context(Action::ProposalOpen, &authorization.proposal_allowed),
            "approvals": approvals::audit_context(&requirement, &outstanding),
        }),
    )
    .await?;
    let result = if outstanding.is_empty() {
        apply_loaded(
            state,
            tx,
            tenant,
            LoadedEffect {
                change_id: proposal.id,
                command,
                payload_hash: &payload_hash,
                actor,
                approver_ids: &[],
            },
        )
        .await?
    } else {
        RelaxationMutationResult {
            change_id: proposal.id,
            outcome: RelaxationMutationOutcome::PendingReview,
            relaxation_id: command.relaxation_id(),
            version_id: command.version_id(),
            revision: None,
        }
    };
    claim.remember(tx, tenant, proposal.id.as_uuid()).await?;
    Ok(result)
}

fn command_sensitivity(command: &RelaxationCommand) -> Sensitivity {
    match command {
        RelaxationCommand::Create { terms, .. } | RelaxationCommand::Revise { terms, .. } => {
            terms.max_sensitivity
        }
        RelaxationCommand::Revoke { .. } => Sensitivity::Internal,
    }
}

struct LoadedEffect<'a> {
    change_id: ProposalId,
    command: &'a RelaxationCommand,
    payload_hash: &'a str,
    actor: IdentityId,
    approver_ids: &'a [IdentityId],
}

async fn apply_loaded(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    effect: LoadedEffect<'_>,
) -> Result<RelaxationMutationResult> {
    let authorization = authorize_command(state, tx, tenant, effect.command).await?;
    let effective =
        configuration::effective_at_scope(&mut *tx, tenant, authorization.target.id).await?;
    let mut effect_tx = tx.begin().await.map_err(|error| Error::Storage {
        message: format!("begin relaxation effect savepoint: {error}"),
    })?;
    let applied = store::apply(
        &mut effect_tx,
        tenant,
        effect.change_id,
        effect.actor,
        effect.approver_ids,
        &effective,
        effect.command,
    )
    .await;
    let applied = match applied {
        Ok(value) => {
            effect_tx.commit().await.map_err(|error| Error::Storage {
                message: format!("commit relaxation effect savepoint: {error}"),
            })?;
            value
        }
        Err(error @ (Error::Conflict { .. } | Error::NotFound { .. } | Error::Invalid { .. })) => {
            effect_tx
                .rollback()
                .await
                .map_err(|rollback| Error::Storage {
                    message: format!("roll back rejected relaxation effect: {rollback}"),
                })?;
            let reason = match error {
                Error::Conflict { .. } => "stale_precondition",
                Error::NotFound { .. } => "target_not_found",
                Error::Invalid { .. } => "invalid_effect",
                _ => unreachable!(),
            };
            if !vedaflow::proposals::close(
                tx,
                tenant,
                effect.change_id,
                ProposalState::Rejected,
                effect.actor,
                Some(reason),
            )
            .await?
            {
                return Err(Error::Conflict {
                    message: format!(
                        "relaxation change {} closed before rejection was recorded",
                        effect.change_id
                    ),
                });
            }
            audit::record(
                tx,
                tenant,
                AuditAction::RelaxationChangeRejected,
                Resource::Scope(authorization.target.id).to_string(),
                Outcome::Deny,
                json!({
                    "change_id": effect.change_id,
                    "command": effect.command.kind(),
                    "payload_hash": effect.payload_hash,
                    "artifact_references": [relaxation_artifact_reference(effect.command, effect.payload_hash)?],
                    "relaxation_id": effect.command.relaxation_id(),
                    "reason_code": reason,
                }),
            )
            .await?;
            return Ok(RelaxationMutationResult {
                change_id: effect.change_id,
                outcome: RelaxationMutationOutcome::Rejected,
                relaxation_id: effect.command.relaxation_id(),
                version_id: effect.command.version_id(),
                revision: None,
            });
        }
        Err(error) => {
            effect_tx
                .rollback()
                .await
                .map_err(|rollback| Error::Storage {
                    message: format!("roll back failed relaxation effect: {rollback}"),
                })?;
            return Err(error);
        }
    };
    store::complete_change(tx, tenant, effect.change_id, applied).await?;
    if !vedaflow::proposals::close(
        tx,
        tenant,
        effect.change_id,
        ProposalState::Applied,
        effect.actor,
        None,
    )
    .await?
    {
        return Err(Error::Conflict {
            message: format!(
                "relaxation change {} closed before effect completion",
                effect.change_id
            ),
        });
    }
    audit::record(
        tx,
        tenant,
        AuditAction::RelaxationChangeApplied,
        Resource::Scope(authorization.target.id).to_string(),
        Outcome::Success,
        json!({
            "change_id": effect.change_id,
            "command": effect.command.kind(),
            "payload_hash": effect.payload_hash,
            "artifact_references": [relaxation_artifact_reference(effect.command, effect.payload_hash)?],
            "relaxation_id": applied.relaxation_id,
            "version_id": applied.version_id,
            "revision": applied.revision,
            "approver_ids": effect.approver_ids,
            "auto_applied": effect.approver_ids.is_empty(),
            "authz": audit::decision_context(Action::RelaxationWrite, &authorization.write_allowed),
        }),
    )
    .await?;
    Ok(RelaxationMutationResult {
        change_id: effect.change_id,
        outcome: RelaxationMutationOutcome::Applied,
        relaxation_id: applied.relaxation_id,
        version_id: applied.version_id,
        revision: Some(applied.revision),
    })
}

fn invalid_change(id: ProposalId) -> Error {
    Error::Internal {
        message: format!("relaxation change {id} failed its VedaFlow payload-integrity check"),
    }
}

async fn verify_change_binding(
    tx: &mut PgConnection,
    tenant: TenantId,
    proposal: &vedaflow::StoredProposal,
    change: &store::RelaxationChange,
) -> Result<()> {
    let members = vedaflow::proposals::members(tx, tenant, proposal.commit).await?;
    let [member] = members.as_slice() else {
        return Err(invalid_change(proposal.id));
    };
    let object = vedaflow::read_object(tx, tenant, member.object)
        .await?
        .ok_or_else(|| invalid_change(proposal.id))?;
    let manifest: Value =
        serde_json::from_slice(&object.content).map_err(|_| invalid_change(proposal.id))?;
    let valid = member.name == "command"
        && object.kind == AssetKind::Policy
        && command_payload_hash(&change.command)? == change.payload_hash
        && manifest.get("command").and_then(Value::as_str) == Some(change.command.kind())
        && manifest.get("payload_hash").and_then(Value::as_str)
            == Some(change.payload_hash.as_str());
    if !valid {
        return Err(invalid_change(proposal.id));
    }
    Ok(())
}

fn proposal_outcome(state: ProposalState) -> Result<RelaxationMutationOutcome> {
    match state {
        ProposalState::Open => Ok(RelaxationMutationOutcome::PendingReview),
        ProposalState::Applied => Ok(RelaxationMutationOutcome::Applied),
        ProposalState::Rejected | ProposalState::Withdrawn => {
            Ok(RelaxationMutationOutcome::Rejected)
        }
        ProposalState::Published => Err(Error::Internal {
            message: "a Policy/apply relaxation was published as a channel".to_owned(),
        }),
    }
}

fn change_result(
    proposal: &vedaflow::StoredProposal,
    change: &store::RelaxationChange,
) -> Result<RelaxationMutationResult> {
    Ok(RelaxationMutationResult {
        change_id: proposal.id,
        outcome: proposal_outcome(proposal.state)?,
        relaxation_id: change
            .resulting_relaxation_id
            .unwrap_or_else(|| change.command.relaxation_id()),
        version_id: change
            .resulting_version_id
            .or_else(|| change.command.version_id()),
        revision: change.resulting_revision,
    })
}

pub(crate) async fn result(state: &AppState, id: ProposalId) -> Result<RelaxationMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Policy && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("relaxation change {id}"),
        })?;
    let target = scope_for(&mut tx, tenant, proposal.target_scope_id).await?;
    let allowed = read_authorized(state, &mut tx, &target).await?;
    let change = store::change(&mut tx, tenant, id)
        .await?
        .ok_or_else(|| invalid_change(id))?;
    let rendered = change_result(&proposal, &change)?;
    audit_read(
        &mut tx,
        tenant,
        "relaxation.change.result",
        Resource::Scope(target.id),
        &allowed,
        json!({"change_id": id}),
    )
    .await?;
    commit(tx).await?;
    Ok(rendered)
}

/// Apply an approved Policy/apply relaxation from the generic review route.
pub(crate) async fn apply_reviewed(
    state: &AppState,
    id: ProposalId,
) -> Result<RelaxationMutationResult> {
    let tenant = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let proposal = vedaflow::proposals::read(&mut tx, tenant, id)
        .await?
        .filter(|proposal| {
            proposal.asset == AssetKind::Policy && proposal.effect == ProposalEffect::Apply
        })
        .ok_or_else(|| Error::NotFound {
            entity: format!("relaxation change {id}"),
        })?;
    let change = store::change(&mut tx, tenant, id)
        .await?
        .ok_or_else(|| invalid_change(id))?;
    if proposal.state != ProposalState::Open {
        return change_result(&proposal, &change);
    }
    verify_change_binding(&mut tx, tenant, &proposal, &change).await?;
    let authorization = authorize_command(state, &mut tx, tenant, &change.command).await?;
    let entries = ["command".to_owned()];
    let requirement = approvals::resolve(
        state,
        &mut tx,
        tenant,
        &authorization.input,
        &Requested {
            target: &authorization.target,
            asset: AssetKind::Policy,
            sensitivity: command_sensitivity(&change.command),
            entries: &entries,
        },
    )
    .await?;
    let recorded = vedaflow::proposals::approvals(&mut tx, tenant, id).await?;
    let cast = vedaflow::proposals::cast_for(&recorded, proposal.commit);
    let outstanding = requirement.outstanding(&cast);
    if !outstanding.is_empty() {
        return Err(Error::Conflict {
            message: format!(
                "relaxation change {id} still needs {}",
                outstanding.describe()
            ),
        });
    }
    let mut approver_ids: Vec<IdentityId> = cast.iter().map(|approval| approval.identity).collect();
    approver_ids.sort_unstable();
    approver_ids.dedup();
    let actor = identity_of(&authorization.input)?;
    approvals::require_effect_actor(&requirement, id, proposal.proposer_id, &cast, actor)?;
    let rendered = apply_loaded(
        state,
        &mut tx,
        tenant,
        LoadedEffect {
            change_id: id,
            command: &change.command,
            payload_hash: &change.payload_hash,
            actor,
            approver_ids: &approver_ids,
        },
    )
    .await?;
    commit(tx).await?;
    Ok(rendered)
}

async fn submit_command(
    state: &AppState,
    headers: &HeaderMap,
    operation: &'static str,
    canonical: Value,
    command: RelaxationCommand,
) -> Result<(StatusCode, Json<RelaxationMutationView>)> {
    let tenant = tenant_id()?;
    let claim = Claim::from_headers(headers, operation, &subject()?, &canonical)?;
    if let Dispatch::Replay(id) = crate::idempotency::dispatch(&state.pool, tenant, &claim).await? {
        return Ok((
            StatusCode::OK,
            Json(result(state, ProposalId::from_uuid(id)).await?.into()),
        ));
    }
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let authorization = authorize_command(state, &mut tx, tenant, &command).await?;
    let opened = open_command(state, &mut tx, tenant, &command, &authorization, &claim).await;
    match opened {
        Ok(value) => {
            commit(tx).await?;
            Ok((StatusCode::CREATED, Json(value.into())))
        }
        Err(conflict @ Error::Conflict { .. }) => {
            drop(tx);
            let id =
                crate::idempotency::resolve_conflict(&state.pool, tenant, &claim, conflict).await?;
            Ok((
                StatusCode::OK,
                Json(result(state, ProposalId::from_uuid(id)).await?.into()),
            ))
        }
        Err(error) => Err(error),
    }
}

/// Create a stable relaxation and its first immutable version through
/// VedaFlow.
#[utoipa::path(
    post,
    path = "/v1/relaxations",
    operation_id = "create_relaxation",
    tag = "policy-relaxations",
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    request_body = CreateRelaxationBody,
    responses(
        (status = 201, description = "Change opened", body = RelaxationMutationView),
        (status = 400, description = "Invalid terms", body = ApiErrorBody),
        (status = 403, description = "Denied", body = ApiErrorBody),
        (status = 409, description = "Conflict", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "relaxations.create", skip_all)]
pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateRelaxationBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = serde_json::to_value(&body).map_err(|error| Error::Invalid {
            message: format!("encode relaxation request: {error}"),
        })?;
        let command = RelaxationCommand::Create {
            relaxation_id: RelaxationId::new(),
            version_id: RelaxationVersionId::new(),
            terms: body.terms.into_terms(body.target_scope_id)?,
        };
        submit_command(&state, &headers, "relaxation.create", canonical, command).await
    }
    .await;
    respond(&state, "create", result).await
}

/// Publish a replacement immutable version with an exact head precondition.
#[utoipa::path(
    patch,
    path = "/v1/relaxations/{id}",
    operation_id = "revise_relaxation",
    tag = "policy-relaxations",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = ReviseRelaxationBody,
    responses(
        (status = 201, description = "Change opened", body = RelaxationMutationView),
        (status = 404, description = "Relaxation absent", body = ApiErrorBody),
        (status = 409, description = "Stale current-version precondition", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "relaxations.revise", skip_all)]
pub(crate) async fn revise(
    State(state): State<AppState>,
    Path(id): Path<RelaxationId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<ReviseRelaxationBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"relaxation_id": id, "body": &body});
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let current =
            store::current(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("relaxation {id}"),
                })?;
        drop(tx);
        let command = RelaxationCommand::Revise {
            relaxation_id: id,
            expected_current_version_id: body.expected_current_version_id,
            version_id: RelaxationVersionId::new(),
            governing_scope_id: current.relaxation.governing_scope_id,
            terms: body
                .terms
                .into_terms(current.relaxation.governing_scope_id)?,
        };
        submit_command(&state, &headers, "relaxation.revise", canonical, command).await
    }
    .await;
    respond(&state, "revise", result).await
}

/// End a current version early through a governed VedaFlow change.
#[utoipa::path(
    post,
    path = "/v1/relaxations/{id}/revoke",
    operation_id = "revoke_relaxation",
    tag = "policy-relaxations",
    params(
        ("id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    request_body = RevokeRelaxationBody,
    responses(
        (status = 201, description = "Revocation change opened", body = RelaxationMutationView),
        (status = 404, description = "Relaxation absent", body = ApiErrorBody),
        (status = 409, description = "Stale or inactive version", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "relaxations.revoke", skip_all)]
pub(crate) async fn revoke(
    State(state): State<AppState>,
    Path(id): Path<RelaxationId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<RevokeRelaxationBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"relaxation_id": id, "body": &body});
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let current =
            store::current(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("relaxation {id}"),
                })?;
        drop(tx);
        let command = RelaxationCommand::Revoke {
            relaxation_id: id,
            expected_current_version_id: body.expected_current_version_id,
            governing_scope_id: current.relaxation.governing_scope_id,
            reason: body.reason,
        };
        submit_command(&state, &headers, "relaxation.revoke", canonical, command).await
    }
    .await;
    respond(&state, "revoke", result).await
}

/// List policy-visible relaxations. The cursor follows the last candidate
/// considered, so a fully denied page can be empty and still advance.
#[utoipa::path(
    get,
    path = "/v1/relaxations",
    operation_id = "list_relaxations",
    tag = "policy-relaxations",
    params(
        ("scope_id" = Option<String>, Query, format = "uuid"),
        ("status" = Option<String>, Query),
        ("cursor" = Option<String>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, description = "Policy-visible relaxations", body = RelaxationListView),
        (status = 400, description = "Invalid cursor or filter", body = ApiErrorBody),
        (status = 403, description = "Denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "relaxations.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = checked_limit(query.limit)?;
        let wanted_status = parse_status(query.status.as_deref())?;
        let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        if let Some(scope_id) = query.scope_id {
            let scope = scope_for(&mut tx, tenant, scope_id).await?;
            read_authorized(&state, &mut tx, &scope).await?;
        }
        let candidates = store::list(&mut tx, tenant, query.scope_id, cursor, limit).await?;
        let full_page = candidates.len() as i64 == limit;
        let considered = candidates.last().map(|row| store::RelaxationCursor {
            updated_at: row.updated_at,
            id: row.id,
        });
        let now = Utc::now();
        let mut visible = Vec::new();
        let mut decisions = 0_usize;
        for aggregate in candidates {
            let target = match scopes::get(&mut *tx, tenant, aggregate.governing_scope_id).await? {
                Some(scope) => scope,
                None => continue,
            };
            decisions += 1;
            if read_authorized(&state, &mut tx, &target).await.is_err() {
                continue;
            }
            let current = store::current(&mut tx, tenant, aggregate.id)
                .await?
                .ok_or_else(|| Error::Storage {
                    message: format!("relaxation {} disappeared during listing", aggregate.id),
                })?;
            if wanted_status
                .is_some_and(|status| current.relaxation.status_at(&current.version, now) != status)
            {
                continue;
            }
            visible.push(RelaxationView::at(current, now));
        }
        audit::record(
            &mut tx,
            tenant,
            AuditAction::AuthzDecision,
            Resource::Tenant(tenant).to_string(),
            Outcome::Allow,
            json!({
                "operation": "relaxation.list",
                "action": Action::PolicyRead.as_str(),
                "scopes_decided": decisions,
                "visible": visible.len(),
                "scope_filter": query.scope_id,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(RelaxationListView {
            relaxations: visible,
            next_cursor: full_page
                .then(|| encode_cursor(considered.expect("full page is non-empty"))),
        }))
    }
    .await;
    respond(&state, "list", result).await
}

/// Read one current stable aggregate.
#[utoipa::path(
    get,
    path = "/v1/relaxations/{id}",
    operation_id = "get_relaxation",
    tag = "policy-relaxations",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Current relaxation", body = RelaxationView),
        (status = 403, description = "Denied", body = ApiErrorBody),
        (status = 404, description = "Relaxation absent", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "relaxations.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<RelaxationId>) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let current =
            store::current(&mut tx, tenant, id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("relaxation {id}"),
                })?;
        let target = scope_for(&mut tx, tenant, current.relaxation.governing_scope_id).await?;
        let allowed = read_authorized(&state, &mut tx, &target).await?;
        audit_read(
            &mut tx,
            tenant,
            "relaxation.get",
            Resource::Scope(target.id),
            &allowed,
            json!({"relaxation_id": id}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(RelaxationView::at(current, Utc::now())))
    }
    .await;
    respond(&state, "get", result).await
}

/// List immutable versions newest first.
#[utoipa::path(
    get,
    path = "/v1/relaxations/{id}/versions",
    operation_id = "list_relaxation_versions",
    tag = "policy-relaxations",
    params(
        ("id" = String, Path, format = "uuid"),
        ("cursor" = Option<i64>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, description = "Immutable relaxation versions", body = RelaxationVersionListView),
        (status = 403, description = "Denied", body = ApiErrorBody),
        (status = 404, description = "Relaxation absent", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "relaxations.versions", skip_all)]
pub(crate) async fn versions(
    State(state): State<AppState>,
    Path(id): Path<RelaxationId>,
    Query(query): Query<VersionListQuery>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = checked_limit(query.limit)?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let aggregate = store::relaxation(&mut tx, tenant, id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("relaxation {id}"),
            })?;
        let target = scope_for(&mut tx, tenant, aggregate.governing_scope_id).await?;
        let allowed = read_authorized(&state, &mut tx, &target).await?;
        let rows = store::versions(&mut tx, tenant, id, query.cursor, limit).await?;
        let next_cursor =
            (rows.len() as i64 == limit).then(|| rows.last().expect("non-empty full page").ordinal);
        let count = rows.len();
        audit_read(
            &mut tx,
            tenant,
            "relaxation.versions",
            Resource::Scope(target.id),
            &allowed,
            json!({"relaxation_id": id, "versions": count}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(RelaxationVersionListView {
            versions: rows.into_iter().map(Into::into).collect(),
            next_cursor,
        }))
    }
    .await;
    respond(&state, "versions", result).await
}

/// Chain due hard-expiry events once. Authority already ended in the
/// database-time predicate, so one tenant's failure cannot widen access.
#[tracing::instrument(name = "relaxations.expiry_sweep", skip_all, err(Display))]
pub async fn expire_once(pool: &sqlx::PgPool) -> Result<usize> {
    let mut chained = 0;
    for tenant in synveda_store::tenants::active(pool).await? {
        match expire_tenant(pool, tenant.id).await {
            Ok(count) => chained += count,
            Err(error) => tracing::error!(
                tenant.id = %tenant.id,
                error = %error,
                "relaxation expiry bookkeeping failed for one tenant"
            ),
        }
    }
    Ok(chained)
}

#[tracing::instrument(
    name = "relaxations.expiry_tenant",
    skip_all,
    fields(tenant.id = %tenant),
    err(Display)
)]
async fn expire_tenant(pool: &sqlx::PgPool, tenant: TenantId) -> Result<usize> {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await?;
    let due = store::due_for_expiry_event(&mut tx, tenant, EXPIRY_BATCH).await?;
    if due.is_empty() {
        return Ok(0);
    }
    let mut chained = 0;
    for current in &due {
        if !store::record_expiry(
            &mut tx,
            tenant,
            current.relaxation.id,
            current.relaxation.revision,
        )
        .await?
        {
            continue;
        }
        let artifact_reference = ArtifactReference::new(
            ArtifactFamily::PolicyRelaxation,
            current.relaxation.id.to_string(),
            "expired",
            current.version.id.to_string(),
            None,
        )?;
        audit::record_as(
            &mut tx,
            tenant,
            Actor::system(EXPIRY_COMPONENT),
            AuditAction::RelaxationExpired,
            Resource::Scope(current.relaxation.governing_scope_id).to_string(),
            Outcome::Success,
            json!({
                "artifact_references": [artifact_reference],
                "relaxation_id": current.relaxation.id,
                "version_id": current.version.id,
                "change_id": current.version.proposal_id,
                "subject_identity_id": current.version.terms.subject_identity_id,
                "target_scope_id": current.version.terms.target_scope_id,
                "action": current.version.terms.action.as_str(),
                "hard_expires_at": current.version.hard_expires_at,
                "content_hash": current.version.content_hash,
                "note": "database time ended authority at hard_expires_at; this event is bookkeeping",
            }),
        )
        .await?;
        chained += 1;
    }
    commit(tx).await?;
    metrics::counter!(RELAXATION_EXPIRIES_TOTAL).increment(chained as u64);
    Ok(chained)
}

/// Runs periodic expiry bookkeeping until the worker begins shutdown.
///
/// Shutdown is selected against each tenant transaction; cancellation rolls
/// an unfinished tenant back and prevents the next tenant from starting.
pub(crate) async fn run_expiry_sweep(
    pool: sqlx::PgPool,
    interval: std::time::Duration,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {}
        }
        if *shutdown.borrow() {
            return;
        }
        match expire_until_shutdown(&pool, &mut shutdown).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => tracing::warn!(error = %error, "relaxation expiry sweep failed"),
        }
    }
}

async fn expire_until_shutdown(
    pool: &sqlx::PgPool,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool> {
    let active = tokio::select! {
        biased;
        () = crate::shutdown::requested(shutdown) => return Ok(false),
        result = synveda_store::tenants::active(pool) => result?,
    };
    for tenant in active {
        if *shutdown.borrow() {
            return Ok(false);
        }
        let result = tokio::select! {
            biased;
            () = crate::shutdown::requested(shutdown) => return Ok(false),
            result = expire_tenant(pool, tenant.id) => result,
        };
        match result {
            Ok(_) => {}
            Err(error) => tracing::error!(
                tenant.id = %tenant.id,
                error = %error,
                "relaxation expiry bookkeeping failed for one tenant"
            ),
        }
    }
    Ok(true)
}
