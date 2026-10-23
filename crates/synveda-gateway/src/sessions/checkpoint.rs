//! Checkpoint derivation from already admitted Session evidence.

use std::collections::HashSet;

use serde_json::Value;
use sqlx::PgConnection;
use synveda_retrieval::TokenCounter;
use synveda_store::sessions::{self, AppendOutcome, NewSessionEvent};
use synveda_types::session::{
    CheckpointEvidence, CheckpointSource, CompactionBoundary, SessionCheckpoint, SessionEvent,
    SessionEventType,
};
use synveda_types::{Error, Result, SessionId, TenantId};

const MAX_EXPECTED_IDS: usize = 64;
const MAX_SOURCE_EVENTS: usize = 64;
const MAX_EXCERPTS: usize = 8;
const MAX_EXCERPT_BYTES: usize = 2_048;
const MAX_RESTART_CHECKPOINTS: usize = 4;

/// Clients with implemented, real compact/start interfaces.
pub(crate) fn supported_client(client: &str) -> bool {
    matches!(client, "claude-code" | "codex")
}

fn source_digest(sources: &[CheckpointSource]) -> String {
    let mut digest = blake3::Hasher::new();
    digest.update(b"synveda.session.checkpoint.sources.v1\0");
    for source in sources {
        digest.update(&source.sequence.to_be_bytes());
        digest.update(source.event_id.as_uuid().as_bytes());
        digest.update(source.payload_hash.as_bytes());
    }
    digest.finalize().to_hex().to_string()
}

pub(super) fn validate_boundary(payload: Option<&Value>) -> Result<CompactionBoundary> {
    let boundary: CompactionBoundary =
        serde_json::from_value(payload.cloned().ok_or_else(|| Error::Invalid {
            message: "compaction boundary needs a typed payload".to_owned(),
        })?)
        .map_err(|_| Error::Invalid {
            message: "invalid compaction boundary payload".to_owned(),
        })?;
    if boundary.schema_version != 1
        || boundary.local_high_sequence == 0
        || boundary.expected_client_event_ids.len() > MAX_EXPECTED_IDS
    {
        return Err(Error::Invalid {
            message: "compaction boundary exceeds the supported version or bounds".to_owned(),
        });
    }
    let mut seen = HashSet::new();
    for id in &boundary.expected_client_event_ids {
        if id.is_empty()
            || id.len() > 200
            || id
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
            || !seen.insert(id)
        {
            return Err(Error::Invalid {
                message: "compaction boundary has an invalid or repeated source id".to_owned(),
            });
        }
    }
    Ok(boundary)
}

