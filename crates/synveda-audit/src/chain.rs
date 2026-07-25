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
use crate::event::{AuditAction, AuditEvent};

/// Counts appended events, labelled by action and outcome. Emitted here;
/// described by the gateway's recorder (ADR-0007).
pub const AUDIT_EVENTS_TOTAL: &str = "synveda_audit_events_total";

/// Counts appends that failed on a best-effort path (deny-path emission
/// must never mask the original error — ADR-0019 decision 5). Emitted by
/// the seam that swallows the failure.
pub const AUDIT_APPEND_FAILURES_TOTAL: &str = "synveda_audit_append_failures_total";

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
    let occurred_at = truncate_to_micros(event.occurred_at);
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
    let hash = compute_hash(&head.head_hash, &canonical);

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
        &head.head_hash[..],
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

/// Walks `tenant`'s whole chain, recomputing every hash from the stored
/// columns (never trusting stored bytes as hash input), and reports the
/// first divergence. Deterministic and side-effect-free; one snapshot —
/// run it inside a tenant transaction.
#[tracing::instrument(name = "audit.verify", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn verify(conn: &mut PgConnection, tenant: TenantId) -> Result<ChainVerification> {
    let head = sqlx::query!(
        "select seq, head_hash from audit_chain_heads where tenant_id = $1",
        tenant.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read chain head", &err))?;

    let mut prev: Vec<u8> = genesis_hash(tenant).to_vec();
    let mut expected = 1i64;
    let mut outcome = None;

    'walk: loop {
        let rows = page(conn, tenant, expected - 1).await?;
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
    } else {
        match head {
            None if events == 0 => ChainVerification::Valid { events: 0 },
            None => ChainVerification::Broken {
                seq: events,
                reason: BreakReason::MissingHead,
            },
            Some(head) if head.seq == events && head.head_hash == prev => {
                ChainVerification::Valid { events }
            }
            Some(head) => ChainVerification::Broken {
                seq: head.seq,
                reason: BreakReason::Head,
            },
        }
    };

    metrics::counter!(
        AUDIT_VERIFICATIONS_TOTAL,
        "outcome" => match verification {
            ChainVerification::Valid { .. } => "valid",
            ChainVerification::Broken { .. } => "broken",
        },
    )
    .increment(1);
    Ok(verification)
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

/// One verification page: events with `seq > after`, ascending.
async fn page(conn: &mut PgConnection, tenant: TenantId, after: i64) -> Result<Vec<StoredEvent>> {
    sqlx::query_as!(
        StoredEvent,
        r#"select seq, occurred_at, actor_kind, actor_subject, action,
                  resource, outcome, payload, trace_id, prev_hash, hash
           from audit_log
           where tenant_id = $1 and seq > $2
           order by seq
           limit $3"#,
        tenant.as_uuid(),
        after,
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
}
