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
//! - **No content leaves here.** A [`Disclosure`] carries stable Knowledge
//!   and immutable revision ids, content hashes and reason codes — the shape
//!   of what was served, never the substance. An auditor reads no content
//!   (seed §5), and this module is the route by which they would otherwise
//!   acquire it (decision 6).
//! - **Absence is reported as absence.** A hashes-only or disabled trace may
//!   omit stable addresses. Every extracted address is therefore optional and
//!   stays absent rather than taking a default. A default here would invent a
//!   fact about the past, which is the one thing an audit answer must never do.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;
use synveda_types::{Error, KnowledgeItemId, Result, TenantId};

use crate::chain::StoredEvent;
use crate::event::{AuditAction, Outcome};

/// The actions that record material being *served* to someone — the
/// disclosure half of "who could see X on date D" (ADR-0045 decision 4),
/// and the whole of "what did agent A know at time T" (decision 5).
///
/// This is exactly the predicate of migration 0028's partial GIN index; a
/// query that widens it stops using that index.
/// `session.context.composed` joined the set with the observe cutover
/// (CPR-12, ADR-0078 decision 5): `/v1/inject` and `/v1/recall` are deleted, so
/// a context run is the **only** way material reaches an agent, and a
/// disclosure query that did not count it would answer "nobody was served
/// anything" about every deployment on the new plane.
pub const DISCLOSURE_ACTIONS: [AuditAction; 1] = [AuditAction::SessionContextComposed];

/// The actions that open or close authority over a scope's material — the
/// *authority* half of "who could see X on date D" (ADR-0045 decision 4).
///
/// Deliberately a list of actions rather than a fold: this module hands
/// back the events, and the caller that knows the scope tree assembles the
/// answer. Historical grants, Configuration changes and immutable relaxation
/// transitions must be reconstructed from the chain rather than inferred
/// from current heads, so these events are not merely tamper evidence: they
/// are the transaction-time record.
///
/// Pass to [`search`] via [`EventFilter::actions`].
pub const AUTHORITY_ACTIONS: [AuditAction; 10] = [
    AuditAction::AccessGranted,
    AuditAction::AccessRevoked,
    AuditAction::ConfigurationChangeApplied,
    AuditAction::CuratorRulesUpdated,
    AuditAction::RelaxationChangeApplied,
    AuditAction::RelaxationExpired,
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
    /// Exact JSON containment predicate over the canonical payload.
    ///
    /// The gateway constructs this only from validated typed artifact,
    /// session and context-run identifiers. Keeping the predicate structured
    /// means no query interprets display strings or searches arbitrary JSON
    /// text (CPR-33, ADR-0092 decision 1).
    pub payload_contains: Option<Value>,
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
             and ($9::jsonb is null or payload @> $9)
           order by seq
           limit $10"#,
        tenant.as_uuid(),
        after,
        filter.actor_subject.as_deref(),
        actions.as_deref(),
        filter.outcome.map(Outcome::as_str),
        filter.resource.as_deref(),
        filter.from,
        filter.until,
        filter.payload_contains.as_ref(),
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("search audit events", &err))?;

    Ok(page(events, frame, limit, |event| event.seq))
}

/// One Knowledge revision being served to one subject, as the chain recorded it.
///
/// A single event discloses many revisions, so an event with four entries and
/// a matching id yields one `Disclosure` for that id — the answer is per
/// (reader, Knowledge item, occasion), which is what "who could see X" asks for.
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
    /// The delivery act (`session.context.composed`) that put the immutable
    /// revision in the session context.
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
    /// Stable Knowledge aggregate served. Absent in hashes-only retention.
    pub knowledge_item_id: Option<String>,
    /// Exact immutable revision served. Absent in hashes-only retention.
    pub knowledge_revision_id: Option<String>,
    /// Canonical revision content hash.
    pub content_hash: Option<String>,
    /// Planner reason vocabulary, content-free and order-preserving.
    pub reason_codes: Vec<String>,
}

