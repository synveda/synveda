//! Chain operations: append, verify, tail (AUD-1, ADR-0019).
//!
//! Append runs inside the caller's tenant transaction (RLS GUC set by
//! `synveda_store::rls::begin_tenant_tx`): it locks the tenant's chain head,
//! hashes the canonical event over the previous hash, inserts the row, and
//! advances the head — so a rollback retracts the event, the head move, and
//! the lock together, and "no action without its record" holds by
//! transaction atomicity. The head is the last row a well-behaved
//! transaction locks (ADR-0019 decision 1).

use blake3::Hasher;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgConnection;
use synveda_types::{Error, Result, TenantId};

use crate::canonical::{canonical_event, truncate_to_micros};
use crate::event::{Actor, AuditAction, AuditEvent};
use crate::query::{EventFilter, search};

/// Counts appended events, labelled by action and outcome. Emitted here;
/// described by the gateway's recorder (ADR-0007).
pub const AUDIT_EVENTS_TOTAL: &str = "synveda_audit_events_total";

/// Counts appends that failed on a best-effort path (deny-path emission
/// must never mask the original error — ADR-0019 decision 5). Emitted by
/// the seam that swallows the failure.
pub const AUDIT_APPEND_FAILURES_TOTAL: &str = "synveda_audit_append_failures_total";

/// Counts generation-one key-provision witness convergence, labelled by
/// `result` (`appended`, `existing`, or `inconsistent`).
pub const TENANT_KEY_PROVISION_WITNESSES_TOTAL: &str =
    "synveda_audit_tenant_key_provision_witnesses_total";

/// Counts chain verifications, labelled by outcome (`valid`/`broken`).
pub const AUDIT_VERIFICATIONS_TOTAL: &str = "synveda_audit_verifications_total";

/// Domain separator for event hashes.
const EVENT_DOMAIN: &[u8] = b"synveda-audit-event-v1";
/// Domain separator for the genesis hash binding a chain to its tenant.
const GENESIS_DOMAIN: &[u8] = b"synveda-audit-genesis-v1";
/// Rows fetched per round while verifying — bounds memory on long chains.
const VERIFY_PAGE: i64 = 1024;

/// The hash a tenant's chain starts from. Including the tenant id means a
/// chain (or its prefix) cannot be transplanted between tenants and still
/// verify.
#[must_use]
pub fn genesis_hash(tenant: TenantId) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(GENESIS_DOMAIN);
    hasher.update(tenant.as_uuid().as_bytes());
    *hasher.finalize().as_bytes()
}

/// `BLAKE3(domain ‖ prev_hash ‖ canonical_event)` — the one hash rule for
/// every event (ADR-0019 decision 2). Public for verification tooling
/// (AUD-3's offline verifier recomputes exactly this).
#[must_use]
pub fn compute_hash(prev_hash: &[u8], canonical: &str) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(EVENT_DOMAIN);
    hasher.update(prev_hash);
    hasher.update(canonical.as_bytes());
    *hasher.finalize().as_bytes()
}

/// What [`append`] minted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendedEvent {
    /// The event's position in its tenant's chain (1-based).
    pub seq: i64,
    /// The event's hash — the chain head after this append commits.
    pub hash: [u8; 32],
}

/// The attempt metadata needed to converge the one exceptional
/// generation-one key-provision witness (ADR-0064 amendment 3).
///
/// Action, resource, outcome, generation and payload shape are deliberately
/// not caller-controlled. Ordinary governed mutations must continue to call
/// [`append`] in the same transaction as their effect (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantKeyProvisionedWitness {
    /// When this repair or initial append was attempted.
    pub occurred_at: DateTime<Utc>,
    /// The OS-attributed subject running break-glass key provisioning.
    pub break_glass_subject: String,
    /// The authoritative KEK reference stored with generation one.
    pub kek_ref: String,
    /// Optional trace correlation for this attempt.
    pub trace_id: Option<String>,
}

/// Result of converging the generation-one key-provision witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantKeyProvisionedAppend {
    /// The existing or newly appended chain position.
    pub seq: i64,
    /// Whether this transaction appended the witness.
    pub appended: bool,
}

struct LockedHead {
    seq: i64,
    hash: Vec<u8>,
}

