//! Bounded, independently authorised Knowledge-relation expansion. When
//! Configuration includes business links, the same frontier follows them
//! too, within the same bounds (ADR-0164 Amendment 3).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{Acquire, PgConnection};
use synveda_policy::Action;
use synveda_retrieval::estimated_tokens;
use synveda_store::knowledge::{self as knowledge, KnowledgeHead, KnowledgeSnapshot};
use synveda_store::knowledge_freshness;
use synveda_types::configuration::{ContextOptimizationMode, GraphRetrievalConfiguration};
use synveda_types::context::{ContextGraphDirection, ContextReasonCode};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeRelation, KnowledgeRelationType, assess_freshness,
};
use synveda_types::{ContextCandidateId, Error, KnowledgeItemId, Result, Sensitivity, TenantId};

use crate::app::AppState;
use crate::audit;

use super::links::{LinkWalker, SourceConfigurations, WalkStart, WalkedLinks};
use super::{
    PlannedCandidate, PlannedEdge, PlannedGraphStep, PlannedPayload, asserting_revision_visible,
    current_active_at, freshness_micros, is_valid_at, load_visible_revision, push_degradation,
    rank_and_deduplicate,
};

mod link_steps;

const RELATIONS_VERSION: &str = "knowledge-relations-v1";
const RELATIONS_AND_LINKS_VERSION: &str = "knowledge-relations-entity-links-v1";
const MAX_ANCHORS: usize = 10;
const HOP_PENALTY_MICROS: i32 = 100_000;

/// The expansion a run records having attempted.
pub(super) fn version(configuration: &GraphRetrievalConfiguration) -> &'static str {
    if configuration.include_entity_links {
        RELATIONS_AND_LINKS_VERSION
    } else {
        RELATIONS_VERSION
    }
}

#[derive(Clone)]
struct GraphFrontier {
    snapshot: KnowledgeSnapshot,
    anchor_micros: i32,
    edge_weight_micros: i32,
    path_rank_micros: i32,
    path: Vec<PlannedGraphStep>,
    visited: Vec<KnowledgeItemId>,
}

/// Whether expansion may reach a link's other end at `at`: its lifecycle is
/// one expansion follows and its current revision holds then.
fn is_reachable_link_end(other: &KnowledgeHead, at: DateTime<Utc>) -> bool {
    other.item.lifecycle_state.is_reachable_by_expansion()
        && is_valid_at(other.valid_from, other.valid_to, at)
}

/// Whether a denial of the item a path reaches may raise the run's policy
/// flag. An anchor or an item reached by a relation may; an item reached by
/// a business link leaves no trace.
fn is_reached_by_relation(path: &[PlannedGraphStep]) -> bool {
    path.last().is_none_or(PlannedGraphStep::follows_relation)
}

#[derive(Clone)]
struct ExpandedCandidate {
    snapshot: KnowledgeSnapshot,
    anchor_micros: i32,
    edge_weight_micros: i32,
    hop_penalty_micros: i32,
    final_micros: i32,
    path: Vec<PlannedGraphStep>,
    exclusion: Option<ContextReasonCode>,
}

#[derive(Default)]
pub(super) struct GraphExpansion {
    candidates: Vec<ExpandedCandidate>,
    warnings: HashMap<KnowledgeItemId, Vec<PlannedGraphStep>>,
    policy_exclusion: bool,
    degraded: Vec<String>,
}

impl GraphExpansion {
    pub(super) fn degradations(&self) -> &[String] {
        &self.degraded
    }
}

fn graph_edge_weight(relation_type: KnowledgeRelationType) -> Option<i32> {
    match relation_type {
        KnowledgeRelationType::Supports => Some(700_000),
        KnowledgeRelationType::References => Some(600_000),
        KnowledgeRelationType::DerivedFrom => Some(650_000),
        KnowledgeRelationType::Supersedes | KnowledgeRelationType::TransitionsTo => Some(500_000),
        KnowledgeRelationType::RelatedTo => Some(350_000),
        KnowledgeRelationType::Contradicts => Some(0),
        KnowledgeRelationType::Duplicates => None,
    }
}

