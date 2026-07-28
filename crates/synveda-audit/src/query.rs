//! Reading the chain: the search, the disclosure answer and the knowledge
//! answer (AUD-2, ADR-0045).
//!
//! Everything here reads and nothing decides. The PDP gates the routes
//! that call these functions (`AuditRead`, ADR-0045 decision 1), RLS bounds
//! them to the caller's tenant, and the indexes they plan against arrive in
//! migration 0028 — which adds no column, because a column inside the
//! canonical form would invalidate every row written since AUD-1 and one
//! outside it would be a field the chain does not protect (decision 7).
//!
//! Two properties are structural rather than checked:
//!
//! - **No content leaves here.** A [`Disclosure`] carries record ids,
//!   object addresses, channels, tiers and staleness — the shape of what
//!   was served, never the substance. An auditor reads no content
//!   (seed §5), and this module is the route by which they would otherwise
//!   acquire it (decision 6).
//! - **Absence is reported as absence.** Payload shapes have grown with the
//!   features that emit them: an entry written before FLOW-2 carries a
//!   `version_hash` and no `object_hash`, one written before AUTHZ-5 has no
//!   `tier`, and one written before MEM-6 has no staleness. Every extracted
//!   field is therefore an `Option` that stays `None` rather than taking a
//!   default. A default here would be this surface inventing a fact about
//!   the past, which is the one thing an audit answer must never do.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;
use synveda_types::{Error, RecordId, Result, TenantId};

use crate::chain::StoredEvent;
use crate::event::{AuditAction, Outcome};

/// The actions that record material being *served* to someone — the
/// disclosure half of "who could see X on date D" (ADR-0045 decision 4),
/// and the whole of "what did agent A know at time T" (decision 5).
///
/// This is exactly the predicate of migration 0028's partial GIN index; a
/// query that widens it stops using that index.
pub const DISCLOSURE_ACTIONS: [AuditAction; 2] =
    [AuditAction::ContextInjected, AuditAction::ContextRecalled];

/// The actions that open or close authority over a scope's material — the
/// *authority* half of "who could see X on date D" (ADR-0045 decision 4).
///
/// Deliberately a list of actions rather than a fold: this module hands
/// back the events, and the caller that knows the hierarchy assembles the
/// answer. Historical bindings, assignments and grants exist nowhere else —
/// `role_bindings`, `policy_pack_assignments` and `policy_lapses` are
/// current-state tables, and an unbound role leaves no row — so these
/// events are not merely the tamper-evident record of what governed a scope
/// in March, they are the only record.
///
/// Pass to [`search`] via [`EventFilter::actions`].
pub const AUTHORITY_ACTIONS: [AuditAction; 13] = [
    AuditAction::RoleBound,
    AuditAction::RoleUnbound,
    AuditAction::PolicyDefaultSet,
    AuditAction::PolicyDefaultCleared,
    AuditAction::PolicyNodeAssigned,
    AuditAction::PolicyNodeUnassigned,
    AuditAction::LapseGranted,
    AuditAction::LapseRevoked,
    AuditAction::LapseExpired,
    AuditAction::MemoryClassified,
    AuditAction::ChannelPublished,
    AuditAction::ChannelRolledBack,
    AuditAction::ChannelPinned,
];

/// Where the chain stood when an answer was taken (ADR-0045 decision 9).
///
/// Every answer carries one, so a finding can be re-derived by someone who
/// does not trust the auditor who found it, and an answer taken before an
/// append can be told from one taken after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainFrame {
    /// The chain head's sequence number; 0 for a tenant with no events.
    pub head_seq: i64,
    /// The head hash — the genesis hash when `head_seq` is 0.
    pub head_hash: Vec<u8>,
}

/// One page of an answer, stamped with the chain it was taken against.
#[derive(Debug, Clone)]
pub struct Page<T> {
    /// The rows, oldest first.
    pub items: Vec<T>,
    /// The chain head when the page was read.
    pub frame: ChainFrame,
    /// The lowest seq in `items`.
    pub first_seq: Option<i64>,
    /// The highest seq in `items`.
    pub last_seq: Option<i64>,
    /// `Some(seq)` when the page hit its limit and more rows match — pass
    /// it back as `after`. `None` means the answer is complete for its
    /// filter. A page that ran out of rows says so rather than ending
    /// quietly (ADR-0045 decision 9).
    pub next_cursor: Option<i64>,
}

