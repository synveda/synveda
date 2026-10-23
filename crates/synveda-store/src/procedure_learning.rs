//! Exact private custody. Authorization and publication remain above store.
use serde_json::Value;
use sqlx::PgConnection;
use synveda_types::{
    CaptureCandidateId, Error, IdentityId, Result, SessionEventId, SessionId, TenantId,
};

/// Content-free reserved outcome custody, independently reauthorized at disclosure.
pub struct OutcomeHeader {
    /// Exact event.
    pub event_id: uuid::Uuid,
    /// Source Session.
    pub session_id: uuid::Uuid,
    /// Exact Procedure aggregate.
    pub procedure_item_id: uuid::Uuid,
    /// Exact involved revision.
    pub procedure_revision_id: uuid::Uuid,
    /// Exact content digest.
    pub procedure_hash: String,
    /// Canonical report request digest.
    pub request_hash: String,
    /// Registered reporter identity.
    pub reporter_id: uuid::Uuid,
    /// Server-derived provenance class.
    pub evidence_class: String,
    /// Closed reported status.
    pub outcome: String,
    /// Reported environment digest.
    pub environment_digest: String,
    /// Exact involved Skill versions; retained without executable content.
    pub skill_versions: Vec<uuid::Uuid>,
    /// Exact optional delivered run, retained without its content.
    pub context_run_id: Option<uuid::Uuid>,
    /// Exact optional delivered block digest.
    pub context_block_hash: Option<String>,
    /// Server observation clock.
    pub received_at: chrono::DateTime<chrono::Utc>,
}
/// Exact immutable improvement identity.
pub struct ImprovementHeader {
    /// Exact reserved event.
    pub event_id: uuid::Uuid,
    /// Source Session.
    pub session_id: uuid::Uuid,
    /// Outcome/target/patch identity.
    pub identity_hash: String,
    /// Canonical request digest.
    pub request_hash: String,
    /// Stable target aggregate.
    pub procedure_item_id: uuid::Uuid,
    /// Exact baseline revision.
    pub procedure_revision_id: uuid::Uuid,
    /// Exact baseline digest.
    pub procedure_hash: String,
    /// Exact frozen proposal digest.
    pub content_hash: String,
}
/// Exact candidate binding to a server improvement request.
pub struct CandidateHeader {
    /// Exact candidate.
    pub candidate_id: uuid::Uuid,
    /// Immutable source request.
    pub request_event_id: uuid::Uuid,
    /// Frozen candidate digest.
    pub content_hash: String,
    /// Stable target aggregate.
    pub procedure_item_id: uuid::Uuid,
    /// Expected baseline head.
    pub procedure_revision_id: uuid::Uuid,
    /// Exact baseline content.
    pub procedure_hash: String,
    /// Source Session.
    pub session_id: uuid::Uuid,
}
/// Frozen controlled suite, retained without held-out answers.
pub struct EvaluationAttempt {
    /// Exact attempt.
    pub id: uuid::Uuid,
    /// Exact candidate.
    pub candidate_id: uuid::Uuid,
    /// Frozen candidate hash.
    pub candidate_hash: String,
    /// Freeze request hash.
    pub request_hash: String,
    /// Governed suite label.
    pub suite_id: String,
    /// Independent registered evaluator.
    pub evaluator_id: uuid::Uuid,
    /// Exact governing configuration.
    pub configuration_hash: String,
    /// Exact fixed suite specification.
    pub specification_hash: String,
    /// Case identities/digests; no grading answers.
    pub specification: Value,
}
/// Exact independently reported qualification receipt.
pub struct EvaluationResult {
    /// Frozen attempt.
    pub attempt_id: uuid::Uuid,
    /// Actual authenticated evaluator.
    pub reporter_id: uuid::Uuid,
    /// Exact result request hash.
    pub request_hash: String,
    /// Server-computed fixed-case gate.
    pub qualified: bool,
    /// Bounded status/digest observations.
    pub results: Value,
}
/// One exact historical source address, never permission to disclose it.
pub struct HistoricalReference {
    /// Stable source aggregate.
    pub knowledge_item_id: uuid::Uuid,
    /// Exact source revision.
    pub revision_id: uuid::Uuid,
    /// Exact source content hash.
    pub content_hash: String,
}

