//! The promotion sweep and rule engine (FLOW-4, ADR-0033).
//!
//! Two phases, both off the read path. **Sweep** folds `context.injected`
//! events forward from a durable per-tenant watermark into the usage
//! projection; **evaluate** takes exactly the records that fold touched,
//! resolves the effective pack at each of their scopes, and opens a
//! proposal for every batch a rule fires on.
//!
//! Nothing here is added to `inject`. The signal it needs — which records
//! were composed into whose block — is already recorded, inside inject's
//! own transaction, under the audit chain's hash. Counting it there would
//! mean another statement inside the one critical section that holds the
//! per-tenant chain-head lock (ADR-0033 decision 1), so this reads it
//! afterwards instead and the read path pays nothing.
//!
//! Every proposal it opens is an ordinary FLOW-3 proposal, opened under
//! the material owner's authority through a real `ProposalOpen` decision
//! (ADR-0033 decision 9). There is no system principal here: a rule
//! cannot propose what its owner could not.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome, StoredEvent};
use synveda_policy::{Action, AuthzContext, AuthzDecision, Pdp, Principal, Resource};
use synveda_store::promotion::{self as usage, UsageDelta, UsageRow};
use synveda_store::records::{self, RecordVersion};
use synveda_store::{ScopeChainCache, identities, policy_assignments, rls, role_bindings, tenants};
use synveda_types::{
    AssetKind, Channel, Error, HierarchyNode, IdentityId, IdentityKind, MemberEvidence,
    PromotionEvidence, PromotionRule, RecordId, Result, ScopeId, Sensitivity, TenantId,
};
use synveda_vedaflow::hash::{ObjectHash, object_hash};
use synveda_vedaflow::{self as vedaflow, PolicySnapshot, Signer};

/// Counter: proposals the rule engine opened, labelled `rule`.
pub const PROMOTION_PROPOSALS_TOTAL: &str = "synveda_promotion_proposals_total";

/// Counter: records a rule selected, labelled `rule` and
/// `outcome = proposed | suppressed`.
pub const PROMOTION_CANDIDATES_TOTAL: &str = "synveda_promotion_candidates_total";

/// Gauge: audit events between the sweeper's watermark and the chain
/// head after a pass. Growing across passes is ADR-0033's reversal
/// trigger (a).
pub const PROMOTION_SWEEP_LAG_EVENTS: &str = "synveda_promotion_sweep_lag_events";

/// Counter: scopes where the open-proposal cap stopped the engine from
/// proposing. Nonzero is ADR-0032's reversal trigger (a) firing.
pub const PROMOTION_QUEUE_FULL_TOTAL: &str = "synveda_promotion_queue_full_total";

/// The audit actor component for this engine (ADR-0022 decision 5's
/// actor kind).
const ACTOR_COMPONENT: &str = "promotion";

/// Which audit actions count as a recall (ADR-0033 decision 5).
///
/// One today. CTX-5's explicit recall is a stronger signal than
/// composition and joins by being added here — the projection, the
/// rules, and the evidence shape do not change. Every proposal records
/// the set that was counted, so one opened before that day cannot be
/// misread as having counted it.
const SWEPT_ACTIONS: &[AuditAction] = &[AuditAction::ContextInjected];

/// The engine's tuning knobs, parsed from `SYNVEDA_PROMOTION_*` by the
/// gateway.
#[derive(Debug, Clone)]
pub struct SweepConfig {
    /// How often a pass runs. Minutes, not the extraction worker's
    /// second: a promotion is not a hot path, and a sweep that runs
    /// while a reviewer is asleep is exactly as useful as one that runs
    /// now.
    pub interval: Duration,
    /// Audit events folded per tenant per pass.
    pub batch: i64,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(300),
            batch: 1024,
        }
    }
}