/// Builds one immutable derived event in the same transaction that admitted
/// the boundary. Replaying the boundary cannot create a second checkpoint.
pub(super) async fn materialize(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    session_id: SessionId,
    marker: &SessionEvent,
    boundary: &CompactionBoundary,
) -> Result<Option<SessionEvent>> {
    let previous = if let Some(head) =
        synveda_store::task_state::latest_checkpoint(tx, tenant_id, session_id, None).await?
    {
        sessions::event(
            &mut *tx,
            tenant_id,
            session_id,
            synveda_types::SessionEventId::from_uuid(head.id),
        )
        .await?
    } else {
        None
    };
    let previous = if let Some(candidate) = previous {
        if let Some(parsed) =
            verified_checkpoint(state, tx, tenant_id, session_id, &candidate).await?
        {
            let boundary =
                sessions::event(&mut *tx, tenant_id, session_id, parsed.boundary_event_id).await?;
            // One append may admit several markers before deriving checkpoints.
            // Their new event sequence is after the batch; order by the actual
            // earlier boundary, rather than treating that derived sequence as
            // the source window's upper bound.
            boundary
                .filter(|boundary| boundary.sequence < marker.sequence)
                .map(|boundary| (candidate, boundary.sequence))
        } else {
            None
        }
    } else {
        None
    };
    let mut task_state_sequence = None;
    let task_state = if let Some(head) =
        synveda_store::task_state::latest(tx, tenant_id, session_id, Some(marker.sequence)).await?
    {
        let event = sessions::event(
            &mut *tx,
            tenant_id,
            session_id,
            synveda_types::SessionEventId::from_uuid(head.id),
        )
        .await?;
        if let Some(event) = event {
            super::task_state::verified(state, tx, tenant_id, session_id, &event)
                .await?
                .map(|_| {
                    task_state_sequence = Some(event.sequence);
                    synveda_types::task_state::CheckpointTaskState {
                        event_id: event.id,
                        payload_hash: event.payload_hash,
                    }
                })
        } else {
            None
        }
    } else {
        None
    };
    if task_state.is_none()
        && synveda_store::task_state::latest(tx, tenant_id, session_id, Some(marker.sequence))
            .await?
            .is_some()
    {
        // Never replace an unavailable current state with older objectives.
        return Ok(None);
    }
    // Complete replacement state supersedes all earlier raw objectives.
    let after = previous
        .as_ref()
        .map_or(0, |(_, sequence)| *sequence)
        .max(task_state_sequence.unwrap_or(0));
    let mut recent = sessions::events_before(
        &mut *tx,
        tenant_id,
        session_id,
        after,
        marker.sequence,
        (MAX_SOURCE_EVENTS + 1) as i64,
    )
    .await?;
    let truncated = recent.len() > MAX_SOURCE_EVENTS;
    recent.truncate(MAX_SOURCE_EVENTS);
    recent.reverse();
    let mut reviewed_ids: Vec<_> = recent.iter().map(|event| event.id).collect();
    reviewed_ids.push(marker.id);
    let withheld: HashSet<_> =
        sessions::withheld_event_ids(&mut *tx, tenant_id, session_id, &reviewed_ids)
            .await?
            .into_iter()
            .collect();
    if withheld.contains(&marker.id) {
        return Ok(None);
    }

    let present: HashSet<String> = sessions::preceding_client_event_ids(
        &mut *tx,
        tenant_id,
        session_id,
        marker.sequence,
        &boundary.expected_client_event_ids,
    )
    .await?
    .into_iter()
    .collect();
    let missing = boundary
        .expected_client_event_ids
        .iter()
        .filter(|id| !present.contains(*id))
        .count();
    let continuous = recent
        .first()
        .zip(recent.last())
        .is_none_or(|(first, last)| {
            last.sequence - first.sequence + 1 == recent.len() as i64
                && last.sequence == marker.sequence - 1
        });
    let coverage = if missing == 0
        && !truncated
        && !boundary.expected_ids_truncated
        && withheld.is_empty()
        && continuous
        && recent.len() as i64 == marker.sequence - after - 1
    {
        "observed_window"
    } else {
        "incomplete"
    };

    let sources: Vec<CheckpointSource> = recent
        .iter()
        .map(|event| CheckpointSource {
            event_id: event.id,
            sequence: event.sequence,
            payload_hash: event.payload_hash.clone(),
        })
        .collect();
    let labelled_excerpts = recent
        .iter()
        .rev()
        .filter(|event| event.event_type == SessionEventType::MessageUser)
        .filter(|event| !withheld.contains(&event.id))
        .filter_map(|event| {
            let text = event.payload.get("text")?.as_str()?;
            let excerpt = if text.len() <= MAX_EXCERPT_BYTES {
                text
            } else {
                text.split("\n\n")
                    .find(|unit| !unit.is_empty() && unit.len() <= MAX_EXCERPT_BYTES)?
            };
            Some(CheckpointEvidence {
                source_event_id: event.id,
                text: excerpt.to_owned(),
                assertion_class: "user_authored".to_owned(),
            })
        })
        .take(MAX_EXCERPTS)
        .collect();
    let checkpoint = SessionCheckpoint {
        schema_version: if task_state.is_some() { 2 } else { 1 },
        method: if task_state.is_some() {
            "exact_task_state_and_event_excerpts_v2"
        } else {
            "deterministic_event_excerpts_v1"
        }
        .to_owned(),
        boundary_event_id: marker.id,
        previous_checkpoint_event_id: previous
            .filter(|_| task_state.is_none())
            .map(|(event, _)| event.id),
        source_first_sequence: recent.first().map(|event| event.sequence),
        source_last_sequence: recent.last().map(|event| event.sequence),
        source_digest: source_digest(&sources),
        sources,
        coverage: coverage.to_owned(),
        declared_missing_count: u32::try_from(missing).unwrap_or(u32::MAX),
        goals: Vec::new(),
        constraints: Vec::new(),
        decisions: Vec::new(),
        unresolved_questions: Vec::new(),
        artefact_references: Vec::new(),
        verification_evidence: Vec::new(),
        labelled_excerpts,
        task_state,
    };
    let payload = serde_json::to_value(&checkpoint).map_err(|_| Error::Internal {
        message: "serialize bounded checkpoint".to_owned(),
    })?;
    let generated = NewSessionEvent {
        event_type: SessionEventType::Checkpoint,
        event_schema_version: 1,
        client_event_id: format!(
            "checkpoint:{}",
            blake3::hash(marker.client_event_id.as_bytes()).to_hex()
        ),
        occurred_at: marker.occurred_at,
        source_payload_hash: sessions::payload_hash(&payload),
        payload,
        redactions: None,
        quarantine: false,
    };
    let mut appended = sessions::append_events(tx, tenant_id, session_id, &[generated]).await?;
    let result = appended.pop().ok_or_else(|| Error::Internal {
        message: "checkpoint append returned no outcome".to_owned(),
    })?;
    Ok((result.outcome == AppendOutcome::Appended).then_some(result.event))
}

