//! SKIL-6 bounded observed outcomes and independently qualified Procedure edits.
use crate::{app::AppState, audit, authz, request};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::HeaderMap,
    response::Response,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{
    capture, configuration, identities, knowledge, procedure_learning as store, rls, scopes,
    sessions,
};
use synveda_types::knowledge::{KnowledgeCommand, KnowledgeType};
use synveda_types::procedure::*;
use synveda_types::{
    AssetKind, CaptureCandidateId, Error, IdentityId, IdentityKind, KnowledgeItemId,
    KnowledgeRevisionId, Result, SessionEventId, SessionId, SkillVersionId, TenantId,
};
use utoipa::ToSchema;

/// Observation over exact involved sources; provenance comes from authentication.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OutcomeBody {
    /// Stable retry address.
    pub client_event_id: String,
    /// Exact involved reviewed Procedure.
    #[schema(value_type=Object)]
    pub procedure: ProcedureRevisionRef,
    /// Optional exact delivery in the same Session.
    #[schema(value_type=Option<Object>)]
    pub context: Option<ProcedureContextRef>,
    /// At most eight exact involved Skill versions, without activation claims.
    #[schema(value_type=Vec<String>)]
    pub skill_versions: Vec<SkillVersionId>,
    /// Bounded reported environment identity.
    #[schema(value_type=Object)]
    pub environment: ProcedureEnvironment,
    /// succeeded, failed or unknown; no causal credit to delivered memory.
    #[schema(value_type=String)]
    pub outcome: ObservedTaskOutcome,
    /// At most eight exact admitted same-Session events.
    #[schema(value_type=Vec<Object>)]
    pub supporting_events: Vec<ProcedureEventRef>,
    /// Digest of runner evidence retained outside the content-free audit plane.
    pub result_digest: String,
}
/// One small human supplied change justified by authenticated runner evidence.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ImprovementBody {
    /// Exact baseline Procedure.
    #[schema(value_type=Object)]
    pub procedure: ProcedureRevisionRef,
    /// One to eight exact authenticated outcome events.
    #[schema(value_type=Vec<Object>)]
    pub outcomes: Vec<ProcedureEventRef>,
    /// At most 1024 UTF-8 bytes to append atomically.
    pub proposed_append: String,
    /// Explicit bounded uncertainty for independent review.
    pub uncertainty: String,
    /// Development case label; held-out cases must differ.
    pub development_case: String,
}
/// Exact receipt; disclosure always rechecks the historical sources.
#[derive(Serialize, ToSchema)]
pub struct ProcedureEventView {
    /// appended or duplicate.
    pub outcome: String,
    /// Exact immutable server-derived evidence.
    pub event: crate::sessions::SessionEventView,
    /// Frozen Capture batch for an improvement request.
    #[schema(value_type=Option<String>,format="uuid")]
    pub batch_id: Option<synveda_types::CaptureBatchId>,
}
/// Freeze one approved suite for this exact candidate before results arrive.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FreezeBody {
    /// Exact candidate hash previously inspected.
    pub candidate_hash: String,
    /// Suite selected from current governed Configuration.
    pub suite_id: String,
}
/// Exact independently reported fixed-case observations.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResultsBody {
    /// Exact candidate hash from the frozen receipt.
    pub candidate_hash: String,
    /// Exact specification hash from the frozen receipt.
    pub specification_hash: String,
    /// Complete fixed case statuses and retained result digests.
    #[schema(value_type=Vec<Object>)]
    pub results: Vec<ProcedureCaseResult>,
}
/// Content-free frozen suite and independently reported status.
#[derive(Serialize, ToSchema)]
pub struct EvaluationView {
    /// Exact attempt address.
    pub attempt_id: String,
    /// Exact candidate address.
    pub candidate_id: String,
    /// Exact proposed content digest.
    pub candidate_hash: String,
    /// Exact governing Configuration digest.
    pub configuration_hash: String,
    /// Exact fixed suite digest.
    pub specification_hash: String,
    /// Fixed specification, without held-out answers.
    #[schema(value_type=Object)]
    pub specification: Value,
    /// Absent before runner results; failed and unknown remain unqualified.
    pub qualified: Option<bool>,
    /// Authenticated observations, never universal quality claims.
    #[schema(value_type=Option<Object>)]
    pub results: Option<Value>,
}
fn missing() -> Error {
    Error::NotFound {
        entity: "Procedure evidence".to_owned(),
    }
}
fn invalid() -> Error {
    Error::Invalid {
        message: "Invalid Procedure evidence or bounds".to_owned(),
    }
}
fn changed() -> Error {
    Error::Conflict {
        message: "Procedure evidence or frozen request changed".to_owned(),
    }
}
fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
fn digest(value: &impl Serialize) -> Result<String> {
    let v = serde_json::to_value(value).map_err(|_| invalid())?;
    if synveda_types::json::canonicalise(&v).to_string().len() > 65_536 {
        return Err(invalid());
    }
    Ok(sessions::payload_hash(&v))
}