/// Exact outcome header, including after authorized erasure of plaintext.
pub async fn outcome(
    tx: &mut PgConnection,
    t: TenantId,
    id: SessionEventId,
) -> Result<Option<OutcomeHeader>> {
    sqlx::query_as!(OutcomeHeader,"select event_id,session_id,procedure_item_id,procedure_revision_id,procedure_hash,request_hash,reporter_id,evidence_class,outcome,environment_digest,skill_versions,context_run_id,context_block_hash,received_at from procedure_outcome_headers where tenant_id=$1 and event_id=$2",t.as_uuid(),id.as_uuid()).fetch_optional(tx).await.map_err(storage)
}
/// Content-free exact outcome retry address, independent of event retention.
pub async fn outcome_by_client(
    tx: &mut PgConnection,
    t: TenantId,
    session: SessionId,
    client: &str,
) -> Result<Option<OutcomeHeader>> {
    sqlx::query_as!(OutcomeHeader,"select h.event_id,h.session_id,h.procedure_item_id,h.procedure_revision_id,h.procedure_hash,h.request_hash,h.reporter_id,h.evidence_class,h.outcome,h.environment_digest,h.skill_versions,h.context_run_id,h.context_block_hash,h.received_at from procedure_outcome_headers h join source_event_addresses a on a.tenant_id=h.tenant_id and a.event_id=h.event_id where h.tenant_id=$1 and h.session_id=$2 and a.client_event_id_digest=encode(sha256(convert_to($3,'UTF8')),'hex')",t.as_uuid(),session.as_uuid(),client).fetch_optional(tx).await.map_err(storage)
}
/// Find one exact improvement identity before accepting a retry.
pub async fn improvement_by_identity(
    tx: &mut PgConnection,
    t: TenantId,
    identity: &str,
) -> Result<Option<ImprovementHeader>> {
    sqlx::query_as!(ImprovementHeader,"select event_id,session_id,identity_hash,request_hash,procedure_item_id,procedure_revision_id,procedure_hash,content_hash from procedure_improvement_headers where tenant_id=$1 and identity_hash=$2",t.as_uuid(),identity).fetch_optional(tx).await.map_err(storage)
}
/// Exact event binding, bounded independently from its payload.
pub async fn improvement(
    tx: &mut PgConnection,
    t: TenantId,
    id: SessionEventId,
) -> Result<Option<ImprovementHeader>> {
    sqlx::query_as!(ImprovementHeader,"select event_id,session_id,identity_hash,request_hash,procedure_item_id,procedure_revision_id,procedure_hash,content_hash from procedure_improvement_headers where tenant_id=$1 and event_id=$2",t.as_uuid(),id.as_uuid()).fetch_optional(tx).await.map_err(storage)
}
/// Serialize materialization of the same request across competing Capture batches.
pub async fn lock_improvement(
    tx: &mut PgConnection,
    t: TenantId,
    id: SessionEventId,
) -> Result<()> {
    let locked = sqlx::query_scalar!(
        r#"select synveda_lock_procedure_request($1,$2) as "locked!""#,
        t.as_uuid(),
        id.as_uuid()
    )
    .fetch_one(tx)
    .await
    .map_err(storage)?;
    if !locked {
        return Err(unavailable());
    }
    Ok(())
}
/// One existing candidate for the exact server request.
pub async fn candidate_for_request(
    tx: &mut PgConnection,
    t: TenantId,
    id: SessionEventId,
) -> Result<Option<CaptureCandidateId>> {
    sqlx::query_scalar!("select candidate_id from procedure_candidate_headers where tenant_id=$1 and request_event_id=$2",t.as_uuid(),id.as_uuid()).fetch_optional(tx).await.map(|id|id.map(CaptureCandidateId::from_uuid)).map_err(storage)
}
/// Private candidate proof; source authorization is mandatory above this seam.
pub async fn candidate(
    tx: &mut PgConnection,
    t: TenantId,
    id: CaptureCandidateId,
) -> Result<Option<CandidateHeader>> {
    sqlx::query_as!(CandidateHeader,r#"select c.candidate_id,c.request_event_id,c.content_hash,r.procedure_item_id,r.procedure_revision_id,r.procedure_hash,r.session_id
       from procedure_candidate_headers c join procedure_improvement_headers r on r.tenant_id=c.tenant_id and r.event_id=c.request_event_id
       where c.tenant_id=$1 and c.candidate_id=$2"#,t.as_uuid(),id.as_uuid()).fetch_optional(tx).await.map_err(storage)
}
/// Bound flattened historical proof; no recursive disclosure or unbounded walk.
pub async fn historical(
    tx: &mut PgConnection,
    t: TenantId,
    kind: &str,
    id: uuid::Uuid,
) -> Result<Vec<HistoricalReference>> {
    let rows=sqlx::query_as!(HistoricalReference,"select knowledge_item_id,revision_id,content_hash from historical_knowledge_dependencies where tenant_id=$1 and dependent_kind=$2 and dependent_id=$3 order by revision_id limit 65",t.as_uuid(),kind,id).fetch_all(tx).await.map_err(storage)?;
    if rows.len() > 64 {
        return Err(Error::Invalid {
            message: "Procedure historical evidence exceeds its supported bound".to_owned(),
        });
    }
    Ok(rows)
}
/// Bounded retained Session authority, without requiring retained plaintext.
pub struct SessionAuthority {
    /// Actual source Session.
    pub session_id: uuid::Uuid,
    /// Actual governing scope.
    pub scope_id: uuid::Uuid,
    /// Actual project anchor.
    pub project_id: Option<uuid::Uuid>,
}
/// Distinct source authorities for a flattened derivative.
pub async fn session_authorities(
    tx: &mut PgConnection,
    t: TenantId,
    kind: &str,
    id: uuid::Uuid,
) -> Result<Vec<SessionAuthority>> {
    let rows=sqlx::query_as!(SessionAuthority,"select distinct a.session_id,a.scope_id,a.project_id from source_dependencies dep join source_event_addresses a on a.tenant_id=dep.tenant_id and a.event_id=dep.source_event_id where dep.tenant_id=$1 and dep.dependent_kind=$2 and dep.dependent_id=$3 order by a.session_id limit 4097",t.as_uuid(),kind,id).fetch_all(tx).await.map_err(storage)?;
    if rows.len() > 4096 {
        return Err(unavailable());
    }
    Ok(rows)
}
/// Distinct imported source addresses for independent current project decisions.
pub async fn artifact_authorities(
    tx: &mut PgConnection,
    t: TenantId,
    kind: &str,
    id: uuid::Uuid,
) -> Result<Vec<crate::artifact_control::ArtifactAddress>> {
    let rows=sqlx::query_as!(crate::artifact_control::ArtifactAddress,"select distinct on (dep.source_key_digest) a.artifact_id,a.job_id,a.project_id,a.scope_id,a.workspace_id,a.content_hash,a.source_key_digest from artifact_dependencies dep join source_artifact_addresses a on a.tenant_id=dep.tenant_id and a.source_key_digest=dep.source_key_digest where dep.tenant_id=$1 and dep.dependent_kind=$2 and dep.dependent_id=$3 order by dep.source_key_digest,a.artifact_id limit 4097",t.as_uuid(),kind,id).fetch_all(tx).await.map_err(storage)?;
    if rows.len() > 4096 {
        return Err(unavailable());
    }
    Ok(rows)
}

/// Flatten involved immutable Skill addresses through retained Session roots.
/// The addresses confer no read or activation permission.
pub async fn skill_authorities(
    tx: &mut PgConnection,
    t: TenantId,
    kind: &str,
    id: uuid::Uuid,
) -> Result<Vec<synveda_types::SkillVersionId>> {
    let ids=sqlx::query_scalar!("select distinct version from source_dependencies dep join procedure_outcome_headers header on header.tenant_id=dep.tenant_id and header.event_id=dep.source_event_id cross join lateral unnest(header.skill_versions) version where dep.tenant_id=$1 and dep.dependent_kind=$2 and dep.dependent_id=$3 order by version limit 65",t.as_uuid(),kind,id).fetch_all(tx).await.map_err(storage)?;
    if ids.len() > 64 {
        return Err(unavailable());
    }
    ids.into_iter()
        .map(|id| {
            id.map(synveda_types::SkillVersionId::from_uuid)
                .ok_or_else(unavailable)
        })
        .collect()
}

/// Immutable bindings for one controlled qualification request.
pub struct FreezeEvaluation<'a> {
    /// Exact candidate address.
    pub candidate: CaptureCandidateId,
    /// Exact proposed content.
    pub candidate_hash: &'a str,
    /// Canonical freeze request.
    pub request_hash: &'a str,
    /// Governed suite label.
    pub suite_id: &'a str,
    /// Independently registered evaluator.
    pub evaluator: IdentityId,
    /// Exact live governing configuration.
    pub configuration_hash: &'a str,
    /// Canonical fixed case specification.
    pub specification_hash: &'a str,
    /// Fixed case identities without answers.
    pub specification: &'a Value,
}
/// Freeze a governed suite before any result can qualify the candidate.
pub async fn freeze(
    tx: &mut PgConnection,
    t: TenantId,
    input: &FreezeEvaluation<'_>,
) -> Result<EvaluationAttempt> {
    let header = candidate(tx, t, input.candidate)
        .await?
        .ok_or_else(unavailable)?;
    lock_improvement(tx, t, SessionEventId::from_uuid(header.request_event_id)).await?;
    let existing = attempts(tx, t, input.candidate).await?;
    if let Some(existing) = existing.into_iter().find(|a| {
        a.candidate_hash == input.candidate_hash
            && a.specification_hash == input.specification_hash
            && a.configuration_hash == input.configuration_hash
    }) {
        if existing.request_hash != input.request_hash
            || existing.suite_id != input.suite_id
            || existing.evaluator_id != input.evaluator.as_uuid()
            || existing.specification != *input.specification
        {
            return Err(Error::Conflict {
                message: "Evaluation freeze retry changed".to_owned(),
            });
        }
        return Ok(existing);
    }
    if attempts(tx, t, input.candidate).await?.len() >= 8 {
        return Err(Error::Invalid {
            message: "Candidate evaluation attempt bound exhausted".to_owned(),
        });
    }
    sqlx::query_as!(EvaluationAttempt,r#"insert into procedure_evaluation_attempts(tenant_id,id,candidate_id,candidate_hash,request_hash,suite_id,evaluator_id,configuration_hash,specification_hash,specification)
      values($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
      returning id,candidate_id,candidate_hash,request_hash,suite_id,evaluator_id,configuration_hash,specification_hash,specification"#,
      t.as_uuid(),uuid::Uuid::now_v7(),input.candidate.as_uuid(),input.candidate_hash,input.request_hash,input.suite_id,input.evaluator.as_uuid(),input.configuration_hash,input.specification_hash,input.specification).fetch_one(tx).await.map_err(storage)
}
/// Exact frozen attempt, never a caller-supplied grading specification.
pub async fn attempt(
    tx: &mut PgConnection,
    t: TenantId,
    id: uuid::Uuid,
) -> Result<Option<EvaluationAttempt>> {
    sqlx::query_as!(EvaluationAttempt,"select id,candidate_id,candidate_hash,request_hash,suite_id,evaluator_id,configuration_hash,specification_hash,specification from procedure_evaluation_attempts where tenant_id=$1 and id=$2",t.as_uuid(),id).fetch_optional(tx).await.map_err(storage)
}
/// Insert once; conflicting replay retains the original controlled result.
pub async fn record_result(
    tx: &mut PgConnection,
    t: TenantId,
    id: uuid::Uuid,
    reporter: IdentityId,
    hash: &str,
    qualified: bool,
    results: &Value,
) -> Result<EvaluationResult> {
    sqlx::query!("insert into procedure_evaluation_results(tenant_id,attempt_id,reporter_id,request_hash,qualified,results) values($1,$2,$3,$4,$5,$6) on conflict(tenant_id,attempt_id) do nothing",t.as_uuid(),id,reporter.as_uuid(),hash,qualified,results).execute(&mut *tx).await.map_err(storage)?;
    let result = result(tx, t, id).await?.ok_or_else(unavailable)?;
    if result.reporter_id != reporter.as_uuid()
        || result.request_hash != hash
        || result.qualified != qualified
        || result.results != *results
    {
        return Err(Error::Conflict {
            message: "Evaluation result retry changed".to_owned(),
        });
    }
    Ok(result)
}
/// Exact saved runner result for current independent disclosure.
pub async fn result(
    tx: &mut PgConnection,
    t: TenantId,
    id: uuid::Uuid,
) -> Result<Option<EvaluationResult>> {
    sqlx::query_as!(EvaluationResult,"select attempt_id,reporter_id,request_hash,qualified,results from procedure_evaluation_results where tenant_id=$1 and attempt_id=$2",t.as_uuid(),id).fetch_optional(tx).await.map_err(storage)
}
/// At most eight attempts for one exact frozen candidate; old failures remain.
pub async fn attempts(
    tx: &mut PgConnection,
    t: TenantId,
    id: CaptureCandidateId,
) -> Result<Vec<EvaluationAttempt>> {
    sqlx::query_as!(EvaluationAttempt,"select id,candidate_id,candidate_hash,request_hash,suite_id,evaluator_id,configuration_hash,specification_hash,specification from procedure_evaluation_attempts where tenant_id=$1 and candidate_id=$2 order by created_at,id limit 9",t.as_uuid(),id.as_uuid()).fetch_all(tx).await.map_err(storage)
}
/// Same-Session sources, including retained metadata after physical disposal.
pub async fn outcome_sessions(
    tx: &mut PgConnection,
    t: TenantId,
    session: SessionId,
) -> Result<Vec<uuid::Uuid>> {
    sqlx::query_scalar!("select event_id from procedure_outcome_headers where tenant_id=$1 and session_id=$2 order by event_id limit 65",t.as_uuid(),session.as_uuid()).fetch_all(tx).await.map_err(storage)
}
fn unavailable() -> Error {
    Error::NotFound {
        entity: "Procedure evidence".to_owned(),
    }
}
fn storage(error: sqlx::Error) -> Error {
    Error::Storage {
        message: format!("Procedure evidence storage: {error}"),
    }
}

/// Exact observed outcome for one immutable delivered ContextRun.
pub struct RelatedOutcome {
    /// Exact authenticated reserved event.
    pub event_id: uuid::Uuid,
    /// Source Session, requiring independent diagnostics permission.
    pub session_id: uuid::Uuid,
    /// Closed observed state.
    pub outcome: String,
    /// Authenticated runner or unverified caller provenance.
    pub evidence_class: String,
    /// Database observation clock, not a caller execution clock.
    pub received_at: chrono::DateTime<chrono::Utc>,
}
/// At most 32 displayed outcomes, with a sentinel to avoid exhaustive scans.
pub async fn for_context(
    tx: &mut PgConnection,
    t: TenantId,
    id: synveda_types::ContextRunId,
) -> Result<Vec<RelatedOutcome>> {
    sqlx::query_as!(RelatedOutcome,"select h.event_id,h.session_id,h.outcome,h.evidence_class,h.received_at from (select event_id,session_id,outcome,evidence_class,received_at from procedure_outcome_headers where tenant_id=$1 and context_run_id=$2 order by received_at desc,event_id limit 33) h join session_events e on e.tenant_id=$1 and e.id=h.event_id order by h.received_at desc,h.event_id",t.as_uuid(),id.as_uuid()).fetch_all(tx).await.map_err(storage)
}
