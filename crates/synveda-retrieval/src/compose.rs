//! The composition engine (CTX-2, ADR-0025): deterministic
//! chain-gradient assembly — user > team > department > org,
//! pinned-first within each level — under an estimated-token budget,
//! with seed §4.4's conflict rules and per-scope channel rules, every
//! block watermarked with BLAKE3 version hashes and record ids.
//!
//! Determinism is the AC: no clock is read here (the valid-time instant
//! is the caller's input), every ordering is total, and no map
//! iteration order reaches the output. Given the same plan, instant,
//! and database state, [`compose`] returns a byte-identical block.
//!
//! Channel rules pre-FLOW-2 (ADR-0025 decision 2): [`RecordKind`] is
//! the stand-in — `pinned` composes as the published channel, `derived`
//! as the derived channel, included only where the scope's effective
//! pack allows and always marked unreviewed in the rendered text.
//! Watermarks pre-FLOW-1 (decision 7): each entry's hash is the BLAKE3
//! content address of exactly the version that composed; FLOW-1's
//! commit hashes take the field over, same shape.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::PgConnection;
use synveda_store::records::RecordVersion;
use synveda_store::search;
use synveda_types::{
    RecordClass, RecordId, RecordKind, Result, ScopeId, ScopeKind, Sensitivity, TenantId,
};

use crate::TOKENS_PER_INJECT;
use crate::hybrid::allowed_sensitivities;

/// One scope of a composition plan: a PDP-allowed chain scope plus the
/// channel rule its effective pack sets (ADR-0025 decisions 1–2).
/// Produced by [`crate::authz::composition_plan`] in the product paths.
#[derive(Debug, Clone)]
pub struct ComposeScope {
    /// The scope records compose from.
    pub scope_id: ScopeId,
    /// The scope's hierarchy level — rendered in the section header.
    pub kind: ScopeKind,
    /// The scope's slug path — the section header's display name.
    pub path: String,
    /// Whether derived-channel records compose here (the scope's
    /// effective pack's [`synveda_types::InjectChannels`]).
    pub include_derived: bool,
}

/// One composition request. `scopes` must be in gradient order,
/// nearest-first — the order [`crate::authz::composition_plan`]
/// produces.
#[derive(Debug, Clone)]
pub struct ComposeRequest {
    /// PDP-allowed scopes, nearest-first. Empty composes the empty
    /// block — there is no unfiltered code path (the CTX-1 discipline).
    pub scopes: Vec<ComposeScope>,
    /// The estimated-token budget (the caller's home-scope pack;
    /// seed §4.4 default 1,500).
    pub budget_tokens: u32,
    /// The inclusive sensitivity ceiling; clamped below `restricted`
    /// exactly as retrieval clamps it (ADR-0024 decision 2).
    pub max_sensitivity: Sensitivity,
    /// The valid-time instant: records compose only if their valid
    /// window covers it. An explicit input — never a clock read — for
    /// the determinism AC, and the valid-time half of CTX-5's as-of.
    pub at: DateTime<Utc>,
    /// Optional relevance ranking (best-first record ids from the
    /// hybrid engine). When present, derived records absent from it do
    /// not compose and ranked ones order by rank within their scope;
    /// pinned material never depends on the task (ADR-0025 decision 5).
    pub relevance: Option<Vec<RecordId>>,
    /// Candidate fetch cap per `(scope, kind)` (ADR-0025 decision 5).
    pub per_scope_kind_candidates: i64,
}

impl ComposeRequest {
    /// A request with the product defaults: the given plan and budget,
    /// `internal` ceiling (the extraction floor), 64 candidates per
    /// `(scope, kind)`, no relevance ranking.
    #[must_use]
    pub fn new(scopes: Vec<ComposeScope>, budget_tokens: u32, at: DateTime<Utc>) -> Self {
        Self {
            scopes,
            budget_tokens,
            max_sensitivity: Sensitivity::Internal,
            at,
            relevance: None,
            per_scope_kind_candidates: 64,
        }
    }
}