async fn baseline(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    session: &synveda_types::session::Session,
    p: &ProcedureRevisionRef,
) -> Result<knowledge::KnowledgeSnapshot> {
    if !valid_digest(&p.content_hash) {
        return Err(invalid());
    }
    let value = knowledge::current(&mut *tx, t, p.item_id)
        .await?
        .ok_or_else(missing)?;
    crate::knowledge_api::authorize_snapshot(state, tx, t, &value).await?;
    if value.item.knowledge_type != KnowledgeType::Procedure
        || value.item.scope_id != session.scope_id
        || value.item.project_id != session.project_id
        || !crate::context_api::current_active_at(&value, Utc::now())
    {
        return Err(missing());
    }
    if value.revision.id != p.revision_id || value.revision.content_hash != p.content_hash {
        return Err(changed());
    }
    let (_, incomplete) =
        crate::context_api::visible_sources_with_policy(state, tx, t, &value.revision).await?;
    if incomplete {
        return Err(missing());
    }
    Ok(value)
}
async fn skills(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    versions: &[SkillVersionId],
) -> Result<()> {
    if versions.len() > 8 || versions.windows(2).any(|p| p[0] >= p[1]) {
        return Err(invalid());
    }
    for id in versions {
        let version = synveda_store::skills::version(&mut *tx, t, *id)
            .await?
            .ok_or_else(missing)?;
        let skill = synveda_store::skills::by_id(&mut *tx, t, version.skill_id)
            .await?
            .ok_or_else(missing)?;
        crate::skills::authorize_skill_read(state, tx, t, &skill, &version).await?;
    }
    Ok(())
}
async fn independent_review(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    scope_id: synveda_types::ScopeId,
    _item: KnowledgeItemId,
    sensitivity: synveda_types::Sensitivity,
) -> Result<()> {
    let scope = scopes::get(&mut *tx, t, scope_id)
        .await?
        .ok_or_else(missing)?;
    let input = authz::gather(
        state,
        tx,
        Some(&scope),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let entries = ["command".to_owned()];
    let requirement = crate::approvals::resolve(
        state,
        tx,
        t,
        &input,
        &crate::approvals::Requested {
            target: &scope,
            asset: AssetKind::Knowledge,
            sensitivity,
            entries: &entries,
        },
    )
    .await?;
    if requirement.is_empty() || !requirement.forbid_author_approval {
        return Err(Error::PolicyDenied {
            action: "procedure.improve".to_owned(),
            resource: "the approval matrix".to_owned(),
            reason: "Procedure learning requires a configured independent reviewer".to_owned(),
        });
    }
    Ok(())
}
async fn reporter(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    scope_id: synveda_types::ScopeId,
) -> Result<(IdentityId, OutcomeEvidenceClass)> {
    let identity = identities::by_subject(&mut *tx, t, &crate::workspaces::subject()?)
        .await?
        .ok_or_else(missing)?;
    if identity.kind != IdentityKind::Service {
        return Ok((identity.id, OutcomeEvidenceClass::ClientReported));
    }
    attest(state, tx, t, scope_id, identity.id).await?;
    Ok((identity.id, OutcomeEvidenceClass::AuthenticatedRunner))
}
async fn attest(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    scope_id: synveda_types::ScopeId,
    id: IdentityId,
) -> Result<()> {
    let scope = scopes::get(&mut *tx, t, scope_id)
        .await?
        .ok_or_else(missing)?;
    let input = authz::gather(
        state,
        tx,
        Some(&scope),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let allowed = authz::decide(
        state,
        &input,
        Action::OutcomeAttest,
        Resource::Scope(scope_id),
    )?;
    let config = configuration::effective_at_scope(tx, t, scope_id).await?;
    if !config
        .document
        .procedure_learning
        .as_ref()
        .is_some_and(|c| c.reporters.contains(&id))
    {
        return Err(Error::PolicyDenied {
            action: Action::OutcomeAttest.as_str().to_owned(),
            resource: Resource::Scope(scope_id).to_string(),
            reason: "The reporter is not admitted by governed Configuration".to_owned(),
        });
    }
    audit::record(tx,t,AuditAction::AuthzDecision,Resource::Scope(scope_id).to_string(),Outcome::Allow,json!({"operation":"procedure.attest","configuration_hash":config.content_hash,"authz":audit::decision_context(Action::OutcomeAttest,&allowed)})).await?;
    Ok(())
}
async fn append(
    tx: &mut PgConnection,
    t: TenantId,
    s: SessionId,
    kind: synveda_types::session::SessionEventType,
    client: String,
    payload: Value,
) -> Result<ProcedureEventView> {
    if synveda_types::json::canonicalise(&payload)
        .to_string()
        .len()
        > 65_536
    {
        return Err(invalid());
    }
    // Only free-form fields are content. Typed UUID/hash proof is validated by
    // the producer and projection, and must not be redacted as numeric prose.
    let scan = match kind {
        synveda_types::session::SessionEventType::TaskOutcome => {
            let value: ProcedureOutcome =
                serde_json::from_value(payload.clone()).map_err(|_| invalid())?;
            synveda_ingest::scan(json!({"task_class":value.environment.task_class,
                "environment_id":value.environment.environment_id}))
        }
        synveda_types::session::SessionEventType::ProcedureImprovement => {
            let value: ProcedureImprovement =
                serde_json::from_value(payload.clone()).map_err(|_| invalid())?;
            synveda_ingest::scan_procedure_improvement(&value)?
        }
        _ => return Err(invalid()),
    };
    if !scan.findings.is_empty() {
        return Err(invalid());
    }
    let event = sessions::NewSessionEvent {
        event_type: kind,
        event_schema_version: 1,
        client_event_id: client,
        occurred_at: Utc::now(),
        source_payload_hash: sessions::payload_hash(&payload),
        payload,
        redactions: None,
        quarantine: false,
    };
    let appended = sessions::append_events(tx, t, s, &[event])
        .await?
        .pop()
        .ok_or_else(missing)?;
    audit::record(tx,t,AuditAction::SessionEventsAppended,format!("session:{s}"),Outcome::Success,json!({"event_id":appended.event.id,"payload_hash":appended.event.payload_hash,"by_type":{kind.as_str():1}})).await?;
    Ok(ProcedureEventView {
        outcome: appended.outcome.as_str().to_owned(),
        event: appended.event.into(),
        batch_id: None,
    })
}

#[utoipa::path(post,path="/v1/sessions/{session_id}/outcomes",operation_id="record_task_outcome",tag="sessions",request_body=OutcomeBody,params(("session_id"=String,Path)),responses((status=200,description="Exact independently authorized outcome",body=ProcedureEventView),(status=400,description="Invalid evidence",body=crate::workspaces::ApiErrorBody),(status=403,description="Reporter denied",body=crate::workspaces::ApiErrorBody),(status=404,description="Unavailable source",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed retry",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn outcome(
    State(state): State<AppState>,
    Path(session): Path<SessionId>,
    payload: std::result::Result<Json<OutcomeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = request::body(payload)?;
        synveda_types::session::validate_client_event_id(&body.client_event_id)?;
        if !label(&body.environment.task_class)
            || !label(&body.environment.environment_id)
            || !valid_digest(&body.environment.environment_digest)
            || !valid_digest(&body.result_digest)
            || body.environment.commit.as_ref().is_some_and(|v| {
                !matches!(v.len(), 40 | 64) || !v.bytes().all(|b| b.is_ascii_hexdigit())
            })
            || body.environment.tree.as_ref().is_some_and(|v| {
                !matches!(v.len(), 40 | 64) || !v.bytes().all(|b| b.is_ascii_hexdigit())
            })
            || body
                .environment
                .dirty_digest
                .as_ref()
                .is_some_and(|v| !valid_digest(v))
            || body.supporting_events.len() > 8
            || body
                .supporting_events
                .windows(2)
                .any(|p| p[0].event_id >= p[1].event_id)
        {
            return Err(invalid());
        }
        let hash = digest(&body)?;
        let t = request::tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, t).await?;
        let (loaded, _, _) =
            crate::sessions::load(&state, &mut tx, t, session, Action::SessionWrite).await?;
        crate::sessions::load(&state, &mut tx, t, session, Action::SessionRead).await?;
        synveda_store::task_state::lock_session(&mut tx, t, session).await?;
        let client = format!(
            "task-outcome:{}",
            blake3::hash(body.client_event_id.as_bytes()).to_hex()
        );
        if let Some(retry) = store::outcome_by_client(&mut tx, t, session, &client).await? {
            if retry.request_hash != hash {
                return Err(changed());
            }
            let (actor, _) = reporter(&state, &mut tx, t, loaded.scope_id).await?;
            if retry.reporter_id != actor.as_uuid() {
                return Err(missing());
            }
            let event = sessions::event(
                &mut *tx,
                t,
                session,
                SessionEventId::from_uuid(retry.event_id),
            )
            .await?
            .ok_or_else(missing)?;
            authorize_event(&state, &mut tx, t, &event).await?;
            request::commit(tx).await?;
            return Ok(Json(ProcedureEventView {
                outcome: "duplicate".to_owned(),
                event: event.into(),
                batch_id: None,
            }));
        }
        // A new observation requires an exact current involved baseline.
        baseline(&state, &mut tx, t, &loaded, &body.procedure).await?;
        skills(&state, &mut tx, t, &body.skill_versions).await?;
        for source in &body.supporting_events {
            crate::sessions::task_state::admitted_event(
                &mut tx,
                t,
                session,
                source.event_id,
                &source.payload_hash,
            )
            .await?;
            crate::knowledge_api::authorize_historical(
                &state,
                &mut tx,
                t,
                "session_event",
                source.event_id.as_uuid(),
            )
            .await?;
        }
        if let Some(context) = &body.context
            && !crate::context_api::procedure_snapshot_available(
                &state,
                &mut tx,
                t,
                session,
                &synveda_types::task_state::TaskContextSnapshot {
                    context_run_id: context.context_run_id,
                    block_hash: context.block_hash.clone(),
                },
            )
            .await?
        {
            return Err(missing());
        }
        let (reporter_id, evidence_class) = reporter(&state, &mut tx, t, loaded.scope_id).await?;
        let value = ProcedureOutcome {
            schema_version: 1,
            request_hash: hash,
            procedure: body.procedure,
            context: body.context,
            skill_versions: body.skill_versions,
            repository_id: loaded.repository_id,
            environment: body.environment,
            outcome: body.outcome,
            evidence_class,
            reporter_id,
            supporting_events: body.supporting_events,
            result_digest: body.result_digest,
        };
        let view = append(
            &mut tx,
            t,
            session,
            synveda_types::session::SessionEventType::TaskOutcome,
            client,
            serde_json::to_value(value).map_err(|_| invalid())?,
        )
        .await?;
        request::commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    finish(&state, "procedure.outcome", result).await
}