/// What the engine holds for its lifetime.
#[derive(Clone)]
pub struct SweepDeps {
    /// The shared connection pool.
    pub pool: PgPool,
    /// The embedded PDP; every proposal is a real decision (seed §2.2).
    pub pdp: Arc<Pdp>,
    /// The gateway's scope-chain cache — pass a clone of the gateway's
    /// `Arc`, never a fresh cache, or move invalidations are lost.
    pub chains: Arc<ScopeChainCache>,
}

/// What one pass did, across every tenant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Tenants examined.
    pub tenants: usize,
    /// Audit events folded into the projection.
    pub events_folded: usize,
    /// Records whose usage moved and were therefore evaluated.
    pub candidates: usize,
    /// Proposals opened.
    pub proposals_opened: usize,
}

/// What one pass did for one tenant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TenantPass {
    /// Audit events folded into the projection.
    pub events_folded: usize,
    /// Records whose usage moved and were therefore evaluated.
    pub candidates: usize,
    /// Proposals opened.
    pub proposals_opened: usize,
}

/// Spawns the engine loop. Abort the handle on shutdown (the
/// pack-refresher shape).
#[must_use]
pub fn spawn(deps: SweepDeps, config: SweepConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = run_once(&deps, &config).await {
                tracing::error!(error = %error, "promotion pass failed; the watermark holds");
            }
        }
    })
}

/// One pass over every active tenant.
#[tracing::instrument(name = "ingest.promotion.run_once", skip_all, err(Display))]
pub async fn run_once(deps: &SweepDeps, config: &SweepConfig) -> Result<SweepReport> {
    let tenants = tenants::active(&deps.pool).await?;
    let mut report = SweepReport::default();
    for tenant in tenants {
        report.tenants += 1;
        // One tenant's failure must not strand the rest: its watermark
        // simply does not advance, and the next pass refolds from where
        // it stood.
        match run_tenant(deps, config, tenant.id).await {
            Ok(pass) => {
                report.events_folded += pass.events_folded;
                report.candidates += pass.candidates;
                report.proposals_opened += pass.proposals_opened;
            }
            Err(error) => tracing::error!(
                tenant.id = %tenant.id,
                error = %error,
                "promotion pass failed for this tenant; the watermark holds"
            ),
        }
    }
    Ok(report)
}

/// One pass for one tenant: fold, then evaluate exactly what the fold
/// touched.
///
/// The per-tenant entry point, on `indexer::sweep_tenant`'s shape (CTX-1)
/// and for the same two reasons: a demo or a test drives one tenant
/// deterministically instead of waiting on a background loop, and a
/// tenant is the unit of work anyway — its watermark, its chain, its
/// lock.
///
/// Evaluation considers what *this* fold touched, so the sweeper that
/// folds is the one that evaluates. That is what keeps work proportional
/// to changed usage (ADR-0033 decision 14) rather than to the corpus.
#[tracing::instrument(
    name = "ingest.promotion.run_tenant",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn run_tenant(
    deps: &SweepDeps,
    config: &SweepConfig,
    tenant_id: TenantId,
) -> Result<TenantPass> {
    let swept = sweep(deps, config, tenant_id).await?;
    let mut pass = TenantPass {
        events_folded: swept.folded,
        candidates: swept.touched.len(),
        proposals_opened: 0,
    };
    if swept.touched.is_empty() {
        return Ok(pass);
    }
    match evaluate(deps, tenant_id, &swept).await {
        Ok(opened) => pass.proposals_opened = opened,
        Err(error) => tracing::error!(
            tenant.id = %tenant_id,
            error = %error,
            "promotion evaluation failed; usage stands, no proposal opened"
        ),
    }
    Ok(pass)
}

/// What a sweep folded.
#[derive(Debug, Clone)]
struct Swept {
    /// Records whose counters moved — exactly what evaluation considers
    /// (ADR-0033 decision 14).
    touched: Vec<RecordId>,
    /// Audit events folded.
    folded: usize,
    /// The last seq the projection now covers, for the evidence range.
    to_seq: i64,
}