/// Appends one event to `tenant`'s chain, inside the caller's transaction.
///
/// The caller's transaction must have been opened by
/// `synveda_store::rls::begin_tenant_tx` for the same tenant — under forced
/// RLS a mismatched GUC makes every statement here see (and write) nothing,
/// which surfaces as a storage error, never a cross-tenant write.
#[tracing::instrument(
    name = "audit.append",
    skip_all,
    fields(tenant.id = %tenant, audit.action = event.action.as_str(), audit.seq = tracing::field::Empty),
    err(Display)
)]
pub async fn append(
    conn: &mut PgConnection,
    tenant: TenantId,
    event: &AuditEvent,
) -> Result<AppendedEvent> {
    let head = lock_head(conn, tenant).await?;
    append_locked(conn, tenant, event, head).await
}

/// Appends the exact generation-one tenant-key witness only when absent.
///
/// The tenant's chain head is locked before the lookup, so concurrent callers
/// cannot both observe absence. Actor subject, occurrence time and trace id
/// describe an attempt rather than the idempotent business fact. Historic
/// duplicate exact witnesses are accepted but never extended; a candidate
/// whose payload only contains the requested shape fails closed.
///
/// This intentionally narrow API is the only exception to ADR-0019's normal
/// same-transaction mutation/audit rule. The external KMS effect cannot join
/// a PostgreSQL transaction, so an exact witness can be repaired after custody
/// is proved. No generic idempotent append surface is exported.
#[tracing::instrument(
    name = "audit.tenant_key_provision_witness",
    skip_all,
    fields(tenant.id = %tenant, audit.action = AuditAction::TenantKeyProvisioned.as_str(), audit.seq = tracing::field::Empty),
    err(Display)
)]
pub async fn append_tenant_key_provisioned_once(
    conn: &mut PgConnection,
    tenant: TenantId,
    witness: &TenantKeyProvisionedWitness,
) -> Result<TenantKeyProvisionedAppend> {
    let event = AuditEvent {
        occurred_at: witness.occurred_at,
        actor: Actor::break_glass(witness.break_glass_subject.clone()),
        action: AuditAction::TenantKeyProvisioned,
        resource: format!("tenant {tenant} key"),
        outcome: crate::event::Outcome::Success,
        payload: serde_json::json!({
            "version": 1,
            "kek_ref": witness.kek_ref,
        }),
        trace_id: witness.trace_id.clone(),
    };
    append_once(conn, tenant, &event).await
}

async fn append_once(
    conn: &mut PgConnection,
    tenant: TenantId,
    event: &AuditEvent,
) -> Result<TenantKeyProvisionedAppend> {
    let head = lock_head(conn, tenant).await?;
    match existing_exact_witness(conn, tenant, event).await? {
        None => {
            let appended = append_locked(conn, tenant, event, head).await?;
            metrics::counter!(
                TENANT_KEY_PROVISION_WITNESSES_TOTAL,
                "result" => "appended",
            )
            .increment(1);
            Ok(TenantKeyProvisionedAppend {
                seq: appended.seq,
                appended: true,
            })
        }
        Some(seq) => {
            tracing::Span::current().record("audit.seq", seq);
            metrics::counter!(
                TENANT_KEY_PROVISION_WITNESSES_TOTAL,
                "result" => "existing",
            )
            .increment(1);
            Ok(TenantKeyProvisionedAppend {
                seq,
                appended: false,
            })
        }
    }
}

