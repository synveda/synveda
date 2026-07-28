//! The graph-linking stage (GRPH-2, ADR-0044): records → entities →
//! episodes, resolved against the vertices that already exist.
//!
//! This runs as a step of the extraction commit, not as a pass of its own
//! (decision 1). By the time [`link`] is called the group's records are
//! inserted on the same transaction, so a record and every claim about it
//! either both land or neither does — the placement MEM-4 chose for the
//! vector (ADR-0023 decision 2) and MEM-5 for the closed window (ADR-0039
//! decision 1), applied to a third kind of derived material. There is no
//! second queue and no window in which the corpus holds a record the graph
//! has never heard of.
//!
//! What it writes, and what it does not:
//!
//! * `entity`: `record --mentions--> name`. The mentions come from the
//!   extractor seam's `entities` list and nowhere else (decision 2) — the
//!   linker never re-reads content, so it cannot disagree with the text
//!   that was actually persisted.
//! * `episode`: `record --occurred_during--> session`, at confidence 1000,
//!   because the session identifier is a property of the event rather than
//!   a judgement anyone made.
//! * `provenance` is **projected**, not written: `record_supersessions`
//!   stays the system of record and
//!   [`synveda_store::graph::supersession_edges`] presents it in the edge
//!   model (decision 14, discharging ADR-0039's trigger (d)).
//!
//! Resolution is [`resolve`] plus the schema's unique constraint. There is
//! no read-then-write race to lose: `upsert_vertex` converges on
//! `(tenant, graph, kind, key)`, which is the convergence point ADR-0043
//! decision 5 built for exactly this. The rules are deterministic and few,
//! and precision is bought by refusing to guess rather than by a threshold
//! — what normalisation *did* do is legible afterwards in
//! `confidence_permille` (decision 4), so a ranker can prefer the mentions
//! that needed no help.
//!
//! Two rules here are compliance properties rather than implementation
//! detail. A vertex is never written without the edge that justifies it
//! (decision 7), so every name in the graph is reachable from a record
//! whose scope and sensitivity govern it. And nothing but a *name* ever
//! reaches a vertex (decision 8): `graph_vertices` carries no scope, so a
//! record-backed vertex's key and label are its record id — never its
//! content, never its class, never a summary.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgConnection;
use synveda_store::graph::{self, EdgeState, VertexState};
use synveda_types::{Graph, GraphEdgeId, GraphVertexId, RecordId, Result, TenantId};

/// Counter: records the stage saw, labelled `graph` and
/// `outcome = linked | orphan`. The orphan rate GRPH-2's AC tracks — per
/// graph, because "no name resolved from this record" and "this event
/// carried no usable session id" are different facts about different
/// pipelines (ADR-0044 decision 15).
pub const LINK_RECORDS_TOTAL: &str = "synveda_graph_link_records_total";

/// Counter: entity mentions, labelled
/// `outcome = resolved | refused | capped`. `refused` is the resolver
/// declining what the schema would decline — an empty key, an
/// over-long one, a redaction placeholder, a bare stopword — and
/// `capped` is the excess past [`MAX_MENTIONS_PER_RECORD`], counted
/// rather than dropped in silence.
pub const LINK_MENTIONS_TOTAL: &str = "synveda_graph_link_mentions_total";

/// Histogram: wall time of one [`link`] call, over the whole group.
pub const LINK_SECONDS: &str = "synveda_graph_link_duration_seconds";

/// Who asserted these claims, recorded on every edge — the seam's name,
/// exactly as `record_supersessions.method` and
/// [`crate::worker`]'s `JUDGE_METHOD`. An LLM-backed resolver takes the
/// same column, which is why it is a name rather than a flag.
pub const LINKER_METHOD: &str = "deterministic";

/// Confidence when the key needed only case folding, whitespace
/// collapsing and edge punctuation to be reached: nothing was discarded,
/// so two mentions that agree here are the same string.
pub const MENTION_EXACT_PERMILLE: i32 = 1000;

