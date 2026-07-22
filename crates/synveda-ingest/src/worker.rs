//! The extraction worker (MEM-3, ADR-0022): the observe queue's first
//! consumer. A polling transport (`spawn`/`run_once`) drives
//! Temporal-shaped stages — load the staged event, extract outside any
//! transaction, embed outside any transaction (MEM-4, ADR-0023), commit
//! under the archive-lock — so the enterprise profile can later host the
//! same stages under a workflow engine (decision 1).
//!
//! Exactly-once is the archive-lock (decision 2): `pgmq.archive` runs
//! inside the tenant write transaction before the record inserts, so a
//! commit means "records exist AND the signal is consumed" atomically,
//! and a redelivery race loses by archiving zero rows. Embed-or-fail
//! rides the same seam: each record inserts with its vector in one
//! statement, so "records exist" now means "embedded records exist" —
//! an embedding failure leaves the signal for redelivery, never a
//! vector-less record (ADR-0023 decisions 1 and 2).
//!
//! Context is explicit throughout — tenant from the signal, actor named
//! per event — never task-local (ADR-0008's worker rule).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource};
use synveda_store::observe::{ObserveMessage, QueuedSignal, StagedEvent};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{ScopeChainCache, identities, observe, policy_assignments, rls, role_bindings};
use synveda_types::{
    Error, HierarchyNode, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, Result,
    ScopeId, Sensitivity, TenantId,
};

use crate::embedding::{AnyEmbedder, Embedder};
use crate::extraction::{AnyExtractor, ExtractionInput, ExtractionOutcome, Extractor};

/// Counter: staged events the worker resolved, labelled
/// `outcome = ok | empty | denied | dead_letter | error | skipped`.
/// `skipped` covers archive-lock losses, missing staging rows, and
/// malformed queue messages — signals consumed without extraction.
pub const EXTRACTION_EVENTS_TOTAL: &str = "synveda_extraction_events_total";

/// Counter: derived records committed, labelled `class`.
pub const EXTRACTION_RECORDS_TOTAL: &str = "synveda_extraction_records_total";

/// Histogram: seconds from an event's admission (`received_at`) to its
/// extraction commit — the pipeline-lag evidence for seed §10's <60s SLO.
pub const EXTRACTION_LAG_SECONDS: &str = "synveda_extraction_lag_seconds";

/// Counter: extractor calls, labelled `method` and `outcome = ok | error`.
pub const EXTRACTOR_REQUESTS_TOTAL: &str = "synveda_extractor_requests_total";

/// Histogram: extractor call duration in seconds, labelled `method`.
pub const EXTRACTOR_REQUEST_SECONDS: &str = "synveda_extractor_request_seconds";

/// Counter: redaction findings in extractor *output* (ADR-0022
/// decision 7's re-scan) — nonzero means an extractor echoed or
/// fabricated secret-shaped content that never reached storage.
pub const EXTRACTION_RESCAN_FINDINGS_TOTAL: &str = "synveda_extraction_rescan_findings_total";

/// Counter: embedder calls, labelled `method` and `outcome = ok | error`
/// (MEM-4, ADR-0023 decision 7).
pub const EMBEDDER_REQUESTS_TOTAL: &str = "synveda_embedder_requests_total";

/// Histogram: embedder call duration in seconds, labelled `method`.
pub const EMBEDDER_REQUEST_SECONDS: &str = "synveda_embedder_request_seconds";

/// The audit actor component name for this pipeline.
const ACTOR_COMPONENT: &str = "extraction";

/// The worker's tuning knobs, parsed from `SYNVEDA_EXTRACTION_*` by the
/// gateway.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Idle poll interval.
    pub poll_interval: Duration,
    /// Signals per `pgmq.read` batch.
    pub batch: i32,
    /// Visibility timeout: how long a read signal stays invisible while
    /// this worker processes it. Must exceed the extractor timeout.
    pub vt_secs: i32,
    /// Dead-letter threshold: a signal read more than this many times is
    /// archived without an extraction attempt (ADR-0022 decision 6).
    pub max_reads: i32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            batch: 16,
            vt_secs: 60,
            max_reads: 5,
        }
    }
}