/// One composed entry: the watermark's unit (ADR-0025 decision 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedEntry {
    /// The record that composed.
    pub record_id: RecordId,
    /// The scope it composed from.
    pub scope_id: ScopeId,
    /// Pinned (published stand-in) or derived.
    pub kind: RecordKind,
    /// What the record asserts.
    pub class: RecordClass,
    /// BLAKE3 over `(record_id, tx_from, content)` — the content
    /// address of exactly the version that composed, hex-encoded.
    /// FLOW-1's commit hashes supersede this field in place.
    pub version_hash: String,
    /// Estimated tokens of the entry's rendered line.
    pub tokens: u32,
}

/// One composed, watermarked context block.
#[derive(Debug, Clone)]
pub struct ComposedBlock {
    /// The rendered block, watermark line included. Empty when nothing
    /// composed.
    pub text: String,
    /// The watermark: every composed record, in block order.
    pub entries: Vec<ComposedEntry>,
    /// BLAKE3 over the ordered entry hashes, hex-encoded — the block's
    /// identity, also on the rendered watermark line.
    pub block_hash: String,
    /// Estimated tokens of `text` — what `synveda_tokens_per_inject`
    /// recorded. Never exceeds the budget.
    pub tokens: u32,
    /// The budget the block was composed under.
    pub budget_tokens: u32,
    /// Candidates dropped by conflict resolution (ADR-0025 decision 6).
    pub dropped_conflicts: usize,
    /// Candidates that survived every rule but did not fit the budget.
    pub skipped_over_budget: usize,
}

/// Deterministic token estimator (ADR-0025 decision 4):
/// `ceil(chars / 4)` over Unicode scalar values. A named seam — the
/// budget bounds *estimated* tokens; per-harness tokenizers are an
/// adapter concern and EVAL-4 measures the bias.
#[must_use]
pub fn estimated_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
}

/// The seed §4.4 conflict resolution order (ADR-0025 decision 6):
/// `Less` means the first candidate wins. Pinned beats derived, then
/// the more specific scope (smaller chain position), then newer
/// valid-from, then newer tx-from, then the smaller record id — a
/// total order, so the winner never depends on evaluation order.
/// Exported for MEM-5, which replaces the exact-match *predicate* with
/// semantic groups and reuses this resolution.
#[must_use]
pub fn conflict_precedence(a: (&RecordVersion, usize), b: (&RecordVersion, usize)) -> Ordering {
    let kind_rank = |version: &RecordVersion| match version.state.kind {
        RecordKind::Pinned => 0_u8,
        RecordKind::Derived => 1,
    };
    kind_rank(a.0)
        .cmp(&kind_rank(b.0))
        .then_with(|| a.1.cmp(&b.1))
        .then_with(|| b.0.state.valid_from.cmp(&a.0.state.valid_from))
        .then_with(|| b.0.tx_from.cmp(&a.0.tx_from))
        .then_with(|| a.0.id.cmp(&b.0.id))
}