/// Confidence when a lossy rule fired — a possessive, a leading article
/// or a trailing corporate suffix was removed. The key is then a claim
/// about equivalence rather than an observation of identity, and
/// ADR-0044 decision 4 says the difference is recorded rather than
/// thresholded.
pub const MENTION_NORMALISED_PERMILLE: i32 = 900;

/// Confidence of an `occurred_during` claim. A session identifier is a
/// property of the event the record came from; nobody inferred it.
pub const SESSION_PERMILLE: i32 = 1000;

/// The most mentions one record contributes. A bound rather than a tuning
/// knob, for the reason `MAX_EXPANSION_SEEDS` is 64: an extractor that
/// returns more names than this from one summarised candidate has
/// malfunctioned, and the excess is counted (`capped`) so the truncation
/// is never silent.
pub const MAX_MENTIONS_PER_RECORD: usize = 32;

/// The schema's bound on `graph_vertices.key` and `.label`
/// (migration 0026). Enforced here so a long mention is refused in Rust
/// and counted, rather than aborting a transaction that holds an archive
/// lock (ADR-0044 decision 10).
const MAX_KEY_CHARS: usize = 512;

/// A vertex that is a record: key and label are the record id, and
/// nothing else ever (decision 8).
const KIND_RECORD: &str = "record";
/// A vertex that is a resolved name. Untyped, because `kind` is part of
/// the resolution key and a type nobody derives would split the node this
/// stage exists to converge (decision 5).
const KIND_NAME: &str = "name";
/// A vertex that is one client session.
const KIND_SESSION: &str = "session";

/// The `entity` graph's relation: this record's text names this thing.
const EDGE_MENTIONS: &str = "mentions";
/// The `episode` graph's relation: this record came out of this session.
const EDGE_OCCURRED_DURING: &str = "occurred_during";

/// Words that are never a name, checked against the finished key. The
/// resolver's last line rather than its first: mention *detection* belongs
/// to the extractor, and this list only catches what an extractor should
/// not have emitted at all.
const REFUSED_KEYS: &[&str] = &[
    "i",
    "me",
    "my",
    "we",
    "us",
    "our",
    "you",
    "your",
    "he",
    "him",
    "his",
    "she",
    "her",
    "it",
    "its",
    "they",
    "them",
    "their",
    "this",
    "that",
    "these",
    "those",
    "there",
    "here",
    "today",
    "tomorrow",
    "yesterday",
    "now",
    "then",
];

/// Trailing tokens that mark a company rather than name one. Removing one
/// is the single most valuable equivalence rule for organisation names —
/// and it is lossy, so it costs the mention its exact tier.
const CORPORATE_SUFFIXES: &[&str] = &[
    "inc",
    "llc",
    "llp",
    "lp",
    "ltd",
    "limited",
    "corp",
    "corporation",
    "co",
    "company",
    "gmbh",
    "plc",
    "sa",
    "ag",
    "bv",
    "nv",
    "pty",
    "srl",
];

/// The opaque marker MEM-2 leaves behind (ADR-0021). A mention carrying
/// one is refused outright: interning it would give a secret a stable
/// graph identity and converge every secret that hit the same rule onto
/// one node (ADR-0044 decision 9).
const REDACTION_MARKER: &str = "[REDACTED:";

/// One committed record, as the linking stage sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedRecord {
    /// The record just inserted on this transaction.
    pub record_id: RecordId,
    /// The client's session identifier for the event it came from; empty
    /// when the event carried none.
    pub session_id: String,
    /// The record's valid-from, which every claim about it starts at.
    pub valid_from: DateTime<Utc>,
    /// The extractor's entity mentions for this record, verbatim.
    pub mentions: Vec<String>,
}

/// A mention, resolved to the key a vertex converges on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The normalised key, unique within `(tenant, graph, kind)`.
    pub key: String,
    /// The display form it was normalised from — trimmed and
    /// whitespace-collapsed, never casefolded.
    pub label: String,
    /// [`MENTION_EXACT_PERMILLE`] or [`MENTION_NORMALISED_PERMILLE`].
    pub confidence_permille: i32,
}

