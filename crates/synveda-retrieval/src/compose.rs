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
//!
//! # Tiers
//!
//! Each planned scope carries the sensitivity tiers the PDP permitted
//! there, and composition applies them per scope rather than as one
//! ceiling over the plan (AUTHZ-5, ADR-0038 decision 3): a chain can admit
//! `confidential` at the reader's own home and only the working tiers one
//! level up. For published members — fetched by id, with no scope
//! predicate, because a tree may name a record living below it — the tier
//! is checked against the *naming* scope's set, which is the only scope
//! whose permission admitted that record at all. Anything above the working
//! tier is marked in the rendered line: the harness cannot know what it is
//! holding unless the block says so.

use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::PgConnection;
use synveda_store::records::{RecordState, RecordVersion};
use synveda_store::search::{self, ScopeClassCutoff};
use synveda_types::{
    Channel, EntryTier, IndexTier, LapseId, RecordClass, RecordId, RecordKind, Result,
    RetentionConfig, ScopeId, ScopeKind, ScopeTier, Sensitivity, TenantId,
};
use synveda_vedaflow::{ChannelRef, MemoryAsset, read_memory_members};

use crate::TOKENS_PER_INJECT;
use crate::hybrid::union_sensitivities;

/// Counts composed entries, labelled by the channel they composed from —
/// the production evidence that bank mode does what its AC says
/// (ADR-0031 decision 15).
pub const COMPOSED_ENTRIES_TOTAL: &str = "synveda_composed_entries_total";

/// Histogram: estimated tokens the index tier spent per composed block —
/// every index line plus the legend when one was placed (CTX-4, ADR-0041
/// decision 14).
///
/// Recorded on every compose, a block with no index entry recording 0, on
/// the ADR-0025 decision 8 precedent: an inject that named nothing is
/// data, not an omission. This is the acceptance criterion's "token cost
/// of index tier measured" in production rather than in a test.
pub const INDEX_TIER_TOKENS: &str = "synveda_index_tier_tokens";

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
    /// The tiers the PDP permitted at this scope, ascending (AUTHZ-5,
    /// ADR-0038 decision 3).
    ///
    /// Per scope, never one ceiling for the plan: a chain can permit
    /// `confidential` at the reader's own home and only the working tiers
    /// one level up, and a single ceiling could express neither without
    /// widening or losing something. Empty never reaches here — a scope
    /// that permits no tier is not planned at all.
    pub sensitivities: Vec<Sensitivity>,
    /// The horizons this scope serves material under, and the half-life it
    /// ranks by (MEM-6, ADR-0040 decisions 2 and 12) — from the same
    /// effective-pack resolution that decided the scope.
    ///
    /// Read at every compose, never stamped on a record: a pack applied a
    /// second ago governs the very next block, which is the whole of
    /// MEM-6's acceptance criterion. It only ever removes, and pinned
    /// material is exempt by seed §4.2 rather than by anything here.
    pub retention: RetentionConfig,
    /// What happens to this scope's material when it does not fit the
    /// budget, and how wide its index line is (CTX-4, ADR-0041 decision
    /// 11) — from the same effective-pack resolution that decided the
    /// scope.
    ///
    /// Per candidate scope rather than once for the plan, exactly as the
    /// channel rule and the horizons are: a department that wants its
    /// material named rather than dropped, under a reader whose own team
    /// does not, is a coherent thing to configure, and one setting for the
    /// plan could express neither.
    pub index_tier: IndexTier,
    /// The index line's content width in characters
    /// ([`synveda_types::DEFAULT_INDEX_ENTRY_CHARS`]).
    pub index_entry_chars: u32,
    /// The lapse this scope is on the plan by, when it is not on the
    /// caller's own chain (AUTHZ-4, ADR-0037 decisions 10 and 12).
    ///
    /// `None` for every chain scope, which is almost all of them. When set,
    /// `include_derived` is always false — a lapse admits what the target
    /// published and nothing else — and the section is marked, because a
    /// block that deliberately contains another scope's material has to say
    /// so (the CTX-3 degradation-header discipline).
    pub lapse: Option<LapseId>,
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
    /// When set, only these records are considered — the recall path
    /// naming what it wants rather than sweeping (CTX-4, ADR-0041
    /// decision 5).
    ///
    /// It can only ever *remove*: every other rule — the per-scope tiers,
    /// the channel attribution, the retention cut, the valid window,
    /// conflict resolution — runs unchanged over what it leaves, which is
    /// what makes a handle a name rather than a capability. An id nobody
    /// may read is not admitted by naming it, and an id the plan admits
    /// composes exactly as a sweep would have composed it.
    pub only: Option<Vec<RecordId>>,
}

