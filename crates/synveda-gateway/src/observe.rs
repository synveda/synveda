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
//!
//! Since MEM-2 (ADR-0021) the redaction scan runs here, between
//! validation and the staging insert: every payload is scanned and
//! redacted *before* anything persists (seed §6), and the effective
//! pack's redaction config picks each event's disposition — admit,
//! quarantine (staged signal-less behind a review row), or deny (refused
//! per event, nothing persisted). The raw finding text survives in no
//! table, response, metric, or audit payload.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::ScanOutcome;
use synveda_policy::{Action, Resource};
use synveda_store::{observe, rls};
use synveda_types::{Error, ObserveEventId, ObserveKind, RedactionMode, Result};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, tenant_id};
use crate::telemetry::{OBSERVE_BATCHES_TOTAL, OBSERVE_EVENTS_TOTAL, REDACTION_FINDINGS_TOTAL};

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
    quarantined: usize,
    denied: usize,
    events: Vec<EventOutcome>,
}

#[derive(Serialize)]
struct EventOutcome {
    idempotency_key: String,
    /// The buffered event: freshly minted on acceptance, the original
    /// delivery's id on a duplicate — so a retried batch acks with the
    /// same ids as the delivery that won. Absent for denied events:
    /// nothing was persisted for them (ADR-0021 decision 4).
    #[serde(skip_serializing_if = "Option::is_none")]
    event_id: Option<ObserveEventId>,
    status: &'static str,
    /// The scan's finding summary for this event (rule, category,
    /// count — never matched text). Absent when the payload was clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    redactions: Option<serde_json::Value>,
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
        let authorized = authz::decide(&state, &input, Action::MemoryWrite, Resource::Scope(home))?;
        // The scan seam (MEM-2, ADR-0021 decision 1): every payload is
        // scanned and redacted before anything persists. The redaction
        // config comes off the effective pack for the write resource —
        // the same resolution the MemoryWrite decision just used
        // (MemoryWrite is not PolicyAssign, so no skip-self divergence).
        let config = state
            .pdp
            .effective(tenant_id, Resource::Scope(home), &input.context())
            .redaction;
        let payloads: Vec<serde_json::Value> = payload
            .events
            .iter()
            .map(|event| event.payload.clone())
            .collect();
        // CPU work, O(payload bytes), worst case 16 MiB: off the
        // reactor. The request span travels along so the scan spans nest
        // under it.
        let span = tracing::Span::current();
        let scans: Vec<ScanOutcome> = tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            payloads.into_iter().map(synveda_ingest::scan).collect()
        })
        .await
        .map_err(|err| Error::Internal {
            message: format!("redaction scan task failed: {err}"),
        })?;

        // Dispositions (ADR-0021 decision 4): denied events never reach
        // the store; the rest stage with their finding summary, the
        // quarantined ones signal-less.
        let mut outcomes: Vec<Option<EventOutcome>> = Vec::with_capacity(payload.events.len());
        outcomes.resize_with(payload.events.len(), || None);
        let mut store_events: Vec<observe::NewObserveEvent> = Vec::new();
        let mut store_slots: Vec<usize> = Vec::new();
        let mut rule_summary: std::collections::BTreeMap<&'static str, u64> =
            std::collections::BTreeMap::new();
        let mut denied = 0usize;
        for (slot, (event, scan)) in payload.events.iter().zip(scans).enumerate() {
            for finding in &scan.findings {
                metrics::counter!(
                    REDACTION_FINDINGS_TOTAL,
                    "rule" => finding.rule,
                    "category" => finding.category.as_str(),
                )
                .increment(finding.count as u64);
                *rule_summary.entry(finding.rule).or_default() += finding.count as u64;
            }
            let disposition = scan.disposition(&config);
            let redactions = if scan.findings.is_empty() {
                None
            } else {
                Some(
                    serde_json::to_value(&scan.findings).map_err(|err| Error::Internal {
                        message: format!("serialise finding summary: {err}"),
                    })?,
                )
            };
            if disposition == Some(RedactionMode::Deny) {
                denied += 1;
                outcomes[slot] = Some(EventOutcome {
                    idempotency_key: event.idempotency_key.clone(),
                    event_id: None,
                    status: "denied",
                    redactions,
                });
                continue;
            }
            store_events.push(observe::NewObserveEvent {
                idempotency_key: event.idempotency_key.clone(),
                kind: event.kind,
                payload: scan.payload,
                occurred_at: event.occurred_at,
                redactions: redactions.clone(),
                quarantine: disposition == Some(RedactionMode::Quarantine),
            });
            store_slots.push(slot);
            outcomes[slot] = Some(EventOutcome {
                idempotency_key: event.idempotency_key.clone(),
                event_id: None,
                status: "accepted",
                redactions,
            });
        }
        let admitted = if store_events.is_empty() {
            Vec::new()
        } else {
            observe::buffer_batch(
                &mut tx,
                tenant_id,
                identity.scope_id,
                identity.id,
                &payload.session_id,
                &store_events,
            )
            .await?
        };
        let mut staged_ids: Vec<ObserveEventId> = Vec::new();
        let mut accepted = 0usize;
        let mut duplicates = 0usize;
        let mut quarantined = 0usize;
        for (event, slot) in admitted.into_iter().zip(&store_slots) {
            let outcome = outcomes[*slot].as_mut().ok_or_else(|| Error::Internal {
                message: "observe outcome slot lost".to_owned(),
            })?;
            outcome.event_id = Some(event.id);
            if event.duplicate {
                duplicates += 1;
                outcome.status = "duplicate";
                // This delivery's copy was scanned but nothing of it was
                // stored; the winning delivery's record stands.
                outcome.redactions = None;
            } else {
                staged_ids.push(event.id);
                if event.quarantined {
                    quarantined += 1;
                    outcome.status = "quarantined";
                } else {
                    accepted += 1;
                }
            }
        }
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
                "quarantined": quarantined,
                "denied": denied,
                // Rule ids and counts only — never matched text
                // (ADR-0021 decision 1). Absent for a clean batch.
                "redactions": if rule_summary.is_empty() {
                    serde_json::Value::Null
                } else {
                    json!(rule_summary)
                },
                // UUIDv7 is time-ordered: the pair brackets the batch's
                // staging rows without writing every id into the chain
                // (ADR-0020 decision 5). Null when nothing staged — an
                // all-duplicates (or all-denied) batch is still an
                // operation, still one event.
                "first_event_id": staged_ids.first(),
                "last_event_id": staged_ids.last(),
            }),
        )
        .await?;
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit observe transaction: {err}"),
        })?;
        for (outcome, count) in [
            ("accepted", accepted),
            ("duplicate", duplicates),
            ("quarantined", quarantined),
            ("denied", denied),
        ] {
            metrics::counter!(OBSERVE_EVENTS_TOTAL, "outcome" => outcome).increment(count as u64);
        }
        let events: Vec<EventOutcome> = outcomes
            .into_iter()
            .map(|outcome| {
                outcome.ok_or_else(|| Error::Internal {
                    message: "observe outcome slot never filled".to_owned(),
                })
            })
            .collect::<Result<_>>()?;
        Ok((
            StatusCode::ACCEPTED,
            Json(ObserveResponse {
                session_id: payload.session_id,
                accepted,
                duplicates,
                quarantined,
                denied,
                events,
            }),
        ))
    }
    .await;
    respond(&state, "create", result).await
}