#[utoipa::path(post,path="/v1/sessions/{session_id}/procedure-improvements",operation_id="propose_procedure_improvement",tag="capture",request_body=ImprovementBody,params(("session_id"=String,Path)),responses((status=200,description="Exact deduplicated Capture request",body=ProcedureEventView),(status=400,description="Invalid small change",body=crate::workspaces::ApiErrorBody),(status=403,description="Independent review not configured",body=crate::workspaces::ApiErrorBody),(status=404,description="Unavailable outcome",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed evidence",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn improve(
    State(state): State<AppState>,
    Path(session): Path<SessionId>,
    payload: std::result::Result<Json<ImprovementBody>, JsonRejection>,
) -> Response {
    let result=async{
        let body=request::body(payload)?;
        if body.proposed_append.trim().is_empty()||body.proposed_append.len()>1024||body.uncertainty.trim().is_empty()||body.uncertainty.len()>1024||!label(&body.development_case)
         ||body.outcomes.is_empty()||body.outcomes.len()>8||body.outcomes.windows(2).any(|p|p[0].event_id>=p[1].event_id){return Err(invalid())}
        let hash=digest(&body)?;let identity=digest(&json!({"session":session,"procedure":body.procedure,"outcomes":body.outcomes,"append":body.proposed_append}))?;
        let t=request::tenant_id()?;let mut tx=rls::begin_tenant_tx(&state.pool,t).await?;
        let (loaded,_,_)=crate::sessions::load(&state,&mut tx,t,session,Action::SessionWrite).await?;crate::sessions::load(&state,&mut tx,t,session,Action::SessionRead).await?;
        synveda_store::task_state::lock_session(&mut tx,t,session).await?;
        if let Some(retry)=store::improvement_by_identity(&mut tx,t,&identity).await? {
            if retry.request_hash!=hash||retry.session_id!=session.as_uuid(){return Err(changed())}
            let event=sessions::event(&mut *tx,t,session,SessionEventId::from_uuid(retry.event_id)).await?.ok_or_else(missing)?;
            authorize_event(&state,&mut tx,t,&event).await?;
            let config=configuration::effective_at_scope(&mut tx,t,loaded.scope_id).await?;
            let frozen=capture::freeze_procedure_request(&mut tx,&loaded,&config,event.id).await?;
            request::commit(tx).await?;return Ok(Json(ProcedureEventView{outcome:"duplicate".to_owned(),event:event.into(),batch_id:Some(frozen.batch.id)}));
        }
        let current=baseline(&state,&mut tx,t,&loaded,&body.procedure).await?;
        let config=configuration::effective_at_scope(&mut tx,t,loaded.scope_id).await?;
        let learning=config.document.procedure_learning.as_ref().ok_or_else(missing)?;
        let mut sensitivity=current.revision.content.sensitivity;
        for source in &body.outcomes {
            let event=crate::sessions::task_state::admitted_event(&mut tx,t,session,source.event_id,&source.payload_hash).await?;
            crate::knowledge_api::authorize_historical(&state,&mut tx,t,"session_event",source.event_id.as_uuid()).await?;
            let observed:ProcedureOutcome=serde_json::from_value(event.payload).map_err(|_|invalid())?;
            let header=store::outcome(&mut tx,t,source.event_id).await?.ok_or_else(missing)?;
            if header.evidence_class!="authenticated_runner"||observed.procedure!=body.procedure||!learning.reporters.contains(&observed.reporter_id){return Err(missing())}
            skills(&state,&mut tx,t,&observed.skill_versions).await?;
            for id in &observed.skill_versions {let v=synveda_store::skills::version(&mut *tx,t,*id).await?.ok_or_else(missing)?;sensitivity=sensitivity.max(v.sensitivity);}
            for dep in store::historical(&mut tx,t,"session_event",source.event_id.as_uuid()).await? {
                let revision=knowledge::revision(&mut *tx,t,KnowledgeItemId::from_uuid(dep.knowledge_item_id),KnowledgeRevisionId::from_uuid(dep.revision_id)).await?.ok_or_else(missing)?;
                sensitivity=sensitivity.max(revision.content.sensitivity);
            }
        }
        independent_review(&state,&mut tx,t,loaded.scope_id,current.item.id,sensitivity).await?;
        let request_event=if let Some(retry)=store::improvement_by_identity(&mut tx,t,&identity).await? {
            if retry.request_hash!=hash||retry.session_id!=session.as_uuid(){return Err(changed())}
            let event=sessions::event(&mut *tx,t,session,SessionEventId::from_uuid(retry.event_id)).await?.ok_or_else(missing)?;
            ProcedureEventView{outcome:"duplicate".to_owned(),event:event.into(),batch_id:None}
        } else {
            let mut content=current.revision.content.clone();content.body_markdown.push_str("\n\n");content.body_markdown.push_str(&body.proposed_append);content.sensitivity=sensitivity;
            content.metadata.as_object_mut().ok_or_else(invalid)?.insert("synveda_procedure_improvement".to_owned(),Value::String(identity.clone()));
            content.metadata.as_object_mut().ok_or_else(invalid)?.insert("procedure_learning".to_owned(),json!({"uncertainty":body.uncertainty,"development_case":body.development_case,"baseline_revision":body.procedure.revision_id}));
            let mut command=KnowledgeCommand::Edit{item_id:current.item.id,expected_revision_id:current.revision.id,revision_id:KnowledgeRevisionId::new(),content,sources:Vec::new()};
            crate::knowledge::apply_freshness_policy(&mut tx,t,&mut command).await?;
            crate::knowledge::apply_freshness_policy(&mut tx,t,&mut command).await?;
            let KnowledgeCommand::Edit{content,..}=command else{return Err(invalid())};
            let content_hash=knowledge::revision_content_hash(&content);
            let value=ProcedureImprovement{schema_version:1,request_hash:hash,identity_hash:identity.clone(),procedure:body.procedure,outcomes:body.outcomes,proposed_append:body.proposed_append,uncertainty:body.uncertainty,development_case:body.development_case,content,content_hash};
            append(&mut tx,t,session,synveda_types::session::SessionEventType::ProcedureImprovement,format!("procedure-improvement:{identity}"),serde_json::to_value(value).map_err(|_|invalid())?).await?
        };
        let frozen=capture::freeze_procedure_request(&mut tx,&loaded,&config,request_event.event.id).await?;
        audit::record(&mut tx,t,AuditAction::CaptureBatchCreated,format!("capture_batch:{}",frozen.batch.id),Outcome::Success,json!({"batch_id":frozen.batch.id,"input_hash":frozen.batch.input_hash,"request_event_id":request_event.event.id,"configuration_hash":config.content_hash})).await?;
        let view=ProcedureEventView{batch_id:Some(frozen.batch.id),..request_event};request::commit(tx).await?;Ok(Json(view))
    }.await;
    finish(&state, "procedure.improve", result).await
}

