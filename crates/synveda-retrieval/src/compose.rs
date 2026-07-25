//! The composition engine (CTX-2, ADR-0025; channels, FLOW-2,
//! ADR-0031): deterministic chain-gradient assembly — user > team >
//! department > org — under an estimated-token budget, with seed §4.4's
//! conflict rules and per-scope channel rules, every block watermarked
//! with VedaFlow content addresses and record ids.
//!
//! Determinism is the AC: no clock is read here (the valid-time instant
//! is the caller's input), every ordering is total, and no map
//! iteration order reaches the output. Given the same plan, instant,
//! and database state, [`compose`] returns a byte-identical block.
//!
//! # Channels
//!
//! A record composes as **published** when its scope's
//! `memory/published` tree names it *at exactly the content it now
//! holds*, and as **derived** otherwise — so editing published content
//! demotes it to unreviewed rather than laundering the edit through a
//! published id (ADR-0031 decision 5). Derived material composes only
//! where the scope's effective pack allows it, and is always marked
//! unreviewed in the rendered text. Under `published-only` — bank mode —
//! the derived sweep is not issued at all for that scope.
//!
//! [`RecordKind`] no longer decides any of this. It means what seed §4.2
//! says: authored/canonical versus pipeline-derived. Authorship is not
//! review, and a pinned record nobody published does not survive bank
//! mode.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::PgConnection;
use synveda_store::records::{RecordState, RecordVersion};
use synveda_store::search;
use synveda_types::{
    Channel, RecordClass, RecordId, RecordKind, Result, ScopeId, ScopeKind, Sensitivity, TenantId,
};
use synveda_vedaflow::{ChannelRef, MemoryAsset, read_memory_members};

use crate::TOKENS_PER_INJECT;
use crate::hybrid::allowed_sensitivities;

/// Counts composed entries, labelled by the channel they composed from —
/// the production evidence that bank mode does what its AC says
/// (ADR-0031 decision 15).
pub const COMPOSED_ENTRIES_TOTAL: &str = "synveda_composed_entries_total";

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

/// One composed entry: the watermark's unit (ADR-0025 decision 7, as
/// ADR-0031 decision 11 upgraded it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedEntry {
    /// The record that composed.
    pub record_id: RecordId,
    /// The scope it composed **from** — the scope whose channel admitted
    /// it and whose `MemoryRead` decision let this caller see it. Since
    /// FLOW-5 that is not always the scope the record lives at: a
    /// department's published channel may name a record that lives at a
    /// team under it (ADR-0034 decision 6). The record's own scope is one
    /// read away, on the record; this field answers "why was this in that
    /// block", which is the question an auditor asks.
    pub scope_id: ScopeId,
    /// Which channel it composed from — the trust label.
    pub channel: Channel,
    /// Authored/canonical or pipeline-derived (seed §4.2). Authorship,
    /// not trust: [`ComposedEntry::channel`] carries that now.
    pub kind: RecordKind,
    /// What the record asserts.
    pub class: RecordClass,
    /// The VedaFlow object address of exactly the version that composed,
    /// hex-encoded — recomputable by an auditor from this response alone,
    /// and equal to the address the scope's published tree names whenever
    /// the entry composed as published (ADR-0031 decisions 5 and 6).
    pub object_hash: String,
    /// Estimated tokens of the entry's rendered line.
    pub tokens: u32,
}

/// One channel the block read: where a scope's ref pointed at
/// composition time (ADR-0031 decision 11).
///
/// Carried on the block and recorded in the inject audit event rather
/// than rendered into the text: tech plan §2.5's "inject responses cite
/// commit hashes", paid for out of the response instead of the budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelWatermark {
    /// The scope whose channel this is.
    pub scope_id: ScopeId,
    /// The ref name, e.g. `memory/published`.
    pub channel: String,
    /// The commit it pointed at, hex-encoded.
    pub commit: String,
}