/// Composes the block: fetches candidates for the plan's scopes inside
/// the caller's tenant transaction, applies channel rules, relevance,
/// conflict resolution, and the gradient assembly under the budget,
/// then renders and watermarks. Records `synveda_tokens_per_inject`
/// on every call — a compose over an empty permitted set records 0
/// (ADR-0025 decision 8).
#[tracing::instrument(
    name = "retrieval.compose",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        scopes.count = request.scopes.len(),
        budget = request.budget_tokens,
        entries = tracing::field::Empty,
        tokens = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn compose(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    request: &ComposeRequest,
) -> Result<ComposedBlock> {
    let span = tracing::Span::current();
    if request.scopes.is_empty() {
        let block = empty_block(request.budget_tokens);
        span.record("entries", 0);
        span.record("tokens", 0);
        metrics::histogram!(TOKENS_PER_INJECT).record(0.0);
        return Ok(block);
    }

    let scope_ids: Vec<ScopeId> = request.scopes.iter().map(|scope| scope.scope_id).collect();
    let sensitivities = allowed_sensitivities(request.max_sensitivity);
    let candidates = search::compose_candidates(
        conn,
        tenant_id,
        &scope_ids,
        &sensitivities,
        request.at,
        request.per_scope_kind_candidates.max(1),
    )
    .await?;

    let chain_position: HashMap<ScopeId, usize> = request
        .scopes
        .iter()
        .enumerate()
        .map(|(position, scope)| (scope.scope_id, position))
        .collect();
    let include_derived: HashMap<ScopeId, bool> = request
        .scopes
        .iter()
        .map(|scope| (scope.scope_id, scope.include_derived))
        .collect();
    let relevance_rank: Option<HashMap<RecordId, usize>> = request.relevance.as_ref().map(|ids| {
        ids.iter()
            .enumerate()
            .map(|(rank, id)| (*id, rank))
            .collect()
    });

    // Channel and relevance rules (ADR-0025 decisions 2 and 5): derived
    // composes only where the scope's pack allows, and — under a
    // relevance ranking — only if retrieval ranked it. Pinned material
    // never depends on the task.
    let mut survivors: Vec<&RecordVersion> = candidates
        .iter()
        .filter(|version| match version.state.kind {
            RecordKind::Pinned => true,
            RecordKind::Derived => {
                include_derived
                    .get(&version.state.scope_id)
                    .copied()
                    .unwrap_or(false)
                    && relevance_rank
                        .as_ref()
                        .is_none_or(|ranks| ranks.contains_key(&version.id))
            }
        })
        .collect();

    // Conflict resolution (decision 6): one winner per trimmed-content
    // group by the seed §4.4 precedence; losers are dropped entirely.
    let position_of =
        |version: &RecordVersion| chain_position.get(&version.state.scope_id).copied();
    let mut winner_by_content: HashMap<&str, &RecordVersion> = HashMap::new();
    for version in &survivors {
        let key = version.state.content.trim();
        // Candidates whose scope fell out of the plan cannot happen (the
        // query filtered on plan scopes); position lookup is total here.
        let Some(position) = position_of(version) else {
            continue;
        };
        winner_by_content
            .entry(key)
            .and_modify(|incumbent| {
                let incumbent_position = position_of(incumbent).unwrap_or(usize::MAX);
                if conflict_precedence((version, position), (incumbent, incumbent_position))
                    == Ordering::Less
                {
                    *incumbent = version;
                }
            })
            .or_insert(version);
    }
    let before = survivors.len();
    survivors.retain(|version| {
        winner_by_content
            .get(version.state.content.trim())
            .is_some_and(|winner| winner.id == version.id)
    });
    let dropped_conflicts = before - survivors.len();

    // Assembly order (decision 5): the gradient — nearest scope first,
    // pinned before derived within a scope, derived by relevance rank
    // when ranked else newest valid-from, record id as the total-order
    // tiebreak.
    survivors.sort_by(|a, b| {
        let scope = position_of(a)
            .unwrap_or(usize::MAX)
            .cmp(&position_of(b).unwrap_or(usize::MAX));
        let kind = |version: &RecordVersion| match version.state.kind {
            RecordKind::Pinned => 0_u8,
            RecordKind::Derived => 1,
        };
        let rank = |version: &RecordVersion| {
            relevance_rank
                .as_ref()
                .and_then(|ranks| ranks.get(&version.id).copied())
                .unwrap_or(usize::MAX)
        };
        scope
            .then_with(|| kind(a).cmp(&kind(b)))
            .then_with(|| rank(a).cmp(&rank(b)))
            .then_with(|| b.state.valid_from.cmp(&a.state.valid_from))
            .then_with(|| a.id.cmp(&b.id))
    });

    // First-fit assembly under the budget (decision 4): every piece the
    // block will contain is counted — preamble, section headers, entry
    // lines, and the watermark line's per-entry growth. Per-piece
    // ceilings over-estimate the concatenation's estimate, so staying
    // under budget here keeps the final text under budget too.
    let preamble = format!(
        "# Synveda context (as of {})\n",
        request.at.to_rfc3339_opts(SecondsFormat::Secs, true)
    );
    // The watermark's fixed cost, hash width included (BLAKE3 hex is 64
    // chars regardless of content).
    let watermark_fixed = watermark_line(&"0".repeat(64), &[]);
    let mut watermark_chars = watermark_fixed.chars().count();
    let mut watermark_tokens = u32::try_from(watermark_chars.div_ceil(4)).unwrap_or(u32::MAX);
    let mut used = estimated_tokens(&preamble) + watermark_tokens;

    let header_of: HashMap<ScopeId, String> = request
        .scopes
        .iter()
        .map(|scope| {
            (
                scope.scope_id,
                format!("\n## {} ({})\n", scope.path, scope.kind),
            )
        })
        .collect();
    let mut open_sections: HashSet<ScopeId> = HashSet::new();
    let mut pieces: Vec<String> = vec![preamble];
    let mut entries: Vec<ComposedEntry> = Vec::new();
    let mut skipped_over_budget = 0_usize;

    for version in survivors {
        let line = entry_line(version);
        let line_tokens = estimated_tokens(&line);
        let header_tokens = if open_sections.contains(&version.state.scope_id) {
            0
        } else {
            header_of
                .get(&version.state.scope_id)
                .map_or(0, |header| estimated_tokens(header))
        };
        // Each entry grows the watermark's record list by ",<uuid>" (the
        // first by "<uuid>"): re-estimate the whole line at its new
        // width so rounding never drifts.
        let id_chars = version.id.to_string().chars().count() + usize::from(!entries.is_empty());
        let new_watermark_chars = watermark_chars + id_chars;
        let new_watermark_tokens =
            u32::try_from(new_watermark_chars.div_ceil(4)).unwrap_or(u32::MAX);
        let cost = line_tokens + header_tokens + (new_watermark_tokens - watermark_tokens);
        if used.saturating_add(cost) > request.budget_tokens {
            skipped_over_budget += 1;
            continue;
        }
        if open_sections.insert(version.state.scope_id)
            && let Some(header) = header_of.get(&version.state.scope_id)
        {
            pieces.push(header.clone());
        }
        pieces.push(line.clone());
        used += cost;
        watermark_chars = new_watermark_chars;
        watermark_tokens = new_watermark_tokens;
        entries.push(ComposedEntry {
            record_id: version.id,
            scope_id: version.state.scope_id,
            kind: version.state.kind,
            class: version.state.class,
            version_hash: version_hash(version),
            tokens: line_tokens,
        });
    }

    if entries.is_empty() {
        let block = ComposedBlock {
            dropped_conflicts,
            skipped_over_budget,
            ..empty_block(request.budget_tokens)
        };
        span.record("entries", 0);
        span.record("tokens", 0);
        metrics::histogram!(TOKENS_PER_INJECT).record(0.0);
        return Ok(block);
    }

    let block_hash = block_hash(&entries);
    let ids: Vec<String> = entries
        .iter()
        .map(|entry| entry.record_id.to_string())
        .collect();
    pieces.push(watermark_line(&block_hash, &ids));
    let text = pieces.concat();
    let tokens = estimated_tokens(&text);
    span.record("entries", entries.len());
    span.record("tokens", tokens);
    metrics::histogram!(TOKENS_PER_INJECT).record(f64::from(tokens));
    Ok(ComposedBlock {
        text,
        entries,
        block_hash,
        tokens,
        budget_tokens: request.budget_tokens,
        dropped_conflicts,
        skipped_over_budget,
    })
}