fn view(
    attempt: store::EvaluationAttempt,
    result: Option<store::EvaluationResult>,
) -> EvaluationView {
    EvaluationView {
        attempt_id: attempt.id.to_string(),
        candidate_id: attempt.candidate_id.to_string(),
        candidate_hash: attempt.candidate_hash,
        configuration_hash: attempt.configuration_hash,
        specification_hash: attempt.specification_hash,
        specification: attempt.specification,
        qualified: result.as_ref().map(|r| r.qualified),
        results: result.map(|r| r.results),
    }
}

#[utoipa::path(post,path="/v1/capture-candidates/{candidate_id}/procedure-evaluations",operation_id="freeze_procedure_evaluation",tag="capture",request_body=FreezeBody,params(("candidate_id"=String,Path)),responses((status=200,description="Frozen independently authorized suite",body=EvaluationView),(status=400,description="Invalid suite",body=crate::workspaces::ApiErrorBody),(status=404,description="Unavailable candidate",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed candidate",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn freeze(
    State(state): State<AppState>,
    Path(candidate): Path<CaptureCandidateId>,
    payload: std::result::Result<Json<FreezeBody>, JsonRejection>,
) -> Response {
    let result=async{
        let body=request::body(payload)?;if !valid_digest(&body.candidate_hash)||!label(&body.suite_id){return Err(invalid())}
        let t=request::tenant_id()?;let mut tx=rls::begin_tenant_tx(&state.pool,t).await?;
        let (value,_,_,_)=crate::capture::load_candidate(&state,&mut tx,t,candidate,Action::SessionWrite).await?;
        let header=store::candidate(&mut tx,t,candidate).await?.ok_or_else(missing)?;
        if body.candidate_hash!=value.content_hash||body.candidate_hash!=header.content_hash{return Err(changed())}
        let session=SessionId::from_uuid(header.session_id);let (loaded,_,_)=crate::sessions::load(&state,&mut tx,t,session,Action::SessionRead).await?;
        let actor=identities::by_subject(&mut *tx,t,&crate::workspaces::subject()?).await?.ok_or_else(missing)?;
        let config=configuration::effective_at_scope(&mut tx,t,loaded.scope_id).await?;
        let suite=config.document.procedure_learning.as_ref().and_then(|c|c.evaluation_suites.iter().find(|s|s.suite_id==body.suite_id)).ok_or_else(missing)?;
        let evaluator=identities::by_id(&mut *tx,t,suite.evaluator_id).await?.ok_or_else(missing)?;
        if evaluator.kind!=IdentityKind::Service||evaluator.id==actor.id||evaluator.subject.as_deref()==Some(loaded.principal_id.as_str()){return Err(invalid())}
        let event=sessions::event(&mut *tx,t,session,SessionEventId::from_uuid(header.request_event_id)).await?.ok_or_else(missing)?;
        let improvement:ProcedureImprovement=serde_json::from_value(event.payload).map_err(|_|invalid())?;
        if suite.cases.iter().any(|c|c.case_id==improvement.development_case){return Err(invalid())}
        baseline(&state,&mut tx,t,&loaded,&improvement.procedure).await?;
        independent_review(&state,&mut tx,t,loaded.scope_id,improvement.procedure.item_id,value.content.sensitivity).await?;
        let specification=serde_json::to_value(suite).map_err(|_|invalid())?;let spec_hash=digest(&specification)?;let hash=digest(&json!({"candidate":candidate,"body":body,"configuration_hash":config.content_hash,"specification_hash":spec_hash}))?;
        let attempt=store::freeze(&mut tx,t,&store::FreezeEvaluation{candidate,candidate_hash:&value.content_hash,request_hash:&hash,suite_id:&suite.suite_id,evaluator:suite.evaluator_id,configuration_hash:&config.content_hash,specification_hash:&spec_hash,specification:&specification}).await?;
        let recorded=store::result(&mut tx,t,attempt.id).await?;let output=view(attempt,recorded);
        audit::record(&mut tx,t,AuditAction::AuthzDecision,format!("capture_candidate:{candidate}"),Outcome::Allow,json!({"operation":"procedure.evaluation.freeze","attempt_id":output.attempt_id,"candidate_hash":output.candidate_hash,"specification_hash":output.specification_hash,"configuration_hash":output.configuration_hash})).await?;
        request::commit(tx).await?;Ok(Json(output))
    }.await;
    finish(&state, "procedure.freeze", result).await
}