impl ComposeRequest {
    /// A request with the product defaults: the given plan and budget, 64
    /// candidates per `(scope, kind)`, no relevance ranking.
    ///
    /// There is no ceiling here to default: every scope of the plan carries
    /// the tiers the PDP permitted at it, so a composition can no longer be
    /// asked for a tier nobody decided (AUTHZ-5, ADR-0038 decision 3). A
    /// caller that wants *less* narrows the plan — which is what
    /// `POST /v1/inject`'s `max_sensitivity` does, never widening it
    /// (decision 12).
    #[must_use]
    pub fn new(scopes: Vec<ComposeScope>, budget_tokens: u32, at: DateTime<Utc>) -> Self {
        Self {
            scopes,
            budget_tokens,
            at,
            relevance: None,
            per_scope_kind_candidates: 64,
            only: None,
        }
    }

    /// The same plan restricted to named records, with the index tier off
    /// and the budget wide: the shape `POST /v1/recall` composes under
    /// (ADR-0041 decisions 5 and 7).
    ///
    /// Recall serves bodies in full — the caller named specific records,
    /// which is what makes it the deep surface rather than a second
    /// inject — so nothing here may demote, and the budget must not be
    /// able to truncate the answer.
    #[must_use]
    pub fn naming(scopes: Vec<ComposeScope>, ids: Vec<RecordId>, at: DateTime<Utc>) -> Self {
        let mut request = ComposeRequest::new(scopes, u32::MAX, at);
        for scope in &mut request.scopes {
            scope.index_tier = IndexTier::Off;
        }
        // The per-(scope, kind) cap exists so a flood of derived records
        // cannot crowd out the fetch. Naming ids is already the bound, and
        // a cap here would silently answer "not found" for a record the
        // caller may perfectly well read.
        request.per_scope_kind_candidates = i64::MAX;
        request.only = Some(ids);
        request
    }