/// What the worker holds for its lifetime: the gateway's pool, PDP, and
/// scope-chain cache (shared, so hierarchy-move invalidations reach the
/// worker's authorization reads), plus the configured extractor.
#[derive(Clone)]
pub struct WorkerDeps {
    /// The shared connection pool.
    pub pool: PgPool,
    /// The embedded PDP; the worker re-decides every write (seed §2.2).
    pub pdp: Arc<Pdp>,
    /// The gateway's scope-chain cache — pass a clone of the gateway's
    /// `Arc`, never a fresh cache, or move invalidations are lost.
    pub chains: Arc<ScopeChainCache>,
    /// The configured extractor.
    pub extractor: AnyExtractor,
    /// The configured embedder: every record commits with its vector,
    /// embed-or-fail (MEM-4, ADR-0023).
    pub embedder: AnyEmbedder,
}

/// Spawns the worker loop: poll, drain while work exists, sleep. Abort
/// the handle on shutdown (the pack-refresher shape).
#[must_use]
pub fn spawn(deps: WorkerDeps, config: WorkerConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            loop {
                match run_once(&deps, &config).await {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(error = %error, "extraction pass failed; signals redeliver");
                        break;
                    }
                }
            }
        }
    })
}

/// One pass: read up to `config.batch` signals, process them grouped by
/// tenant, return how many messages were read. Public so tests and the
/// demo can drain the queue without the timing of a background loop.
#[tracing::instrument(name = "ingest.worker.run_once", skip_all, err(Display))]
pub async fn run_once(deps: &WorkerDeps, config: &WorkerConfig) -> Result<usize> {
    let mut conn = deps.pool.acquire().await.map_err(|err| Error::Storage {
        message: format!("acquire worker connection: {err}"),
    })?;
    let messages = observe::read_signals(&mut conn, config.vt_secs, config.batch).await?;
    let read = messages.len();
    if read == 0 {
        return Ok(0);
    }

    // Malformed messages (only a database-credentialed writer can mint
    // one) are archived defensively: they can never become processable
    // and must not wedge the queue. No tenant, so no chain event.
    let mut signals: Vec<QueuedSignal> = Vec::with_capacity(read);
    for message in messages {
        match message {
            ObserveMessage::Signal(signal) => signals.push(signal),
            ObserveMessage::Malformed { msg_id } => {
                tracing::warn!(msg.id = msg_id, "malformed observe signal; archiving");
                observe::archive_signal(&mut conn, msg_id).await?;
                metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "skipped").increment(1);
            }
        }
    }
    drop(conn);

    // Group by tenant, preserving read order within each group.
    let mut groups: Vec<(TenantId, Vec<QueuedSignal>)> = Vec::new();
    for signal in signals {
        match groups
            .iter_mut()
            .find(|(tenant, _)| *tenant == signal.tenant_id)
        {
            Some((_, group)) => group.push(signal),
            None => groups.push((signal.tenant_id, vec![signal])),
        }
    }
    for (tenant_id, group) in groups {
        if let Err(error) = process_group(deps, config, tenant_id, group).await {
            metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "error").increment(1);
            tracing::error!(
                tenant.id = %tenant_id,
                error = %error,
                "extraction group failed; its signals redeliver after the visibility timeout"
            );
        }
    }
    Ok(read)
}

/// One signal's fate after the load stage.
enum Loaded {
    /// A staged event ready for extraction.
    Work(Box<(QueuedSignal, StagedEvent)>),
    /// `read_ct` exhausted — archive + chain a failure, no extraction.
    DeadLetter(QueuedSignal),
    /// No staging row (disposal raced, or a bogus signal) — archive only.
    Missing(i64),
}

/// One extracted event waiting for the embed stage.
struct Extracted {
    msg_id: i64,
    read_ct: i32,
    input: ExtractionInput,
    received_at: DateTime<Utc>,
    outcome: ExtractionOutcome,
}

/// One candidate past the embed stage: the final rescanned content and
/// the vector computed over exactly that text (ADR-0023 decision 5).
struct EmbeddedCandidate {
    class: RecordClass,
    content: String,
    confidence: f64,
    sensitivity: Option<Sensitivity>,
    entities: Vec<String>,
    vector: Vec<f32>,
}

/// One event past the embed stage, waiting for the write transaction.
struct Embedded {
    msg_id: i64,
    input: ExtractionInput,
    received_at: DateTime<Utc>,
    method: String,
    model_version: String,
    candidates: Vec<EmbeddedCandidate>,
    rescan_findings: usize,
}