pub(crate) fn qualifies(
    suite: &ProcedureEvaluationSuite,
    results: &[ProcedureCaseResult],
) -> Result<bool> {
    if results.len() != suite.cases.len()
        || results.windows(2).any(|p| p[0].case_id >= p[1].case_id)
    {
        return Err(invalid());
    }
    let mut qualified = true;
    for (case, result) in suite.cases.iter().zip(results) {
        if case.case_id != result.case_id
            || case.digest != result.digest
            || !valid_digest(&result.result_digest)
        {
            return Err(invalid());
        }
        if ((case.required || result.baseline == CaseOutcome::Passed)
            && result.candidate != CaseOutcome::Passed)
            || result.baseline == CaseOutcome::Unknown
        {
            qualified = false
        }
    }
    Ok(qualified)
}
#[utoipa::path(post,path="/v1/procedure-evaluations/{attempt_id}/results",operation_id="record_procedure_evaluation",tag="capture",request_body=ResultsBody,params(("attempt_id"=String,Path)),responses((status=200,description="Authenticated frozen-case result",body=EvaluationView),(status=400,description="Invalid fixed cases",body=crate::workspaces::ApiErrorBody),(status=403,description="Evaluator denied",body=crate::workspaces::ApiErrorBody),(status=404,description="Unavailable evidence",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed frozen binding or retry",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn results(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    payload: std::result::Result<Json<ResultsBody>, JsonRejection>,
) -> Response {
    let result=async{
        let body=request::body(payload)?;let hash=digest(&body)?;let t=request::tenant_id()?;let mut tx=rls::begin_tenant_tx(&state.pool,t).await?;
        let attempt=store::attempt(&mut tx,t,id).await?.ok_or_else(missing)?;
        let candidate=CaptureCandidateId::from_uuid(attempt.candidate_id);let (value,_,_,_)=crate::capture::load_candidate(&state,&mut tx,t,candidate,Action::SessionRead).await?;
        if body.candidate_hash!=attempt.candidate_hash||value.content_hash!=attempt.candidate_hash||body.specification_hash!=attempt.specification_hash{return Err(changed())}
        let (actor,class)=reporter(&state,&mut tx,t,value.proposed_scope_id).await?;
        if class!=OutcomeEvidenceClass::AuthenticatedRunner||actor.as_uuid()!=attempt.evaluator_id{return Err(missing())}
        let config=configuration::effective_at_scope(&mut tx,t,value.proposed_scope_id).await?;
        if config.content_hash!=attempt.configuration_hash{return Err(changed())}
        let suite:ProcedureEvaluationSuite=serde_json::from_value(attempt.specification.clone()).map_err(|_|invalid())?;
        let qualified=qualifies(&suite,&body.results)?;let observations=serde_json::to_value(&body.results).map_err(|_|invalid())?;
        let result=store::record_result(&mut tx,t,id,actor,&hash,qualified,&observations).await?;
        audit::record(&mut tx,t,AuditAction::AuthzDecision,format!("procedure_evaluation:{id}"),Outcome::Allow,json!({"operation":"procedure.evaluation.result","candidate_id":candidate,"candidate_hash":attempt.candidate_hash,"specification_hash":attempt.specification_hash,"reporter_id":actor,"qualified":qualified,"request_hash":hash})).await?;
        let output=view(attempt,Some(result));request::commit(tx).await?;Ok(Json(output))
    }.await;
    finish(&state, "procedure.results", result).await
}