    /// Narrows every planned scope to tiers at or below `ceiling`.
    ///
    /// A scope left with no tier stops composing entirely — the honest
    /// consequence of asking for less, and never a widening: the plan is
    /// the PDP's answer and this only removes from it.
    #[must_use]
    pub fn narrowed_to(mut self, ceiling: Sensitivity) -> Self {
        for scope in &mut self.scopes {
            scope.sensitivities.retain(|tier| *tier <= ceiling);
        }
        self.scopes.retain(|scope| !scope.sensitivities.is_empty());
        self
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
    /// How fresh the entry was at the composition instant, per mille:
    /// `1000` is fresh, halving every half-life the scope's pack
    /// configures (MEM-6, ADR-0040 decision 12).
    ///
    /// Per mille rather than a float for the reason ADR-0019 decision 2
    /// gives about the audit payload and ADR-0039 decision 13 gives about
    /// the supersession edge: a number jsonb or a client may reshape is a
    /// number nobody can compare later. Pinned material is always `1000` —
    /// it cannot be decayed (seed §4.2).
    ///
    /// It rides the response rather than the rendered block: the block's
    /// labels are trust statements, and an age is not one (ADR-0040
    /// decision 12).
    pub staleness_permille: u16,
    /// How much of the record composed: its full content, or its elided
    /// head plus a recall handle (CTX-4, ADR-0041 decision 9).
    ///
    /// On the entry rather than derived from the text, because "was that
    /// agent given the payments runbook or only told it exists" is a
    /// question an auditor asks about a corpus that has since moved, and
    /// re-deriving it from rendered widths would need the record as it was
    /// rather than as it is.
    pub tier: EntryTier,
}

/// One channel the block read: the commit a scope's channel served at
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
    /// The commit it served, hex-encoded.
    pub commit: String,
    /// Whether a pin chose that commit rather than the ref (FLOW-7,
    /// ADR-0036 decision 10).
    ///
    /// A watermark that cites a frozen commit without saying it is frozen
    /// invites "the agent had the latest reviewed material", which is
    /// exactly what a pinned scope has decided against. The same
    /// discipline as CTX-3's degradation header: a response that
    /// deliberately differs from the expected one has to say so.
    pub pinned: bool,
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
    /// Candidates that survived every rule but did not fit the budget —
    /// and, since CTX-4, could not be named within it either. A candidate
    /// that was demoted to an index entry is *not* counted here: it
    /// composed (ADR-0041 decision 2).
    pub skipped_over_budget: usize,
    /// Entries that composed as index lines rather than bodies.
    pub index_entries: usize,
    /// Estimated tokens the index tier spent: every index line plus the
    /// legend, when one was placed.
    ///
    /// The AC's measurement (ADR-0041 decision 14), carried on the block
    /// so the number is the product's own rather than a test's
    /// re-derivation.
    pub index_tokens: u32,
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
    /// The freshness score at the composition instant, `0.0..=1.0`, under
    /// the retention config of the scope it composed from (MEM-6,
    /// ADR-0040 decision 12). `1.0` for pinned material and wherever a
    /// pack configures no half-life.
    pub staleness: f64,
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

/// When a record was last *asserted* — the staleness clock, which is
/// deliberately not the retention clock (MEM-6, ADR-0040 decisions 3 and
/// 12).
///
/// Retention asks how long we have held a fact and runs from `valid_from`;
/// staleness asks how long since anyone confirmed it, so a MEM-5 merge
/// counts. `provenance.merged.last_observed_at` is the stamp
/// `records::reinforce` writes on every absorbed restatement (ADR-0039
/// decision 10), and a record nobody has restated simply has none.
///
/// A malformed stamp — hand-written provenance, a client that wrote
/// something else — falls back to `valid_from` rather than failing the
/// compose: freshness is a ranking heuristic, and a heuristic must not be
/// able to break a block.
fn last_asserted_at(state: &RecordState) -> DateTime<Utc> {
    state
        .provenance
        .get("merged")
        .and_then(|merged| merged.get("last_observed_at"))
        .and_then(serde_json::Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|stamp| stamp.with_timezone(&Utc))
        .filter(|stamp| *stamp > state.valid_from)
        .unwrap_or(state.valid_from)
}

/// A record's freshness at `at` under `retention`: `1.0` for pinned
/// material, which seed §4.2 says cannot be decayed, and the config's
/// half-life decay over time since last assertion for everything else.
fn staleness_of(state: &RecordState, retention: &RetentionConfig, at: DateTime<Utc>) -> f64 {
    if !RetentionConfig::governs(state.kind) {
        return 1.0;
    }
    retention.staleness(last_asserted_at(state), at)
}

/// A relevance rank aged by a freshness score: `(rank + 1) / staleness`
/// (MEM-6, ADR-0040 decision 12).
///
/// Fresh material (`1.0`) keeps its rank exactly, so a corpus with no
/// half-life configured sorts precisely as it did before this feature.
/// A zero or non-finite score — unreachable through
/// [`synveda_types::RetentionConfig::staleness`], which clamps — sorts
/// last rather than producing an infinity the comparator cannot order.
fn decayed_rank(rank: usize, staleness: f64) -> f64 {
    let position = (rank as f64) + 1.0;
    if staleness <= 0.0 || !staleness.is_finite() {
        return f64::MAX;
    }
    position / staleness
}

/// A freshness score as it is reported: per mille, rounded, clamped.
fn permille(score: f64) -> u16 {
    if score.is_nan() {
        return 0;
    }
    (score * 1000.0).round().clamp(0.0, 1000.0) as u16
}

/// Whether `retention` still serves a record of this state at `at` — the
/// read cut for material fetched by id rather than swept (ADR-0040
/// decision 2).
///
/// The derived sweep applies the same rule in SQL; this is the published
/// path, where the predicate cannot be a scope column because a published
/// tree may name a record living below it (ADR-0034 decision 6).
fn retention_admits(state: &RecordState, retention: &RetentionConfig, at: DateTime<Utc>) -> bool {
    if !RetentionConfig::governs(state.kind) {
        return true;
    }
    retention
        .cutoff(state.class, at)
        .is_none_or(|cutoff| state.valid_from > cutoff)
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
        index.entries = tracing::field::Empty,
        index.tokens = tracing::field::Empty,
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
        span.record("index.entries", 0);
        span.record("index.tokens", 0);
        metrics::histogram!(TOKENS_PER_INJECT).record(0.0);
        metrics::histogram!(INDEX_TIER_TOKENS).record(0.0);
        return Ok(block);
    }

    let Admission {
        records,
        channels,
        dropped_conflicts,
    } = admit(conn, tenant_id, request).await?;
    let survivors: Vec<Candidate<'_>> = records.iter().map(Admitted::candidate).collect();
    assemble(request, survivors, channels, dropped_conflicts, &span)
}