impl<T> Page<T> {
    /// Whether the limit truncated the answer.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.next_cursor.is_some()
    }
}

/// The search route's parameters (ADR-0045 decision 3).
///
/// `resource` matches the chain's `resource` column **exactly**, and that
/// column is a display string by AUD-1's specification — `"scope <uuid>"`,
/// `"tenant <uuid>"`, `"scope none"` for an unplaced caller, and hand-built
/// strings for actions whose subject is not a policy resource. Callers
/// format what they are looking for; nothing here parses it, because a
/// parse that succeeded for some actions and failed for others would
/// silently omit rows (ADR-0045 decision 2).
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Restrict to one acting subject.
    pub actor_subject: Option<String>,
    /// Restrict to these actions; empty means every action.
    pub actions: Vec<AuditAction>,
    /// Restrict to one outcome — `deny` is a filter value like any other,
    /// which is what "read-only incl. denials" asks for.
    pub outcome: Option<Outcome>,
    /// Exact match on the recorded resource string.
    pub resource: Option<String>,
    /// Inclusive lower bound on `occurred_at`.
    pub from: Option<DateTime<Utc>>,
    /// Exclusive upper bound on `occurred_at`.
    pub until: Option<DateTime<Utc>>,
}

/// The chain head, for stamping an answer.
#[tracing::instrument(name = "audit.frame", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn frame(conn: &mut PgConnection, tenant: TenantId) -> Result<ChainFrame> {
    let row = sqlx::query!(
        "select seq, head_hash from audit_chain_heads where tenant_id = $1",
        tenant.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit chain frame", &err))?;

    Ok(match row {
        Some(row) => ChainFrame {
            head_seq: row.seq,
            head_hash: row.head_hash,
        },
        // No head row means no chain yet: the frame is genesis, which is
        // what a first append will link to.
        None => ChainFrame {
            head_seq: 0,
            head_hash: crate::chain::genesis_hash(tenant).to_vec(),
        },
    })
}

/// Events matching `filter` with `seq > after`, oldest first.
///
/// Cursor-paginated on `seq` rather than offset-paginated because the chain
/// grows underneath a reader who is reading it — not least because reading
/// it appends an `authz.decision` of its own (ADR-0045 decision 8). `seq`
/// is 1-based and contiguous per tenant (a gap is a verification failure),
/// so a single integer is an unambiguous cursor.
#[tracing::instrument(
    name = "audit.search",
    skip_all,
    fields(tenant.id = %tenant, after = after, limit = limit),
    err(Display)
)]
pub async fn search(
    conn: &mut PgConnection,
    tenant: TenantId,
    filter: &EventFilter,
    after: i64,
    limit: i64,
) -> Result<Page<StoredEvent>> {
    let frame = frame(&mut *conn, tenant).await?;
    let actions: Option<Vec<String>> = (!filter.actions.is_empty()).then(|| {
        filter
            .actions
            .iter()
            .map(|action| action.as_str().to_owned())
            .collect()
    });

    // One statement with nullable predicates rather than SQL assembled from
    // parts: compile-time checked queries only (CLAUDE.md), and a filter
    // combination that never runs is still a filter combination the
    // compiler has verified.
    let events = sqlx::query_as!(
        StoredEvent,
        r#"select seq, occurred_at, actor_kind, actor_subject, action,
                  resource, outcome, payload, trace_id, prev_hash, hash
           from audit_log
           where tenant_id = $1
             and seq > $2
             and ($3::text is null or actor_subject = $3)
             and ($4::text[] is null or action = any($4))
             and ($5::text is null or outcome = $5)
             and ($6::text is null or resource = $6)
             and ($7::timestamptz is null or occurred_at >= $7)
             and ($8::timestamptz is null or occurred_at < $8)
           order by seq
           limit $9"#,
        tenant.as_uuid(),
        after,
        filter.actor_subject.as_deref(),
        actions.as_deref(),
        filter.outcome.map(Outcome::as_str),
        filter.resource.as_deref(),
        filter.from,
        filter.until,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("search audit events", &err))?;

    Ok(page(events, frame, limit, |event| event.seq))
}