/// The block over nothing: empty text, the hash of zero entries.
fn empty_block(budget_tokens: u32) -> ComposedBlock {
    ComposedBlock {
        text: String::new(),
        entries: Vec::new(),
        block_hash: blake3::Hasher::new().finalize().to_hex().to_string(),
        tokens: 0,
        budget_tokens,
        dropped_conflicts: 0,
        skipped_over_budget: 0,
    }
}

/// One entry's rendered line. Derived is always marked unreviewed
/// (seed tech plan §2.2: "clearly watermarked as unreviewed").
fn entry_line(version: &RecordVersion) -> String {
    match version.state.kind {
        RecordKind::Pinned => {
            format!("- [{}] {}\n", version.state.class, version.state.content)
        }
        RecordKind::Derived => format!(
            "- [{}] {} [unreviewed]\n",
            version.state.class, version.state.content
        ),
    }
}

/// The rendered watermark line: block hash plus every composed record
/// id, in block order.
fn watermark_line(block_hash: &str, record_ids: &[String]) -> String {
    format!(
        "\n<!-- synveda:watermark v1 blake3={block_hash} records={} -->\n",
        record_ids.join(",")
    )
}

/// The entry's content address (ADR-0025 decision 7): BLAKE3 over the
/// record id, the version's transaction-time start (which uniquely
/// names the version, ADR-0006), and the content — recomputable from
/// the bitemporal store forever.
fn version_hash(version: &RecordVersion) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(version.id.as_uuid().as_bytes());
    hasher.update(&version.tx_from.timestamp_micros().to_be_bytes());
    hasher.update(version.state.content.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// The block's identity: BLAKE3 over the ordered entry hashes.
fn block_hash(entries: &[ComposedEntry]) -> String {
    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        hasher.update(entry.version_hash.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use synveda_types::IdentityId;

    fn version(
        kind: RecordKind,
        content: &str,
        valid_from: DateTime<Utc>,
        id_byte: u8,
    ) -> RecordVersion {
        RecordVersion {
            id: RecordId::from_uuid(uuid::Uuid::from_bytes([id_byte; 16])),
            tenant_id: TenantId::from_uuid(uuid::Uuid::from_bytes([9; 16])),
            state: synveda_store::records::RecordState {
                scope_id: ScopeId::from_uuid(uuid::Uuid::from_bytes([7; 16])),
                owner_id: IdentityId::from_uuid(uuid::Uuid::from_bytes([8; 16])),
                kind,
                class: RecordClass::Fact,
                content: content.to_owned(),
                sensitivity: Sensitivity::Internal,
                provenance: serde_json::json!({}),
                valid_from,
                valid_to: None,
            },
            tx_from: valid_from,
            tx_to: None,
        }
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    #[test]
    fn estimator_is_ceil_of_quarter_chars() {
        assert_eq!(estimated_tokens(""), 0);
        assert_eq!(estimated_tokens("abc"), 1);
        assert_eq!(estimated_tokens("abcd"), 1);
        assert_eq!(estimated_tokens("abcde"), 2);
        // Unicode scalars count, not bytes.
        assert_eq!(estimated_tokens("ééé"), 1);
    }

    /// The seed §4.4 order: pinned beats derived even from a broader
    /// scope; among equals, nearer scope; then newer valid-from; then
    /// the id tiebreak — total, so winners never depend on order.
    #[test]
    fn conflict_precedence_is_the_seed_order() {
        let pinned_broad = version(RecordKind::Pinned, "x", at(0), 1);
        let derived_near = version(RecordKind::Derived, "x", at(100), 2);
        assert_eq!(
            conflict_precedence((&pinned_broad, 3), (&derived_near, 0)),
            Ordering::Less,
            "pinned beats derived across levels"
        );

        let derived_broad = version(RecordKind::Derived, "x", at(100), 3);
        assert_eq!(
            conflict_precedence((&derived_near, 0), (&derived_broad, 2)),
            Ordering::Less,
            "more specific scope beats less specific"
        );

        let older = version(RecordKind::Derived, "x", at(0), 4);
        assert_eq!(
            conflict_precedence((&derived_near, 1), (&older, 1)),
            Ordering::Less,
            "newer valid-time beats older"
        );

        let twin_a = version(RecordKind::Derived, "x", at(100), 5);
        let twin_b = version(RecordKind::Derived, "x", at(100), 6);
        assert_eq!(
            conflict_precedence((&twin_a, 1), (&twin_b, 1)),
            Ordering::Less,
            "the id tiebreak makes the order total"
        );
    }

    #[test]
    fn version_hash_binds_id_version_and_content() {
        let a = version(RecordKind::Derived, "same", at(50), 1);
        let mut b = a.clone();
        assert_eq!(version_hash(&a), version_hash(&b), "recomputable");
        b.state.content = "different".to_owned();
        assert_ne!(version_hash(&a), version_hash(&b), "content-bound");
        let mut c = a.clone();
        c.tx_from = at(51);
        assert_ne!(version_hash(&a), version_hash(&c), "version-bound");
    }
}