/// One record the plan admits, with everything the decision produced:
/// which scope admitted it, that scope's gradient position, the channel it
/// is on there, and how fresh it was at the instant asked about.
///
/// Owned rather than borrowed because two callers need it — [`compose`],
/// which renders it into a budgeted block, and `POST /v1/recall`, which
/// serves it in full (ADR-0041 decision 5). One admission function is what
/// makes a recall handle a name rather than a capability: there is no
/// second place where "may this caller see this record" is decided, so
/// there is nothing for a handle to reach around.
#[derive(Debug, Clone)]
pub struct Admitted {
    /// The record version that was admitted.
    pub version: RecordVersion,
    /// The scope that admitted it — the one whose channel carried it and
    /// whose `MemoryRead` decision covered it, which since FLOW-5 need not
    /// be where the record lives (ADR-0034 decision 6).
    pub scope_id: ScopeId,
    /// That scope's position in the gradient, nearest = 0.
    pub position: usize,
    /// Published or derived at that scope — the trust label.
    pub channel: Channel,
    /// Freshness at the instant, `0.0..=1.0` (MEM-6, ADR-0040 decision 12).
    pub staleness: f64,
}

impl Admitted {
    /// The borrowed view assembly and the conflict comparator work over.
    #[must_use]
    pub fn candidate(&self) -> Candidate<'_> {
        Candidate {
            version: &self.version,
            scope_id: self.scope_id,
            position: self.position,
            channel: self.channel,
            staleness: self.staleness,
        }
    }
}

/// What a composition plan admits: the records, the channels the plan
/// read, and how many candidates conflict resolution dropped.
#[derive(Debug, Clone)]
pub struct Admission {
    /// Every record the plan admits, conflict-resolved. Unordered — the
    /// gradient is applied by whoever renders.
    pub records: Vec<Admitted>,
    /// The channel refs this plan read, in scope order — present even
    /// where a channel contributed nothing.
    pub channels: Vec<ChannelWatermark>,
    /// Candidates dropped by conflict resolution (ADR-0025 decision 6).
    pub dropped_conflicts: usize,
}