async fn verified_checkpoint(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    session_id: SessionId,
    event: &SessionEvent,
) -> Result<Option<SessionCheckpoint>> {
    if event.event_type != SessionEventType::Checkpoint {
        return Ok(None);
    }
    let Ok(parsed) = serde_json::from_value::<SessionCheckpoint>(event.payload.clone()) else {
        return Ok(None);
    };
    let method_valid = matches!(
        (
            parsed.schema_version,
            parsed.method.as_str(),
            &parsed.task_state
        ),
        (1, "deterministic_event_excerpts_v1", None)
            | (2, "exact_task_state_and_event_excerpts_v2", Some(_))
    );
    if !method_valid
        || parsed.sources.len() > MAX_SOURCE_EVENTS
        || parsed.source_digest != source_digest(&parsed.sources)
        || !matches!(parsed.coverage.as_str(), "observed_window" | "incomplete")
        || parsed.source_first_sequence != parsed.sources.first().map(|source| source.sequence)
        || parsed.source_last_sequence != parsed.sources.last().map(|source| source.sequence)
        || parsed.labelled_excerpts.len() > MAX_EXCERPTS
        || (parsed.sources.is_empty() && !parsed.labelled_excerpts.is_empty())
        || !parsed.goals.is_empty()
        || !parsed.constraints.is_empty()
        || !parsed.decisions.is_empty()
        || !parsed.unresolved_questions.is_empty()
        || !parsed.artefact_references.is_empty()
        || !parsed.verification_evidence.is_empty()
    {
        return Ok(None);
    }
    let boundary =
        sessions::event(&mut *tx, tenant_id, session_id, parsed.boundary_event_id).await?;
    let Some(boundary) = boundary else {
        return Ok(None);
    };
    if boundary.event_type != SessionEventType::CompactionBoundary
        || boundary.sequence >= event.sequence
        || parsed
            .source_last_sequence
            .is_some_and(|sequence| sequence >= boundary.sequence)
    {
        return Ok(None);
    }
    let mut reviewed_ids: Vec<_> = parsed
        .sources
        .iter()
        .map(|source| source.event_id)
        .collect();
    reviewed_ids.push(parsed.boundary_event_id);
    reviewed_ids.push(event.id);
    let withheld: HashSet<_> =
        sessions::withheld_event_ids(&mut *tx, tenant_id, session_id, &reviewed_ids)
            .await?
            .into_iter()
            .collect();
    if withheld.contains(&parsed.boundary_event_id) || withheld.contains(&event.id) {
        return Ok(None);
    }
    if let Some(previous_id) = parsed
        .previous_checkpoint_event_id
        .filter(|_| parsed.schema_version == 1)
    {
        let previous = sessions::event(&mut *tx, tenant_id, session_id, previous_id).await?;
        if !previous.is_some_and(|held| held.event_type == SessionEventType::Checkpoint) {
            return Ok(None);
        }
    }
    if let Some(reference) = &parsed.task_state {
        let Some(held) =
            sessions::event(&mut *tx, tenant_id, session_id, reference.event_id).await?
        else {
            return Ok(None);
        };
        if held.payload_hash != reference.payload_hash
            || held.sequence >= boundary.sequence
            || super::task_state::verified(state, tx, tenant_id, session_id, &held)
                .await?
                .is_none()
        {
            return Ok(None);
        }
    }
    let Some(first) = parsed.sources.first() else {
        return Ok(Some(parsed));
    };
    let held = sessions::events(
        &mut *tx,
        tenant_id,
        session_id,
        first.sequence - 1,
        parsed.sources.len() as i64,
    )
    .await?;
    for source in &parsed.sources {
        if let Some(event) = held.iter().find(|event| event.id == source.event_id) {
            if event.sequence != source.sequence || event.payload_hash != source.payload_hash {
                return Ok(None);
            }
        } else {
            // Coverage is bookkeeping, not copied evidence. A permanently
            // suppressed coverage-only event may be checked by its immutable
            // header; retention loss without suppression still fails closed.
            if parsed
                .labelled_excerpts
                .iter()
                .any(|excerpt| excerpt.source_event_id == source.event_id)
            {
                return Ok(None);
            }
            let address = synveda_store::source_control::address(
                &mut *tx,
                tenant_id,
                session_id,
                source.event_id,
            )
            .await?;
            if !address.is_some_and(|address| {
                address.sequence == source.sequence && address.payload_hash == source.payload_hash
            }) || synveda_store::source_control::available(
                &mut *tx,
                tenant_id,
                "session_event",
                source.event_id.as_uuid(),
            )
            .await?
            {
                return Ok(None);
            }
        }
    }
    for excerpt in &parsed.labelled_excerpts {
        if excerpt.assertion_class != "user_authored"
            || excerpt.text.is_empty()
            || excerpt.text.len() > MAX_EXCERPT_BYTES
            || withheld.contains(&excerpt.source_event_id)
            || !held.iter().any(|source| {
                source.id == excerpt.source_event_id
                    && source.event_type == SessionEventType::MessageUser
                    && source
                        .payload
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| text.contains(&excerpt.text))
            })
        {
            return Ok(None);
        }
    }
    Ok(Some(parsed))
}

