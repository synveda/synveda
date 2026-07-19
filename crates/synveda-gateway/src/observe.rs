//! The observe ingestion API (MEM-1, ADR-0020): the data plane's write
//! primitive (seed §3). `POST /v1/observe` admits a batch of session
//! events into the RLS-staged buffer and enqueues content-free work
//! signals for the pipeline (MEM-2/3), acking without any extraction,
//! embedding, or LLM work — enqueue-only, ack <20ms (seed §10).
//!
//! The write lands at the caller's personal (home) scope, and only there:
//! the endpoint takes no scope parameter; placement decides (ADR-0020
//! decision 4). The PDP gates the batch with `MemoryWrite` — the
//! role-free own-home floor every placed principal holds (zero-config,
//! seed §2.1). Idempotency is the buffer's: duplicate delivery is
//! reported per event and acked as success, never re-enqueued (ADR-0020
//! decision 2). Each admitted batch chains one `memory.observed` audit
//! event in the ingest transaction (ADR-0019 decision 4).

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{observe, rls};
use synveda_types::{Error, ObserveEventId, ObserveKind, Result};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::hierarchy::{body, tenant_id};
use crate::telemetry::{OBSERVE_BATCHES_TOTAL, OBSERVE_EVENTS_TOTAL};

/// Batch caps (ADR-0020 decision 5). A batch over the cap is a 422 with
/// the violation named; nothing partial persists.
pub(crate) const MAX_EVENTS_PER_BATCH: usize = 256;

/// Per-event payload cap, measured over the serialised JSON.
pub(crate) const MAX_EVENT_PAYLOAD_BYTES: usize = 64 * 1024;

/// Cap for `session_id` and `idempotency_key`; mirrors the staging
/// table's CHECK constraints.
const MAX_TEXT_FIELD_CHARS: usize = 200;

/// The route's request body limit: the worst-case batch
/// (256 × 64 KiB payloads) plus envelope headroom.
pub(crate) const BODY_LIMIT_BYTES: usize = 20 * 1024 * 1024;