/// Folds one tenant's chain forward from its watermark.
#[tracing::instrument(
    name = "ingest.promotion.sweep",
    skip_all,
    fields(tenant.id = %tenant_id, folded = tracing::field::Empty),
    err(Display)
)]
async fn sweep(deps: &SweepDeps, config: &SweepConfig, tenant_id: TenantId) -> Result<Swept> {
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;

    // The idle check, before any write and before the lock. A pass visits
    // every tenant, and most tenants have nothing new on most passes: an
    // engine that took a row lock and wrote a watermark just to discover
    // that would make an idle estate the expensive case.
    //
    // Reading the head *before* the events is also what makes advancing
    // to it safe later — anything appended after this read has a higher
    // seq and is picked up next pass.
    let watermark = usage::watermark(&mut *tx, tenant_id).await?;
    let head = synveda_audit::head_seq(&mut tx, tenant_id).await?;
    if head <= watermark {
        return Ok(Swept {
            touched: Vec::new(),
            folded: 0,
            to_seq: watermark,
        });
    }

    // There is work, so take the per-tenant sweep lock and re-read: two
    // sweepers that both acted on the same watermark would fold the same
    // events twice and inflate every count that came out of them.
    let watermark = usage::watermark_for_update(&mut tx, tenant_id).await?;
    if head <= watermark {
        // Another sweeper got here first and folded exactly this range.
        return Ok(Swept {
            touched: Vec::new(),
            folded: 0,
            to_seq: watermark,
        });
    }
    let events =
        synveda_audit::since(&mut tx, tenant_id, watermark, SWEPT_ACTIONS, config.batch).await?;

    let drained = (events.len() as i64) < config.batch;
    let last_seq = events.last().map_or(watermark, |event| event.seq);
    // A tenant whose chain is full of events this engine does not count
    // must not be rescanned from the same watermark forever: when the
    // batch came back short, everything up to the head has been examined,
    // counted or not.
    let to_seq = if drained {
        head.max(last_seq)
    } else {
        last_seq
    };

    let deltas = fold_events(&events);
    let touched: Vec<RecordId> = deltas
        .iter()
        .map(|delta| delta.record_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    usage::fold(&mut tx, tenant_id, &deltas).await?;
    if to_seq > watermark {
        usage::advance(&mut *tx, tenant_id, to_seq).await?;
    }
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit promotion sweep: {err}"),
    })?;

    tracing::Span::current().record("folded", events.len());
    metrics::gauge!(PROMOTION_SWEEP_LAG_EVENTS).set((head - to_seq).max(0) as f64);
    Ok(Swept {
        touched,
        folded: events.len(),
        to_seq,
    })
}

/// Turns audit events into pre-aggregated per-(record, member) deltas.
///
/// Pre-aggregation is required, not an optimisation: `ON CONFLICT` cannot
/// affect one row twice in a single statement, so a batch carrying the
/// same pair twice is a runtime error from Postgres.
fn fold_events(events: &[StoredEvent]) -> Vec<UsageDelta> {
    let mut deltas: HashMap<(RecordId, &str), UsageDelta> = HashMap::new();
    for event in events {
        for record_id in composed_records(event) {
            deltas
                .entry((record_id, event.actor_subject.as_str()))
                .and_modify(|delta| {
                    delta.recalls += 1;
                    delta.first_recall_at = delta.first_recall_at.min(event.occurred_at);
                    delta.last_recall_at = delta.last_recall_at.max(event.occurred_at);
                })
                .or_insert_with(|| UsageDelta {
                    record_id,
                    subject: event.actor_subject.clone(),
                    recalls: 1,
                    first_recall_at: event.occurred_at,
                    last_recall_at: event.occurred_at,
                });
        }
    }
    deltas.into_values().collect()
}