/// Processes one tenant's signals: load (read tx) → extract (no tx) →
/// commit records + archives + audit (one write tx, ADR-0022).
async fn process_group(
    deps: &WorkerDeps,
    config: &WorkerConfig,
    tenant_id: TenantId,
    group: Vec<QueuedSignal>,
) -> Result<()> {
    // Load stage: a short read transaction; committed before any
    // extractor runs so no transaction spans a network call.
    let mut loaded: Vec<Loaded> = Vec::with_capacity(group.len());
    {
        let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
        for signal in group {
            if signal.read_ct > config.max_reads {
                loaded.push(Loaded::DeadLetter(signal));
                continue;
            }
            match observe::load_event(&mut tx, tenant_id, signal.event_id).await? {
                Some(event) => loaded.push(Loaded::Work(Box::new((signal, event)))),
                None => {
                    tracing::warn!(
                        tenant.id = %tenant_id,
                        event.id = %signal.event_id,
                        "signal names no staging row; archiving"
                    );
                    loaded.push(Loaded::Missing(signal.msg_id));
                }
            }
        }
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit extraction read transaction: {err}"),
        })?;
    }

    // Extract stage: outside any transaction. A failure leaves the
    // signal un-archived — the visibility timeout redelivers and
    // read_ct climbs toward the dead-letter threshold.
    let mut extracted: Vec<Extracted> = Vec::new();
    let mut dead_letters: Vec<QueuedSignal> = Vec::new();
    let mut missing: Vec<i64> = Vec::new();
    for item in loaded {
        match item {
            Loaded::DeadLetter(signal) => dead_letters.push(signal),
            Loaded::Missing(msg_id) => missing.push(msg_id),
            Loaded::Work(boxed) => {
                let (signal, event) = *boxed;
                let received_at = event.received_at;
                let input = ExtractionInput {
                    event_id: event.id,
                    tenant_id,
                    scope_id: event.scope_id,
                    owner_id: event.owner_id,
                    session_id: event.session_id,
                    kind: event.kind,
                    payload: event.payload,
                    occurred_at: event.occurred_at,
                    redactions: event.redactions,
                };
                let started = std::time::Instant::now();
                let result = deps.extractor.extract(&input).await;
                let method = deps.extractor.method();
                metrics::histogram!(EXTRACTOR_REQUEST_SECONDS, "method" => method)
                    .record(started.elapsed().as_secs_f64());
                match result {
                    Ok(outcome) => {
                        metrics::counter!(
                            EXTRACTOR_REQUESTS_TOTAL,
                            "method" => method, "outcome" => "ok"
                        )
                        .increment(1);
                        extracted.push(Extracted {
                            msg_id: signal.msg_id,
                            read_ct: signal.read_ct,
                            input,
                            received_at,
                            outcome,
                        });
                    }
                    Err(error) => {
                        metrics::counter!(
                            EXTRACTOR_REQUESTS_TOTAL,
                            "method" => method, "outcome" => "error"
                        )
                        .increment(1);
                        metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "error")
                            .increment(1);
                        tracing::warn!(
                            tenant.id = %tenant_id,
                            event.id = %input.event_id,
                            read.count = signal.read_ct,
                            error = %error,
                            "extraction failed; signal redelivers"
                        );
                    }
                }
            }
        }
    }
    // Embed stage (MEM-4, ADR-0023 decisions 1 and 5): outside any
    // transaction, one call per event, over the final rescanned text.
    // A failure leaves the signal un-archived — the same redelivery
    // flow as an extractor failure — and costs no other event its
    // commit: this is the AC's partial-batch-failure semantics.
    let mut embedded: Vec<Embedded> = Vec::with_capacity(extracted.len());
    for item in extracted {
        let event_id = item.input.event_id;
        let read_ct = item.read_ct;
        match embed_event(deps, item).await {
            Ok(item) => embedded.push(item),
            Err(error) => {
                metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "error").increment(1);
                tracing::warn!(
                    tenant.id = %tenant_id,
                    event.id = %event_id,
                    read.count = read_ct,
                    error = %error,
                    "embedding failed; signal redelivers"
                );
            }
        }
    }

    if embedded.is_empty() && dead_letters.is_empty() && missing.is_empty() {
        return Ok(());
    }

    commit_group(deps, tenant_id, embedded, dead_letters, missing).await
}

