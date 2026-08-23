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
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource, ScopeNode};
use synveda_store::dedup as store_dedup;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::sessions::{QueuedSignal, SignalMessage, StagedEvent};
use synveda_store::{anchors, identities, policy_assignments, rls, sessions};
use synveda_types::{
    Channel, DedupConfig, Error, IdentityId, IdentityKind, IdentityStatus, RecordClass, RecordId,
    RecordKind, Result, ScopeId, Sensitivity, TenantId, permille,
};
use synveda_vedaflow::hash::ObjectHash;
use synveda_vedaflow::{
    self as vedaflow, MemoryAsset, PolicySnapshot, Signer, read_memory_members,
};

use crate::chain::scope_chain;
use crate::dedup::{DEDUP_CANDIDATES, DEDUP_DECISIONS_TOTAL, DEDUP_SECONDS};
use crate::embedding::{AnyEmbedder, Embedder};
use crate::extraction::{AnyExtractor, ExtractionInput, ExtractionOutcome, Extractor};
use crate::linking::{self, LinkedRecord};

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
    let messages = sessions::read_signals(&mut conn, config.vt_secs, config.batch).await?;
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
            SignalMessage::Signal(signal) => signals.push(signal),
            SignalMessage::Malformed { msg_id } => {
                tracing::warn!(msg.id = msg_id, "malformed observe signal; archiving");
                sessions::archive_signal(&mut conn, msg_id).await?;
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
            match sessions::staged_event(&mut tx, tenant_id, signal.event_id).await? {
                Some(event) => loaded.push(Loaded::Work(Box::new((signal, event)))),
                None => {
                    tracing::warn!(
                        tenant.id = %tenant_id,
                        event.id = %signal.event_id,
                        "signal names no event row; archiving"
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
                    session_id: event.session_id,
                    principal_id: event.principal_id,
                    event_type: event.event_type,
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
        // A mention has to be true of the text this record will actually
        // hold (GRPH-2, ADR-0044 decision 9). Only a candidate the rescan
        // *changed* can carry one that is not — an echoed secret the
        // admission scan missed — so the check runs exactly there, and a
        // candidate the scanner left alone keeps every mention the
        // extractor found, including the normalised forms an LLM returns
        // that never appear verbatim.
        let entities = if findings == 0 {
            candidate.entities
        } else {
            candidate
                .entities
                .into_iter()
                .filter(|entity| content.contains(entity.as_str()))
                .collect()
        };
        candidates.push(EmbeddedCandidate {
            class: candidate.class,
            content,
            confidence: candidate.confidence,
            sensitivity: candidate.sensitivity,
            entities,
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

/// The judge that decided a supersession, recorded on every edge row and in
/// every audit payload. A model-backed judge takes the same field (ADR-0039
/// decision 6), which is why the column is a name rather than a flag.
const JUDGE_METHOD: &str = "deterministic";

/// What identifies one authorization on this plane (CPR-12, ADR-0078
/// decision 3): the token subject that opened a run, and the governed scope
/// the run was decided at.
///
/// It was an `IdentityId` alone until the observe cutover, because every write
/// landed at its submitter's own home and the two were the same grouping. They
/// are not any more: one person's runs in three projects are three
/// authorizations at three scopes, and collapsing them to the person would let
/// a permit in one project write a memory into another.
type RunKey = (String, ScopeId);

/// One run's authorization verdict for this commit group.
enum RunAuth {
    /// The write may land at the scope the run was decided at.
    Allowed {
        /// Where the memory lands — the *session's* scope, not the
        /// submitter's home.
        scope: ScopeId,
        /// The identity behind the run's token subject. Records carry an
        /// owner, so a subject with no identity row is denied rather than
        /// guessed at.
        owner_id: IdentityId,
        pack_name: String,
        pack_version: i64,
        roles: Vec<String>,
        /// The dedup configuration of the pack that governs this write
        /// (MEM-5, ADR-0039 decision 12) — resolved from the same
        /// effective-pack walk that decided it.
        dedup: DedupConfig,
    },
    /// Fail closed: the reason names policies or invariants, never
    /// content.
    Denied { reason: String },
}

/// The authorization key of one extraction input.
fn run_key(input: &ExtractionInput) -> RunKey {
    (input.principal_id.clone(), input.scope_id)
}

/// What one run's records contribute to its scope's derived channel (FLOW-2,
/// ADR-0031 decision 13).
///
/// Keyed by [`RunKey`] rather than by owner: since the observe cutover a
/// record lands at the scope its *run* was decided at, so one owner can
/// contribute to several scopes in one group and each scope's commit needs its
/// own author. Blame and lineage (tech plan §2.5) run through this field.
struct DerivedBatch {
    scope: ScopeId,
    /// The identity that authors this scope's commit. Carried on the batch
    /// rather than derived from the key, because the key holds a token subject
    /// and a commit is authored by an identity.
    owner: IdentityId,
    pack_name: String,
    pack_version: i64,
    /// `(record id, content address)` per record inserted this group.
    members: Vec<(String, ObjectHash)>,
    events: usize,
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
        sessions::archive_signal(&mut tx, msg_id).await?;
        metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "skipped").increment(1);
    }

    // Dead-letters: archived (the queue must drain) and chained with
    // outcome failure; the staging row stays re-drivable provenance.
    let mut dead_lettered: Vec<serde_json::Value> = Vec::new();
    for signal in dead_letters {
        if sessions::archive_signal(&mut tx, signal.msg_id).await? {
            dead_lettered.push(json!({
                "event_id": signal.event_id,
                "read_count": signal.read_ct,
            }));
            metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "dead_letter").increment(1);
        }
    }

    // Authorize each distinct run once, at current facts (decision 4).
    let mut auth_by_run: HashMap<RunKey, RunAuth> = HashMap::new();
    for item in &embedded {
        let key = run_key(&item.input);
        if let std::collections::hash_map::Entry::Vacant(entry) = auth_by_run.entry(key) {
            let (subject, scope) = entry.key().clone();
            entry.insert(authorize_run(deps, &mut tx, tenant_id, &subject, scope).await?);
        }
    }

    let now = Utc::now();
    let missing_auth = RunAuth::Denied {
        reason: "run authorization missing".to_owned(),
    };
    let mut committed: Vec<serde_json::Value> = Vec::new();
    let mut denied: HashMap<RunKey, Vec<serde_json::Value>> = HashMap::new();
    let mut methods: Vec<(String, String)> = Vec::new();
    let mut rescan_findings = 0usize;
    let mut lags: Vec<f64> = Vec::new();
    let mut batches: HashMap<RunKey, DerivedBatch> = HashMap::new();
    // What each home scope publishes, read once per scope: the governance
    // boundary the judge needs (ADR-0039 decision 9), and the same indexed
    // read composition makes.
    let mut published_at: HashMap<ScopeId, HashMap<RecordId, ObjectHash>> = HashMap::new();
    // Windows this group closed, and the contradictions it declined to act
    // on — one chained `memory.superseded` event for the group.
    let mut superseded: Vec<serde_json::Value> = Vec::new();
    let mut refused_published: Vec<serde_json::Value> = Vec::new();
    // What the graph-linking stage will resolve, collected as records are
    // inserted and linked once for the whole group (GRPH-2, ADR-0044
    // decision 1) — so a name mentioned by three of this group's records
    // is one vertex, interned once.
    let mut linkable: Vec<LinkedRecord> = Vec::new();
    // Valid-time order, so "which of these two statements is the newer
    // assertion" is answered the same way whatever order the queue
    // delivered them in. The archive-lock is order-independent.
    let mut embedded = embedded;
    embedded.sort_by_key(|item| (item.input.occurred_at, item.input.event_id));
    for item in embedded {
        // The archive-lock: zero rows means a racing consumer already
        // committed this signal's work — skip without inserting.
        if !sessions::archive_signal(&mut tx, item.msg_id).await? {
            metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "skipped").increment(1);
            continue;
        }
        let key = run_key(&item.input);
        let auth = auth_by_run.get(&key).unwrap_or(&missing_auth);
        let RunAuth::Allowed {
            scope: home,
            owner_id,
            pack_name,
            pack_version,
            roles,
            dedup: dedup_config,
        } = auth
        else {
            denied
                .entry(key)
                .or_default()
                .push(json!(item.input.event_id));
            metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => "denied").increment(1);
            continue;
        };
        let owner_id = *owner_id;
        // The scope's published set, read once and reused for every
        // candidate: a contradiction against reviewed material is refused,
        // and a restatement of it merges into the reviewed copy rather than
        // making a second unreviewed one (ADR-0039 decisions 9 and 10).
        if dedup_config.mode.merges() && !published_at.contains_key(home) {
            let channels =
                read_memory_members(&mut tx, tenant_id, &[*home], Channel::Published).await?;
            let members = channels
                .into_iter()
                .find(|channel| channel.scope_id == *home)
                .map(|channel| channel.members)
                .unwrap_or_default();
            published_at.insert(*home, members);
        }
        let published = published_at.get(home).cloned().unwrap_or_default();

        let batch = batches.entry(key).or_insert_with(|| DerivedBatch {
            scope: *home,
            owner: owner_id,
            pack_name: pack_name.clone(),
            pack_version: *pack_version,
            members: Vec::new(),
            events: 0,
        });
        batch.events += 1;
        let mut classes: Vec<&'static str> = Vec::new();
        let mut merged: Vec<serde_json::Value> = Vec::new();
        for candidate in item.candidates {
            // Floored at the working tier because auto-derived content is
            // never `public` (ADR-0022 decision 7), and — since AUTHZ-5 —
            // bounded above at `confidential` (ADR-0038 decision 8).
            // `restricted` is defined by the invariant approval floor as the
            // tier carrying a compliance signature, and an uncalibrated,
            // self-reported model judgement cannot manufacture one: a model
            // that says `restricted` gets `confidential`, which is a real
            // tier with real consequences and no forged provenance. The top
            // tier arrives only through a reviewed reclassification.
            let sensitivity = candidate
                .sensitivity
                .unwrap_or(Sensitivity::WORKING)
                .clamp(Sensitivity::WORKING, Sensitivity::MAX_DERIVED);
            // Dedup & conflict detection (MEM-5, ADR-0039 decision 1): in
            // this transaction, before the insert, so a record and the
            // window it closes commit together — and so a candidate sees
            // the ones this same group already inserted.
            let judgement = judge_candidate(
                &mut tx,
                deps,
                dedup_config,
                &JudgeInput {
                    tenant_id,
                    scope_id: *home,
                    owner_id,
                    class: candidate.class,
                    content: &candidate.content,
                    vector: &candidate.vector,
                    valid_from: item.input.occurred_at,
                },
                &published,
            )
            .await?;

            for pairing in &judgement.refused_published {
                metrics::counter!(DEDUP_DECISIONS_TOTAL, "outcome" => "refused_published")
                    .increment(1);
                refused_published.push(json!({
                    "record": pairing.record_id,
                    "reason": pairing.reason.as_str(),
                    "event_id": item.input.event_id,
                }));
            }

            // A restatement writes no record: the survivor absorbs the
            // observation, keeping its content, its vector, its signature
            // and therefore its content address (ADR-0039 decision 10).
            if let Some(pairing) = judgement.merge_into {
                records::reinforce(
                    &mut *tx,
                    tenant_id,
                    pairing.record_id,
                    item.input.event_id,
                    item.input.occurred_at,
                )
                .await?;
                metrics::counter!(DEDUP_DECISIONS_TOTAL, "outcome" => "merge").increment(1);
                merged.push(json!({
                    "into": pairing.record_id,
                    "reason": pairing.reason.as_str(),
                    "class": candidate.class.as_str(),
                    "jaccard_permille": permille(pairing.jaccard),
                    "cosine_permille": pairing.cosine.map(permille),
                }));
                continue;
            }

            let state = RecordState {
                scope_id: *home,
                owner_id,
                kind: RecordKind::Derived,
                class: candidate.class,
                content: candidate.content,
                sensitivity,
                provenance: json!({
                    "event_id": item.input.event_id,
                    "session_id": item.input.session_id,
                    // The event type, carried so the model-asserted /
                    // host-observed distinction survives to read time
                    // (ADR-0057 decision 8, ADR-0078 decision 2). A composed
                    // block echoes `provenance` verbatim, so writing it here
                    // is what makes it provenance rather than telemetry that
                    // dies in the ledger.
                    "event_type": item.input.event_type,
                    "method": item.method,
                    "model_version": item.model_version,
                    "confidence": candidate.confidence,
                    "entities": candidate.entities,
                    "redactions": item.input.redactions,
                    "extracted_at": now.to_rfc3339(),
                }),
                valid_from: item.input.occurred_at,
                // A candidate that observed something already replaced by a
                // newer record lands with its window shut rather than being
                // dropped: never ADD-only cuts both ways (decision 8).
                valid_to: judgement.valid_to(),
            };
            // Record and vector in one statement (ADR-0023 decision 2):
            // this commit cannot produce an embedding-less record.
            let embedding = RecordEmbedding {
                model: deps.embedder.model().to_owned(),
                vector: candidate.vector,
            };
            let record_id = RecordId::new();
            records::insert(&mut *tx, record_id, tenant_id, &state, &embedding).await?;
            // The derived-channel object, in the same transaction as the
            // record it addresses (FLOW-2, ADR-0031 decision 13; the
            // forward obligation ADR-0022 recorded). Content-addressed,
            // so re-extracting identical content at the same scope stores
            // nothing new.
            let asset = memory_asset(record_id, &state);
            let object = vedaflow::put_memory(&mut tx, tenant_id, &asset).await?;
            batch.members.push((asset.entry_name(), object.hash));
            metrics::counter!(EXTRACTION_RECORDS_TOTAL, "class" => candidate.class.as_str())
                .increment(1);
            classes.push(candidate.class.as_str());
            // A restatement absorbed above contributes nothing here: it
            // `continue`d before this point, because text nobody stored
            // asserts nothing a reader can audit (ADR-0044 decision 12).
            linkable.push(LinkedRecord {
                record_id,
                session_id: item.input.session_id.to_string(),
                valid_from: state.valid_from,
                mentions: candidate.entities,
            });

            // The candidate arrived after the fact that replaced it: the
            // edge points the other way, and the window it records is the
            // one this insert already carries.
            if let Some((pairing, closed_at)) = judgement.closed_by {
                store_dedup::record_supersession(
                    &mut tx,
                    tenant_id,
                    &store_dedup::Supersession {
                        superseded_id: record_id,
                        superseding_id: pairing.record_id,
                        method: JUDGE_METHOD.to_owned(),
                        reason: pairing.reason.as_str().to_owned(),
                        jaccard_permille: Some(permille(pairing.jaccard)),
                        cosine_permille: pairing.cosine.map(permille),
                        closed_at,
                    },
                )
                .await?;
                metrics::counter!(DEDUP_DECISIONS_TOTAL, "outcome" => "superseded_on_arrival")
                    .increment(1);
                superseded.push(json!({
                    "record": record_id,
                    "by": pairing.record_id,
                    "reason": pairing.reason.as_str(),
                    "method": JUDGE_METHOD,
                    "on_arrival": true,
                    "jaccard_permille": permille(pairing.jaccard),
                    "cosine_permille": pairing.cosine.map(permille),
                    "closed_at": closed_at.to_rfc3339(),
                }));
            }

            // Every record this statement contradicts stops being current:
            // its window closes at the new record's valid-from, an edge
            // records why, and its changed address is re-committed to the
            // derived channel — the obligation ADR-0031 decision 6 left
            // here, discharged in the same commit as the cause.
            for pairing in &judgement.closes {
                let Some(closed) =
                    records::close_window(&mut *tx, tenant_id, pairing.record_id, state.valid_from)
                        .await?
                else {
                    // Another candidate in this same group already closed
                    // it at or before this instant. The window only ever
                    // narrows, so there is nothing left to do and nothing
                    // to record twice.
                    continue;
                };
                store_dedup::record_supersession(
                    &mut tx,
                    tenant_id,
                    &store_dedup::Supersession {
                        superseded_id: pairing.record_id,
                        superseding_id: record_id,
                        method: JUDGE_METHOD.to_owned(),
                        reason: pairing.reason.as_str().to_owned(),
                        jaccard_permille: Some(permille(pairing.jaccard)),
                        cosine_permille: pairing.cosine.map(permille),
                        closed_at: state.valid_from,
                    },
                )
                .await?;
                let closed_asset = memory_asset(pairing.record_id, &closed.state);
                let closed_object = vedaflow::put_memory(&mut tx, tenant_id, &closed_asset).await?;
                // Pushed after the insert's own entry, so if this group both
                // created and closed the same record the log commit names it
                // at the address it ends up holding: a log channel's tree is
                // exactly this write's members, last one wins per name.
                batch
                    .members
                    .push((closed_asset.entry_name(), closed_object.hash));
                metrics::counter!(DEDUP_DECISIONS_TOTAL, "outcome" => "supersede").increment(1);
                superseded.push(json!({
                    "record": pairing.record_id,
                    "by": record_id,
                    "reason": pairing.reason.as_str(),
                    "method": JUDGE_METHOD,
                    "on_arrival": false,
                    "jaccard_permille": permille(pairing.jaccard),
                    "cosine_permille": pairing.cosine.map(permille),
                    "closed_at": state.valid_from.to_rfc3339(),
                }));
            }
        }
        let outcome_label = if classes.is_empty() { "empty" } else { "ok" };
        metrics::counter!(EXTRACTION_EVENTS_TOTAL, "outcome" => outcome_label).increment(1);
        lags.push((now - item.received_at).num_milliseconds() as f64 / 1000.0);
        methods.push((item.method, item.model_version));
        rescan_findings += item.rescan_findings;
        committed.push(json!({
            "event_id": item.input.event_id,
            "owner": owner_id,
            "session": item.input.session_id,
            "pack": format!("{pack_name}@{pack_version}"),
            "roles": roles,
            "records": classes.len(),
            "classes": classes,
            // Restatements absorbed into records that already assert them
            // (ADR-0039 decision 13): an outcome of this extraction, so it
            // rides this event rather than asserting a second fact.
            "merged": merged,
        }));
    }

    // The graph-linking stage (GRPH-2, ADR-0044 decision 1): on this
    // transaction, after the records it describes exist and before the
    // channel commit takes its locks, so a record and every claim about it
    // either both land or neither does. A failure here aborts the group
    // and the signals redeliver — which is correct, because the resolver
    // has already refused everything the schema would refuse and what is
    // left can only fail on a genuine invariant breach (decision 10).
    let linked = linking::link(&mut tx, tenant_id, &linkable).await?;

    // The derived-channel commits: one per run key (a scope and the subject
    // that ran there), inside this same transaction and before the audit
    // append, so the lock order ADR-0019 decision 1 fixes is preserved with the
    // chain head last. Scopes are visited in id order so two workers touching
    // the same two scopes cannot deadlock by approaching them from opposite
    // ends.
    let mut ordered: Vec<(&RunKey, &DerivedBatch)> = batches
        .iter()
        .filter(|(_, batch)| !batch.members.is_empty())
        .collect();
    ordered
        .sort_unstable_by(|(a_key, a), (b_key, b)| (a.scope, &a_key.0).cmp(&(b.scope, &b_key.0)));
    let mut channels: Vec<serde_json::Value> = Vec::with_capacity(ordered.len());
    for (_key, batch) in ordered {
        let owner = batch.owner;
        let snapshot = PolicySnapshot::new(batch.pack_name.clone(), batch.pack_version);
        let message = format!(
            "extraction: {} record(s) from {} event(s)",
            batch.members.len(),
            batch.events
        );
        let committed = vedaflow::append(
            &mut tx,
            tenant_id,
            &vedaflow::ChannelWrite {
                scope: batch.scope,
                channel: vedaflow::ChannelRef::memory(Channel::Derived),
                members: &batch.members,
                merge_parents: &[],
                author: owner,
                message: &message,
                committed_at: now,
                policy_snapshot: &snapshot,
            },
            &Signer::Unsigned,
        )
        .await?;
        channels.push(json!({
            "scope_id": batch.scope,
            "ref": vedaflow::ChannelRef::memory(Channel::Derived).name(),
            "commit": committed.commit.to_hex(),
            "parent": committed.parent.map(|parent| parent.to_hex()),
            "records": committed.entries,
            "attempts": committed.attempts,
        }));
    }

    // Denials chain standalone decision events (ADR-0019 decision 4).
    for (key, event_ids) in &denied {
        let (subject, scope) = key;
        let reason = match auth_by_run.get(key) {
            Some(RunAuth::Denied { reason }) => reason.clone(),
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
                "principal": subject,
                "scope": scope,
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
                // Where this group's records landed on the derived
                // channel (FLOW-2, ADR-0031 decision 14). Aggregated into
                // the group's one event rather than chained separately:
                // a second event asserting the same fact is noise an
                // auditor has to reconcile (ADR-0019 decision 4).
                "channels": channels,
                // What the graph learned from this commit (GRPH-2,
                // ADR-0044's compliance note): linking adds no action
                // type, because an edge is derived material written
                // inside the transaction whose event this is.
                "graph": linked.summary(),
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
    // What stopped being current, and what the pipeline found and declined
    // to touch — its own action, because it asserts a different fact from
    // "these records were created" and it is the one an auditor arrives
    // with (MEM-5, ADR-0039 decision 13). One event per group, never one
    // per pair. Similarities ride as integers: canonicalisation rejects
    // floats (ADR-0019 decision 2).
    if !superseded.is_empty() || !refused_published.is_empty() {
        append_event(
            &mut tx,
            tenant_id,
            AuditAction::MemorySuperseded,
            format!("tenant {tenant_id}"),
            Outcome::Success,
            json!({
                "superseded": superseded,
                "refused_published": refused_published,
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

/// What judging one candidate needs, beside the config and the scope's
/// published set.
struct JudgeInput<'a> {
    tenant_id: TenantId,
    scope_id: ScopeId,
    owner_id: IdentityId,
    class: RecordClass,
    /// The final, rescanned content — exactly what will be persisted, so
    /// the tokens and the stored signature describe the same text.
    content: &'a str,
    /// The vector computed over that content; empty is impossible through
    /// the embed stage but costs only the dense leg if it ever were.
    vector: &'a [f32],
    valid_from: DateTime<Utc>,
}

/// Nominates and judges one candidate (MEM-5, ADR-0039 decisions 2–5).
///
/// Two legs, union'd by record id: the lexical one over LSH bands, which is
/// meaningful under every configuration, and the dense one over the stored
/// embedding, which is only as meaningful as the embedder — the default hash
/// embedder reaches the near-duplicate band exactly when texts are identical
/// (ADR-0023 decision 6), and that is the honest floor rather than a bug.
///
/// Returns the empty judgement when the pack has dedup off, without touching
/// either index.
async fn judge_candidate(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    deps: &WorkerDeps,
    config: &DedupConfig,
    input: &JudgeInput<'_>,
    published: &HashMap<RecordId, ObjectHash>,
) -> Result<crate::dedup::Judgement> {
    if !config.mode.merges() {
        metrics::counter!(DEDUP_DECISIONS_TOTAL, "outcome" => "insert").increment(1);
        return Ok(crate::dedup::Judgement::default());
    }
    let started = std::time::Instant::now();
    let tokens = store_dedup::tokenise(input.content);
    let signature = store_dedup::signature_of(&tokens);
    let group = store_dedup::CandidateGroup {
        tenant_id: input.tenant_id,
        scope_id: input.scope_id,
        owner_id: input.owner_id,
        class: input.class,
        at: input.valid_from,
    };
    let limit = i64::from(config.neighbours);

    let lexical = store_dedup::nominate_lexical(tx, &group, &signature.bands, limit).await?;
    metrics::histogram!(DEDUP_CANDIDATES, "leg" => "lexical").record(lexical.len() as f64);
    // The dense leg runs only for a dimension that has an ANN index
    // (ADR-0024 decision 5). A model outside that set — a custom embedder,
    // a re-embed in flight — costs this candidate its *semantic*
    // nomination and nothing else: the lexical leg still runs, and a
    // write must never fail because dedup could not consult an index.
    let indexed = synveda_store::search::SUPPORTED_ANN_DIMS.contains(&input.vector.len());
    let dense = if input.vector.is_empty() || !indexed {
        if !input.vector.is_empty() {
            tracing::debug!(
                dim = input.vector.len(),
                model = deps.embedder.model(),
                "no ANN index for this dimension; nominating on the lexical leg alone"
            );
        }
        Vec::new()
    } else {
        let hits =
            store_dedup::nominate_dense(tx, &group, deps.embedder.model(), input.vector, limit)
                .await?;
        metrics::histogram!(DEDUP_CANDIDATES, "leg" => "dense").record(hits.len() as f64);
        hits
    };

    // Union by id, dense first so a neighbour both legs found keeps its
    // cosine. Distance is `1 - similarity` for pgvector's `<=>`.
    let mut nominees: Vec<crate::dedup::Nominee> = Vec::new();
    let mut seen: Vec<RecordId> = Vec::new();
    for (version, distance) in dense {
        seen.push(version.id);
        nominees.push(nominee(version, Some(1.0 - distance), published));
    }
    for version in lexical {
        if seen.contains(&version.id) {
            continue;
        }
        nominees.push(nominee(version, None, published));
    }

    let judgement = crate::dedup::judge(
        config,
        &crate::dedup::Incoming {
            content: input.content,
            tokens: &tokens,
            valid_from: input.valid_from,
        },
        &nominees,
    );
    metrics::histogram!(DEDUP_SECONDS).record(started.elapsed().as_secs_f64());
    if judgement.merge_into.is_none() {
        metrics::counter!(DEDUP_DECISIONS_TOTAL, "outcome" => "insert").increment(1);
    }
    Ok(judgement)
}

/// One hydrated neighbour, with the publication flag the judge's governance
/// boundary needs. A tree entry counts as publication only when it names the
/// address the record's *current* content produces — an edited record is
/// unreviewed again (ADR-0031 decision 5), and unreviewed material is the
/// pipeline's to supersede.
fn nominee(
    version: synveda_store::records::RecordVersion,
    cosine: Option<f64>,
    published: &HashMap<RecordId, ObjectHash>,
) -> crate::dedup::Nominee {
    let address = memory_asset(version.id, &version.state).address();
    let published = published.get(&version.id) == Some(&address);
    crate::dedup::Nominee {
        record_id: version.id,
        tokens: store_dedup::tokenise(&version.state.content),
        content: version.state.content,
        valid_from: version.state.valid_from,
        tx_from: version.tx_from,
        cosine,
        published,
    }
}

/// Re-decides `MemoryWrite` for one run at the **scope the run was decided
/// at**, under current facts — the same reads the gateway's gather seam
/// performs, with explicit context instead of task-locals.
///
/// The resource is the session's scope and not the submitter's home (CPR-12,
/// ADR-0078 decision 3), which is the whole product consequence of the observe
/// cutover: a run against a shared project writes project memories, and a run
/// at somebody's own principal scope writes private ones that `base.cedar`'s
/// personal-scope forbid keeps private.
///
/// The principal keeps its **own** chain in `principal_scopes` while `scopes`
/// carries the resource's, because `standard` shares by `principal.ambit` and
/// collapsing the two would make every run look like it happened where the
/// person lives.
async fn authorize_run(
    deps: &WorkerDeps,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    subject: &str,
    scope_id: ScopeId,
) -> Result<RunAuth> {
    // A record carries an owner, so a token subject with no identity row is
    // denied rather than guessed at. A run can be opened by a subject that has
    // no identity (a grant may precede one, ADR-0072), and that is exactly the
    // case this refuses: the run is real and its memories have nobody to
    // belong to.
    let Some(identity) = identities::by_subject(&mut *tx, tenant_id, subject).await? else {
        return Ok(RunAuth::Denied {
            reason: "run's principal has no identity to own a record".to_owned(),
        });
    };
    // A departed person writes nothing, even from a run they opened before
    // they left (AUTH-4, ADR-0059 decision 8).
    //
    // `base.cedar`'s seal forbid used to carry this on its own: the write
    // landed at the person's own scope, and departure seals that scope. Since
    // CPR-12 a memory lands at the **run's** scope (ADR-0078 decision 3), which
    // is a workspace nobody sealed — so the rule the seal expressed has to be
    // asserted here or an extraction worker draining a queue after somebody's
    // last day would commit their material anyway. Checked before the decision
    // rather than inside it, because it is a fact about the writer and not
    // about what any pack permits.
    if identity.status != IdentityStatus::Active {
        return Ok(RunAuth::Denied {
            reason: format!("run's principal is {} at extraction time", identity.status),
        });
    }
    let owner_id = identity.id;
    let subject = subject.to_owned();
    // Quarantine is only ever "not provisioned" now (CPR-7, ADR-0074
    // decision 3): the identity row exists by construction here, so the
    // placement-derived flag is gone and nothing replaces it.
    let quarantined = false;
    // Two chains, and they are different facts. `principal_chain` is where the
    // person lives; `chain` is where the run happened and what the decision is
    // about.
    let principal_chain: Vec<ScopeNode> = scope_chain(tx, tenant_id, identity.scope_id).await?;
    let chain: Vec<ScopeNode> = scope_chain(tx, tenant_id, scope_id).await?;
    // The confinement scope (ADR-0018 decision 4): a service identity's
    // anchor is the scope above its own; unresolvable means quarantined,
    // never unconfined.
    let token_scope = if identity.kind == IdentityKind::Service {
        let anchor = principal_chain.get(1).map(|node| node.id);
        if anchor.is_none() {
            // Fail closed on the Principal the same way the flag used to.
            return Ok(RunAuth::Denied {
                reason: "service identity lost its anchor scope".to_owned(),
            });
        }
        anchor
    } else {
        None
    };
    let principal = Principal {
        tenant_id,
        subject: subject.clone(),
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
    let anchor_set = anchors::resolve(
        &mut *tx,
        tenant_id,
        &subject,
        anchors::AnchorSelection::none(),
    )
    .await?;
    let groups = anchors::groups_of(&mut *tx, tenant_id, &subject).await?;
    let context = AuthzContext {
        scopes: &chain,
        principal_scopes: &principal_chain,
        anchors: anchor_set.as_slice(),
        groups: &groups,
        resources: &[],
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        sensitivity: None,
        // A lapse relaxes reads, never writes: the vocabulary has one
        // action and it is `MemoryRead` (ADR-0037 decision 2).
        lapses: &[],
    };
    let decision = deps.pdp.authorize(
        &principal,
        Action::MemoryWrite,
        Resource::Scope(scope_id),
        &context,
    )?;
    if decision.allowed {
        // The grant keys that reached this decision (CPR-6, ADR-0073
        // decision 5): the record's provenance says who may act on it, and
        // that is the set the decision actually weighed.
        let mut roles: Vec<String> =
            synveda_policy::effective_role_keys_at(Resource::Scope(scope_id), &context)
                .into_iter()
                .map(|key| key.as_str().to_owned())
                .collect();
        roles.sort_unstable();
        roles.dedup();
        // The same effective pack the decision came from, read for its
        // non-Cedar configuration (ADR-0039 decision 12) — the resolution
        // MEM-2 and CTX-2 already do for redaction and composition.
        let dedup = deps
            .pdp
            .effective(tenant_id, Resource::Scope(scope_id), &context)
            .dedup;
        Ok(RunAuth::Allowed {
            scope: scope_id,
            owner_id,
            pack_name: decision.pack_name,
            pack_version: decision.pack_version,
            roles,
            dedup,
        })
    } else {
        let determining = if decision.determining.is_empty() {
            "no policy permitted it".to_owned()
        } else {
            decision.determining.join(", ")
        };
        Ok(RunAuth::Denied {
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

/// The VedaFlow view of a record about to be inserted (ADR-0031
/// decision 6): the governed fields, and none of the provenance.
///
/// Spelled out here rather than shared with `synveda-retrieval`, which
/// needs the same mapping: `synveda-store` and `synveda-vedaflow` are
/// siblings, so neither can host a conversion between their types, and
/// a field copy duplicated across a layering boundary is the trade
/// ADR-0030 already took for the RLS backstop marker. The address it
/// produces is pinned to retrieval's by the AC test.
pub(crate) fn memory_asset(id: RecordId, state: &RecordState) -> MemoryAsset {
    MemoryAsset {
        id,
        scope_id: state.scope_id,
        owner_id: state.owner_id,
        kind: state.kind,
        class: state.class,
        content: state.content.clone(),
        sensitivity: state.sensitivity,
        valid_from: state.valid_from,
        valid_to: state.valid_to,
    }
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