/// What one [`link`] call did. Every number is an integer: this becomes
/// an audit payload, and canonicalisation rejects floats (ADR-0019
/// decision 2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkOutcome {
    /// Records that gained at least one `mentions` edge.
    pub entity_linked: usize,
    /// Records from which no name resolved. A normal outcome — "the
    /// staging cluster restarted" names nothing — and the numerator of
    /// the entity graph's orphan rate.
    pub entity_orphans: usize,
    /// Records that gained an `occurred_during` edge.
    pub episode_linked: usize,
    /// Records whose event carried no usable session identifier.
    pub episode_orphans: usize,
    /// Distinct names this group resolved to.
    pub names: usize,
    /// Mentions that reached a key — counted per record, so a name three
    /// records mention is three resolutions against one `names` entry.
    pub resolved: usize,
    /// Claims newly asserted.
    pub edges: usize,
    /// Claims that already held, so nothing was written (decision 11).
    pub held: usize,
    /// Mentions the resolver declined.
    pub refused: usize,
    /// Mentions past [`MAX_MENTIONS_PER_RECORD`].
    pub capped: usize,
}

impl LinkOutcome {
    /// The audit summary carried by the group's existing
    /// `memory.extracted` event (ADR-0044's compliance note): what the
    /// graph learned from this commit, without a second event asserting
    /// the same fact.
    #[must_use]
    pub fn summary(&self) -> serde_json::Value {
        json!({
            "entity": { "linked": self.entity_linked, "orphans": self.entity_orphans },
            "episode": { "linked": self.episode_linked, "orphans": self.episode_orphans },
            "names": self.names,
            "edges": self.edges,
            "held": self.held,
            "mentions_resolved": self.resolved,
            "mentions_refused": self.refused,
            "mentions_capped": self.capped,
        })
    }

    /// Nothing was written and nothing was seen.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Resolves one mention to the key its vertex converges on, or refuses it
/// (ADR-0044 decisions 3, 9 and 10).
///
/// The rules, in order and few on purpose: reject a redaction placeholder;
/// trim and collapse whitespace; casefold; strip punctuation from the ends
/// of the string and of each token; strip a possessive; strip a leading
/// article; strip a trailing corporate suffix. The first four keep the
/// mention's exact tier because they discard nothing about *which* string
/// it is; the last three remove a word, which is a claim about equivalence
/// and is recorded as [`MENTION_NORMALISED_PERMILLE`].
///
/// Returns `None` for everything the schema would refuse — a placeholder,
/// an empty result, one past [`MAX_KEY_CHARS`], a bare stopword — so a
/// malformed mention costs a counter rather than a transaction.
#[must_use]
pub fn resolve(mention: &str) -> Option<Resolution> {
    if mention.contains(REDACTION_MARKER) {
        return None;
    }
    let label: String = mention.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() || label.chars().count() > MAX_KEY_CHARS {
        return None;
    }

    let mut lossy = false;
    let mut tokens: Vec<String> = Vec::new();
    for token in label.to_lowercase().split_whitespace() {
        let trimmed = token.trim_matches(is_edge_punctuation);
        if !trimmed.is_empty() {
            tokens.push(trimmed.to_owned());
        }
    }

    // A possessive is a grammatical form of the name, not part of it.
    if let Some(last) = tokens.last_mut()
        && let Some(stem) = last
            .strip_suffix("'s")
            .or_else(|| last.strip_suffix("\u{2019}s"))
        && !stem.is_empty()
    {
        *last = stem.to_owned();
        lossy = true;
    }
    // A leading article, but never the whole mention: "The Times" is a
    // name that starts with an article, and "the" alone is not a name.
    if tokens.len() > 1 && matches!(tokens[0].as_str(), "the" | "a" | "an") {
        tokens.remove(0);
        lossy = true;
    }
    // A trailing corporate suffix, on the same condition and for the same
    // reason: "Co" alone names nothing.
    if tokens.len() > 1 && CORPORATE_SUFFIXES.contains(&tokens[tokens.len() - 1].as_str()) {
        tokens.pop();
        lossy = true;
    }

    let key = tokens.join(" ");
    if key.is_empty() || key.chars().count() > MAX_KEY_CHARS || REFUSED_KEYS.contains(&key.as_str())
    {
        return None;
    }
    Some(Resolution {
        key,
        label,
        confidence_permille: if lossy {
            MENTION_NORMALISED_PERMILLE
        } else {
            MENTION_EXACT_PERMILLE
        },
    })
}

/// Punctuation that is typography rather than identity: it may sit at
/// either end of a token without changing which string the token names.
fn is_edge_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | '"'
            | '\''
            | '\u{2018}'
            | '\u{2019}'
            | '\u{201c}'
            | '\u{201d}'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '*'
            | '_'
            | '`'
    )
}