/// Counts the operation and renders the result — the same funnel shape as
/// every governed plane (`ok` / `rejected` / `error`); error-path audit
/// events chain here (AUD-1, ADR-0019 decision 5).
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(OBSERVE_BATCHES_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ObserveBody {
    /// Opaque harness session identifier; groups this batch's events.
    session_id: String,
    events: Vec<ObserveEventBody>,
}

#[derive(Deserialize)]
struct ObserveEventBody {
    /// Client-minted admission key (ADR-0020 decision 2): only the sender
    /// can distinguish a retry from a new event with identical content.
    idempotency_key: String,
    kind: ObserveKind,
    payload: serde_json::Value,
    /// Client-asserted event time (RFC 3339).
    occurred_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct ObserveResponse {
    session_id: String,
    accepted: usize,
    duplicates: usize,
    events: Vec<EventOutcome>,
}

#[derive(Serialize)]
struct EventOutcome {
    idempotency_key: String,
    /// The buffered event: freshly minted on acceptance, the original
    /// delivery's id on a duplicate — so a retried batch acks with the
    /// same ids as the delivery that won.
    event_id: ObserveEventId,
    status: &'static str,
}

/// All-or-nothing validation (ADR-0020 decision 5): a malformed batch is
/// rejected whole, naming the first violation; nothing partial persists.
fn validate(payload: &ObserveBody) -> Result<()> {
    let invalid = |message: String| Err(Error::Invalid { message });
    if payload.events.is_empty() {
        return invalid("a batch must carry at least one event".to_owned());
    }
    if payload.events.len() > MAX_EVENTS_PER_BATCH {
        return invalid(format!(
            "batch carries {} events; the cap is {MAX_EVENTS_PER_BATCH}",
            payload.events.len()
        ));
    }
    text_field("session_id", &payload.session_id)?;
    for (index, event) in payload.events.iter().enumerate() {
        text_field(
            &format!("events[{index}].idempotency_key"),
            &event.idempotency_key,
        )?;
        let size = serde_json::to_vec(&event.payload)
            .map_err(|err| Error::Internal {
                message: format!("re-serialising a parsed payload failed: {err}"),
            })?
            .len();
        if size > MAX_EVENT_PAYLOAD_BYTES {
            return invalid(format!(
                "events[{index}].payload is {size} bytes; the cap is {MAX_EVENT_PAYLOAD_BYTES}"
            ));
        }
    }
    Ok(())
}

fn text_field(field: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.chars().count() > MAX_TEXT_FIELD_CHARS {
        return Err(Error::Invalid {
            message: format!("{field} must be 1..={MAX_TEXT_FIELD_CHARS} characters"),
        });
    }
    Ok(())
}

/// `POST /v1/observe` — admit a batch of session events (202: the ack
/// promises durable admission, not processing; the pipeline runs async).
pub(crate) async fn create(
    State(state): State<AppState>,
    payload: std::result::Result<Json<ObserveBody>, JsonRejection>,
) -> Response {
    let result = async {
        let payload = body(payload)?;
        validate(&payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // The write resource is the caller's own placement leaf — there is
        // no ownership probe to 404 on, and the placement chain doubles as
        // the resource chain (one identity read, ADR-0020 decision 4). A
        // subject with no identity row has no home scope, so no decidable
        // resource exists: refused at the seam, fail closed, chained as a
        // deny like any policy denial (quarantined subjects DO have a home
        // and get the base layer's quarantine forbid through the PDP
        // proper).
        let input = authz::gather_at_home(&state, &mut tx).await?;
        let Some(identity) = input.identity.clone() else {
            return Err(Error::PolicyDenied {
                action: Action::MemoryWrite.as_str().to_owned(),
                resource: "scope none".to_owned(),
                reason: "principal has no placement scope (fail closed)".to_owned(),
            });
        };
        let home = identity.scope_id;
        let authorized = authz::decide(
            &state,
            &input,
            Action::MemoryWrite,
            Resource::Scope(home),
            None,
        )?;
        let events: Vec<observe::NewObserveEvent> = payload
            .events
            .iter()
            .map(|event| observe::NewObserveEvent {
                idempotency_key: event.idempotency_key.clone(),
                kind: event.kind,
                payload: event.payload.clone(),
                occurred_at: event.occurred_at,
            })
            .collect();
        let admitted = observe::buffer_batch(
            &mut tx,
            tenant_id,
            identity.scope_id,
            identity.id,
            &payload.session_id,
            &events,
        )
        .await?;
        let accepted_ids: Vec<ObserveEventId> = admitted
            .iter()
            .filter(|event| !event.duplicate)
            .map(|event| event.id)
            .collect();
        let accepted = accepted_ids.len();
        let duplicates = admitted.len() - accepted;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::MemoryObserved,
            Resource::Scope(home).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::MemoryWrite, &authorized),
                "session_id": payload.session_id,
                "accepted": accepted,
                "duplicates": duplicates,
                // UUIDv7 is time-ordered: the pair brackets the batch's
                // staging rows without writing every id into the chain
                // (ADR-0020 decision 5). Null when the batch was all
                // duplicates — still an operation, still one event.
                "first_event_id": accepted_ids.first(),
                "last_event_id": accepted_ids.last(),
            }),
        )
        .await?;
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit observe transaction: {err}"),
        })?;
        metrics::counter!(OBSERVE_EVENTS_TOTAL, "outcome" => "accepted").increment(accepted as u64);
        metrics::counter!(OBSERVE_EVENTS_TOTAL, "outcome" => "duplicate")
            .increment(duplicates as u64);
        let events = admitted
            .into_iter()
            .map(|event| EventOutcome {
                idempotency_key: event.idempotency_key,
                event_id: event.id,
                status: if event.duplicate {
                    "duplicate"
                } else {
                    "accepted"
                },
            })
            .collect();
        Ok((
            StatusCode::ACCEPTED,
            Json(ObserveResponse {
                session_id: payload.session_id,
                accepted,
                duplicates,
                events,
            }),
        ))
    }
    .await;
    respond(&state, "create", result).await
}