/// One composed, watermarked context block.
#[derive(Debug, Clone)]
pub struct ComposedBlock {
    /// The rendered block, watermark line included. Empty when nothing
    /// composed.
    pub text: String,
    /// The watermark: every composed record, in block order.
    pub entries: Vec<ComposedEntry>,
    /// The channel refs this block composed against, in scope order —
    /// present even when a channel contributed no entry, because "the
    /// published channel was here and held nothing you could read" is
    /// the auditable fact.
    pub channels: Vec<ChannelWatermark>,
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

/// One candidate as composition sees it: a record version, the scope it
/// composed from and that scope's position in the gradient, and the
/// channel it is on there.
///
/// [`Candidate::scope_id`] is the scope the record **composed from**, not
/// necessarily the scope it lives at: since FLOW-5 a scope's published
/// tree may name a record that lives below it, and the entry then belongs
/// to the publishing scope's section of the block, at that scope's
/// position (ADR-0034 decision 6). For derived material the two are
/// always the same scope.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// The record version.
    pub version: &'a RecordVersion,
    /// The scope it composed from.
    pub scope_id: ScopeId,
    /// That scope's position in the gradient, nearest = 0.
    pub position: usize,
    /// Published or derived at that scope.
    pub channel: Channel,
}

/// The conflict resolution order (ADR-0025 decision 6, with ADR-0031
/// decision 8's tier 0): `Less` means the first candidate wins.
///
/// Published beats unpublished, then seed §4.4 unchanged — pinned beats
/// derived, then the more specific scope, then newer valid-from, then
/// newer tx-from, then the smaller record id. A total order, so the
/// winner never depends on evaluation order.
///
/// Channel goes first because seed §4.4's list predates channels. When
/// identical content is published at one scope and unpublished at
/// another, the nearer-scope rule would otherwise render the unreviewed
/// copy, drop the reviewed one, and watermark the block with the address
/// of the version nobody approved.
///
/// Exported for MEM-5, which replaces the exact-match *predicate* with
/// semantic groups and reuses this resolution.
#[must_use]
pub fn conflict_precedence(a: Candidate<'_>, b: Candidate<'_>) -> Ordering {
    let channel_rank = |channel: Channel| match channel {
        Channel::Published => 0_u8,
        _ => 1,
    };
    let kind_rank = |version: &RecordVersion| match version.state.kind {
        RecordKind::Pinned => 0_u8,
        RecordKind::Derived => 1,
    };
    channel_rank(a.channel)
        .cmp(&channel_rank(b.channel))
        .then_with(|| kind_rank(a.version).cmp(&kind_rank(b.version)))
        .then_with(|| a.position.cmp(&b.position))
        .then_with(|| b.version.state.valid_from.cmp(&a.version.state.valid_from))
        .then_with(|| b.version.tx_from.cmp(&a.version.tx_from))
        .then_with(|| a.version.id.cmp(&b.version.id))
}