fn graph_step(
    relation: &KnowledgeRelation,
    from: &KnowledgeSnapshot,
    to: &KnowledgeSnapshot,
) -> PlannedGraphStep {
    let direction = if relation.source_item_id == from.item.id {
        ContextGraphDirection::Outbound
    } else {
        ContextGraphDirection::Inbound
    };
    let edge_weight_micros = graph_edge_weight(relation.relation_type).unwrap_or(0);
    let relation_hash = blake3::hash(
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            relation.id,
            relation.relation_type,
            direction,
            from.revision.id,
            from.revision.content_hash,
            to.revision.id,
            to.revision.content_hash,
            relation.asserting_revision_id,
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    PlannedGraphStep {
        edge: PlannedEdge::Relation {
            id: relation.id,
            relation_type: relation.relation_type,
        },
        relation_hash,
        direction,
        from_item_id: from.item.id,
        from_revision_id: from.revision.id,
        to_item_id: to.item.id,
        to_revision_id: to.revision.id,
        asserting_revision_id: relation.asserting_revision_id,
        from_content_hash: from.revision.content_hash.clone(),
        to_content_hash: to.revision.content_hash.clone(),
        edge_weight_micros,
        supporting: relation.relation_type != KnowledgeRelationType::Contradicts,
    }
}

fn graph_target(
    relation: &KnowledgeRelation,
    frontier: KnowledgeItemId,
) -> Option<KnowledgeItemId> {
    if relation.source_item_id == frontier {
        Some(relation.target_item_id)
    } else if relation.target_item_id == frontier {
        Some(relation.source_item_id)
    } else {
        None
    }
}

fn graph_candidate_tokens(snapshot: &KnowledgeSnapshot) -> u32 {
    estimated_tokens(&format!(
        "{}\n{}\n{}",
        snapshot.revision.content.title,
        snapshot.revision.content.summary,
        snapshot.revision.content.body_markdown,
    ))
}

/// Whether expansion may go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Continue,
    /// The candidate budget is spent.
    Exhausted,
}

/// One expansion, level by level. Both edge kinds share its candidate,
/// token and time budgets, and the best score reaching an item wins
/// whichever kind of edge carried it.
struct Traversal<'a> {
    state: &'a AppState,
    tenant_id: TenantId,
    configuration: &'a GraphRetrievalConfiguration,
    max_sensitivity: Option<Sensitivity>,
    at: DateTime<Utc>,
    expansion: GraphExpansion,
    best: HashMap<KnowledgeItemId, i32>,
    expanded_index: HashMap<KnowledgeItemId, usize>,
    expanded_tokens: u32,
    next_level: Vec<GraphFrontier>,
    /// Present when Configuration includes business links.
    links: Option<LinkWalker<'a>>,
}

async fn graph_expansion_inner(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    anchors: Vec<GraphFrontier>,
    configuration: &GraphRetrievalConfiguration,
    max_sensitivity: Option<Sensitivity>,
    at: DateTime<Utc>,
) -> Result<GraphExpansion> {
    let mut traversal = Traversal {
        state,
        tenant_id,
        configuration,
        max_sensitivity,
        at,
        expansion: GraphExpansion::default(),
        best: anchors
            .iter()
            .map(|anchor| (anchor.snapshot.item.id, i32::MAX))
            .collect(),
        expanded_index: HashMap::new(),
        expanded_tokens: 0,
        next_level: Vec::new(),
        links: configuration
            .include_entity_links
            .then(|| LinkWalker::for_expansion(state, tenant_id, max_sensitivity, at)),
    };
    let mut pending = anchors;
    while !pending.is_empty() {
        pending.retain(|frontier| traversal.is_best(frontier));
        let level = traversal.authorized(tx, pending).await?;
        let mut links = traversal.walk_links(tx, &level).await?;
        for frontier in level {
            let walked = links.remove(&frontier.snapshot.item.id);
            if traversal.expand(tx, frontier, walked).await? == Reach::Exhausted {
                return Ok(traversal.expansion);
            }
        }
        pending = std::mem::take(&mut traversal.next_level);
    }
    Ok(traversal.expansion)
}