/// Follow only the bounded checkpoint chain whose text can enter one
/// restart. An invalid dependency rejects the whole derived chain while the
/// caller can still use independently admitted recent events.
async fn verified_chain(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    session_id: SessionId,
    latest: SessionEvent,
) -> Result<Option<Vec<(SessionEvent, SessionCheckpoint)>>> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = latest;
    loop {
        if !seen.insert(current.id) {
            return Ok(None);
        }
        let Some(parsed) = verified_checkpoint(state, tx, tenant_id, session_id, &current).await?
        else {
            return Ok(None);
        };
        // V2 is a complete replacement state. Older structured objectives and
        // their obsolete sources cannot become part of the new restoration.
        let previous_id = parsed
            .previous_checkpoint_event_id
            .filter(|_| parsed.task_state.is_none());
        chain.push((current, parsed));
        if chain.len() == MAX_RESTART_CHECKPOINTS {
            return Ok(Some(chain));
        }
        let Some(previous_id) = previous_id else {
            return Ok(Some(chain));
        };
        let Some(previous) = sessions::event(&mut *tx, tenant_id, session_id, previous_id).await?
        else {
            return Ok(None);
        };
        current = previous;
    }
}

/// Recheck an exact checkpoint dependency before disclosing a retained
/// ContextRun's rendered copy of its derivative.
pub(crate) async fn source_available(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    session_id: SessionId,
    checkpoint_id: synveda_types::SessionEventId,
) -> Result<bool> {
    let Some(event) = sessions::event(&mut *tx, tenant_id, session_id, checkpoint_id).await? else {
        return Ok(false);
    };
    Ok(verified_chain(state, tx, tenant_id, session_id, event)
        .await?
        .is_some())
}