/// Rescans and embeds one extracted event (ADR-0023 decision 5: scan →
/// embed → commit, so the vector is computed over exactly the persisted
/// text — a secret redacted from content never reaches vector space).
#[tracing::instrument(
    name = "ingest.worker.embed",
    skip_all,
    fields(event.id = %item.input.event_id),
    err(Display)
)]
async fn embed_event(deps: &WorkerDeps, item: Extracted) -> Result<Embedded> {
    let Extracted {
        msg_id,
        read_ct: _,
        input,
        received_at,
        outcome,
    } = item;
    let mut rescan_findings = 0usize;
    let mut candidates: Vec<EmbeddedCandidate> = Vec::with_capacity(outcome.candidates.len());
    for candidate in outcome.candidates {
        // Decision 7 of ADR-0022: extractor output re-enters the
        // scanner, so an echoed or fabricated live-format secret never
        // persists — and now (ADR-0023) is never embedded either.
        let (content, findings) = rescan(candidate.content);
        rescan_findings += findings;
        candidates.push(EmbeddedCandidate {
            class: candidate.class,
            content,
            confidence: candidate.confidence,
            sensitivity: candidate.sensitivity,
            entities: candidate.entities,
            vector: Vec::new(),
        });
    }
    if rescan_findings > 0 {
        metrics::counter!(EXTRACTION_RESCAN_FINDINGS_TOTAL).increment(rescan_findings as u64);
    }
    if !candidates.is_empty() {
        let contents: Vec<String> = candidates
            .iter()
            .map(|candidate| candidate.content.clone())
            .collect();
        let started = std::time::Instant::now();
        let result = deps.embedder.embed(&contents).await;
        let method = deps.embedder.method();
        metrics::histogram!(EMBEDDER_REQUEST_SECONDS, "method" => method)
            .record(started.elapsed().as_secs_f64());
        let outcome_label = if result.is_ok() { "ok" } else { "error" };
        metrics::counter!(
            EMBEDDER_REQUESTS_TOTAL,
            "method" => method, "outcome" => outcome_label
        )
        .increment(1);
        let vectors = result?;
        // The seam's contract (one vector per input, in order) is
        // checked by the implementations; a zip would silently
        // misattribute on breach, so re-check before pairing.
        if vectors.len() != candidates.len() {
            return Err(Error::Dependency {
                service: method.to_owned(),
                message: format!(
                    "expected {} vectors, got {}",
                    candidates.len(),
                    vectors.len()
                ),
            });
        }
        for (candidate, vector) in candidates.iter_mut().zip(vectors) {
            candidate.vector = vector;
        }
    }
    Ok(Embedded {
        msg_id,
        input,
        received_at,
        method: outcome.method,
        model_version: outcome.model_version,
        candidates,
        rescan_findings,
    })
}

/// One owner's authorization verdict for this commit group.
enum OwnerAuth {
    /// The write may land at the owner's current home.
    Allowed {
        home: ScopeId,
        pack: String,
        roles: Vec<String>,
    },
    /// Fail closed: the reason names policies or invariants, never
    /// content.
    Denied { reason: String },
}