/// One record being served to one subject, as the chain recorded it.
///
/// A single event discloses many records, so an event with four entries and
/// a matching id yields one `Disclosure` for that id — the answer is per
/// (reader, record, occasion), which is what "who could see X" asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disclosure {
    /// Position in the chain — the evidence anyone can re-read.
    pub seq: i64,
    /// When the block was composed.
    pub occurred_at: DateTime<Utc>,
    /// Attribution strength (`subject` / `break_glass` / `system`).
    pub actor_kind: String,
    /// Who was served.
    pub actor_subject: String,
    /// `context.injected` or `context.recalled` — being *given* material
    /// and *asking for* it are different acts, so the answer keeps them
    /// apart rather than merging them into "saw".
    pub action: String,
    /// The session the block went to, when the caller named one.
    pub session_id: Option<String>,
    /// What was served.
    pub entry: DisclosedEntry,
}

/// The entry a disclosure carries — the shape of what was served, never
/// its substance (ADR-0045 decision 6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisclosedEntry {
    /// The record served.
    pub record_id: String,
    /// The VedaFlow object address of exactly the version that composed
    /// (ADR-0031 decision 11). `None` on entries written before FLOW-2.
    pub object_hash: Option<String>,
    /// The BLAKE3 version hash CTX-2 watermarked with (ADR-0025
    /// decision 7), on entries old enough to predate the object address.
    /// Kept distinct rather than folded into `object_hash`: a content
    /// address and a version hash are different claims, and reporting one
    /// as the other would be this surface inventing a fact.
    pub version_hash: Option<String>,
    /// The channel it composed from — the trust label an auditor reads
    /// before the content they are not going to get.
    pub channel: Option<String>,
    /// The sensitivity tier it was served at (ADR-0041 decision 9).
    pub tier: Option<String>,
    /// Staleness as integer per mille (ADR-0040); never a float, because
    /// audit canonicalisation refuses one.
    pub staleness_permille: Option<i64>,
}