/// One record's resolved work, computed before any statement runs.
struct Plan<'a> {
    record: &'a LinkedRecord,
    /// Deduplicated by key and ordered by it.
    names: Vec<Resolution>,
    /// The session key, when the event carried a usable one.
    session: Option<&'a str>,
}

/// Links one commit group's records into the `entity` and `episode`
/// graphs, on the caller's transaction (ADR-0044 decision 1).
///
/// Idempotent by construction: vertices converge on their resolution key
/// and claims are asserted with [`graph::assert_edge`], so a re-drive
/// writes nothing and reports it as `held` (decision 11).
///
/// Shared rows are touched in key order and records in id order
/// (decision 17), so two workers linking the same popular name approach
/// them in the same sequence — the discipline `commit_group` already
/// applies to scopes.
#[tracing::instrument(
    name = "ingest.linking.link",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        records.count = records.len(),
        names = tracing::field::Empty,
        edges = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn link(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    records: &[LinkedRecord],
) -> Result<LinkOutcome> {
    if records.is_empty() {
        return Ok(LinkOutcome::default());
    }
    let started = std::time::Instant::now();
    let mut outcome = LinkOutcome::default();

    // ── Resolve, in memory, before touching the database ───────────────
    let mut records: Vec<&LinkedRecord> = records.iter().collect();
    records.sort_by_key(|record| record.record_id);
    let mut plans: Vec<Plan<'_>> = Vec::with_capacity(records.len());
    for record in records {
        let mut names: BTreeMap<String, Resolution> = BTreeMap::new();
        for mention in &record.mentions {
            if names.len() >= MAX_MENTIONS_PER_RECORD {
                outcome.capped += 1;
                continue;
            }
            match resolve(mention) {
                // First surface form wins within a record; the group-wide
                // choice below is made the same way, so the label a vertex
                // ends up with does not depend on iteration order.
                Some(resolution) => {
                    names.entry(resolution.key.clone()).or_insert(resolution);
                }
                None => outcome.refused += 1,
            }
        }
        // A session identifier is opaque: trimmed and length-checked, never
        // casefolded, because two ids that differ only in case are two
        // sessions and merging them would invent an episode.
        let session = record.session_id.trim();
        let session =
            (!session.is_empty() && session.chars().count() <= MAX_KEY_CHARS).then_some(session);
        let names: Vec<Resolution> = names.into_values().collect();
        outcome.resolved += names.len();
        plans.push(Plan {
            record,
            names,
            session,
        });
    }
    // ── Shared vertices first, in key order ────────────────────────────
    let mut name_labels: BTreeMap<&str, &str> = BTreeMap::new();
    for plan in &plans {
        for resolution in &plan.names {
            name_labels
                .entry(resolution.key.as_str())
                .or_insert(resolution.label.as_str());
        }
    }
    let mut name_vertices: BTreeMap<&str, GraphVertexId> = BTreeMap::new();
    for (key, label) in &name_labels {
        let vertex = graph::upsert_vertex(
            &mut *conn,
            GraphVertexId::new(),
            tenant_id,
            Graph::Entity,
            &VertexState {
                kind: KIND_NAME.to_owned(),
                key: (*key).to_owned(),
                label: (*label).to_owned(),
                // A name is identity, not content. Backing it with the
                // record that happened to mention it first would privilege
                // that record and make the vertex a disclosure of it
                // (decision 8).
                record_id: None,
            },
        )
        .await?;
        name_vertices.insert(key, vertex.id);
    }
    outcome.names = name_vertices.len();

    let sessions: BTreeSet<&str> = plans.iter().filter_map(|plan| plan.session).collect();
    let mut session_vertices: BTreeMap<&str, GraphVertexId> = BTreeMap::new();
    for session in sessions {
        let vertex = graph::upsert_vertex(
            &mut *conn,
            GraphVertexId::new(),
            tenant_id,
            Graph::Episode,
            &VertexState {
                kind: KIND_SESSION.to_owned(),
                key: session.to_owned(),
                label: session.to_owned(),
                record_id: None,
            },
        )
        .await?;
        session_vertices.insert(session, vertex.id);
    }

    // ── Then each record's own vertex and its claims ───────────────────
    for plan in &plans {
        let record_id = plan.record.record_id;
        if plan.names.is_empty() {
            outcome.entity_orphans += 1;
        } else {
            let src = record_vertex(&mut *conn, tenant_id, Graph::Entity, record_id).await?;
            for resolution in &plan.names {
                let dst = name_vertices[resolution.key.as_str()];
                let asserted = graph::assert_edge(
                    &mut *conn,
                    GraphEdgeId::new(),
                    tenant_id,
                    Graph::Entity,
                    &EdgeState {
                        kind: EDGE_MENTIONS.to_owned(),
                        src_id: src,
                        dst_id: dst,
                        method: LINKER_METHOD.to_owned(),
                        confidence_permille: resolution.confidence_permille,
                        source_record_id: Some(record_id),
                        valid_from: plan.record.valid_from,
                        // A mention does not expire: the text does not
                        // change, and the record's own validity is the
                        // corpus's answer, not the graph's (decision 13).
                        valid_to: None,
                    },
                )
                .await?;
                if asserted.is_some() {
                    outcome.edges += 1;
                } else {
                    outcome.held += 1;
                }
            }
            outcome.entity_linked += 1;
        }

        let Some(session) = plan.session else {
            outcome.episode_orphans += 1;
            continue;
        };
        let src = record_vertex(&mut *conn, tenant_id, Graph::Episode, record_id).await?;
        let asserted = graph::assert_edge(
            &mut *conn,
            GraphEdgeId::new(),
            tenant_id,
            Graph::Episode,
            &EdgeState {
                kind: EDGE_OCCURRED_DURING.to_owned(),
                src_id: src,
                dst_id: session_vertices[session],
                method: LINKER_METHOD.to_owned(),
                confidence_permille: SESSION_PERMILLE,
                source_record_id: Some(record_id),
                valid_from: plan.record.valid_from,
                valid_to: None,
            },
        )
        .await?;
        if asserted.is_some() {
            outcome.edges += 1;
        } else {
            outcome.held += 1;
        }
        outcome.episode_linked += 1;
    }

    metrics::histogram!(LINK_SECONDS).record(started.elapsed().as_secs_f64());
    record_metrics(&outcome);
    tracing::Span::current().record("names", outcome.names);
    tracing::Span::current().record("edges", outcome.edges);
    Ok(outcome)
}