/// The record ids one `context.injected` event says were composed.
///
/// A payload that does not carry the expected shape contributes nothing
/// rather than failing the sweep: the audit chain is append-only history
/// written by several versions of this product over its life, and a fold
/// that halted on the first unfamiliar payload would stop counting
/// everything after it.
fn composed_records(event: &StoredEvent) -> Vec<RecordId> {
    let Some(entries) = event
        .payload
        .get("entries")
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| entry.get("record_id").and_then(|value| value.as_str()))
        .filter_map(|raw| raw.parse::<RecordId>().ok())
        .collect()
}

/// One batch a rule fired on: the unit that becomes one proposal.
///
/// Keyed by owner as well as scope and tier because a proposal is opened
/// under *one* identity's authority (ADR-0033 decision 9), and a shared
/// scope can hold material owned by several service identities. Keyed by
/// tier because a proposal is governed by its most sensitive member
/// (ADR-0032 decision 3), so mixing tiers would drag a routine batch to
/// the stricter requirement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BatchKey {
    scope_id: ScopeId,
    owner_id: IdentityId,
    sensitivity: Sensitivity,
}

/// A candidate record with everything the rule and the proposal need.
#[derive(Debug, Clone)]
struct Candidate {
    version: RecordVersion,
    usage: UsageRow,
    address: ObjectHash,
}

/// Evaluates the records a sweep touched and opens what fires.
#[tracing::instrument(
    name = "ingest.promotion.evaluate",
    skip_all,
    fields(tenant.id = %tenant_id, candidates = swept.touched.len(), opened = tracing::field::Empty),
    err(Display)
)]
async fn evaluate(deps: &SweepDeps, tenant_id: TenantId, swept: &Swept) -> Result<usize> {
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    let usage_rows = usage::usage_for(&mut *tx, tenant_id, &swept.touched).await?;
    let versions = records::current_many(&mut *tx, tenant_id, &swept.touched).await?;
    tx.rollback().await.map_err(|err| Error::Storage {
        message: format!("close promotion read transaction: {err}"),
    })?;

    let usage_by_id: HashMap<RecordId, UsageRow> = usage_rows
        .into_iter()
        .map(|row| (row.record_id, row))
        .collect();

    // Group by scope: one effective pack read per scope, not per record.
    let mut by_scope: BTreeMap<ScopeId, Vec<Candidate>> = BTreeMap::new();
    for version in versions {
        let Some(usage) = usage_by_id.get(&version.id).cloned() else {
            continue;
        };
        let asset = crate::worker::memory_asset(version.id, &version.state);
        let address = object_hash(AssetKind::Memory, &asset.canonical_bytes());
        by_scope
            .entry(version.state.scope_id)
            .or_default()
            .push(Candidate {
                version,
                usage,
                address,
            });
    }

    let now = Utc::now();
    let mut opened = 0;
    for (scope_id, candidates) in by_scope {
        match evaluate_scope(deps, tenant_id, scope_id, &candidates, swept.to_seq, now).await {
            Ok(count) => opened += count,
            Err(error) => tracing::error!(
                tenant.id = %tenant_id,
                scope.id = %scope_id,
                error = %error,
                "promotion evaluation failed at this scope; others continue"
            ),
        }
    }
    tracing::Span::current().record("opened", opened);
    Ok(opened)
}