/// Typed source bindings enforce the learning gate even if caller metadata is removed.
pub(crate) async fn guard_command(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    command: &KnowledgeCommand,
) -> Result<()> {
    let sources = match command {
        KnowledgeCommand::Create { sources, .. }
        | KnowledgeCommand::Edit { sources, .. }
        | KnowledgeCommand::Supersede { sources, .. }
        | KnowledgeCommand::Merge { sources, .. } => sources.as_slice(),
        _ => return Ok(()),
    };
    for source in sources {
        let Some(event) = source.session_event_id else {
            continue;
        };
        let Some(improvement) = store::improvement(tx, t, event).await? else {
            continue;
        };
        let retained = sessions::event(
            &mut *tx,
            t,
            SessionId::from_uuid(improvement.session_id),
            event,
        )
        .await?
        .ok_or_else(missing)?;
        authorize_event(state, tx, t, &retained).await?;
        let Some(candidate) = store::candidate_for_request(tx, t, event).await? else {
            return Err(missing());
        };
        let KnowledgeCommand::Edit {
            item_id,
            expected_revision_id,
            content,
            ..
        } = command
        else {
            return Err(invalid());
        };
        if item_id.as_uuid() != improvement.procedure_item_id
            || expected_revision_id.as_uuid() != improvement.procedure_revision_id
            || knowledge::revision_content_hash(content) != improvement.content_hash
        {
            return Err(changed());
        }
        let session = SessionId::from_uuid(improvement.session_id);
        let (loaded, _, _) =
            crate::sessions::load(state, tx, t, session, Action::SessionRead).await?;
        baseline(
            state,
            tx,
            t,
            &loaded,
            &ProcedureRevisionRef {
                item_id: *item_id,
                revision_id: *expected_revision_id,
                content_hash: improvement.procedure_hash,
            },
        )
        .await?;
        crate::knowledge_api::authorize_historical(
            state,
            tx,
            t,
            "capture_candidate",
            candidate.as_uuid(),
        )
        .await?;
        independent_review(state, tx, t, loaded.scope_id, *item_id, content.sensitivity).await?;
        let config = configuration::effective_at_scope(tx, t, loaded.scope_id).await?;
        let attempts = store::attempts(tx, t, candidate).await?;
        let mut passed = false;
        for attempt in attempts {
            if attempt.configuration_hash != config.content_hash
                || attempt.candidate_hash != improvement.content_hash
            {
                continue;
            }
            let suite: ProcedureEvaluationSuite =
                serde_json::from_value(attempt.specification).map_err(|_| invalid())?;
            if !config
                .document
                .procedure_learning
                .as_ref()
                .is_some_and(|c| c.evaluation_suites.contains(&suite))
            {
                continue;
            }
            let Some(result) = store::result(tx, t, attempt.id).await? else {
                continue;
            };
            let results: Vec<ProcedureCaseResult> =
                serde_json::from_value(result.results).map_err(|_| invalid())?;
            if result.reporter_id == attempt.evaluator_id
                && result.qualified
                && qualifies(&suite, &results)?
            {
                passed = true
            }
        }
        if !passed {
            return Err(Error::PolicyDenied {
                action: "procedure.publish".to_owned(),
                resource: "the frozen quality gate".to_owned(),
                reason: "The exact Procedure candidate lacks passing independent qualification"
                    .to_owned(),
            });
        }
    }
    Ok(())
}
async fn finish<T: axum::response::IntoResponse>(
    state: &AppState,
    operation: &'static str,
    result: Result<T>,
) -> Response {
    metrics::counter!("synveda_procedure_learning_requests_total","operation"=>operation,"outcome"=>if result.is_ok(){"ok"}else{"error"}).increment(1);
    crate::response::finish(state, operation, result).await
}