async fn existing_exact_witness(
    conn: &mut PgConnection,
    tenant: TenantId,
    event: &AuditEvent,
) -> Result<Option<i64>> {
    const PAGE_SIZE: i64 = 64;
    const MAX_CANDIDATES: usize = 4_096;

    let filter = EventFilter {
        actions: vec![event.action],
        outcome: Some(event.outcome),
        resource: Some(event.resource.clone()),
        // Inspect every generation-one candidate, including a conflicting
        // KEK reference or malformed superset. Later generations are a
        // different historic fact and must not block repair of generation 1.
        payload_contains: Some(serde_json::json!({ "version": 1 })),
        ..EventFilter::default()
    };
    let mut after = 0;
    let mut first_seq = None;
    let mut inspected = 0usize;

    loop {
        let page = search(conn, tenant, &filter, after, PAGE_SIZE).await?;
        if page
            .items
            .iter()
            .any(|candidate| !exact_witness(candidate, event))
        {
            metrics::counter!(
                TENANT_KEY_PROVISION_WITNESSES_TOTAL,
                "result" => "inconsistent",
            )
            .increment(1);
            return Err(Error::Conflict {
                message: "audit idempotency witness is inconsistent".to_owned(),
            });
        }
        if first_seq.is_none() {
            first_seq = page.items.first().map(|candidate| candidate.seq);
        }
        inspected = inspected
            .checked_add(page.items.len())
            .ok_or_else(|| Error::Internal {
                message: "audit idempotency witness count overflowed".to_owned(),
            })?;
        if inspected > MAX_CANDIDATES {
            metrics::counter!(
                TENANT_KEY_PROVISION_WITNESSES_TOTAL,
                "result" => "inconsistent",
            )
            .increment(1);
            return Err(Error::Conflict {
                message: "audit idempotency witness exceeds the bounded history".to_owned(),
            });
        }
        let Some(cursor) = page.next_cursor else {
            return Ok(first_seq);
        };
        after = cursor;
    }
}

fn exact_witness(existing: &StoredEvent, event: &AuditEvent) -> bool {
    existing.actor_kind == event.actor.kind.as_str()
        && existing.action == event.action.as_str()
        && existing.resource == event.resource
        && existing.outcome == event.outcome.as_str()
        && existing.payload == event.payload
}

async fn lock_head(conn: &mut PgConnection, tenant: TenantId) -> Result<LockedHead> {
    let genesis = genesis_hash(tenant);

    // First append wins the race to create the head; every later append
    // finds it. DO NOTHING keeps this idempotent under concurrency.
    sqlx::query!(
        "insert into audit_chain_heads (tenant_id, seq, head_hash)
         values ($1, 0, $2)
         on conflict (tenant_id) do nothing",
        tenant.as_uuid(),
        &genesis[..],
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("ensure chain head", &err))?;

    // The per-tenant append lock: held until the caller commits or rolls
    // back, serialising chain growth (ADR-0019 option 1).
    let head = sqlx::query!(
        "select seq, head_hash from audit_chain_heads where tenant_id = $1 for update",
        tenant.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| storage_error("lock chain head", &err))?;

    Ok(LockedHead {
        seq: head.seq,
        hash: head.head_hash,
    })
}

async fn append_locked(
    conn: &mut PgConnection,
    tenant: TenantId,
    event: &AuditEvent,
    head: LockedHead,
) -> Result<AppendedEvent> {
    let occurred_at = truncate_to_micros(event.occurred_at);

    let seq = head.seq + 1;
    let canonical = canonical_event(
        tenant.as_uuid(),
        seq,
        occurred_at,
        event.actor.kind.as_str(),
        &event.actor.subject,
        event.action.as_str(),
        &event.resource,
        event.outcome.as_str(),
        &event.payload,
        event.trace_id.as_deref(),
    )?;
    let hash = compute_hash(&head.hash, &canonical);

    sqlx::query!(
        "insert into audit_log
             (tenant_id, seq, occurred_at, actor_kind, actor_subject, action,
              resource, outcome, payload, trace_id, prev_hash, hash)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        tenant.as_uuid(),
        seq,
        occurred_at,
        event.actor.kind.as_str(),
        &event.actor.subject,
        event.action.as_str(),
        &event.resource,
        event.outcome.as_str(),
        &event.payload,
        event.trace_id.as_deref(),
        &head.hash[..],
        &hash[..],
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("insert audit event", &err))?;

    sqlx::query!(
        "update audit_chain_heads set seq = $2, head_hash = $3 where tenant_id = $1",
        tenant.as_uuid(),
        seq,
        &hash[..],
    )
    .execute(&mut *conn)
    .await
    .map_err(|err| storage_error("advance chain head", &err))?;

    tracing::Span::current().record("audit.seq", seq);
    metrics::counter!(
        AUDIT_EVENTS_TOTAL,
        "action" => event.action.as_str(),
        "outcome" => event.outcome.as_str(),
    )
    .increment(1);
    Ok(AppendedEvent { seq, hash })
}