/// The VedaFlow view of a stored record version (ADR-0031 decision 6).
///
/// Duplicated from the ingestion pipeline's identical mapping for the
/// reason given there: `synveda-store` and `synveda-vedaflow` are
/// siblings, so neither can host a conversion between their types. The
/// AC test pins the two addresses together, which is what makes the
/// duplication safe rather than merely small.
fn memory_asset(id: RecordId, state: &RecordState) -> MemoryAsset {
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

    let chain_position: HashMap<ScopeId, usize> = request
        .scopes
        .iter()
        .enumerate()
        .map(|(position, scope)| (scope.scope_id, position))
        .collect();

    // The published channel of every planned scope: one indexed read for
    // the whole chain (ADR-0031 decision 3). Kept in gradient order, so
    // "the nearest scope that published this" is the first match
    // (ADR-0034 decision 6).
    let published = read_memory_members(conn, tenant_id, &scope_ids, Channel::Published).await?;
    let mut admitted: Vec<(usize, ScopeId, &HashMap<RecordId, _>)> = published
        .iter()
        .filter_map(|channel| {
            chain_position
                .get(&channel.scope_id)
                .map(|position| (*position, channel.scope_id, &channel.members))
        })
        .collect();
    admitted.sort_unstable_by_key(|(position, _, _)| *position);
    let published_ids: Vec<RecordId> = published
        .iter()
        .flat_map(|channel| channel.members.keys().copied())
        .collect();

    // Published members are fetched by id and uncapped; derived is the
    // capped per-(scope, kind) sweep, and only over scopes whose pack
    // admits it — bank mode removes the read rather than filtering its
    // results (ADR-0031 decisions 9 and 10).
    let derived_scopes: Vec<ScopeId> = request
        .scopes
        .iter()
        .filter(|scope| scope.include_derived)
        .map(|scope| scope.scope_id)
        .collect();
    // Published members are fetched by id alone: tree membership at a
    // planned scope is the predicate, because that tree may name a record
    // living below it (ADR-0034 decision 6). Residence still decides for
    // the derived sweep.
    let members =
        search::compose_members(conn, tenant_id, &published_ids, &sensitivities, request.at)
            .await?;
    let swept = if derived_scopes.is_empty() {
        Vec::new()
    } else {
        search::compose_candidates(
            conn,
            tenant_id,
            &derived_scopes,
            &sensitivities,
            request.at,
            request.per_scope_kind_candidates.max(1),
        )
        .await?
    };

    let relevance_rank: Option<HashMap<RecordId, usize>> = request.relevance.as_ref().map(|ids| {
        ids.iter()
            .enumerate()
            .map(|(rank, id)| (*id, rank))
            .collect()
    });

    // Where each candidate composes as *published* (ADR-0031 decision 5,
    // as ADR-0034 decision 6 widened it): the nearest planned scope whose
    // tree names it at exactly the address its current content produces.
    // A record whose content moved since publication fails the comparison
    // and is unreviewed again — publication binds bytes, not ids — and
    // one no tree names is not hashed at all.
    let published_at = |version: &RecordVersion| {
        if !admitted
            .iter()
            .any(|(_, _, members)| members.contains_key(&version.id))
        {
            return None;
        }
        let address = memory_asset(version.id, &version.state).address();
        admitted
            .iter()
            .find(|(_, _, members)| members.get(&version.id) == Some(&address))
            .map(|(position, scope_id, _)| (*position, *scope_id))
    };

    // The two fetches can name the same record — a promoted extraction is
    // still `kind = derived`, so the sweep returns it too. Published wins,
    // and the id keys the union so nothing composes twice.
    let mut by_id: HashMap<RecordId, Candidate<'_>> = HashMap::new();
    for version in members.iter().chain(swept.iter()) {
        let (position, scope_id, channel) = match published_at(version) {
            Some((position, scope_id)) => (position, scope_id, Channel::Published),
            // Not published anywhere the caller can read it, so it is
            // derived material — and derived material composes only from
            // the scope it lives at, which must itself be planned. A
            // published fetch that came back for a record living outside
            // the plan lands here and is dropped, which is the safe
            // reading of "the tree no longer names this content".
            None => {
                let Some(position) = chain_position.get(&version.state.scope_id).copied() else {
                    continue;
                };
                (position, version.state.scope_id, Channel::Derived)
            }
        };
        // Relevance gates derived material only: published content is the
        // trust anchor and composes regardless of the task (ADR-0031
        // decision 9, ADR-0025 decision 5's rule moved to the channel it
        // was always about).
        if channel == Channel::Derived {
            let ranked = relevance_rank
                .as_ref()
                .is_none_or(|ranks| ranks.contains_key(&version.id));
            if !ranked || !admits_derived(request, position) {
                continue;
            }
        }
        let candidate = Candidate {
            version,
            scope_id,
            position,
            channel,
        };
        by_id
            .entry(version.id)
            .and_modify(|incumbent| {
                if conflict_precedence(candidate, *incumbent) == Ordering::Less {
                    *incumbent = candidate;
                }
            })
            .or_insert(candidate);
    }
    let mut survivors: Vec<Candidate<'_>> = by_id.into_values().collect();

    // Conflict resolution (ADR-0025 decision 6, ADR-0031 decision 8): one
    // winner per trimmed-content group; losers are dropped entirely.
    let mut winner_by_content: HashMap<&str, Candidate<'_>> = HashMap::new();
    for candidate in &survivors {
        winner_by_content
            .entry(candidate.version.state.content.trim())
            .and_modify(|incumbent| {
                if conflict_precedence(*candidate, *incumbent) == Ordering::Less {
                    *incumbent = *candidate;
                }
            })
            .or_insert(*candidate);
    }
    let before = survivors.len();
    survivors.retain(|candidate| {
        winner_by_content
            .get(candidate.version.state.content.trim())
            .is_some_and(|winner| winner.version.id == candidate.version.id)
    });
    let dropped_conflicts = before - survivors.len();

    // Assembly order: the gradient — nearest scope first, published
    // before derived within a scope, then pinned before derived-kind,
    // then derived by relevance rank when ranked else newest valid-from,
    // record id as the total-order tiebreak.
    survivors.sort_by(|a, b| {
        let channel = |candidate: &Candidate<'_>| match candidate.channel {
            Channel::Published => 0_u8,
            _ => 1,
        };
        let kind = |candidate: &Candidate<'_>| match candidate.version.state.kind {
            RecordKind::Pinned => 0_u8,
            RecordKind::Derived => 1,
        };
        let rank = |candidate: &Candidate<'_>| {
            relevance_rank
                .as_ref()
                .and_then(|ranks| ranks.get(&candidate.version.id).copied())
                .unwrap_or(usize::MAX)
        };
        a.position
            .cmp(&b.position)
            .then_with(|| channel(a).cmp(&channel(b)))
            .then_with(|| kind(a).cmp(&kind(b)))
            .then_with(|| rank(a).cmp(&rank(b)))
            .then_with(|| b.version.state.valid_from.cmp(&a.version.state.valid_from))
            .then_with(|| a.version.id.cmp(&b.version.id))
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

    for candidate in survivors {
        let version = candidate.version;
        let line = entry_line(candidate);
        let line_tokens = estimated_tokens(&line);
        // Sectioned by the scope it composed *from*, which for climbed
        // material is the scope that published it rather than the scope
        // it lives at (ADR-0034 decision 6) — the reader is shown the
        // section they can actually see, and a source scope's path never
        // reaches a block through a record that climbed out of it.
        let header_tokens = if open_sections.contains(&candidate.scope_id) {
            0
        } else {
            header_of
                .get(&candidate.scope_id)
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
        if open_sections.insert(candidate.scope_id)
            && let Some(header) = header_of.get(&candidate.scope_id)
        {
            pieces.push(header.clone());
        }
        pieces.push(line.clone());
        used += cost;
        watermark_chars = new_watermark_chars;
        watermark_tokens = new_watermark_tokens;
        metrics::counter!(COMPOSED_ENTRIES_TOTAL, "channel" => candidate.channel.as_str())
            .increment(1);
        entries.push(ComposedEntry {
            record_id: version.id,
            scope_id: candidate.scope_id,
            channel: candidate.channel,
            kind: version.state.kind,
            class: version.state.class,
            object_hash: memory_asset(version.id, &version.state).address().to_hex(),
            tokens: line_tokens,
        });
    }

    // The channels this block read, in scope order — kept whether or not
    // they contributed an entry.
    let channels: Vec<ChannelWatermark> = request
        .scopes
        .iter()
        .filter_map(|scope| {
            published
                .iter()
                .find(|channel| channel.scope_id == scope.scope_id)
                .map(|channel| ChannelWatermark {
                    scope_id: channel.scope_id,
                    channel: ChannelRef::memory(Channel::Published).name(),
                    commit: channel.commit.to_hex(),
                })
        })
        .collect();

    if entries.is_empty() {
        let block = ComposedBlock {
            channels,
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
        channels,
        block_hash,
        tokens,
        budget_tokens: request.budget_tokens,
        dropped_conflicts,
        skipped_over_budget,
    })
}