/// Exact frozen candidate submitted through the normal Capture decision.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitBody {
    /// Previously inspected frozen hash.
    pub candidate_hash: String,
}
#[utoipa::path(post,path="/v1/capture-candidates/{candidate_id}/procedure-submit",operation_id="submit_procedure_improvement",tag="capture",request_body=SubmitBody,params(("candidate_id"=String,Path),("Idempotency-Key"=String,Header)),responses((status=201,description="Normal governed Capture decision",body=crate::capture::CaptureDecisionView),(status=403,description="Qualification or review denied",body=crate::workspaces::ApiErrorBody),(status=404,description="Unavailable source",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed candidate",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn submit(
    State(state): State<AppState>,
    Path(candidate): Path<CaptureCandidateId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<SubmitBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = request::body(payload)?;
        if !valid_digest(&body.candidate_hash) {
            return Err(invalid());
        }
        let view =
            crate::capture::submit_procedure(&state, &headers, candidate, &body.candidate_hash)
                .await?;
        Ok((axum::http::StatusCode::CREATED, Json(view)))
    }
    .await;
    finish(&state, "procedure.submit", result).await
}
#[utoipa::path(get,path="/v1/procedure-evaluations/{attempt_id}",operation_id="get_procedure_evaluation",tag="capture",params(("attempt_id"=String,Path)),responses((status=200,description="Currently authorized exact evaluation",body=EvaluationView),(status=404,description="Unavailable candidate or sources",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<uuid::Uuid>) -> Response {
    let result=async{let t=request::tenant_id()?;let mut tx=rls::begin_tenant_tx(&state.pool,t).await?;
        let attempt=store::attempt(&mut tx,t,id).await?.ok_or_else(missing)?;
        crate::capture::load_candidate(&state,&mut tx,t,CaptureCandidateId::from_uuid(attempt.candidate_id),Action::SessionRead).await?;
        let result=store::result(&mut tx,t,id).await?;
        audit::record(&mut tx,t,AuditAction::AuthzDecision,format!("procedure_evaluation:{id}"),Outcome::Allow,json!({"operation":"procedure.evaluation.read","candidate_hash":attempt.candidate_hash,"specification_hash":attempt.specification_hash})).await?;
        let view=view(attempt,result);request::commit(tx).await?;Ok(Json(view))}.await;
    finish(&state, "procedure.get", result).await
}
/// Retained outcome Skill addresses require fresh independent SkillRead as well
/// as the flattened Knowledge/root checks; inclusion never activates a Skill.
pub(crate) async fn authorize_event(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    event: &synveda_types::session::SessionEvent,
) -> Result<()> {
    crate::knowledge_api::authorize_historical(state, tx, t, "session_event", event.id.as_uuid())
        .await?;
    Ok(())
}