/// Current independent authority for a separate source snapshot.
pub(crate) async fn handoff_available(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    handoff: &synveda_types::task_state::SessionHandoff,
    ceiling: Option<synveda_types::Sensitivity>,
) -> Result<bool> {
    if !super::task_state::hash_valid(&handoff.payload_hash) {
        return Ok(false);
    }
    match super::load(
        state,
        tx,
        tenant,
        handoff.source_session_id,
        synveda_policy::Action::SessionRead,
    )
    .await
    {
        Ok((_, allowed, resource)) => {
            super::read_event(
                tx,
                tenant,
                "session.handoff.source",
                synveda_policy::Action::SessionRead,
                &allowed,
                resource,
                serde_json::json!({"session_id":handoff.source_session_id}),
            )
            .await?;
        }
        Err(Error::NotFound { .. } | Error::PolicyDenied { .. }) => return Ok(false),
        Err(error) => return Err(error),
    }
    let Some(event) = sessions::event(
        &mut *tx,
        tenant,
        handoff.source_session_id,
        handoff.snapshot_event_id,
    )
    .await?
    else {
        return Ok(false);
    };
    if event.payload_hash != handoff.payload_hash {
        return Ok(false);
    }
    if !synveda_store::task_state::within_ceiling(tx, tenant, event.id, ceiling).await? {
        return Ok(false);
    }
    match event.event_type {
        SessionEventType::TaskState => {
            Ok(
                super::task_state::verified(state, tx, tenant, handoff.source_session_id, &event)
                    .await?
                    .is_some(),
            )
        }
        SessionEventType::Checkpoint => {
            source_available(state, tx, tenant, handoff.source_session_id, event.id).await
        }
        _ => Ok(false),
    }
}

fn state_lines(
    task: &synveda_types::task_state::TaskState,
    source_session: SessionId,
    event_id: synveda_types::SessionEventId,
    receiver: Option<&synveda_types::session::Session>,
    checkout: Option<&crate::context_api::CheckoutObservationBody>,
) -> String {
    let checkout_state = match (&task.checkout, checkout) {
        (Some(old), Some(new))
            if old.worktree_ref != new.r#ref
                || old.branch != new.branch
                || old.commit != new.commit
                || old.tree != new.tree
                || old.dirty != new.dirty
                || old.dirty_digest != new.dirty_digest =>
        {
            "changed_revalidation_required"
        }
        (Some(old), Some(new))
            if old.commit.is_some()
                && new.commit.is_some()
                && old.dirty == Some(false)
                && new.dirty == Some(false) =>
        {
            "reported_match_verification_historical"
        }
        (Some(old), None) if receiver.is_some_and(|s| s.branch != old.branch) => {
            "branch_changed_revalidation_required"
        }
        _ => "unknown_revalidation_required",
    };
    let repository_state = receiver.map_or("unknown", |s| {
        if s.repository_id == task.repository_id {
            "same_attachment"
        } else {
            "changed_attachment"
        }
    });
    let mut text = format!(
        "- {}\n",
        serde_json::json!({"kind":"task_state_metadata","source_session_id":source_session,
        "task_state_event_id":event_id,"repository_id":task.repository_id,"repository_state":repository_state,
        "checkout":task.checkout,"checkout_state":checkout_state,"context_snapshot":task.context_snapshot,
        "verification":"historical_reports_only; authenticated execution evidence is not supplied"})
    );
    for entry in &task.statements {
        text.push_str(&format!(
            "- {}\n",
            serde_json::json!({"kind":"task_state_statement","source_session_id":source_session,
            "task_state_event_id":event_id,"statement":entry})
        ));
    }
    text
}