/// Whether derived material composes at the plan's `position`.
fn admits_derived(request: &ComposeRequest, position: usize) -> bool {
    request
        .scopes
        .get(position)
        .is_some_and(|scope| scope.include_derived)
}

/// The block over nothing: empty text, the hash of zero entries.
fn empty_block(budget_tokens: u32) -> ComposedBlock {
    ComposedBlock {
        text: String::new(),
        entries: Vec::new(),
        channels: Vec::new(),
        block_hash: blake3::Hasher::new().finalize().to_hex().to_string(),
        tokens: 0,
        budget_tokens,
        dropped_conflicts: 0,
        skipped_over_budget: 0,
    }
}

/// One entry's rendered line. Anything not published is marked
/// unreviewed (tech plan §2.2: "clearly watermarked as unreviewed") —
/// which since FLOW-2 includes authored material nobody has published.
fn entry_line(candidate: Candidate<'_>) -> String {
    let state = &candidate.version.state;
    match candidate.channel {
        Channel::Published => format!("- [{}] {}\n", state.class, state.content),
        _ => format!("- [{}] {} [unreviewed]\n", state.class, state.content),
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

/// The block's identity: BLAKE3 over the ordered entry addresses.
fn block_hash(entries: &[ComposedEntry]) -> String {
    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        hasher.update(entry.object_hash.as_bytes());
        // The channel is in the block's identity because it is in the
        // block's *text*: publishing a record removes its unreviewed
        // marker, and two blocks that read differently must not share
        // one hash.
        hasher.update(entry.channel.as_str().as_bytes());
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

    fn candidate(version: &RecordVersion, position: usize, channel: Channel) -> Candidate<'_> {
        Candidate {
            version,
            scope_id: version.state.scope_id,
            position,
            channel,
        }
    }

    /// Seed §4.4's order, below ADR-0031 decision 8's channel tier:
    /// pinned beats derived even from a broader scope; among equals,
    /// nearer scope; then newer valid-from; then the id tiebreak —
    /// total, so winners never depend on order.
    #[test]
    fn conflict_precedence_is_the_seed_order() {
        let pinned_broad = version(RecordKind::Pinned, "x", at(0), 1);
        let derived_near = version(RecordKind::Derived, "x", at(100), 2);
        assert_eq!(
            conflict_precedence(
                candidate(&pinned_broad, 3, Channel::Derived),
                candidate(&derived_near, 0, Channel::Derived)
            ),
            Ordering::Less,
            "pinned beats derived across levels"
        );

        let derived_broad = version(RecordKind::Derived, "x", at(100), 3);
        assert_eq!(
            conflict_precedence(
                candidate(&derived_near, 0, Channel::Derived),
                candidate(&derived_broad, 2, Channel::Derived)
            ),
            Ordering::Less,
            "more specific scope beats less specific"
        );

        let older = version(RecordKind::Derived, "x", at(0), 4);
        assert_eq!(
            conflict_precedence(
                candidate(&derived_near, 1, Channel::Derived),
                candidate(&older, 1, Channel::Derived)
            ),
            Ordering::Less,
            "newer valid-time beats older"
        );

        let twin_a = version(RecordKind::Derived, "x", at(100), 5);
        let twin_b = version(RecordKind::Derived, "x", at(100), 6);
        assert_eq!(
            conflict_precedence(
                candidate(&twin_a, 1, Channel::Derived),
                candidate(&twin_b, 1, Channel::Derived)
            ),
            Ordering::Less,
            "the id tiebreak makes the order total"
        );
    }

    /// ADR-0031 decision 8: the reviewed copy wins even when the
    /// unreviewed one is nearer *and* pinned. Otherwise the block would
    /// render the copy nobody approved and watermark it with that
    /// version's address.
    #[test]
    fn published_outranks_every_seed_tier() {
        let published_far = version(RecordKind::Derived, "x", at(0), 1);
        let pinned_near = version(RecordKind::Pinned, "x", at(100), 2);
        assert_eq!(
            conflict_precedence(
                candidate(&published_far, 4, Channel::Published),
                candidate(&pinned_near, 0, Channel::Derived)
            ),
            Ordering::Less,
            "published beats nearer, newer, pinned material"
        );
    }

    /// The rendered marker follows the channel, not the authorship: a
    /// pinned record nobody published still says so.
    #[test]
    fn only_published_material_renders_without_the_unreviewed_marker() {
        let pinned = version(RecordKind::Pinned, "canonical", at(0), 1);
        assert!(
            entry_line(candidate(&pinned, 0, Channel::Derived)).contains("[unreviewed]"),
            "authorship is not review"
        );
        assert!(!entry_line(candidate(&pinned, 0, Channel::Published)).contains("[unreviewed]"));
        let derived = version(RecordKind::Derived, "extracted", at(0), 2);
        assert!(!entry_line(candidate(&derived, 0, Channel::Published)).contains("[unreviewed]"));
    }

    /// The watermark is the VedaFlow object address (ADR-0031
    /// decision 11): content-bound, and *not* version-bound — the same
    /// content at the same scope has one address however many times the
    /// bitemporal pair rewrites around it.
    #[test]
    fn the_entry_address_is_the_vedaflow_object_address() {
        let a = version(RecordKind::Derived, "same", at(50), 1);
        let address = |v: &RecordVersion| memory_asset(v.id, &v.state).address();
        assert_eq!(address(&a), address(&a.clone()), "recomputable");

        let mut edited = a.clone();
        edited.state.content = "different".to_owned();
        assert_ne!(address(&a), address(&edited), "content-bound");

        let mut rewritten = a.clone();
        rewritten.tx_from = at(51);
        assert_eq!(
            address(&a),
            address(&rewritten),
            "transaction time is not content"
        );
    }
}