/// An applied receipt is read authority plus current write/proposal authority,
/// not another attempt to publish against its already-consumed old head.
pub(crate) async fn authorize_command_sources(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    command: &KnowledgeCommand,
) -> Result<()> {
    let sources = match command {
        KnowledgeCommand::Create { sources, .. }
        | KnowledgeCommand::Edit { sources, .. }
        | KnowledgeCommand::Supersede { sources, .. }
        | KnowledgeCommand::Merge { sources, .. } => sources.as_slice(),
        _ => return Ok(()),
    };
    for source in sources {
        if let Some(id) = source.session_event_id
            && let Some(header) = store::improvement(tx, t, id).await?
        {
            let event = sessions::event(&mut *tx, t, SessionId::from_uuid(header.session_id), id)
                .await?
                .ok_or_else(missing)?;
            authorize_event(state, tx, t, &event).await?;
        }
    }
    Ok(())
}

/// Exact retained involvement still requires live SkillRead; it is not activation.
pub(crate) async fn authorize_skill_versions(
    state: &AppState,
    tx: &mut PgConnection,
    t: TenantId,
    versions: &[SkillVersionId],
) -> Result<()> {
    if versions.len() > 64 {
        return Err(missing());
    }
    for id in versions {
        let version = synveda_store::skills::version(&mut *tx, t, *id)
            .await?
            .ok_or_else(missing)?;
        let skill = synveda_store::skills::by_id(&mut *tx, t, version.skill_id)
            .await?
            .ok_or_else(missing)?;
        crate::skills::authorize_skill_read(state, tx, t, &skill, &version).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_case_gate_refuses_unknown_missing_and_baseline_regression() {
        let case = ProcedureEvaluationCase {
            case_id: "held-out".to_owned(),
            digest: "a".repeat(64),
            required: true,
        };
        let suite = ProcedureEvaluationSuite {
            suite_id: "owned".to_owned(),
            evaluator_id: IdentityId::new(),
            harness: "fixture".to_owned(),
            harness_version: "v1".to_owned(),
            configuration_digest: "b".repeat(64),
            cases: vec![case.clone()],
        };
        let mut result = ProcedureCaseResult {
            case_id: case.case_id.clone(),
            digest: case.digest.clone(),
            baseline: CaseOutcome::Failed,
            candidate: CaseOutcome::Passed,
            result_digest: "c".repeat(64),
        };
        assert!(qualifies(&suite, &[result.clone()]).unwrap());
        result.candidate = CaseOutcome::Unknown;
        assert!(!qualifies(&suite, &[result.clone()]).unwrap());
        result.baseline = CaseOutcome::Passed;
        result.candidate = CaseOutcome::Failed;
        assert!(!qualifies(&suite, &[result.clone()]).unwrap());
        assert!(qualifies(&suite, &[]).is_err());
        result.digest = "d".repeat(64);
        assert!(qualifies(&suite, &[result]).is_err());
    }
}