/// One receiver and assigned allowance shared by both restoration paths.
pub(crate) struct RestoreInputs<'a> {
    pub receiver: &'a synveda_types::session::Session,
    pub checkout: Option<&'a crate::context_api::CheckoutObservationBody>,
    pub ceiling: Option<synveda_types::Sensitivity>,
    pub budget: u32,
    pub counter: &'a TokenCounter,
}

/// An explicit handoff is an atomic required component, never partial state.
pub(crate) async fn handoff_context(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    handoff: &synveda_types::task_state::SessionHandoff,
    inputs: RestoreInputs<'_>,
) -> Result<(String, Vec<synveda_types::SessionEventId>)> {
    let RestoreInputs {
        receiver,
        checkout,
        ceiling,
        budget,
        counter,
    } = inputs;
    if !handoff_available(state, tx, tenant, handoff, ceiling).await? {
        return Err(Error::NotFound {
            entity: "Handoff evidence".to_owned(),
        });
    }
    let event = sessions::event(
        &mut *tx,
        tenant,
        handoff.source_session_id,
        handoff.snapshot_event_id,
    )
    .await?
    .ok_or_else(|| Error::NotFound {
        entity: "Handoff evidence".to_owned(),
    })?;
    let mut delivered_roots = vec![event.id];
    let mut text = format!(
        "# Synveda source-grounded task handoff\nSource Session: {}; snapshot: {}. Historical evidence only. Newer instructions and current policy govern. This snapshot grants no permission to repeat an external effect; an unknown outcome remains unknown.\n",
        handoff.source_session_id, event.id
    );
    if event.event_type == SessionEventType::TaskState {
        let task =
            super::task_state::verified(state, tx, tenant, handoff.source_session_id, &event)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: "Handoff evidence".to_owned(),
                })?;
        text.push_str(&state_lines(
            &task,
            handoff.source_session_id,
            event.id,
            Some(receiver),
            checkout,
        ));
    } else {
        let chain = verified_chain(state, tx, tenant, handoff.source_session_id, event)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: "Handoff evidence".to_owned(),
            })?;
        for (checkpoint, parsed) in chain.iter().rev() {
            delivered_roots.push(checkpoint.id);
            if !synveda_store::task_state::within_ceiling(tx, tenant, checkpoint.id, ceiling)
                .await?
            {
                return Err(Error::NotFound {
                    entity: "Handoff evidence".to_owned(),
                });
            }
            if let Some(reference) = &parsed.task_state {
                delivered_roots.push(reference.event_id);
                let event = sessions::event(
                    &mut *tx,
                    tenant,
                    handoff.source_session_id,
                    reference.event_id,
                )
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: "Handoff evidence".to_owned(),
                })?;
                let task = super::task_state::verified(
                    state,
                    tx,
                    tenant,
                    handoff.source_session_id,
                    &event,
                )
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: "Handoff evidence".to_owned(),
                })?;
                text.push_str(&state_lines(
                    &task,
                    handoff.source_session_id,
                    event.id,
                    Some(receiver),
                    checkout,
                ));
            }
            for evidence in &parsed.labelled_excerpts {
                delivered_roots.push(evidence.source_event_id);
                text.push_str(&format!("- {}\n",serde_json::json!({"kind":"checkpoint_user_excerpt","checkpoint_event_id":checkpoint.id,
                    "source_session_id":handoff.source_session_id,"session_event_id":evidence.source_event_id,
                    "assertion_class":evidence.assertion_class,"text":evidence.text})));
            }
        }
    }
    let required_tokens = counter.count(&text);
    if text.len() > 16 * 1024 || required_tokens.saturating_add(counter.count("\n\n")) > budget {
        return Err(Error::InsufficientBudget {
            required_tokens: required_tokens.saturating_add(counter.count("\n\n")),
            budget_tokens: budget,
        });
    }
    delivered_roots.sort_unstable();
    delivered_roots.dedup();
    Ok((text, delivered_roots))
}