/// Decides what the plan admits: the published-channel attribution, the
/// derived sweep under each scope's channel rule and horizons, the
/// per-scope tier check, relevance gating, and conflict resolution.
///
/// The whole of "what may this caller see", in one place. [`compose`]
/// renders its answer under a budget; recall serves it in full. Neither
/// re-decides anything, which is the seed §2.2 posture applied to a
/// surface that could otherwise have become a way around it.
#[tracing::instrument(
    name = "retrieval.admit",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        scopes.count = request.scopes.len(),
        named = request.only.as_ref().map_or(-1, |ids| i64::try_from(ids.len()).unwrap_or(i64::MAX)),
        admitted = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn admit(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    request: &ComposeRequest,
) -> Result<Admission> {
    if request.scopes.is_empty() {
        return Ok(Admission {
            records: Vec::new(),
            channels: Vec::new(),
            dropped_conflicts: 0,
        });
    }
    let scope_ids: Vec<ScopeId> = request.scopes.iter().map(|scope| scope.scope_id).collect();
    // The plan's own pairs: one scope's permitted tiers never leak into
    // another's, which is what a single ceiling could not express
    // (ADR-0038 decision 3).
    let allowed: Vec<ScopeTier> = request
        .scopes
        .iter()
        .flat_map(|scope| ScopeTier::expand(scope.scope_id, &scope.sensitivities))
        .collect();
    // What every planned scope permits, taken together. It bounds the
    // published-member read, which has no scope predicate by design
    // (ADR-0034 decision 6) — the exact pair is enforced below, where the
    // tree that named each member is known.
    let sensitivities = union_sensitivities(&allowed);
    let tier_at: HashMap<ScopeId, &[Sensitivity]> = request
        .scopes
        .iter()
        .map(|scope| (scope.scope_id, scope.sensitivities.as_slice()))
        .collect();
    // The horizons each planned scope serves under, and the half-life it
    // ranks by (MEM-6, ADR-0040 decisions 10 and 12). Keyed by scope for
    // the same reason as the tiers: a record composes from the scope whose
    // channel admitted it, which since FLOW-5 need not be where it lives.
    let retention_at: HashMap<ScopeId, RetentionConfig> = request
        .scopes
        .iter()
        .map(|scope| (scope.scope_id, scope.retention))
        .collect();

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
    // Named ids narrow the published read the same way they narrow the
    // derived one: recall asks for records, not for a channel's contents
    // (ADR-0041 decision 5).
    let published_ids: Vec<RecordId> = published
        .iter()
        .flat_map(|channel| channel.members.keys().copied())
        .filter(|id| request.only.as_ref().is_none_or(|ids| ids.contains(id)))
        .collect();

    // Published members are fetched by id and uncapped; derived is the
    // capped per-(scope, kind) sweep, and only over scopes whose pack
    // admits it — bank mode removes the read rather than filtering its
    // results (ADR-0031 decisions 9 and 10).
    let derived_allowed: Vec<ScopeTier> = request
        .scopes
        .iter()
        .filter(|scope| scope.include_derived)
        .flat_map(|scope| ScopeTier::expand(scope.scope_id, &scope.sensitivities))
        .collect();
    // The retention cut, per (scope, class), carried into SQL beside the
    // tier pairs (MEM-6, ADR-0040 decision 2). Only classes the scope's
    // pack actually schedules appear: a class it keeps is absent, never
    // present with a cutoff at the beginning of time.
    let horizons: Vec<ScopeClassCutoff> = request
        .scopes
        .iter()
        .filter(|scope| scope.include_derived)
        .flat_map(|scope| {
            RecordClass::ALL.into_iter().filter_map(move |class| {
                scope
                    .retention
                    .cutoff(class, request.at)
                    .map(|cutoff| ScopeClassCutoff {
                        scope_id: scope.scope_id,
                        class,
                        cutoff,
                    })
            })
        })
        .collect();
    // Published members are fetched by id alone: tree membership at a
    // planned scope is the predicate, because that tree may name a record
    // living below it (ADR-0034 decision 6). Residence still decides for
    // the derived sweep.
    let members =
        search::compose_members(conn, tenant_id, &published_ids, &sensitivities, request.at)
            .await?;
    let swept = if derived_allowed.is_empty() {
        Vec::new()
    } else {
        search::compose_candidates(
            conn,
            tenant_id,
            &derived_allowed,
            &horizons,
            request.at,
            request.per_scope_kind_candidates.max(1),
            request.only.as_deref(),
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
    // tree names it at exactly the address its current content produces
    // **and whose own tier set admits its sensitivity**. A record whose
    // content moved since publication fails the comparison and is
    // unreviewed again — publication binds bytes, not ids — and one no tree
    // names is not hashed at all.
    //
    // The tier is checked here rather than in SQL because this read has no
    // scope predicate by design: a published tree may name a record living
    // below its scope (ADR-0034 decision 6), so residence cannot answer
    // "which scope's permission admitted this". The tree that named it can,
    // and this is where that is known (ADR-0038 decision 3).
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
            .find(|(_, scope_id, members)| {
                members.get(&version.id) == Some(&address)
                    && tier_at
                        .get(scope_id)
                        .is_some_and(|tiers| tiers.contains(&version.state.sensitivity))
                    // A scope's pack decides what that scope *serves*
                    // (ADR-0040 decision 10). A record past the publishing
                    // scope's horizon stops composing there; if it lives at
                    // another planned scope whose own schedule still keeps
                    // it, it falls through to that scope's derived material,
                    // which is the honest reading of two schedules
                    // disagreeing.
                    && retention_at
                        .get(scope_id)
                        .is_some_and(|retention| {
                            retention_admits(&version.state, retention, request.at)
                        })
            })
            .map(|(position, scope_id, _)| (*position, *scope_id))
    };

    // The two fetches can name the same record — a promoted extraction is
    // still `kind = derived`, so the sweep returns it too. Published wins,
    // and the id keys the union so nothing composes twice.
    let mut by_id: HashMap<RecordId, Admitted> = HashMap::new();
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
            // The derived sweep already applied this scope's horizon in
            // SQL; a record that arrived through the published fetch and
            // fell through to here has not been asked (ADR-0040
            // decision 2). Asking twice costs a comparison and closes the
            // only path around the cut.
            if !retention_at
                .get(&scope_id)
                .is_some_and(|retention| retention_admits(&version.state, retention, request.at))
            {
                continue;
            }
        }
        let record = Admitted {
            version: version.clone(),
            scope_id,
            position,
            channel,
            // Scored under the pack of the scope it composed *from*: the
            // block is that scope's material as far as this reader is
            // concerned, so it is that scope's half-life that ages it.
            staleness: retention_at.get(&scope_id).map_or(1.0, |retention| {
                staleness_of(&version.state, retention, request.at)
            }),
        };
        match by_id.entry(version.id) {
            Entry::Occupied(mut slot) => {
                if conflict_precedence(record.candidate(), slot.get().candidate()) == Ordering::Less
                {
                    slot.insert(record);
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(record);
            }
        }
    }
    let mut records: Vec<Admitted> = by_id.into_values().collect();

    // Conflict resolution (ADR-0025 decision 6, ADR-0031 decision 8): one
    // winner per trimmed-content group; losers are dropped entirely.
    let mut winner_by_content: HashMap<&str, (RecordId, Candidate<'_>)> = HashMap::new();
    for record in &records {
        let candidate = record.candidate();
        winner_by_content
            .entry(record.version.state.content.trim())
            .and_modify(|(id, incumbent)| {
                if conflict_precedence(candidate, *incumbent) == Ordering::Less {
                    *id = record.version.id;
                    *incumbent = candidate;
                }
            })
            .or_insert((record.version.id, candidate));
    }
    let winners: HashSet<RecordId> = winner_by_content.values().map(|(id, _)| *id).collect();
    drop(winner_by_content);
    let before = records.len();
    records.retain(|record| winners.contains(&record.version.id));
    let dropped_conflicts = before - records.len();

    // The channels this plan read, in scope order — kept whether or not
    // they contributed a record.
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
                    pinned: channel.pinned,
                })
        })
        .collect();

    tracing::Span::current().record("admitted", records.len());
    Ok(Admission {
        records,
        channels,
        dropped_conflicts,
    })
}