/// The write transaction (ADR-0022 decisions 2, 4, 5; ADR-0023
/// decision 2): archive-lock each signal, re-authorize each owner at
/// current facts, insert each record atomically with its embedding, and
/// chain the group's audit events — all atomically.
async fn commit_group(
    deps: &WorkerDeps,
    tenant_id: TenantId,
    embedded: Vec<Embedded>,
    dead_letters: Vec<QueuedSignal>,
    missing: Vec<i64>,
) -> Result<()> {
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;

    for msg_id in missing {
        observe::archive_signal(&mut tx, msg_id).await?;
        metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "skipped").increment(1);
    }

    // Dead-letters: archived (the queue must drain) and chained with
    // outcome failure; the staging row stays re-drivable provenance.
    let mut dead_lettered: Vec<serde_json::Value> = Vec::new();
    for signal in dead_letters {
        if observe::archive_signal(&mut tx, signal.msg_id).await? {
            dead_lettered.push(json!({
                "event_id": signal.event_id,
                "read_count": signal.read_ct,
            }));
            metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "dead_letter").increment(1);
        }
    }

    // Authorize each distinct owner once, at current facts (decision 4).
    let mut auth_by_owner: HashMap<IdentityId, OwnerAuth> = HashMap::new();
    for item in &embedded {
        if let std::collections::hash_map::Entry::Vacant(entry) =
            auth_by_owner.entry(item.input.owner_id)
        {
            entry.insert(authorize_owner(deps, &mut tx, tenant_id, item.input.owner_id).await?);
        }
    }

    let now = Utc::now();
    let missing_auth = OwnerAuth::Denied {
        reason: "owner authorization missing".to_owned(),
    };
    let mut committed: Vec<serde_json::Value> = Vec::new();
    let mut denied: HashMap<IdentityId, Vec<serde_json::Value>> = HashMap::new();
    let mut methods: Vec<(String, String)> = Vec::new();
    let mut rescan_findings = 0usize;
    let mut lags: Vec<f64> = Vec::new();
    for item in embedded {
        // The archive-lock: zero rows means a racing consumer already
        // committed this signal's work — skip without inserting.
        if !observe::archive_signal(&mut tx, item.msg_id).await? {
            metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "skipped").increment(1);
            continue;
        }
        let auth = auth_by_owner
            .get(&item.input.owner_id)
            .unwrap_or(&missing_auth);
        let OwnerAuth::Allowed { home, pack, roles } = auth else {
            denied
                .entry(item.input.owner_id)
                .or_default()
                .push(json!(item.input.event_id));
            metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "denied").increment(1);
            continue;
        };
        let mut classes: Vec<&'static str> = Vec::new();
        for candidate in item.candidates {
            let sensitivity = candidate
                .sensitivity
                .unwrap_or(Sensitivity::Internal)
                .max(Sensitivity::Internal);
            let state = RecordState {
                scope_id: *home,
                owner_id: item.input.owner_id,
                kind: RecordKind::Derived,
                class: candidate.class,
                content: candidate.content,
                sensitivity,
                provenance: json!({
                    "event_id": item.input.event_id,
                    "session_id": item.input.session_id,
                    "method": item.method,
                    "model_version": item.model_version,
                    "confidence": candidate.confidence,
                    "entities": candidate.entities,
                    "redactions": item.input.redactions,
                    "extracted_at": now.to_rfc3339(),
                }),
                valid_from: item.input.occurred_at,
                valid_to: None,
            };
            // Record and vector in one statement (ADR-0023 decision 2):
            // this commit cannot produce an embedding-less record.
            let embedding = RecordEmbedding {
                model: deps.embedder.model().to_owned(),
                vector: candidate.vector,
            };
            records::insert(&mut *tx, RecordId::new(), tenant_id, &state, &embedding).await?;
            metrics::counter!(EXTRACTION_RECORDS_TOTAL, "class" => candidate.class.as_str())
                .increment(1);
            classes.push(candidate.class.as_str());
        }
        let outcome_label = if classes.is_empty() { "empty" } else { "ok" };
        metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => outcome_label).increment(1);
        lags.push((now - item.received_at).num_milliseconds() as f64 / 1000.0);
        methods.push((item.method, item.model_version));
        rescan_findings += item.rescan_findings;
        committed.push(json!({
            "event_id": item.input.event_id,
            "owner": item.input.owner_id,
            "pack": pack,
            "roles": roles,
            "records": classes.len(),
            "classes": classes,
        }));
    }

    // Denials chain standalone decision events (ADR-0019 decision 4).
    for (owner_id, event_ids) in &denied {
        let reason = match auth_by_owner.get(owner_id) {
            Some(OwnerAuth::Denied { reason }) => reason.clone(),
            _ => "unknown".to_owned(),
        };
        append_event(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            format!("tenant {tenant_id}"),
            Outcome::Deny,
            json!({
                "op": "extraction",
                "action": Action::MemoryWrite.as_str(),
                "owner": owner_id,
                "event_ids": event_ids,
                "reason": reason,
            }),
        )
        .await?;
    }
    // One aggregated event per commit group (ADR-0022 decision 5) —
    // never one row per record. Confidence is a float and stays in
    // record provenance: audit canonicalisation rejects floats.
    if !committed.is_empty() {
        let (method, model_version) = methods.swap_remove(0);
        append_event(
            &mut tx,
            tenant_id,
            AuditAction::MemoryExtracted,
            format!("tenant {tenant_id}"),
            Outcome::Success,
            json!({
                "events": committed,
                "method": method,
                "model_version": model_version,
                "embedder": deps.embedder.method(),
                "embedding_model": deps.embedder.model(),
                "rescan_findings": rescan_findings,
            }),
        )
        .await?;
    }
    if !dead_lettered.is_empty() {
        append_event(
            &mut tx,
            tenant_id,
            AuditAction::MemoryExtracted,
            format!("tenant {tenant_id}"),
            Outcome::Failure,
            json!({
                "events": dead_lettered,
                "reason": "retries exhausted",
            }),
        )
        .await?;
    }

    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit extraction write transaction: {err}"),
    })?;
    for lag in lags {
        metrics::histogram!(EXTRACTION_LAG_SECONDS).record(lag);
    }
    Ok(())
}