/// Evaluates one scope's candidates under the pack in force there.
async fn evaluate_scope(
    deps: &SweepDeps,
    tenant_id: TenantId,
    scope_id: ScopeId,
    candidates: &[Candidate],
    to_seq: i64,
    now: DateTime<Utc>,
) -> Result<usize> {
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
    let Some(chain) = deps.chains.resolve(&mut *tx, tenant_id, scope_id).await? else {
        // A scope that has since been deleted takes its material's
        // promotion with it.
        return Ok(0);
    };
    let chain_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    let assignments = policy_assignments::for_scopes(&mut *tx, tenant_id, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(&mut *tx, tenant_id).await?;
    // The pack is resolved for the *scope*, with no principal in it: which
    // rules run here is a property of the node, not of whoever's material
    // happens to sit on it.
    let scope_context = AuthzContext {
        scopes: &chain,
        principal_scopes: &[],
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        role_bindings: &[],
        grant: None,
    };
    let pack = deps
        .pdp
        .effective(tenant_id, Resource::Scope(scope_id), &scope_context);
    if pack.promotion.is_empty() {
        return Ok(0);
    }

    let mut batches: BTreeMap<(String, BatchKey), Vec<&Candidate>> = BTreeMap::new();
    for rule in &pack.promotion.rules {
        for candidate in candidates {
            if !fires(rule, candidate, now) {
                continue;
            }
            batches
                .entry((
                    rule.name.clone(),
                    BatchKey {
                        scope_id,
                        owner_id: candidate.version.state.owner_id,
                        sensitivity: candidate.version.state.sensitivity,
                    },
                ))
                .or_default()
                .push(candidate);
        }
    }
    if batches.is_empty() {
        return Ok(0);
    }

    // Suppression is per scope and covers every candidate at once
    // (ADR-0033 decision 11).
    let addresses: Vec<ObjectHash> = candidates.iter().map(|c| c.address).collect();
    let suppressed =
        vedaflow::proposals::suppressed_addresses(&mut tx, tenant_id, scope_id, &addresses).await?;
    let open_here = vedaflow::proposals::count_open(&mut tx, tenant_id, scope_id).await?;
    tx.rollback().await.map_err(|err| Error::Storage {
        message: format!("close promotion scope transaction: {err}"),
    })?;

    let node = chain
        .iter()
        .find(|node| node.id == scope_id)
        .cloned()
        .ok_or_else(|| Error::Internal {
            message: format!("scope {scope_id} is absent from its own chain"),
        })?;

    let mut opened = 0;
    let mut room = vedaflow::MAX_OPEN_PROPOSALS - open_here;
    for ((rule_name, key), members) in batches {
        let (proposable, held): (Vec<&Candidate>, Vec<&Candidate>) = members
            .into_iter()
            .partition(|candidate| !suppressed.contains(&candidate.address));
        if !held.is_empty() {
            metrics::counter!(
                PROMOTION_CANDIDATES_TOTAL,
                "rule" => rule_name.clone(),
                "outcome" => "suppressed",
            )
            .increment(held.len() as u64);
        }
        if proposable.is_empty() {
            continue;
        }
        if room <= 0 {
            // The cap is a fact about a scope, and material a rule
            // declined to raise is exactly the silence a governance log
            // exists to break (ADR-0033's compliance note).
            report_queue_full(deps, tenant_id, &node, &rule_name, proposable.len()).await;
            break;
        }
        let chunk: Vec<&Candidate> = proposable
            .into_iter()
            .take(vedaflow::MAX_PROPOSAL_MEMBERS)
            .collect();
        match open_proposal(
            deps, tenant_id, &node, &chain, &key, &rule_name, &pack, &chunk, to_seq,
        )
        .await
        {
            Ok(true) => {
                opened += 1;
                room -= 1;
                metrics::counter!(PROMOTION_PROPOSALS_TOTAL, "rule" => rule_name.clone())
                    .increment(1);
                metrics::counter!(
                    PROMOTION_CANDIDATES_TOTAL,
                    "rule" => rule_name.clone(),
                    "outcome" => "proposed",
                )
                .increment(chunk.len() as u64);
            }
            Ok(false) => {}
            Err(error) => tracing::error!(
                tenant.id = %tenant_id,
                scope.id = %scope_id,
                promotion.rule = %rule_name,
                error = %error,
                "opening an auto-promotion proposal failed; the material stays proposable"
            ),
        }
    }
    Ok(opened)
}

/// Whether one rule fires on one candidate, at one sweep instant.
fn fires(rule: &PromotionRule, candidate: &Candidate, now: DateTime<Utc>) -> bool {
    let state = &candidate.version.state;
    if !rule.matches(state.class, state.sensitivity) {
        return false;
    }
    // Age is measured from `tx_from` — how long *these exact bytes* have
    // stood — not from the record's creation. An edit resets the clock,
    // which is the same rule the content-address idempotency key follows
    // (ADR-0033 decision 11): edited material is new material.
    let age_hours = hours_between(candidate.version.tx_from, now);
    let since_recall = hours_between(candidate.usage.last_recall_at, now);
    rule.fires(&candidate.usage.facts(), age_hours, since_recall)
}

fn hours_between(from: DateTime<Utc>, to: DateTime<Utc>) -> u32 {
    let hours = (to - from).num_hours();
    u32::try_from(hours.max(0)).unwrap_or(u32::MAX)
}

/// Opens one proposal under the material owner's authority.
///
/// Returns whether one was opened: an owner the PDP denies, or one that
/// no longer exists, is not an error — it is the answer.
#[allow(clippy::too_many_arguments)]
async fn open_proposal(
    deps: &SweepDeps,
    tenant_id: TenantId,
    node: &HierarchyNode,
    scope_chain: &[HierarchyNode],
    key: &BatchKey,
    rule_name: &str,
    pack: &synveda_policy::EffectivePack,
    members: &[&Candidate],
    to_seq: i64,
) -> Result<bool> {
    let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;

    let Some(authorized) = authorize_owner(deps, &mut tx, tenant_id, key, scope_chain).await?
    else {
        return Ok(false);
    };
    let OwnerAuth {
        subject,
        decision,
        principal_scopes,
    } = authorized;

    // Objects first: each member's address, computed from the version
    // being proposed — the same binding the human route makes
    // (ADR-0032 decision 6).
    let mut entries: Vec<(String, ObjectHash)> = Vec::with_capacity(members.len());
    for candidate in members {
        let asset = crate::worker::memory_asset(candidate.version.id, &candidate.version.state);
        let written = vedaflow::put_memory(&mut tx, tenant_id, &asset).await?;
        entries.push((asset.entry_name(), written.hash));
    }

    let evidence = PromotionEvidence {
        rule: rule_name.to_owned(),
        pack_name: pack.name.clone(),
        pack_version: pack.version,
        actions: SWEPT_ACTIONS
            .iter()
            .map(|action| action.as_str().to_owned())
            .collect(),
        // The projection folds the whole chain from its start; `reset`
        // clears the watermark with it, so a rebuild resumes at 1 too.
        from_seq: 1,
        to_seq,
        members: members
            .iter()
            .map(|candidate| MemberEvidence {
                record_id: candidate.version.id,
                recalls: candidate.usage.facts().recalls,
                distinct_members: candidate.usage.facts().distinct_members,
                first_recall_at: candidate.usage.first_recall_at,
                last_recall_at: candidate.usage.last_recall_at,
            })
            .collect(),
    };
    let title = evidence.summary();

    let snapshot = PolicySnapshot::new(decision.pack_name.clone(), decision.pack_version);
    let proposal = vedaflow::proposals::open(
        &mut tx,
        tenant_id,
        &vedaflow::NewProposal {
            target_scope: node.id,
            // FLOW-4 is same-scope: the material is already where its
            // channel would move. FLOW-5's climb is what makes these
            // differ (ADR-0033 decision 8).
            source_scope: node.id,
            asset: AssetKind::Memory,
            channel: Channel::Published,
            members: &entries,
            sensitivity: key.sensitivity,
            title: &title,
            proposer: key.owner_id,
            proposer_subject: &subject,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
            evidence: Some(&evidence),
        },
        &Signer::Unsigned,
    )
    .await?;

    // The requirement as resolved right now, recorded exactly as the
    // human route records it (ADR-0032 decision 3; one shape, ADR-0033
    // decision 9).
    let member_names: Vec<String> = entries.iter().map(|(name, _)| name.clone()).collect();
    let requirement = resolve_requirement(
        deps,
        &mut tx,
        tenant_id,
        node,
        scope_chain,
        &principal_scopes,
        key.sensitivity,
        &member_names,
    )
    .await?;
    let outstanding = requirement.outstanding(&[]);

    synveda_audit::append(
        &mut tx,
        tenant_id,
        &AuditEvent {
            occurred_at: Utc::now(),
            // Who acted and whose authority it was are two different
            // facts, and the trail records both (ADR-0033 decision 9).
            actor: Actor::system(ACTOR_COMPONENT),
            action: AuditAction::ProposalOpened,
            resource: Resource::Scope(node.id).to_string(),
            outcome: Outcome::Success,
            payload: json!({
                "authz": {
                    "action": Action::ProposalOpen.as_str(),
                    "decision": "allow",
                    "pack": decision.pack_name,
                    "pack_version": decision.pack_version,
                    "determining": decision.determining,
                },
                "proposal_id": proposal.id,
                "asset": AssetKind::Memory.as_str(),
                "channel": Channel::Published.as_str(),
                "title": title,
                "sensitivity": key.sensitivity.as_str(),
                "commit": proposal.commit.to_hex(),
                "proposer": {
                    "identity_id": key.owner_id,
                    "subject": subject,
                },
                "records": entries.iter().map(|(name, hash)| json!({
                    "record_id": name,
                    "object_hash": hash.to_hex(),
                })).collect::<Vec<_>>(),
                "approvals": serde_json::to_value(requirement.audit_view(&outstanding))
                    .unwrap_or(serde_json::Value::Null),
                "promotion": serde_json::to_value(&evidence)
                    .unwrap_or(serde_json::Value::Null),
            }),
            trace_id: None,
        },
    )
    .await?;

    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit auto-promotion proposal: {err}"),
    })?;
    vedaflow::proposals::act("opened", AssetKind::Memory);
    tracing::info!(
        tenant.id = %tenant_id,
        scope.id = %node.id,
        promotion.rule = rule_name,
        proposal.id = %proposal.id,
        members = entries.len(),
        "auto-promotion proposal opened"
    );
    Ok(true)
}