/// Orders the admitted set by the seed §4.4 gradient and assembles it
/// under the budget, demoting what does not fit to the index tier where
/// the scope's pack allows (ADR-0041 decision 2), then renders and
/// watermarks.
fn assemble(
    request: &ComposeRequest,
    mut survivors: Vec<Candidate<'_>>,
    channels: Vec<ChannelWatermark>,
    dropped_conflicts: usize,
    span: &tracing::Span,
) -> Result<ComposedBlock> {
    let relevance_rank: Option<HashMap<RecordId, usize>> = request.relevance.as_ref().map(|ids| {
        ids.iter()
            .enumerate()
            .map(|(rank, id)| (*id, rank))
            .collect()
    });

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
        // Relevance, decayed by freshness (MEM-6, ADR-0040 decision 12):
        // a record that has halved in freshness sorts as though it ranked
        // twice as far down. Total and deterministic — the score is a
        // function of the rank and the instant the caller passed in, never
        // of a clock — and it only ever reorders *within* a gradient
        // position, because position, channel and kind are compared first
        // and seed §4.4 owns those.
        //
        // Unranked material (a taskless session) keeps its existing order:
        // `valid_from` descending is already newest-first, so there is
        // nothing for a freshness score to add there.
        let rank = |candidate: &Candidate<'_>| {
            relevance_rank
                .as_ref()
                .and_then(|ranks| ranks.get(&candidate.version.id).copied())
                .map(|rank| decayed_rank(rank, candidate.staleness))
        };
        a.position
            .cmp(&b.position)
            .then_with(|| channel(a).cmp(&channel(b)))
            .then_with(|| kind(a).cmp(&kind(b)))
            .then_with(|| match (rank(a), rank(b)) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
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
            // A lapsed section says so. The reader is not a member of this
            // scope and reached it through a time-boxed grant; a header
            // identical to their own team's would be the block claiming
            // otherwise (ADR-0037 decision 12).
            let marker = if scope.lapse.is_some() {
                " [lapse]"
            } else {
                ""
            };
            (
                scope.scope_id,
                format!("\n## {} ({}){marker}\n", scope.path, scope.kind),
            )
        })
        .collect();
    // What each planned scope does with material that does not fit, and
    // how wide its index line is (CTX-4, ADR-0041 decision 11) — keyed by
    // scope for the same reason the tiers and the horizons are.
    let index_at: HashMap<ScopeId, (IndexTier, u32)> = request
        .scopes
        .iter()
        .map(|scope| (scope.scope_id, (scope.index_tier, scope.index_entry_chars)))
        .collect();
    let legend_tokens = estimated_tokens(INDEX_LEGEND);

    let mut open_sections: HashSet<ScopeId> = HashSet::new();
    let mut pieces: Vec<String> = vec![preamble];
    let mut entries: Vec<ComposedEntry> = Vec::new();
    let mut skipped_over_budget = 0_usize;
    let mut index_entries = 0_usize;
    let mut index_tokens = 0_u32;
    let mut legend_placed = false;

    for candidate in survivors {
        let version = candidate.version;
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
        // width so rounding never drifts. An index entry is watermarked
        // like a body: the block disclosed that the record exists, and a
        // disclosure the watermark does not cover is one nobody can audit
        // (ADR-0041 decision 10).
        let id_chars = version.id.to_string().chars().count() + usize::from(!entries.is_empty());
        let new_watermark_chars = watermark_chars + id_chars;
        let new_watermark_tokens =
            u32::try_from(new_watermark_chars.div_ceil(4)).unwrap_or(u32::MAX);
        let fixed = header_tokens + (new_watermark_tokens - watermark_tokens);

        let body = body_line(candidate);
        let body_tokens = estimated_tokens(&body);
        // The body first, always: the index tier is what happens when the
        // budget has run out for this candidate, never a policy about how
        // much of a record a reader deserves.
        let placed = if used.saturating_add(body_tokens + fixed) <= request.budget_tokens {
            Some((body, body_tokens, EntryTier::Body, 0))
        } else {
            demote(
                candidate,
                index_at.get(&candidate.scope_id).copied(),
                body_tokens,
                if legend_placed { 0 } else { legend_tokens },
            )
            .filter(|(_, line_tokens, _, legend_cost)| {
                used.saturating_add(line_tokens + fixed + legend_cost) <= request.budget_tokens
            })
        };
        let Some((line, line_tokens, tier, legend_cost)) = placed else {
            skipped_over_budget += 1;
            continue;
        };

        if open_sections.insert(candidate.scope_id)
            && let Some(header) = header_of.get(&candidate.scope_id)
        {
            pieces.push(header.clone());
        }
        pieces.push(line);
        used += line_tokens + fixed + legend_cost;
        watermark_chars = new_watermark_chars;
        watermark_tokens = new_watermark_tokens;
        if tier == EntryTier::Index {
            index_entries += 1;
            index_tokens += line_tokens + legend_cost;
            legend_placed = true;
        }
        metrics::counter!(
            COMPOSED_ENTRIES_TOTAL,
            "channel" => candidate.channel.as_str(),
            "tier" => tier.as_str(),
        )
        .increment(1);
        entries.push(ComposedEntry {
            record_id: version.id,
            scope_id: candidate.scope_id,
            channel: candidate.channel,
            kind: version.state.kind,
            class: version.state.class,
            object_hash: memory_asset(version.id, &version.state).address().to_hex(),
            tokens: line_tokens,
            staleness_permille: permille(candidate.staleness),
            tier,
        });
    }

    // The legend goes after the preamble and before the first section —
    // its cost was already charged to the demotion that earned it, so the
    // placement moves no tokens (ADR-0041 decision 12).
    if legend_placed {
        pieces.insert(1, INDEX_LEGEND.to_owned());
    }

    if entries.is_empty() {
        let block = ComposedBlock {
            channels,
            dropped_conflicts,
            skipped_over_budget,
            ..empty_block(request.budget_tokens)
        };
        span.record("entries", 0);
        span.record("tokens", 0);
        span.record("index.entries", 0);
        span.record("index.tokens", 0);
        metrics::histogram!(TOKENS_PER_INJECT).record(0.0);
        metrics::histogram!(INDEX_TIER_TOKENS).record(0.0);
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
    span.record("index.entries", index_entries);
    span.record("index.tokens", index_tokens);
    metrics::histogram!(TOKENS_PER_INJECT).record(f64::from(tokens));
    metrics::histogram!(INDEX_TIER_TOKENS).record(f64::from(index_tokens));
    Ok(ComposedBlock {
        text,
        entries,
        channels,
        block_hash,
        tokens,
        budget_tokens: request.budget_tokens,
        dropped_conflicts,
        skipped_over_budget,
        index_entries,
        index_tokens,
    })
}