/// One event as stored — what verification recomputes from and what the
/// CLI renders.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    /// Position in the tenant's chain (1-based).
    pub seq: i64,
    /// When the operation happened (microsecond precision).
    pub occurred_at: DateTime<Utc>,
    /// Attribution strength (`subject` / `break_glass`).
    pub actor_kind: String,
    /// The acting subject.
    pub actor_subject: String,
    /// The dotted action name.
    pub action: String,
    /// The acted-on resource.
    pub resource: String,
    /// How it ended.
    pub outcome: String,
    /// Event-specific detail.
    pub payload: Value,
    /// The OTel trace id, when one was live.
    pub trace_id: Option<String>,
    /// The previous event's hash (genesis hash for seq 1).
    pub prev_hash: Vec<u8>,
    /// This event's hash.
    pub hash: Vec<u8>,
}

impl StoredEvent {
    /// The event hash as lowercase hex, for rendering.
    #[must_use]
    pub fn hash_hex(&self) -> String {
        to_hex(&self.hash)
    }
}

/// Lowercase hex without pulling in a dependency for sixteen characters.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
        out
    })
}

/// Why a chain failed verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakReason {
    /// The sequence skipped: `expected` was next, but this row's seq is not
    /// it (a row was removed, or a hole was left by tampering).
    Gap {
        /// The seq verification expected at this position.
        expected: i64,
    },
    /// This row's `prev_hash` does not equal the previous event's hash.
    Linkage,
    /// This row's content does not hash to its stored `hash` — the row was
    /// rewritten in place.
    Content,
    /// Every row verified, but the chain head does not match the last
    /// event (the head was moved, or rows were appended/truncated around
    /// it).
    Head,
    /// Events exist but the head row is gone.
    MissingHead,
}

impl std::fmt::Display for BreakReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BreakReason::Gap { expected } => write!(f, "sequence gap (expected seq {expected})"),
            BreakReason::Linkage => write!(f, "previous-hash linkage broken"),
            BreakReason::Content => write!(f, "row content does not match its hash"),
            BreakReason::Head => write!(f, "chain head does not match the last event"),
            BreakReason::MissingHead => write!(f, "chain head row is missing"),
        }
    }
}

/// The result of walking a tenant's chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainVerification {
    /// Every event re-hashed to its stored value, linkage and head
    /// included.
    Valid {
        /// Number of events verified.
        events: i64,
    },
    /// The chain is broken at `seq` for `reason`; nothing after that point
    /// can be trusted (and for a moved head, nothing before it either).
    Broken {
        /// Where verification first diverged.
        seq: i64,
        /// What diverged.
        reason: BreakReason,
    },
}

impl std::fmt::Display for ChainVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainVerification::Valid { events } => {
                write!(f, "chain valid ({events} events)")
            }
            ChainVerification::Broken { seq, reason } => {
                write!(f, "chain BROKEN at seq {seq}: {reason}")
            }
        }
    }
}

/// A verification verdict together with the exact tenant-chain head it
/// checked.
///
/// The head is captured once before the paged walk starts. A later append is
/// outside the verified prefix, not evidence that the prefix was broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    /// Whether the frozen prefix is valid, or its first divergence.
    pub verification: ChainVerification,
    /// The frozen prefix's final sequence number; zero for an empty chain.
    pub head_seq: i64,
    /// The frozen prefix's head hash; the tenant-bound genesis for an empty
    /// chain.
    pub head_hash: Vec<u8>,
}

#[derive(Debug)]
struct VerificationSnapshot {
    head: Option<(i64, Vec<u8>)>,
    max_seq: i64,
}

impl VerificationSnapshot {
    fn report_head(&self, tenant: TenantId) -> (i64, Vec<u8>) {
        self.head
            .clone()
            .unwrap_or_else(|| (0, genesis_hash(tenant).to_vec()))
    }
}

/// Walks `tenant`'s whole chain, recomputing every hash from the stored
/// columns (never trusting stored bytes as hash input), and reports the
/// first divergence.
///
/// This convenience helper discards the frozen frame. A caller that renders
/// a completeness frame should use [`verify_report`] so it cannot accidentally
/// pair this verdict with a later chain head.
pub async fn verify(conn: &mut PgConnection, tenant: TenantId) -> Result<ChainVerification> {
    Ok(verify_report(conn, tenant).await?.verification)
}