/// What an owner's `ProposalOpen` decision produced.
struct OwnerAuth {
    subject: String,
    decision: AuthzDecision,
    principal_scopes: Vec<HierarchyNode>,
}

/// Re-decides `ProposalOpen` at the target **as the material's owner**,
/// under that owner's current placement and quarantine state.
///
/// ADR-0022's `authorize_owner` discipline, one action over: a background
/// pipeline running a governed act runs the PDP as the identity whose
/// authority the act rides on, with explicit context instead of
/// task-locals. An owner who has since been quarantined, moved out of the
/// scope, or deleted stops having material proposed on their behalf, with
/// no special case anywhere.
async fn authorize_owner(
    deps: &SweepDeps,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    key: &BatchKey,
    scope_chain: &[HierarchyNode],
) -> Result<Option<OwnerAuth>> {
    let Some(identity) = identities::by_id(&mut *tx, tenant_id, key.owner_id).await? else {
        return Ok(None);
    };
    let mut quarantined = identity.quarantined;
    let principal_chain: Vec<HierarchyNode> = deps
        .chains
        .resolve(&mut *tx, tenant_id, identity.scope_id)
        .await?
        .map(|chain| chain.to_vec())
        .unwrap_or_default();
    // The confinement scope (ADR-0018 decision 4): a service identity's
    // anchor is the node above its personal leaf; unresolvable means
    // quarantined, never unconfined.
    let token_scope = if identity.kind == IdentityKind::Service {
        let anchor = principal_chain.get(1).map(|node| node.id);
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
    let chain_ids: Vec<ScopeId> = scope_chain.iter().map(|node| node.id).collect();
    let assignments = policy_assignments::for_scopes(&mut *tx, tenant_id, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(&mut *tx, tenant_id).await?;
    let bindings =
        role_bindings::for_subject_on_scopes(&mut *tx, tenant_id, &identity.subject, &chain_ids)
            .await?;
    let context = AuthzContext {
        scopes: scope_chain,
        principal_scopes: &principal_chain,
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        role_bindings: &bindings,
        grant: None,
    };
    let decision = deps.pdp.authorize(
        &principal,
        Action::ProposalOpen,
        Resource::Scope(key.scope_id),
        &context,
    )?;
    if !decision.allowed {
        tracing::debug!(
            tenant.id = %tenant_id,
            scope.id = %key.scope_id,
            identity.subject = %identity.subject,
            "promotion declined: the owner may not propose here",
        );
        return Ok(None);
    }
    Ok(Some(OwnerAuth {
        subject: identity.subject,
        decision,
        principal_scopes: principal_chain,
    }))
}

/// What it takes to publish this batch here — the same resolution the
/// gateway's publish and proposal routes perform (ADR-0032 decision 3).
#[allow(clippy::too_many_arguments)]
async fn resolve_requirement(
    deps: &SweepDeps,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    node: &HierarchyNode,
    scope_chain: &[HierarchyNode],
    principal_scopes: &[HierarchyNode],
    sensitivity: Sensitivity,
    entries: &[String],
) -> Result<synveda_types::ApprovalRequirement> {
    let chain_ids: Vec<ScopeId> = scope_chain.iter().map(|node| node.id).collect();
    let assignments = policy_assignments::for_scopes(&mut *tx, tenant_id, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(&mut *tx, tenant_id).await?;
    let context = AuthzContext {
        scopes: scope_chain,
        principal_scopes,
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        role_bindings: &[],
        grant: None,
    };
    let pack = deps
        .pdp
        .effective(tenant_id, Resource::Scope(node.id), &context);
    let mut requirement = pack
        .approvals
        .resolve(AssetKind::Memory, sensitivity, node.kind);
    if let Some(stored) = vedaflow::nearest_curators(tx, tenant_id, &chain_ids).await? {
        stored.file.apply(
            stored.scope_id,
            AssetKind::Memory,
            entries,
            &mut requirement,
        );
    }
    Ok(requirement)
}

/// Records that a scope's queue is full and candidates went unraised.
async fn report_queue_full(
    deps: &SweepDeps,
    tenant_id: TenantId,
    node: &HierarchyNode,
    rule_name: &str,
    held: usize,
) {
    metrics::counter!(PROMOTION_QUEUE_FULL_TOTAL).increment(1);
    tracing::warn!(
        tenant.id = %tenant_id,
        scope.id = %node.id,
        promotion.rule = rule_name,
        held,
        "promotion held: the scope is at its open-proposal cap"
    );
    let recorded = async {
        let mut tx = rls::begin_tenant_tx(&deps.pool, tenant_id).await?;
        synveda_audit::append(
            &mut tx,
            tenant_id,
            &AuditEvent {
                occurred_at: Utc::now(),
                actor: Actor::system(ACTOR_COMPONENT),
                // No new action (ADR-0033's compliance note): the act
                // attempted was opening a proposal, and it failed.
                action: AuditAction::ProposalOpened,
                resource: Resource::Scope(node.id).to_string(),
                outcome: Outcome::Failure,
                payload: json!({
                    "reason": "open_proposal_cap",
                    "promotion": {
                        "rule": rule_name,
                        "held_records": held,
                        "cap": vedaflow::MAX_OPEN_PROPOSALS,
                    },
                }),
                trace_id: None,
            },
        )
        .await?;
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit promotion cap event: {err}"),
        })
    }
    .await;
    if let Err(error) = recorded {
        tracing::error!(error = %error, "recording the promotion cap event failed");
    }
}