/// Re-decides `MemoryWrite` for one owner at its *current* home under
/// its *current* quarantine state — the same reads the gateway's gather
/// seam performs, with explicit context instead of task-locals.
async fn authorize_owner(
    deps: &WorkerDeps,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    owner_id: IdentityId,
) -> Result<OwnerAuth> {
    let Some(identity) = identities::by_id(&mut *tx, tenant_id, owner_id).await? else {
        return Ok(OwnerAuth::Denied {
            reason: "owner identity no longer exists".to_owned(),
        });
    };
    let mut quarantined = identity.quarantined;
    let chain: Arc<[HierarchyNode]> = deps
        .chains
        .resolve(&mut *tx, tenant_id, identity.scope_id)
        .await?
        .unwrap_or_else(|| Vec::new().into());
    // The confinement scope (ADR-0018 decision 4): a service identity's
    // anchor is the node above its personal leaf; unresolvable means
    // quarantined, never unconfined.
    let token_scope = if identity.kind == IdentityKind::Service {
        let anchor = chain.get(1).map(|node| node.id);
        if anchor.is_none() {
            quarantined = true;
        }
        anchor
    } else {
        None
    };
    let principal = Principal {
        tenant_id,
        subject: identity.subject.clone(),
        quarantined,
        scope_id: Some(identity.scope_id),
        token_scope,
    };
    let chain_ids: Vec<_> = chain.iter().map(|node| node.id).collect();
    let assignments = if chain_ids.is_empty() {
        Vec::new()
    } else {
        policy_assignments::for_scopes(&mut *tx, tenant_id, &chain_ids).await?
    };
    let default_pack = policy_assignments::default_pack(&mut *tx, tenant_id).await?;
    let bindings =
        role_bindings::for_subject_on_scopes(&mut *tx, tenant_id, &identity.subject, &chain_ids)
            .await?;
    let context = AuthzContext {
        scopes: &chain,
        principal_scopes: &chain,
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        role_bindings: &bindings,
        grant: None,
    };
    let decision = deps.pdp.authorize(
        &principal,
        Action::MemoryWrite,
        Resource::Scope(identity.scope_id),
        &context,
    )?;
    if decision.allowed {
        let mut roles: Vec<String> = bindings
            .iter()
            .map(|binding| binding.role.as_str().to_owned())
            .collect();
        roles.sort_unstable();
        roles.dedup();
        Ok(OwnerAuth::Allowed {
            home: identity.scope_id,
            pack: format!("{}@{}", decision.pack_name, decision.pack_version),
            roles,
        })
    } else {
        let determining = if decision.determining.is_empty() {
            "no policy permitted it".to_owned()
        } else {
            decision.determining.join(", ")
        };
        Ok(OwnerAuth::Denied {
            reason: format!(
                "pack {}@{} denied ({determining})",
                decision.pack_name, decision.pack_version
            ),
        })
    }
}

/// Appends one chain event with the pipeline actor. Runs on the group's
/// write transaction: the chain-head lock is the last lock it takes
/// (ADR-0019 decision 1). Worker events carry no trace id — the gateway
/// owns the OTel exporter (ADR-0007).
async fn append_event(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    action: AuditAction,
    resource: String,
    outcome: Outcome,
    payload: serde_json::Value,
) -> Result<()> {
    synveda_audit::append(
        tx,
        tenant_id,
        &AuditEvent {
            occurred_at: Utc::now(),
            actor: Actor::system(ACTOR_COMPONENT),
            action,
            resource,
            outcome,
            payload,
            trace_id: None,
        },
    )
    .await?;
    Ok(())
}

/// Runs extracted content back through the admission scanner (ADR-0022
/// decision 7). Placeholders already in the text pass through untouched;
/// live-format secrets come back redacted, and the count is the metric's
/// evidence that the hole is real.
fn rescan(content: String) -> (String, usize) {
    let outcome = crate::scan(serde_json::Value::String(content));
    let findings = outcome
        .findings
        .iter()
        .map(|finding| finding.count)
        .sum::<usize>();
    let content = match outcome.payload {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    };
    (content, findings)
}