/// Verifies one frozen tenant-chain prefix and returns both its verdict and
/// the exact head that verdict covers.
///
/// PostgreSQL's default `READ COMMITTED` isolation takes a fresh snapshot for
/// every statement. Capturing the head once is therefore not sufficient by
/// itself: every page must also carry `seq <= frozen_head`. A normal append
/// committed while a long verification is walking later pages is ignored by
/// this run and belongs to the next one.
#[tracing::instrument(name = "audit.verify", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn verify_report(
    conn: &mut PgConnection,
    tenant: TenantId,
) -> Result<VerificationReport> {
    let snapshot = verification_snapshot(conn, tenant).await?;
    verify_snapshot(conn, tenant, snapshot).await
}

/// Captures the chain head and log extent in one SQL statement and therefore
/// one MVCC snapshot. The extent preserves AUD-1's detection of a head moved
/// ahead of or behind the log without making the verifier read event content
/// beyond the frozen head.
async fn verification_snapshot(
    conn: &mut PgConnection,
    tenant: TenantId,
) -> Result<VerificationSnapshot> {
    let row = sqlx::query!(
        r#"select head.seq as "head_seq?",
                  head.head_hash as "head_hash?",
                  coalesce(log.max_seq, 0) as "max_seq!"
             from (
                    select max(seq) as max_seq
                      from audit_log
                     where tenant_id = $1
                  ) log
             left join audit_chain_heads head on head.tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| storage_error("freeze audit chain head", &err))?;

    let head = match (row.head_seq, row.head_hash) {
        (Some(seq), Some(hash)) => Some((seq, hash)),
        (None, None) => None,
        // Both columns are NOT NULL on the same row. Reaching this branch
        // means the database returned a shape the schema cannot represent.
        _ => {
            return Err(Error::Storage {
                message: "freeze audit chain head: incomplete head row".to_owned(),
            });
        }
    };
    Ok(VerificationSnapshot {
        head,
        max_seq: row.max_seq,
    })
}

async fn verify_snapshot(
    conn: &mut PgConnection,
    tenant: TenantId,
    snapshot: VerificationSnapshot,
) -> Result<VerificationReport> {
    let (head_seq, head_hash) = snapshot.report_head(tenant);

    // No head and no rows is the never-started, valid empty chain. Rows
    // without a head are already conclusively broken; there is no trusted
    // prefix boundary whose event content could be walked.
    if snapshot.head.is_none() {
        let verification = if snapshot.max_seq == 0 {
            ChainVerification::Valid { events: 0 }
        } else {
            ChainVerification::Broken {
                seq: snapshot.max_seq,
                reason: BreakReason::MissingHead,
            }
        };
        record_verification_metric(verification);
        return Ok(VerificationReport {
            verification,
            head_seq,
            head_hash,
        });
    }

    let mut prev: Vec<u8> = genesis_hash(tenant).to_vec();
    let mut expected = 1i64;
    let mut outcome = None;

    'walk: loop {
        let rows = page(conn, tenant, expected - 1, head_seq).await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            if row.seq != expected {
                outcome = Some((row.seq, BreakReason::Gap { expected }));
                break 'walk;
            }
            if row.prev_hash != prev {
                outcome = Some((row.seq, BreakReason::Linkage));
                break 'walk;
            }
            // A canonicalisation error (e.g. an injected float) means the
            // payload is something append could never have hashed: content.
            let canonical = canonical_event(
                tenant.as_uuid(),
                row.seq,
                row.occurred_at,
                &row.actor_kind,
                &row.actor_subject,
                &row.action,
                &row.resource,
                &row.outcome,
                &row.payload,
                row.trace_id.as_deref(),
            );
            let recomputed = match canonical {
                Ok(canonical) => compute_hash(&prev, &canonical),
                Err(_) => {
                    outcome = Some((row.seq, BreakReason::Content));
                    break 'walk;
                }
            };
            if recomputed[..] != row.hash[..] {
                outcome = Some((row.seq, BreakReason::Content));
                break 'walk;
            }
            prev = row.hash;
            expected += 1;
        }
    }

    let events = expected - 1;
    let verification = if let Some((seq, reason)) = outcome {
        ChainVerification::Broken { seq, reason }
    } else if events == head_seq && snapshot.max_seq == head_seq && head_hash == prev {
        ChainVerification::Valid { events }
    } else {
        ChainVerification::Broken {
            seq: head_seq,
            reason: BreakReason::Head,
        }
    };

    record_verification_metric(verification);
    Ok(VerificationReport {
        verification,
        head_seq,
        head_hash,
    })
}