/// Every disclosure of `knowledge_item` in `[from, until)`, oldest first — the
/// evidentiary half of "who could see X on date D" (ADR-0045 decision 4).
///
/// The other half is the authority that governed the item's scope that
/// day: [`search`] with [`AUTHORITY_ACTIONS`]. The two are deliberately not
/// merged here — merging them means deciding, and deciding over
/// reconstructed inputs is the replay ADR-0042 option 5 rejected.
///
/// An item that does not exist, belongs to another tenant, or was never served
/// answers the same empty result: the surface is not an existence oracle.
#[tracing::instrument(
    name = "audit.disclosures",
    skip_all,
    fields(tenant.id = %tenant, knowledge.item.id = %knowledge_item, limit = limit),
    err(Display)
)]
pub async fn disclosures(
    conn: &mut PgConnection,
    tenant: TenantId,
    knowledge_item: KnowledgeItemId,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
    after: i64,
    limit: i64,
) -> Result<Page<Disclosure>> {
    let frame = frame(&mut *conn, tenant).await?;
    let containment = json!({ "knowledge": [{ "knowledge_item_id": knowledge_item.to_string() }] });

    // The action list is spelled out rather than parameterised so it
    // matches migration 0028's partial index predicate exactly — a
    // parameterised `= any($n)` would not let the planner prove the
    // partial index applies.
    let rows = sqlx::query!(
        r#"select seq, occurred_at, actor_kind, actor_subject, action, payload
           from audit_log
           where tenant_id = $1
             and action = 'session.context.composed'
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

    let target = knowledge_item.to_string();
    let items = rows
        .into_iter()
        .filter_map(|row| {
            // Containment matched the event; this picks the entry out of
            // it. An event whose match came from somewhere other than an
            // entry's `knowledge_item_id` yields nothing rather than a disclosure
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
    before: i64,
    limit: i64,
) -> Result<Page<Disclosure>> {
    let frame = frame(&mut *conn, tenant).await?;
    let fetch = limit.checked_add(1).ok_or_else(|| Error::Invalid {
        message: "audit Knowledge limit is too large".to_owned(),
    })?;
    let mut rows = sqlx::query!(
        r#"select seq, occurred_at, actor_kind, actor_subject, action, payload
           from audit_log
           where tenant_id = $1
             and action = 'session.context.composed'
             and actor_subject = $2
             and occurred_at <= $3
             and seq < $4
           order by seq desc
           limit $5"#,
        tenant.as_uuid(),
        subject,
        at,
        before,
        fetch,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read audit knowledge", &err))?;

    let truncated = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    if truncated {
        rows.pop();
    }
    let lowest_read = rows.last().map(|row| row.seq);
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
        next_cursor: truncated.then(|| lowest_read.unwrap_or(0)),
        items,
        frame,
        first_seq,
        last_seq,
    })
}

/// One cursor page from a frozen contiguous chain prefix.
///
/// `through` is the prefix head captured by the first request. Later pages
/// repeat it, so audit-read events appended while walking the export can never
/// enter the artifact being assembled (CPR-33, ADR-0092 decision 4).
#[tracing::instrument(
    name = "audit.export_page",
    skip_all,
    fields(tenant.id = %tenant, after = after, through = ?through, limit = limit),
    err(Display)
)]
pub async fn export_page(
    conn: &mut PgConnection,
    tenant: TenantId,
    after: i64,
    through: Option<i64>,
    limit: i64,
) -> Result<Page<StoredEvent>> {
    let live = frame(&mut *conn, tenant).await?;
    let through = through.unwrap_or(live.head_seq);
    if through < 0 || through > live.head_seq {
        return Err(Error::Invalid {
            message: format!(
                "audit export through must be between 0 and the current head {}",
                live.head_seq
            ),
        });
    }
    if after < 0 || after > through {
        return Err(Error::Invalid {
            message: format!("audit export after must be between 0 and through {through}"),
        });
    }

    let snapshot_hash = if through == 0 {
        crate::chain::genesis_hash(tenant).to_vec()
    } else {
        sqlx::query_scalar!(
            "select hash from audit_log where tenant_id = $1 and seq = $2",
            tenant.as_uuid(),
            through,
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(|err| storage_error("read audit export snapshot hash", &err))?
        .ok_or_else(|| Error::Storage {
            message: format!("audit chain has no row at frozen head {through}"),
        })?
    };

    // Fetch one look-ahead row so `truncated` means another row definitely
    // exists inside this frozen prefix, rather than merely that the page was
    // exactly full.
    let fetch = limit.checked_add(1).ok_or_else(|| Error::Invalid {
        message: "audit export limit is too large".to_owned(),
    })?;
    let mut events = sqlx::query_as!(
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
        fetch,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|err| storage_error("read frozen audit export page", &err))?;
    let truncated = i64::try_from(events.len()).unwrap_or(i64::MAX) > limit;
    if truncated {
        events.pop();
    }
    let first_seq = events.first().map(|event| event.seq);
    let last_seq = events.last().map(|event| event.seq);
    Ok(Page {
        next_cursor: truncated.then_some(last_seq.unwrap_or(after)),
        items: events,
        frame: ChainFrame {
            head_seq: through,
            head_hash: snapshot_hash,
        },
        first_seq,
        last_seq,
    })
}

/// What one subject was last served of one Knowledge item — the fold behind "what
/// did agent A know at time T" (ADR-0045 decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The version last delivered, with its address and labels.
    pub entry: DisclosedEntry,
    /// The chain position of the last delivery.
    pub seq: i64,
    /// When it was last delivered.
    pub occurred_at: DateTime<Utc>,
    /// The delivery act (`session.context.composed`) from the last occasion.
    pub action: String,
    /// How many times it was served in the window read.
    pub occasions: usize,
}