/// The index line a candidate takes when its body did not fit — or `None`
/// when the scope's pack does not demote, or when naming the record would
/// not actually be cheaper than showing it (ADR-0041 decision 2).
///
/// Returns the line, its estimate, the tier, and the legend's cost when
/// this demotion is the one that has to pay for it.
fn demote<'a>(
    candidate: Candidate<'a>,
    index: Option<(IndexTier, u32)>,
    body_tokens: u32,
    legend_cost: u32,
) -> Option<(String, u32, EntryTier, u32)> {
    let (tier, entry_chars) = index?;
    if !tier.demotes() {
        return None;
    }
    let line = index_line(candidate, entry_chars);
    let line_tokens = estimated_tokens(&line);
    // Strictly cheaper or not at all. Demoting a short record would spend
    // budget to say less, and the median memory record is short because
    // MEM-3 summarises at write time — this one comparison is what keeps a
    // mechanism built for assets that do not exist yet from making the
    // corpus that does exist worse.
    (line_tokens < body_tokens).then_some((line, line_tokens, EntryTier::Index, legend_cost))
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
        index_entries: 0,
        index_tokens: 0,
    }
}

/// One entry's rendered line. Anything not published is marked
/// unreviewed (tech plan §2.2: "clearly watermarked as unreviewed") —
/// which since FLOW-2 includes authored material nobody has published.
///
/// A tier above the working one is marked too (AUTHZ-5, ADR-0038
/// decision 11): the harness is a guest (seed §2.6) and cannot know what it
/// is holding unless the block says so, and an agent that has been told a
/// line is `confidential` can behave differently about pasting it into a
/// pull request. `public` and `internal` are left unmarked — a label on
/// every line is a label nobody reads.
fn body_line(candidate: Candidate<'_>) -> String {
    render_line(candidate, &candidate.version.state.content, "")
}