/// Every disclosure of `record` in `[from, until)`, oldest first — the
/// evidentiary half of "who could see X on date D" (ADR-0045 decision 4).
///
/// The other half is the authority that governed the record's scope that
/// day: [`search`] with [`AUTHORITY_ACTIONS`]. The two are deliberately not
/// merged here — merging them means deciding, and deciding over
/// reconstructed inputs is the replay ADR-0042 option 5 rejected.
///
/// A record that does not exist, belongs to another tenant, or has been
/// disposed of under MEM-6 all answer the same empty answer: the surface is
/// not an existence oracle (ADR-0041 decision 6's shape, for the same
/// reason).
#[tracing::instrument(
    name = "audit.disclosures",
    skip_all,
    fields(tenant.id = %tenant, record.id = %record, limit = limit),
    err(Display)
)]
pub async fn disclosures(
    conn: &mut PgConnection,
    tenant: TenantId,
    record: RecordId,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
    after: i64,
    limit: i64,
) -> Result<Page<Disclosure>> {
    let frame = frame(&mut *conn, tenant).await?;
    let containment = json!({ "entries": [{ "record_id": record.to_string() }] });

    // The action list is spelled out rather than parameterised so it
    // matches migration 0028's partial index predicate exactly — a
    // parameterised `= any($n)` would not let the planner prove the
    // partial index applies.
    let rows = sqlx::query!(
        r#"select seq, occurred_at, actor_kind, actor_subject, action, payload
           from audit_log
           where tenant_id = $1
             and action in ('context.injected', 'context.recalled')
             and payload @> $2::jsonb
             and seq > $3
             and occurred_at >= $4
             and occurred_at < $5
           order by seq
           limit $6"#,
        tenant.as_uuid(),
        containment,
        after,
        from,
        until,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit disclosures", &err))?;

    // Truncation is a fact about the rows the limit returned, not about
    // the disclosures they yielded: the filter below can drop a row, and a
    // full page that lost one to it is still a full page. Computing this
    // after the filter would report a truncated answer as complete, which
    // is the one thing decision 9 exists to prevent.
    let truncated = i64::try_from(rows.len()).unwrap_or(i64::MAX) >= limit;
    let last_read = rows.last().map(|row| row.seq);

    let target = record.to_string();
    let items = rows
        .into_iter()
        .filter_map(|row| {
            // Containment matched the event; this picks the entry out of
            // it. An event whose match came from somewhere other than an
            // entry's `record_id` yields nothing rather than a disclosure
            // with a guessed entry.
            entry_for(&row.payload, &target).map(|entry| Disclosure {
                seq: row.seq,
                occurred_at: row.occurred_at,
                actor_kind: row.actor_kind,
                actor_subject: row.actor_subject,
                action: row.action,
                session_id: string_field(&row.payload, "session_id"),
                entry,
            })
        })
        .collect::<Vec<_>>();

    Ok(Page {
        // The cursor is the last seq *read*, not the last one yielded, so
        // a page whose final rows all filtered out still advances.
        next_cursor: truncated.then(|| last_read.unwrap_or(0)),
        first_seq: items.first().map(|disclosure| disclosure.seq),
        last_seq: items.last().map(|disclosure| disclosure.seq),
        items,
        frame,
    })
}

/// Every disclosure to `subject` at or before `at`, oldest first — the
/// evidence "what did agent A know at time T" is folded from (ADR-0045
/// decision 5).
///
/// Read newest-first internally and reversed, so a chain longer than
/// `limit` loses its *oldest* disclosures rather than its most recent
/// ones: the question is what A knew at T, and the answer degrades toward
/// recency rather than away from it. The page says it was truncated either
/// way.
#[tracing::instrument(
    name = "audit.knowledge",
    skip_all,
    fields(tenant.id = %tenant, subject = subject, limit = limit),
    err(Display)
)]
pub async fn knowledge(
    conn: &mut PgConnection,
    tenant: TenantId,
    subject: &str,
    at: DateTime<Utc>,
    limit: i64,
) -> Result<Page<Disclosure>> {
    let frame = frame(&mut *conn, tenant).await?;
    let rows = sqlx::query!(
        r#"select seq, occurred_at, actor_kind, actor_subject, action, payload
           from audit_log
           where tenant_id = $1
             and action in ('context.injected', 'context.recalled')
             and actor_subject = $2
             and occurred_at <= $3
           order by seq desc
           limit $4"#,
        tenant.as_uuid(),
        subject,
        at,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit knowledge", &err))?;

    let truncated = i64::try_from(rows.len()).unwrap_or(i64::MAX) >= limit;
    let mut items = rows
        .into_iter()
        .flat_map(|row| {
            let session_id = string_field(&row.payload, "session_id");
            entries(&row.payload)
                .into_iter()
                .map(move |entry| Disclosure {
                    seq: row.seq,
                    occurred_at: row.occurred_at,
                    actor_kind: row.actor_kind.clone(),
                    actor_subject: row.actor_subject.clone(),
                    action: row.action.clone(),
                    session_id: session_id.clone(),
                    entry,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    items.sort_by_key(|disclosure| disclosure.seq);

    let first_seq = items.first().map(|disclosure| disclosure.seq);
    let last_seq = items.last().map(|disclosure| disclosure.seq);
    Ok(Page {
        // Truncation here drops the *oldest* events, so the cursor is the
        // lowest seq reached: a caller who wants more walks backwards from
        // it, which is the opposite direction to [`search`] and is why this
        // function assembles its own page rather than calling `page`.
        next_cursor: truncated.then(|| first_seq.unwrap_or(0)),
        items,
        frame,
        first_seq,
        last_seq,
    })
}

/// What one subject was last served of one record — the fold behind "what
/// did agent A know at time T" (ADR-0045 decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The version last delivered, with its address and labels.
    pub entry: DisclosedEntry,
    /// The chain position of the last delivery.
    pub seq: i64,
    /// When it was last delivered.
    pub occurred_at: DateTime<Utc>,
    /// `context.injected` or `context.recalled` — how it arrived that
    /// last time.
    pub action: String,
    /// How many times it was served in the window read.
    pub occasions: usize,
}

/// Fold disclosures to one row per record: the version *last* delivered at
/// or before the instant asked at, with the number of occasions behind it.
///
/// Last-wins by `seq`, which is the chain's own order — not by
/// `occurred_at`, which two events can share at microsecond precision.
/// Deterministic for a given input, so the same evidence folds the same way
/// for whoever re-derives it.
#[must_use]
pub fn fold_knowledge(disclosures: &[Disclosure]) -> Vec<Known> {
    let mut known: Vec<Known> = Vec::new();
    for disclosure in disclosures {
        match known
            .iter_mut()
            .find(|item| item.entry.record_id == disclosure.entry.record_id)
        {
            Some(item) => {
                item.occasions += 1;
                if disclosure.seq >= item.seq {
                    item.entry = disclosure.entry.clone();
                    item.seq = disclosure.seq;
                    item.occurred_at = disclosure.occurred_at;
                    item.action = disclosure.action.clone();
                }
            }
            None => known.push(Known {
                entry: disclosure.entry.clone(),
                seq: disclosure.seq,
                occurred_at: disclosure.occurred_at,
                action: disclosure.action.clone(),
                occasions: 1,
            }),
        }
    }
    known.sort_by_key(|item| item.seq);
    known
}

/// Assemble a page and its cursor from rows already read.
fn page<T>(items: Vec<T>, frame: ChainFrame, limit: i64, seq_of: impl Fn(&T) -> i64) -> Page<T> {
    let first_seq = items.first().map(&seq_of);
    let last_seq = items.last().map(&seq_of);
    let truncated = i64::try_from(items.len()).unwrap_or(i64::MAX) >= limit;
    Page {
        next_cursor: truncated.then(|| last_seq.unwrap_or(0)),
        items,
        frame,
        first_seq,
        last_seq,
    }
}

/// Every entry of a disclosure payload, in the order the block composed
/// them. An event with no `entries` array — an empty block, a payload shape
/// this build does not know — yields none rather than an invented one.
fn entries(payload: &Value) -> Vec<DisclosedEntry> {
    payload
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(entry_from).collect())
        .unwrap_or_default()
}

/// The entry naming `record_id`, if this payload has one.
fn entry_for(payload: &Value, record_id: &str) -> Option<DisclosedEntry> {
    entries(payload)
        .into_iter()
        .find(|entry| entry.record_id == record_id)
}

/// One entry, read defensively: every field that is absent stays absent.
fn entry_from(value: &Value) -> Option<DisclosedEntry> {
    Some(DisclosedEntry {
        record_id: string_field(value, "record_id")?,
        object_hash: string_field(value, "object_hash"),
        version_hash: string_field(value, "version_hash"),
        channel: string_field(value, "channel"),
        tier: string_field(value, "tier"),
        staleness_permille: value.get("staleness_permille").and_then(Value::as_i64),
    })
}

/// A string field, or `None` — including when it is present but not a
/// string, which is a payload this build does not understand rather than a
/// value to coerce.
fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn storage_error(context: &str, err: &sqlx::Error) -> Error {
    Error::Storage {
        message: format!("{context}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disclosure(seq: i64, record: &str, object_hash: Option<&str>) -> Disclosure {
        Disclosure {
            seq,
            occurred_at: DateTime::from_timestamp(1_700_000_000 + seq, 0).expect("valid instant"),
            actor_kind: "subject".to_owned(),
            actor_subject: "alice".to_owned(),
            action: "context.injected".to_owned(),
            session_id: None,
            entry: DisclosedEntry {
                record_id: record.to_owned(),
                object_hash: object_hash.map(ToOwned::to_owned),
                ..DisclosedEntry::default()
            },
        }
    }

    #[test]
    fn the_fold_keeps_the_last_version_delivered_and_counts_the_occasions() {
        let folded = fold_knowledge(&[
            disclosure(1, "rec-a", Some("hash-1")),
            disclosure(2, "rec-b", Some("hash-b")),
            disclosure(3, "rec-a", Some("hash-2")),
        ]);

        assert_eq!(folded.len(), 2, "one row per record, not per delivery");
        let rec_a = folded
            .iter()
            .find(|item| item.entry.record_id == "rec-a")
            .expect("rec-a folded");
        assert_eq!(
            rec_a.entry.object_hash.as_deref(),
            Some("hash-2"),
            "the version last delivered wins, not the first"
        );
        assert_eq!(rec_a.occasions, 2);
        assert_eq!(rec_a.seq, 3);
    }

    #[test]
    fn the_fold_is_ordered_by_the_chain_and_not_by_the_clock() {
        // Two deliveries sharing an instant: seq decides, because
        // `occurred_at` is microsecond-truncated and two appends can
        // share one.
        let mut first = disclosure(7, "rec-a", Some("older"));
        let mut second = disclosure(8, "rec-a", Some("newer"));
        let shared = DateTime::from_timestamp(1_700_000_000, 0).expect("valid instant");
        first.occurred_at = shared;
        second.occurred_at = shared;

        let folded = fold_knowledge(&[second, first]);

        assert_eq!(folded.len(), 1);
        assert_eq!(
            folded[0].entry.object_hash.as_deref(),
            Some("newer"),
            "seq 8 is later than seq 7 whatever the timestamps say"
        );
    }

    #[test]
    fn an_entry_predating_flow_2_keeps_its_version_hash_and_gains_no_object_address() {
        // A real shape from the chain: CTX-2 watermarked with
        // `version_hash`, and FLOW-2 replaced it with the object address.
        let payload = json!({
            "entries": [{"record_id": "rec-a", "version_hash": "1602..."}],
        });

        let entry = entry_for(&payload, "rec-a").expect("the entry is found");

        assert_eq!(entry.version_hash.as_deref(), Some("1602..."));
        assert_eq!(
            entry.object_hash, None,
            "a version hash is not a content address and is never reported as one"
        );
        assert_eq!(entry.channel, None, "absent is absent, never a default");
        assert_eq!(entry.tier, None);
        assert_eq!(entry.staleness_permille, None);
    }

    #[test]
    fn a_current_entry_carries_every_label_the_chain_recorded() {
        let payload = json!({
            "session_id": "s-1",
            "entries": [{
                "record_id": "rec-a",
                "object_hash": "abcd",
                "channel": "acme/eng/published",
                "tier": "internal",
                "staleness_permille": 250,
            }],
        });

        let entry = entry_for(&payload, "rec-a").expect("the entry is found");

        assert_eq!(entry.object_hash.as_deref(), Some("abcd"));
        assert_eq!(entry.channel.as_deref(), Some("acme/eng/published"));
        assert_eq!(entry.tier.as_deref(), Some("internal"));
        assert_eq!(entry.staleness_permille, Some(250));
        assert_eq!(string_field(&payload, "session_id").as_deref(), Some("s-1"));
    }

    #[test]
    fn a_payload_without_entries_discloses_nothing() {
        // The empty block CTX-3 serves a quarantined or unplaced caller is
        // still audited, and it disclosed nothing.
        assert!(entries(&json!({"entries": []})).is_empty());
        assert!(entries(&json!({"block_hash": "abcd"})).is_empty());
        assert!(entries(&json!({"entries": "not-an-array"})).is_empty());
        assert!(entry_for(&json!({"entries": [{"tier": "internal"}]}), "rec-a").is_none());
    }

    #[test]
    fn a_field_of_the_wrong_type_reads_as_absent_rather_than_coerced() {
        let payload = json!({
            "entries": [{"record_id": "rec-a", "tier": 3, "staleness_permille": "250"}],
        });

        let entry = entry_for(&payload, "rec-a").expect("the entry is found");

        assert_eq!(entry.tier, None, "a number is not a tier");
        assert_eq!(
            entry.staleness_permille, None,
            "a string is not an integer per mille"
        );
    }

    #[test]
    fn the_disclosure_actions_are_exactly_the_partial_index_predicate() {
        // Migration 0028's index is partial on these two action names; a
        // rename here without a migration silently stops using it.
        let names: Vec<&str> = DISCLOSURE_ACTIONS
            .iter()
            .map(|action| action.as_str())
            .collect();
        assert_eq!(names, ["context.injected", "context.recalled"]);
    }
}