/// Fold disclosures to one row per retained Knowledge identity: the revision
/// *last* delivered at or before the instant asked at, with the number of
/// occasions behind it. Full/redacted evidence is keyed by stable item ID;
/// hashes-only evidence is keyed by the retained content hash and never grows a
/// synthetic item ID.
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
            .find(|item| same_retained_identity(&item.entry, &disclosure.entry))
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

/// Every Knowledge entry in a disclosure payload, in delivery order. An event
/// with no `knowledge` array — an empty block, a payload shape
/// this build does not know — yields none rather than an invented one.
fn entries(payload: &Value) -> Vec<DisclosedEntry> {
    payload
        .get("knowledge")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(entry_from).collect())
        .unwrap_or_default()
}

/// The entry naming `knowledge_item_id`, if this payload has one.
fn entry_for(payload: &Value, knowledge_item_id: &str) -> Option<DisclosedEntry> {
    entries(payload)
        .into_iter()
        .find(|entry| entry.knowledge_item_id.as_deref() == Some(knowledge_item_id))
}

/// One entry, read defensively: every field that is absent stays absent. A
/// hashes-only trace is still evidence, so a content hash is sufficient to
/// retain the entry; a value carrying neither an address nor a hash is not.
fn entry_from(value: &Value) -> Option<DisclosedEntry> {
    let knowledge_item_id = string_field(value, "knowledge_item_id");
    let content_hash = string_field(value, "content_hash");
    if knowledge_item_id.is_none() && content_hash.is_none() {
        return None;
    }
    Some(DisclosedEntry {
        knowledge_item_id,
        knowledge_revision_id: string_field(value, "knowledge_revision_id"),
        content_hash,
        reason_codes: value
            .get("reason_codes")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Compare only evidence the configured retention mode actually kept. Item IDs
/// dominate when present; addressless hashes-only rows can only be correlated
/// by their content hash.
fn same_retained_identity(left: &DisclosedEntry, right: &DisclosedEntry) -> bool {
    match (
        left.knowledge_item_id.as_deref(),
        right.knowledge_item_id.as_deref(),
    ) {
        (Some(left), Some(right)) => left == right,
        (None, None) => left.content_hash == right.content_hash,
        _ => false,
    }
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

    fn disclosure(seq: i64, item: &str, revision: Option<&str>) -> Disclosure {
        Disclosure {
            seq,
            occurred_at: DateTime::from_timestamp(1_700_000_000 + seq, 0).expect("valid instant"),
            actor_kind: "subject".to_owned(),
            actor_subject: "alice".to_owned(),
            action: "session.context.composed".to_owned(),
            session_id: None,
            entry: DisclosedEntry {
                knowledge_item_id: Some(item.to_owned()),
                knowledge_revision_id: revision.map(ToOwned::to_owned),
                ..DisclosedEntry::default()
            },
        }
    }

    #[test]
    fn the_fold_keeps_the_last_version_delivered_and_counts_the_occasions() {
        let folded = fold_knowledge(&[
            disclosure(1, "item-a", Some("revision-1")),
            disclosure(2, "item-b", Some("revision-b")),
            disclosure(3, "item-a", Some("revision-2")),
        ]);

        assert_eq!(folded.len(), 2, "one row per item, not per delivery");
        let rec_a = folded
            .iter()
            .find(|item| item.entry.knowledge_item_id.as_deref() == Some("item-a"))
            .expect("item-a folded");
        assert_eq!(
            rec_a.entry.knowledge_revision_id.as_deref(),
            Some("revision-2"),
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
        let mut first = disclosure(7, "item-a", Some("older"));
        let mut second = disclosure(8, "item-a", Some("newer"));
        let shared = DateTime::from_timestamp(1_700_000_000, 0).expect("valid instant");
        first.occurred_at = shared;
        second.occurred_at = shared;

        let folded = fold_knowledge(&[second, first]);

        assert_eq!(folded.len(), 1);
        assert_eq!(
            folded[0].entry.knowledge_revision_id.as_deref(),
            Some("newer"),
            "seq 8 is later than seq 7 whatever the timestamps say"
        );
    }

    #[test]
    fn hashes_only_absence_stays_absent() {
        let payload = json!({
            "knowledge": [{"content_hash": "1602..."}],
        });

        let entry = entries(&payload).pop().expect("hash evidence is retained");

        assert_eq!(entry.knowledge_item_id, None);
        assert_eq!(entry.content_hash.as_deref(), Some("1602..."));
        assert_eq!(entry.knowledge_revision_id, None);
        assert!(entry.reason_codes.is_empty());
    }

    #[test]
    fn hashes_only_entries_fold_without_inventing_an_address() {
        let mut first = disclosure(1, "discarded-address", None);
        first.entry.knowledge_item_id = None;
        first.entry.content_hash = Some("same-content".to_owned());
        let mut second = disclosure(2, "discarded-address", None);
        second.entry.knowledge_item_id = None;
        second.entry.content_hash = Some("same-content".to_owned());

        let folded = fold_knowledge(&[first, second]);

        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].entry.knowledge_item_id, None);
        assert_eq!(
            folded[0].entry.content_hash.as_deref(),
            Some("same-content")
        );
        assert_eq!(folded[0].occasions, 2);
    }

    #[test]
    fn a_current_entry_carries_every_label_the_chain_recorded() {
        let payload = json!({
            "session_id": "s-1",
            "knowledge": [{
                "knowledge_item_id": "item-a",
                "knowledge_revision_id": "revision-a",
                "content_hash": "abcd",
                "reason_codes": ["keyword_match", "freshness_boost"],
            }],
        });

        let entry = entry_for(&payload, "item-a").expect("the entry is found");

        assert_eq!(entry.knowledge_revision_id.as_deref(), Some("revision-a"));
        assert_eq!(entry.content_hash.as_deref(), Some("abcd"));
        assert_eq!(
            entry.reason_codes,
            ["keyword_match".to_owned(), "freshness_boost".to_owned()]
        );
        assert_eq!(string_field(&payload, "session_id").as_deref(), Some("s-1"));
    }

    #[test]
    fn a_payload_without_entries_discloses_nothing() {
        assert!(entries(&json!({"knowledge": []})).is_empty());
        assert!(entries(&json!({"block_hash": "abcd"})).is_empty());
        assert!(entries(&json!({"knowledge": "not-an-array"})).is_empty());
        assert!(entries(&json!({"knowledge": [{}]})).is_empty());
        assert!(entry_for(&json!({"knowledge": [{"content_hash": "abcd"}]}), "item-a").is_none());
    }

    #[test]
    fn a_field_of_the_wrong_type_reads_as_absent_rather_than_coerced() {
        let payload = json!({
            "knowledge": [{
                "knowledge_item_id": "item-a",
                "knowledge_revision_id": 3,
                "reason_codes": "keyword_match"
            }],
        });

        let entry = entry_for(&payload, "item-a").expect("the entry is found");

        assert_eq!(entry.knowledge_revision_id, None, "a number is not an id");
        assert!(entry.reason_codes.is_empty(), "a string is not an array");
    }

    #[test]
    fn the_disclosure_actions_are_exactly_the_partial_index_predicate() {
        // The index is partial on exactly these action names — migration
        // 0028's originally, rebuilt by 0046 when the observe cutover made a
        // context run the only way material reaches anybody. A rename or an
        // addition here without a matching migration silently stops using it.
        let names: Vec<&str> = DISCLOSURE_ACTIONS
            .iter()
            .map(|action| action.as_str())
            .collect();
        assert_eq!(names, ["session.context.composed"]);
    }
}