impl Traversal<'_> {
    /// An anchor always expands; a reached item expands only along the best
    /// path found to it.
    fn is_best(&self, frontier: &GraphFrontier) -> bool {
        frontier.path.is_empty()
            || self.best.get(&frontier.snapshot.item.id).copied() == Some(frontier.path_rank_micros)
    }

    fn fan_out(&self) -> usize {
        usize::try_from(self.configuration.fan_out_per_node).unwrap_or(usize::MAX)
    }

    /// The level's frontiers the caller may still read. Each is
    /// re-authorised before its adjacency is read (ADR-0097).
    async fn authorized(
        &mut self,
        tx: &mut PgConnection,
        level: Vec<GraphFrontier>,
    ) -> Result<Vec<GraphFrontier>> {
        let mut authorized = Vec::with_capacity(level.len());
        for frontier in level {
            let decision = crate::knowledge_api::authorize_snapshot(
                self.state,
                tx,
                self.tenant_id,
                &frontier.snapshot,
            );
            match decision.await {
                Ok(_) => authorized.push(frontier),
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {
                    self.expansion.policy_exclusion |= is_reached_by_relation(&frontier.path);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(authorized)
    }

    /// The business links every item in the level may follow, decided
    /// together.
    async fn walk_links(
        &mut self,
        tx: &mut PgConnection,
        level: &[GraphFrontier],
    ) -> Result<HashMap<KnowledgeItemId, WalkedLinks>> {
        let (fan_out, at) = (self.fan_out(), self.at);
        let Some(walker) = self.links.as_mut() else {
            return Ok(HashMap::new());
        };
        let starts: Vec<WalkStart> = level
            .iter()
            .map(|frontier| WalkStart {
                item_id: frontier.snapshot.item.id,
                revision_id: frontier.snapshot.revision.id,
                scope_id: frontier.snapshot.item.scope_id,
            })
            .collect();
        walker
            .walk(tx, &starts, fan_out, |other| {
                is_reachable_link_end(other, at)
            })
            .await
    }

    async fn expand(
        &mut self,
        tx: &mut PgConnection,
        frontier: GraphFrontier,
        links: Option<WalkedLinks>,
    ) -> Result<Reach> {
        // An earlier frontier in this level may have found a better path.
        if !self.is_best(&frontier) {
            return Ok(Reach::Continue);
        }
        if self.follow_relations(tx, &frontier).await? == Reach::Exhausted {
            return Ok(Reach::Exhausted);
        }
        match links {
            Some(links) => self.follow_links(tx, &frontier, links).await,
            None => Ok(Reach::Continue),
        }
    }

    async fn follow_relations(
        &mut self,
        tx: &mut PgConnection,
        frontier: &GraphFrontier,
    ) -> Result<Reach> {
        let fan_out = self.fan_out();
        let limit = i64::from(self.configuration.fan_out_per_node).saturating_add(1);
        let mut relations = knowledge::bounded_retrieval_relations(
            &mut *tx,
            self.tenant_id,
            frontier.snapshot.item.id,
            limit,
        )
        .await?;
        if relations.len() > fan_out {
            relations.truncate(fan_out);
            push_degradation(&mut self.expansion.degraded, "graph_fanout_truncated");
        }
        for relation in relations {
            let Some(target_id) = graph_target(&relation, frontier.snapshot.item.id) else {
                continue;
            };
            if frontier.visited.contains(&target_id) {
                continue;
            }
            let Some(target) = knowledge::current(&mut *tx, self.tenant_id, target_id).await?
            else {
                continue;
            };
            if self
                .max_sensitivity
                .is_some_and(|ceiling| target.revision.content.sensitivity > ceiling)
            {
                continue;
            }
            match crate::knowledge_api::authorize_snapshot(self.state, tx, self.tenant_id, &target)
                .await
            {
                Ok(_) => {}
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {
                    self.expansion.policy_exclusion = true;
                    continue;
                }
                Err(error) => return Err(error),
            }
            let step = graph_step(&relation, &frontier.snapshot, &target);
            if !step.supporting {
                if frontier.path.is_empty() {
                    self.expansion
                        .warnings
                        .entry(frontier.snapshot.item.id)
                        .or_default()
                        .push(step);
                }
                continue;
            }
            if !target.item.lifecycle_state.is_reachable_by_expansion() {
                continue;
            }
            if self.reach(tx, frontier, step, target).await? == Reach::Exhausted {
                return Ok(Reach::Exhausted);
            }
        }
        Ok(Reach::Continue)
    }

    /// Follows the frontier's business links that passed the walk. Their
    /// other ends were decided by head; a body is read only for an end that
    /// passed, and one whose revision moved on since is left out.
    async fn follow_links(
        &mut self,
        tx: &mut PgConnection,
        frontier: &GraphFrontier,
        links: WalkedLinks,
    ) -> Result<Reach> {
        if links.truncated {
            push_degradation(&mut self.expansion.degraded, "graph_fanout_truncated");
        }
        for visible in links.links {
            let target_id = visible.other.item.id;
            if frontier.visited.contains(&target_id) {
                continue;
            }
            let Some(target) = knowledge::current(&mut *tx, self.tenant_id, target_id).await?
            else {
                continue;
            };
            if target.revision.id != visible.other.item.current_revision_id {
                continue;
            }
            let step = link_steps::link_step(
                &visible.link,
                visible.direction,
                &frontier.snapshot,
                &target,
            );
            if self.reach(tx, frontier, step, target).await? == Reach::Exhausted {
                return Ok(Reach::Exhausted);
            }
        }
        Ok(Reach::Continue)
    }

    /// Scores `target` along the frontier's path plus `step`, and keeps it
    /// when that beats the best path found to it so far.
    async fn reach(
        &mut self,
        tx: &mut PgConnection,
        frontier: &GraphFrontier,
        step: PlannedGraphStep,
        target: KnowledgeSnapshot,
    ) -> Result<Reach> {
        let candidate = self.score(tx, frontier, step, target).await?;
        if self
            .best
            .get(&candidate.snapshot.item.id)
            .is_some_and(|held| *held >= candidate.final_micros)
        {
            return Ok(Reach::Continue);
        }
        Ok(self.admit(frontier, candidate))
    }

    /// The target as a candidate reached along the frontier's path plus
    /// `step`.
    async fn score(
        &self,
        tx: &mut PgConnection,
        frontier: &GraphFrontier,
        step: PlannedGraphStep,
        target: KnowledgeSnapshot,
    ) -> Result<ExpandedCandidate> {
        let hop = frontier.path.len().saturating_add(1);
        let hop_penalty = i32::try_from(hop)
            .unwrap_or(i32::MAX)
            .saturating_mul(HOP_PENALTY_MICROS);
        let edge_weight = frontier
            .edge_weight_micros
            .saturating_add(step.edge_weight_micros)
            .min(2_000_000);
        let freshness = freshness_micros(&target, self.at);
        let mut exclusion = match target.item.lifecycle_state {
            KnowledgeLifecycleState::Stale => Some(ContextReasonCode::Stale),
            KnowledgeLifecycleState::Superseded => Some(ContextReasonCode::Superseded),
            _ => None,
        };
        if exclusion.is_none() && stale_at(tx, self.tenant_id, &target, self.at).await? {
            exclusion = Some(ContextReasonCode::Stale);
        }
        let current = if exclusion.is_none() { 100_000 } else { 0 };
        let final_micros = frontier
            .anchor_micros
            .saturating_add(edge_weight)
            .saturating_sub(hop_penalty)
            .saturating_add(freshness)
            .saturating_add(current)
            .clamp(0, 5_000_000);
        let mut path = frontier.path.clone();
        path.push(step);
        Ok(ExpandedCandidate {
            snapshot: target,
            anchor_micros: frontier.anchor_micros,
            edge_weight_micros: edge_weight,
            hop_penalty_micros: hop_penalty,
            final_micros,
            path,
            exclusion,
        })
    }

    /// Keeps a candidate that beat the best path to its item, within the
    /// candidate and token budgets, and queues it to expand further.
    fn admit(&mut self, frontier: &GraphFrontier, candidate: ExpandedCandidate) -> Reach {
        let target_id = candidate.snapshot.item.id;
        if let Some(index) = self.expanded_index.get(&target_id).copied() {
            self.expansion.candidates[index] = candidate.clone();
        } else {
            if self.expansion.candidates.len()
                >= usize::try_from(self.configuration.max_expanded_candidates).unwrap_or(usize::MAX)
            {
                push_degradation(
                    &mut self.expansion.degraded,
                    "graph_candidate_budget_exceeded",
                );
                return Reach::Exhausted;
            }
            let tokens = graph_candidate_tokens(&candidate.snapshot);
            if self.expanded_tokens.saturating_add(tokens) > self.configuration.token_budget {
                push_degradation(&mut self.expansion.degraded, "graph_token_budget_exceeded");
                return Reach::Continue;
            }
            self.expanded_tokens = self.expanded_tokens.saturating_add(tokens);
            self.expanded_index
                .insert(target_id, self.expansion.candidates.len());
            self.expansion.candidates.push(candidate.clone());
        }
        self.best.insert(target_id, candidate.final_micros);
        if candidate.exclusion.is_none()
            && candidate.path.len() < usize::from(self.configuration.max_hops)
        {
            let mut visited = frontier.visited.clone();
            visited.push(target_id);
            self.next_level.push(GraphFrontier {
                snapshot: candidate.snapshot,
                anchor_micros: candidate.anchor_micros,
                edge_weight_micros: candidate.edge_weight_micros,
                path_rank_micros: candidate.final_micros,
                path: candidate.path,
                visited,
            });
        }
        Reach::Continue
    }
}

pub(super) async fn expand_graph_with_fallback(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    candidates: &[PlannedCandidate],
    configuration: &GraphRetrievalConfiguration,
    max_sensitivity: Option<Sensitivity>,
    at: DateTime<Utc>,
) -> Result<(GraphExpansion, bool)> {
    if !configuration.enabled {
        return Ok((GraphExpansion::default(), false));
    }
    let anchors = candidates
        .iter()
        .filter(|candidate| candidate.exclusion.is_none())
        .filter_map(|candidate| {
            candidate
                .knowledge()
                .cloned()
                .map(|snapshot| GraphFrontier {
                    visited: vec![snapshot.item.id],
                    snapshot,
                    anchor_micros: candidate.anchor_micros,
                    edge_weight_micros: 0,
                    path_rank_micros: candidate.final_micros,
                    path: Vec::new(),
                })
        })
        .take(MAX_ANCHORS)
        .collect();
    let mut graph_tx = tx.begin().await.map_err(|error| Error::Storage {
        message: format!("begin context graph savepoint: {error}"),
    })?;
    let result = tokio::time::timeout(
        Duration::from_millis(u64::from(configuration.time_budget_ms)),
        graph_expansion_inner(
            state,
            &mut graph_tx,
            tenant_id,
            anchors,
            configuration,
            max_sensitivity,
            at,
        ),
    )
    .await;
    match result {
        Ok(Ok(expansion)) => {
            graph_tx.commit().await.map_err(|error| Error::Storage {
                message: format!("commit context graph savepoint: {error}"),
            })?;
            Ok((expansion, true))
        }
        Ok(Err(Error::Storage { .. })) => {
            graph_tx.rollback().await.map_err(|error| Error::Storage {
                message: format!("roll back failed context graph expansion: {error}"),
            })?;
            let mut expansion = GraphExpansion::default();
            push_degradation(&mut expansion.degraded, "graph_unavailable");
            Ok((expansion, true))
        }
        Ok(Err(error)) => {
            graph_tx
                .rollback()
                .await
                .map_err(|rollback| Error::Storage {
                    message: format!("roll back rejected context graph expansion: {rollback}"),
                })?;
            Err(error)
        }
        Err(_) => {
            graph_tx.rollback().await.map_err(|error| Error::Storage {
                message: format!("roll back timed-out context graph expansion: {error}"),
            })?;
            let mut expansion = GraphExpansion::default();
            push_degradation(&mut expansion.degraded, "graph_time_budget_exceeded");
            Ok((expansion, true))
        }
    }
}

/// The item whose revision asserted a step's edge, when that edge is still
/// recorded as the step names it: the same vocabulary, asserting revision
/// and ends. With `holding_at`, a business link must also be valid then;
/// relations never end.
pub(super) async fn recorded_asserter(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    step: &PlannedGraphStep,
    holding_at: Option<DateTime<Utc>>,
) -> Result<Option<KnowledgeItemId>> {
    match step.edge {
        PlannedEdge::Relation { id, relation_type } => {
            let Some(relation) = knowledge::relation(&mut *tx, tenant_id, id).await? else {
                return Ok(None);
            };
            let source = relation.source_item_id;
            Ok((relation.relation_type == relation_type
                && relation.asserting_revision_id == step.asserting_revision_id
                && (source == step.from_item_id || source == step.to_item_id))
                .then_some(source))
        }
        PlannedEdge::EntityLink { id, predicate } => {
            link_steps::recorded_source(tx, tenant_id, id, predicate, step, holding_at).await
        }
    }
}

/// Whether the caller may read a step's two ends at the revisions it names,
/// and the revision that asserted its edge when that is not one of them.
pub(super) async fn revisions_visible(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    step: &PlannedGraphStep,
    asserter: KnowledgeItemId,
) -> Result<bool> {
    let ends = [
        (step.from_item_id, step.from_revision_id),
        (step.to_item_id, step.to_revision_id),
    ];
    for (item_id, revision_id) in ends {
        if load_visible_revision(state, tx, tenant_id, item_id, revision_id, false)
            .await?
            .is_none()
        {
            return Ok(false);
        }
    }
    let asserting = step.asserting_revision_id;
    asserting_revision_visible(state, tx, tenant_id, asserter, asserting, ends).await
}

/// Rechecks planned steps for the caller at one instant, each step once.
pub(super) struct StepCheck<'a> {
    state: &'a AppState,
    tenant_id: TenantId,
    at: DateTime<Utc>,
    /// Whether both ends must also be current active Knowledge then.
    active_ends: bool,
    configurations: SourceConfigurations,
    /// Each step checked so far, by its evidence hash. Paths that share a
    /// first step check it once.
    checked: HashMap<String, bool>,
}

/// What re-reading an expanded candidate found.
enum Rehydrated {
    Kept(Box<PlannedCandidate>),
    /// Gone or moved on since expansion, or denied where a denial leaves no
    /// trace.
    Dropped,
    /// Denied where a relation carried the path, which the run's policy
    /// flag records.
    Denied,
}

impl<'a> StepCheck<'a> {
    fn new(state: &'a AppState, tenant_id: TenantId, at: DateTime<Utc>, active_ends: bool) -> Self {
        Self {
            state,
            tenant_id,
            at,
            active_ends,
            configurations: SourceConfigurations::new(tenant_id),
            checked: HashMap::new(),
        }
    }

    /// Checks for the plan being made: ends current at the step's revisions.
    fn for_planning(state: &'a AppState, tenant_id: TenantId, at: DateTime<Utc>) -> Self {
        Self::new(state, tenant_id, at, false)
    }

    /// Checks for replaying a saved delivery: ends also current active
    /// Knowledge at `at`.
    pub(super) fn for_replay(state: &'a AppState, tenant_id: TenantId, at: DateTime<Utc>) -> Self {
        Self::new(state, tenant_id, at, true)
    }

    /// Whether the step still holds: both ends are current at the revisions
    /// it names, its edge is recorded as it names it, a business link is
    /// valid and still enabled at its source's scope, and the caller may
    /// read the step's revisions.
    pub(super) async fn holds(
        &mut self,
        tx: &mut PgConnection,
        step: &PlannedGraphStep,
    ) -> Result<bool> {
        if let Some(held) = self.checked.get(&step.relation_hash) {
            return Ok(*held);
        }
        let held = self.check(tx, step).await?;
        self.checked.insert(step.relation_hash.clone(), held);
        Ok(held)
    }

    async fn check(&mut self, tx: &mut PgConnection, step: &PlannedGraphStep) -> Result<bool> {
        let mut ends = Vec::with_capacity(2);
        for (item_id, revision_id) in [
            (step.from_item_id, step.from_revision_id),
            (step.to_item_id, step.to_revision_id),
        ] {
            let Some(current) = knowledge::current(&mut *tx, self.tenant_id, item_id).await? else {
                return Ok(false);
            };
            if current.revision.id != revision_id
                || (self.active_ends && !current_active_at(&current, self.at))
            {
                return Ok(false);
            }
            ends.push(current);
        }
        let Some(asserter) = recorded_asserter(tx, self.tenant_id, step, Some(self.at)).await?
        else {
            return Ok(false);
        };
        if let PlannedEdge::EntityLink { predicate, .. } = step.edge {
            let Some(source) = ends.iter().find(|end| end.item.id == asserter) else {
                return Ok(false);
            };
            let scope_id = source.item.scope_id;
            if !self.configurations.enable(tx, scope_id, predicate).await? {
                return Ok(false);
            }
        }
        revisions_visible(self.state, tx, self.tenant_id, step, asserter).await
    }

    /// The expanded candidate as the plan keeps it, re-read and
    /// re-authorised with its whole path.
    async fn rehydrate(
        &mut self,
        tx: &mut PgConnection,
        expanded: ExpandedCandidate,
    ) -> Result<Rehydrated> {
        let item_id = expanded.snapshot.item.id;
        let Some(current) = knowledge::current(&mut *tx, self.tenant_id, item_id).await? else {
            return Ok(Rehydrated::Dropped);
        };
        if current.revision.id != expanded.snapshot.revision.id {
            return Ok(Rehydrated::Dropped);
        }
        let denied = if is_reached_by_relation(&expanded.path) {
            Rehydrated::Denied
        } else {
            Rehydrated::Dropped
        };
        let authorized =
            crate::knowledge_api::authorize_snapshot(self.state, tx, self.tenant_id, &current);
        let authorization = match authorized.await {
            Ok(allowed) => audit::decision_context(Action::KnowledgeRead, &allowed),
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => return Ok(denied),
            Err(error) => return Err(error),
        };
        for step in &expanded.path {
            if !self.holds(tx, step).await? {
                return Ok(if step.follows_relation() {
                    Rehydrated::Denied
                } else {
                    Rehydrated::Dropped
                });
            }
        }
        Ok(Rehydrated::Kept(Box::new(planned(
            expanded,
            current,
            authorization,
            self.at,
        ))))
    }
}

pub(super) async fn merge_graph_expansion(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    candidates: &mut Vec<PlannedCandidate>,
    expansion: GraphExpansion,
    at: DateTime<Utc>,
    mode: ContextOptimizationMode,
) -> Result<bool> {
    let mut policy_exclusion = expansion.policy_exclusion;
    let mut steps = StepCheck::for_planning(state, tenant_id, at);
    for candidate in candidates.iter_mut() {
        let Some(item_id) = candidate.item_id() else {
            continue;
        };
        let Some(warnings) = expansion.warnings.get(&item_id) else {
            continue;
        };
        for warning in warnings {
            if steps.holds(tx, warning).await? {
                if candidate.graph_path.is_empty() {
                    candidate.graph_path.push(warning.clone());
                }
                if !candidate
                    .reasons
                    .contains(&ContextReasonCode::ContradictionWarning)
                {
                    candidate
                        .reasons
                        .push(ContextReasonCode::ContradictionWarning);
                }
            } else {
                policy_exclusion = true;
            }
        }
    }

    let existing: HashSet<KnowledgeItemId> = candidates
        .iter()
        .filter_map(PlannedCandidate::item_id)
        .collect();
    for expanded in expansion.candidates {
        if existing.contains(&expanded.snapshot.item.id) {
            continue;
        }
        match steps.rehydrate(tx, expanded).await? {
            Rehydrated::Kept(candidate) => candidates.push(*candidate),
            Rehydrated::Dropped => {}
            Rehydrated::Denied => policy_exclusion = true,
        }
    }
    rank_and_deduplicate(candidates, mode);
    Ok(policy_exclusion)
}

/// A rehydrated expanded candidate, ready to rank.
fn planned(
    expanded: ExpandedCandidate,
    current: KnowledgeSnapshot,
    authorization: serde_json::Value,
    at: DateTime<Utc>,
) -> PlannedCandidate {
    let freshness = freshness_micros(&current, at);
    let current_score = if expanded.exclusion.is_none() {
        100_000
    } else {
        0
    };
    let mut reasons = vec![ContextReasonCode::GraphExpansion];
    if freshness > 0 {
        reasons.push(ContextReasonCode::FreshnessBoost);
    }
    if let Some(reason) = expanded.exclusion {
        reasons.push(reason);
    }
    PlannedCandidate {
        id: ContextCandidateId::new(),
        payload: PlannedPayload::Knowledge(current),
        sources: Vec::new(),
        keyword_micros: 0,
        semantic_micros: 0,
        anchor_micros: expanded.anchor_micros,
        edge_weight_micros: expanded.edge_weight_micros,
        hop_penalty_micros: expanded.hop_penalty_micros,
        freshness_micros: freshness,
        pin_micros: 0,
        current_state_micros: current_score,
        final_micros: expanded.final_micros,
        reasons,
        exclusion: expanded.exclusion,
        authorization,
        selected_tokens: None,
        delivery_span: None,
        required: false,
        rehydrated: false,
        graph_path: expanded.path,
    }
}

pub(super) async fn stale_at(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    snapshot: &KnowledgeSnapshot,
    at: DateTime<Utc>,
) -> Result<bool> {
    let evidence = knowledge_freshness::evidence(
        tx,
        tenant_id,
        snapshot.revision.id,
        snapshot.item.project_id,
    )
    .await?;
    Ok(assess_freshness(
        snapshot.item.knowledge_type,
        snapshot.item.lifecycle_state,
        snapshot.revision.content.stale_after,
        evidence,
        at,
    )
    .stale)
}