/// Bounded, source-labelled working context. The caller must have separately
/// authorised SessionRead before calling this function.
pub(crate) async fn restart_context(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    session_id: SessionId,
    inputs: RestoreInputs<'_>,
) -> Result<(
    String,
    Option<synveda_types::SessionEventId>,
    String,
    Vec<synveda_types::SessionEventId>,
    Vec<synveda_types::SessionEventId>,
)> {
    let RestoreInputs {
        receiver,
        checkout,
        ceiling,
        budget,
        counter,
    } = inputs;
    const MAX_TAIL_EVENTS: usize = 16;
    const MAX_RESTART_BYTES: usize = 16 * 1024;
    let latest_state = if let Some(head) =
        synveda_store::task_state::latest(tx, tenant_id, session_id, None).await?
    {
        let id = synveda_types::SessionEventId::from_uuid(head.id);
        let Some(event) = sessions::event(&mut *tx, tenant_id, session_id, id).await? else {
            return Ok((
                String::new(),
                None,
                "task_state_source_unavailable".to_owned(),
                vec![],
                vec![],
            ));
        };
        let Some(parsed) =
            super::task_state::verified(state, tx, tenant_id, session_id, &event).await?
        else {
            return Ok((
                String::new(),
                None,
                "task_state_source_unavailable".to_owned(),
                vec![],
                vec![],
            ));
        };
        if !synveda_store::task_state::within_ceiling(tx, tenant_id, id, ceiling).await? {
            return Ok((
                String::new(),
                None,
                "task_state_sensitivity_unavailable".to_owned(),
                vec![],
                vec![],
            ));
        }
        Some((event, parsed))
    } else {
        None
    };
    let mut checkpoint = None;
    let mut status = "uncheckpointed".to_owned();
    let mut after = 0;
    if let Some(head) =
        synveda_store::task_state::latest_checkpoint(tx, tenant_id, session_id, None).await?
    {
        after = head.sequence;
        if let Some(event) = sessions::event(
            &mut *tx,
            tenant_id,
            session_id,
            synveda_types::SessionEventId::from_uuid(head.id),
        )
        .await?
            && let Some(chain) = verified_chain(state, tx, tenant_id, session_id, event).await?
        {
            let (_, parsed) = chain.first().ok_or_else(|| Error::Internal {
                message: "empty checkpoint chain".to_owned(),
            })?;
            let matches_latest = latest_state.as_ref().is_none_or(|(event, _)| {
                parsed
                    .task_state
                    .as_ref()
                    .is_some_and(|reference| reference.event_id == event.id)
            });
            if matches_latest {
                status = parsed.coverage.clone();
                after = sessions::event(&mut *tx, tenant_id, session_id, parsed.boundary_event_id)
                    .await?
                    .map_or(head.sequence, |e| e.sequence);
                if chain.len() == MAX_RESTART_CHECKPOINTS
                    && chain
                        .last()
                        .is_some_and(|(_, p)| p.previous_checkpoint_event_id.is_some())
                {
                    status.push_str(";older_checkpoints_omitted");
                }
                checkpoint = Some(chain);
            } else {
                status = "newer_task_state".to_owned();
            }
        } else {
            status = "checkpoint_source_unavailable".to_owned();
        }
    }
    if let Some((event, _)) = &latest_state {
        after = after.max(event.sequence);
    }
    let mut tail = sessions::events_before(
        &mut *tx,
        tenant_id,
        session_id,
        after,
        i64::MAX,
        (MAX_TAIL_EVENTS + 1) as i64,
    )
    .await?;
    if tail.len() > MAX_TAIL_EVENTS {
        status.push_str(";tail_truncated");
    }
    tail.truncate(MAX_TAIL_EVENTS);
    tail.reverse();
    let held_ids: Vec<_> = tail.iter().map(|e| e.id).collect();
    let withheld: HashSet<_> =
        sessions::withheld_event_ids(&mut *tx, tenant_id, session_id, &held_ids)
            .await?
            .into_iter()
            .collect();
    let checkpoint_id = checkpoint
        .as_ref()
        .and_then(|chain| chain.first().map(|(e, _)| e.id));
    let mut rendered = format!(
        "# Synveda Session restart evidence\nSession: {session_id}; checkpoint: {}; coverage: {status}. Historical evidence only; newer instructions and current policy govern. Unknown outcomes never authorize repeating an external effect.\n",
        checkpoint_id.map_or_else(|| "none".to_owned(), |id| id.to_string())
    );
    let header_bytes = rendered.len();
    if header_bytes > MAX_RESTART_BYTES || counter.count(&rendered) > budget {
        return Ok((String::new(), None, status, vec![], vec![]));
    }
    let mut delivered_roots = Vec::new();
    let mut tail_ids = Vec::new();
    if let Some((event, task)) = &latest_state {
        let structured = state_lines(task, session_id, event.id, Some(receiver), checkout);
        if rendered.len() + structured.len() > MAX_RESTART_BYTES
            || counter.count(&format!("{rendered}{structured}")) > budget
        {
            return Ok((
                String::new(),
                None,
                "task_state_omitted_budget".to_owned(),
                vec![],
                vec![],
            ));
        }
        rendered.push_str(&structured);
        delivered_roots.push(event.id);
    }
    if let Some(chain) = checkpoint {
        for (checkpoint, parsed) in chain.iter().rev() {
            for evidence in parsed.labelled_excerpts.iter().rev() {
                let line = format!(
                    "- {}\n",
                    serde_json::json!({"kind":"checkpoint_user_excerpt","checkpoint_event_id":checkpoint.id,
                    "session_event_id":evidence.source_event_id,"assertion_class":evidence.assertion_class,"text":evidence.text})
                );
                if rendered.len() + line.len() <= MAX_RESTART_BYTES
                    && counter.count(&format!("{rendered}{line}")) <= budget
                {
                    rendered.push_str(&line);
                    delivered_roots.push(evidence.source_event_id);
                }
            }
        }
    }
    for event in tail {
        if event.event_type != SessionEventType::MessageUser || withheld.contains(&event.id) {
            continue;
        }
        let Some(text) = event.payload.get("text").and_then(Value::as_str) else {
            continue;
        };
        if text.len() > MAX_EXCERPT_BYTES {
            continue;
        }
        let line = format!(
            "- {}\n",
            serde_json::json!({"kind":"recent_user_event","session_event_id":event.id,"assertion_class":"user_authored","text":text})
        );
        if rendered.len() + line.len() <= MAX_RESTART_BYTES
            && counter.count(&format!("{rendered}{line}")) <= budget
        {
            rendered.push_str(&line);
            tail_ids.push(event.id);
            delivered_roots.push(event.id);
        }
    }
    if rendered.len() == header_bytes {
        return Ok((String::new(), None, status, vec![], vec![]));
    }
    delivered_roots.sort_unstable();
    delivered_roots.dedup();
    Ok((rendered, checkpoint_id, status, tail_ids, delivered_roots))
}