fn record_verification_metric(verification: ChainVerification) {
    metrics::counter!(
        AUDIT_VERIFICATIONS_TOTAL,
        "outcome" => match verification {
            ChainVerification::Valid { .. } => "valid",
            ChainVerification::Broken { .. } => "broken",
        },
    )
    .increment(1);
}

/// The most recent `limit` events, newest first — the CLI's `audit tail`
/// and the demo's chain listing.
pub async fn tail(
    conn: &mut PgConnection,
    tenant: TenantId,
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    sqlx::query_as!(
        StoredEvent,
        r#"select seq, occurred_at, actor_kind, actor_subject, action,
                  resource, outcome, payload, trace_id, prev_hash, hash
           from audit_log
           where tenant_id = $1
           order by seq desc
           limit $2"#,
        tenant.as_uuid(),
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit tail", &err))
}

/// Events with `seq > after` whose action is one of `actions`, oldest
/// first, at most `limit` of them.
///
/// The forward read a projection folds from (FLOW-4, ADR-0033 decision
/// 2). `audit_log.seq` is 1-based and contiguous per tenant — a gap is a
/// verification failure, ADR-0019 — which is what lets a single integer
/// serve as a cursor with no ambiguity in it: everything at or below
/// `after` has been folded, everything above it has not.
///
/// Actions are named from the closed in-process vocabulary rather than
/// as strings, so a reader cannot quietly fold an action that does not
/// exist and conclude the tenant is idle.
#[tracing::instrument(
    name = "audit.since",
    skip_all,
    fields(tenant.id = %tenant, after = after, limit = limit),
    err(Display)
)]
pub async fn since(
    conn: &mut PgConnection,
    tenant: TenantId,
    after: i64,
    actions: &[AuditAction],
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    if actions.is_empty() {
        return Ok(Vec::new());
    }
    let names: Vec<String> = actions
        .iter()
        .map(|action| action.as_str().to_owned())
        .collect();
    sqlx::query_as!(
        StoredEvent,
        r#"select seq, occurred_at, actor_kind, actor_subject, action,
                  resource, outcome, payload, trace_id, prev_hash, hash
           from audit_log
           where tenant_id = $1 and seq > $2 and action = any($3)
           order by seq
           limit $4"#,
        tenant.as_uuid(),
        after,
        &names,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit events since", &err))
}

/// How long the tenant's chain is: the seq of its most recent event, or
/// 0 for a chain with none.
///
/// Read from `audit_chain_heads` rather than `max(seq)` because the head
/// row is the chain's own statement of its length, and a reader that
/// disagreed with it would be reading past a truncation rather than
/// noticing one.
#[tracing::instrument(
    name = "audit.head_seq",
    skip_all,
    fields(tenant.id = %tenant),
    err(Display)
)]
pub async fn head_seq(conn: &mut PgConnection, tenant: TenantId) -> Result<i64> {
    let seq = sqlx::query_scalar!(
        "select seq from audit_chain_heads where tenant_id = $1",
        tenant.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit chain head", &err))?;
    Ok(seq.unwrap_or(0))
}

/// One verification page inside the head captured before the walk began:
/// events with `after < seq <= through`, ascending.
async fn page(
    conn: &mut PgConnection,
    tenant: TenantId,
    after: i64,
    through: i64,
) -> Result<Vec<StoredEvent>> {
    sqlx::query_as!(
        StoredEvent,
        r#"select seq, occurred_at, actor_kind, actor_subject, action,
                  resource, outcome, payload, trace_id, prev_hash, hash
             from audit_log
            where tenant_id = $1 and seq > $2 and seq <= $3
            order by seq
            limit $4"#,
        tenant.as_uuid(),
        after,
        through,
        VERIFY_PAGE,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit page", &err))
}

fn storage_error(context: &str, err: &sqlx::Error) -> Error {
    Error::Storage {
        message: format!("{context}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::{PgPool, Postgres, Transaction};

    use crate::event::{Actor, Outcome};

    async fn test_pool() -> Option<PgPool> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping frozen verification test: DATABASE_URL is not set \
                     (run `make dev-up` then `make db-test`)"
                );
                return None;
            }
        };
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect to DATABASE_URL");
        let migrated =
            sqlx::query_scalar::<_, Option<String>>("select to_regclass('public.audit_log')::text")
                .fetch_one(&pool)
                .await
                .expect("probe for audit_log");
        if migrated.is_none() {
            eprintln!(
                "skipping frozen verification test: audit tables missing -- apply \
                 migrations first (`synveda db migrate`)"
            );
            return None;
        }
        Some(pool)
    }

    async fn tenant_tx(pool: &PgPool, tenant: TenantId) -> Transaction<'static, Postgres> {
        let mut tx = pool.begin().await.expect("begin tenant transaction");
        sqlx::query!(
            "select set_config('synveda.tenant_id', $1, true)",
            tenant.as_uuid().to_string(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("set tenant GUC");
        tx
    }

    fn event(seq: i64) -> AuditEvent {
        AuditEvent {
            occurred_at: Utc::now(),
            actor: Actor::subject("frozen-verifier"),
            action: AuditAction::AuthzDecision,
            resource: format!("audit verification fixture {seq}"),
            outcome: Outcome::Allow,
            payload: serde_json::json!({"seq": seq}),
            trace_id: None,
        }
    }

    #[test]
    fn genesis_hashes_bind_chains_to_their_tenant() {
        let a = TenantId::new();
        let b = TenantId::new();
        assert_ne!(genesis_hash(a), genesis_hash(b));
        assert_eq!(genesis_hash(a), genesis_hash(a));
    }

    #[test]
    fn hashes_cover_prev_hash_and_content() {
        let prev_a = [1u8; 32];
        let prev_b = [2u8; 32];
        assert_ne!(
            compute_hash(&prev_a, "event"),
            compute_hash(&prev_b, "event")
        );
        assert_ne!(
            compute_hash(&prev_a, "event"),
            compute_hash(&prev_a, "other")
        );
        assert_eq!(
            compute_hash(&prev_a, "event"),
            compute_hash(&prev_a, "event")
        );
    }

    #[test]
    fn hex_rendering_is_lowercase_and_padded() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xa5]), "000fa5");
    }

    #[tokio::test]
    async fn append_after_freeze_stays_outside_a_two_page_verification() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let tenant = TenantId::new();

        // One row beyond VERIFY_PAGE forces the verifier to issue a second
        // SELECT under READ COMMITTED, which is where the moving-snapshot
        // defect appeared.
        let frozen_events = VERIFY_PAGE + 1;
        let mut seed_tx = tenant_tx(&pool, tenant).await;
        for seq in 1..=frozen_events {
            append(&mut seed_tx, tenant, &event(seq))
                .await
                .expect("append frozen prefix");
        }
        seed_tx.commit().await.expect("commit frozen prefix");

        let mut verification_tx = tenant_tx(&pool, tenant).await;
        let snapshot = verification_snapshot(&mut verification_tx, tenant)
            .await
            .expect("freeze verification head");
        let (frozen_head_seq, frozen_head_hash) = snapshot.report_head(tenant);
        assert_eq!(frozen_head_seq, frozen_events);

        // Commit a normal append on another connection after the verifier
        // froze its boundary but before it reads either page.
        let mut append_tx = tenant_tx(&pool, tenant).await;
        let appended = append(&mut append_tx, tenant, &event(frozen_events + 1))
            .await
            .expect("append beyond frozen prefix");
        append_tx.commit().await.expect("commit concurrent append");

        let report = verify_snapshot(&mut verification_tx, tenant, snapshot)
            .await
            .expect("verify frozen prefix");
        assert_eq!(
            report.verification,
            ChainVerification::Valid {
                events: frozen_events,
            }
        );
        assert_eq!(report.head_seq, frozen_head_seq);
        assert_eq!(report.head_hash, frozen_head_hash);
        assert_eq!(appended.seq, frozen_events + 1);
        assert_ne!(report.head_hash, appended.hash);

        // The next verification takes a new boundary and includes the append;
        // the first report was a valid prefix, not a stale global claim.
        let current = verify_report(&mut verification_tx, tenant)
            .await
            .expect("verify current chain");
        assert_eq!(
            current.verification,
            ChainVerification::Valid {
                events: frozen_events + 1,
            }
        );
        assert_eq!(current.head_seq, appended.seq);
        assert_eq!(current.head_hash, appended.hash);
    }
}