/// One entry's rendered line at the index tier (CTX-4, ADR-0041 decision
/// 3): the same class and the same trust markers over an elided head,
/// followed by the handle that fetches the rest.
///
/// The ellipsis and the handle are the marker. One piece of text says both
/// "this is not the whole thing" and "here is how to get the whole thing",
/// and a block that spent budget on a separate `[index]` label would be
/// spending it to say what the line already says. Decision 2 keeps that
/// honest: a record short enough not to elide is never demoted here, so
/// every index line in a block is genuinely truncated.
fn index_line(candidate: Candidate<'_>, entry_chars: u32) -> String {
    let head = elide(&candidate.version.state.content, entry_chars);
    let handle = format!(" (recall {})", candidate.version.id);
    render_line(candidate, &head, &handle)
}

/// The shared line shape, so a body and an index entry can never disagree
/// about a trust marker.
fn render_line(candidate: Candidate<'_>, content: &str, suffix: &str) -> String {
    let state = &candidate.version.state;
    let tier = if state.sensitivity > Sensitivity::WORKING {
        format!(" [{}]", state.sensitivity)
    } else {
        String::new()
    };
    match candidate.channel {
        Channel::Published => format!("- [{}] {content}{tier}{suffix}\n", state.class),
        _ => format!("- [{}] {content}{tier} [unreviewed]{suffix}\n", state.class),
    }
}

/// Truncates to `chars` Unicode scalar values on a character boundary,
/// marking the cut. Deterministic and allocation-bounded; no clock, no
/// model, nothing the read path may not do (ADR-0024 decision 7).
fn elide(content: &str, chars: u32) -> String {
    let limit = usize::try_from(chars).unwrap_or(usize::MAX);
    let mut head: String = content.chars().take(limit).collect();
    if content.chars().nth(limit).is_some() {
        // Trailing space before an ellipsis reads as a typo, and the
        // trimmed form is just as deterministic.
        head.truncate(head.trim_end().len());
        head.push('…');
    }
    head
}

/// The one line that makes an index entry navigable. Charged to the first
/// demotion (ADR-0041 decision 12), so a block with no index entry never
/// pays for it and stays byte-identical to what CTX-2 rendered.
///
/// It deliberately does **not** contain the parenthesised `(recall …)`
/// form itself. An agent locating handles by scanning the block for that
/// form would otherwise find this sentence first and go looking for a
/// record called `<id>`: a legend has to describe the marker without
/// being one.
const INDEX_LEGEND: &str =
    "Summarised entries end with a recall handle; `synveda recall <id>` fetches the full text.\n";

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
            // These unit tests are about seed §4.4's order, which
            // freshness never reorders across (ADR-0040 decision 12).
            staleness: 1.0,
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
            body_line(candidate(&pinned, 0, Channel::Derived)).contains("[unreviewed]"),
            "authorship is not review"
        );
        assert!(!body_line(candidate(&pinned, 0, Channel::Published)).contains("[unreviewed]"));
        let derived = version(RecordKind::Derived, "extracted", at(0), 2);
        assert!(!body_line(candidate(&derived, 0, Channel::Published)).contains("[unreviewed]"));
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