/// Interns the vertex that *is* this record, in `graph`. Key and label are
/// the record id and nothing else — `graph_vertices` carries no scope, so
/// a label is readable tenant-wide and record content must never reach one
/// (ADR-0044 decision 8).
async fn record_vertex(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    graph: Graph,
    record_id: RecordId,
) -> Result<GraphVertexId> {
    let key = record_id.to_string();
    let vertex = graph::upsert_vertex(
        conn,
        GraphVertexId::new(),
        tenant_id,
        graph,
        &VertexState {
            kind: KIND_RECORD.to_owned(),
            key: key.clone(),
            label: key,
            record_id: Some(record_id),
        },
    )
    .await?;
    Ok(vertex.id)
}

/// Emits the stage's counters. Split out so the per-graph orphan labels
/// are written in one place rather than at four call sites.
fn record_metrics(outcome: &LinkOutcome) {
    for (graph, linked, orphans) in [
        (Graph::Entity, outcome.entity_linked, outcome.entity_orphans),
        (
            Graph::Episode,
            outcome.episode_linked,
            outcome.episode_orphans,
        ),
    ] {
        if linked > 0 {
            metrics::counter!(
                LINK_RECORDS_TOTAL,
                "graph" => graph.as_str(), "outcome" => "linked"
            )
            .increment(linked as u64);
        }
        if orphans > 0 {
            metrics::counter!(
                LINK_RECORDS_TOTAL,
                "graph" => graph.as_str(), "outcome" => "orphan"
            )
            .increment(orphans as u64);
        }
    }
    for (outcome_label, count) in [
        ("resolved", outcome.resolved),
        ("refused", outcome.refused),
        ("capped", outcome.capped),
    ] {
        if count > 0 {
            metrics::counter!(LINK_MENTIONS_TOTAL, "outcome" => outcome_label)
                .increment(count as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(mention: &str) -> Option<String> {
        resolve(mention).map(|resolution| resolution.key)
    }

    #[test]
    fn case_and_whitespace_alone_keep_the_exact_tier() {
        let resolution = resolve("  Ada   Lovelace ").expect("resolves");
        assert_eq!(resolution.key, "ada lovelace");
        assert_eq!(resolution.label, "Ada Lovelace");
        assert_eq!(resolution.confidence_permille, MENTION_EXACT_PERMILLE);
    }

    #[test]
    fn edge_punctuation_is_typography_not_identity() {
        let resolution = resolve("Postgres.").expect("resolves");
        assert_eq!(resolution.key, "postgres");
        assert_eq!(resolution.confidence_permille, MENTION_EXACT_PERMILLE);
    }

    #[test]
    fn a_removed_word_drops_the_tier() {
        for mention in ["Ada's", "The Ada Foundation", "Ada Foundation Ltd."] {
            let resolution = resolve(mention).expect("resolves");
            assert_eq!(
                resolution.confidence_permille, MENTION_NORMALISED_PERMILLE,
                "{mention} removed a word and must not claim the exact tier"
            );
        }
    }

    #[test]
    fn corporate_forms_converge() {
        assert_eq!(key("ACME Corp"), key("Acme Corporation"));
        assert_eq!(key("ACME Corp."), key("acme"));
    }

    #[test]
    fn an_article_or_suffix_alone_is_not_a_name() {
        // Both survive as themselves rather than normalising to nothing:
        // the rules only fire when something is left behind.
        assert_eq!(key("The"), Some("the".to_owned()));
        assert_eq!(key("Co."), Some("co".to_owned()));
    }

    #[test]
    fn a_redaction_placeholder_is_never_an_entity() {
        assert_eq!(key("[REDACTED:github-pat]"), None);
        assert_eq!(key("Deploy key [REDACTED:aws-key]"), None);
    }

    #[test]
    fn the_schema_bounds_are_enforced_here() {
        assert_eq!(key(""), None);
        assert_eq!(key("   "), None);
        assert_eq!(key(&"a".repeat(MAX_KEY_CHARS + 1)), None);
        assert!(key(&"a".repeat(MAX_KEY_CHARS)).is_some());
    }

    #[test]
    fn a_pronoun_an_extractor_should_not_have_emitted_is_refused() {
        assert_eq!(key("It"), None);
        assert_eq!(key("They"), None);
        assert_eq!(key("Today"), None);
    }

    #[test]
    fn distinct_names_stay_distinct() {
        assert_ne!(key("Ada Lovelace"), key("Ada Byron"));
        assert_ne!(key("IBM"), key("International Business Machines"));
    }
}
