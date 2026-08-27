//! Bounded, independently authorised Knowledge-relation expansion.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{Acquire, PgConnection};
use synveda_policy::Action;
use synveda_retrieval::estimated_tokens;
use synveda_store::knowledge::{self as knowledge, KnowledgeSnapshot};
use synveda_store::knowledge_freshness;
use synveda_types::configuration::GraphRetrievalConfiguration;
use synveda_types::context::{ContextGraphDirection, ContextReasonCode};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeRelation, KnowledgeRelationType, assess_freshness,
};
use synveda_types::{ContextCandidateId, Error, KnowledgeItemId, Result, Sensitivity, TenantId};

use crate::app::AppState;
use crate::audit;

use super::{
    PlannedCandidate, PlannedGraphStep, PlannedPayload, freshness_micros, load_visible_revision,
    push_degradation, rank_and_deduplicate,
};

pub(super) const VERSION: &str = "knowledge-relations-v1";
const MAX_ANCHORS: usize = 10;
const HOP_PENALTY_MICROS: i32 = 100_000;

#[derive(Clone)]
struct GraphFrontier {
    snapshot: KnowledgeSnapshot,
    anchor_micros: i32,
    edge_weight_micros: i32,
    path_rank_micros: i32,
    path: Vec<PlannedGraphStep>,
    visited: Vec<KnowledgeItemId>,
}

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
        relation_id: relation.id,
        relation_hash,
        relation_type: relation.relation_type,
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

async fn graph_expansion_inner(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    anchors: Vec<GraphFrontier>,
    configuration: &GraphRetrievalConfiguration,
    max_sensitivity: Option<Sensitivity>,
    at: DateTime<Utc>,
) -> Result<GraphExpansion> {
    let mut expansion = GraphExpansion::default();
    let mut queue: VecDeque<GraphFrontier> = anchors.into();
    let mut best: HashMap<KnowledgeItemId, i32> = queue
        .iter()
        .map(|anchor| (anchor.snapshot.item.id, i32::MAX))
        .collect();
    let mut expanded_index: HashMap<KnowledgeItemId, usize> = HashMap::new();
    let mut expanded_tokens = 0_u32;

    'frontier: while let Some(frontier) = queue.pop_front() {
        if !frontier.path.is_empty()
            && best.get(&frontier.snapshot.item.id).copied() != Some(frontier.path_rank_micros)
        {
            continue;
        }
        match crate::knowledge_api::authorize_snapshot(state, tx, tenant_id, &frontier.snapshot)
            .await
        {
            Ok(_) => {}
            Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {
                expansion.policy_exclusion = true;
                continue;
            }
            Err(error) => return Err(error),
        }
        let limit = i64::from(configuration.fan_out_per_node).saturating_add(1);
        let mut relations = knowledge::bounded_retrieval_relations(
            &mut *tx,
            tenant_id,
            frontier.snapshot.item.id,
            limit,
        )
        .await?;
        if relations.len() > usize::try_from(configuration.fan_out_per_node).unwrap_or(usize::MAX) {
            relations
                .truncate(usize::try_from(configuration.fan_out_per_node).unwrap_or(usize::MAX));
            push_degradation(&mut expansion.degraded, "graph_fanout_truncated");
        }

        for relation in relations {
            let Some(target_id) = graph_target(&relation, frontier.snapshot.item.id) else {
                continue;
            };
            if frontier.visited.contains(&target_id) {
                continue;
            }
            let Some(target) = knowledge::current(&mut *tx, tenant_id, target_id).await? else {
                continue;
            };
            if max_sensitivity.is_some_and(|ceiling| target.revision.content.sensitivity > ceiling)
            {
                continue;
            }
            match crate::knowledge_api::authorize_snapshot(state, tx, tenant_id, &target).await {
                Ok(_) => {}
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {
                    expansion.policy_exclusion = true;
                    continue;
                }
                Err(error) => return Err(error),
            }
            let step = graph_step(&relation, &frontier.snapshot, &target);
            if !step.supporting {
                if frontier.path.is_empty() {
                    expansion
                        .warnings
                        .entry(frontier.snapshot.item.id)
                        .or_default()
                        .push(step);
                }
                continue;
            }
            if matches!(
                target.item.lifecycle_state,
                KnowledgeLifecycleState::Transitional
                    | KnowledgeLifecycleState::Archived
                    | KnowledgeLifecycleState::ErasurePending
                    | KnowledgeLifecycleState::Erased
            ) {
                continue;
            }

            let hop = frontier.path.len().saturating_add(1);
            let hop_penalty = i32::try_from(hop)
                .unwrap_or(i32::MAX)
                .saturating_mul(HOP_PENALTY_MICROS);
            let edge_weight = frontier
                .edge_weight_micros
                .saturating_add(step.edge_weight_micros)
                .min(2_000_000);
            let freshness = freshness_micros(&target, at);
            let mut exclusion = match target.item.lifecycle_state {
                KnowledgeLifecycleState::Stale => Some(ContextReasonCode::Stale),
                KnowledgeLifecycleState::Superseded => Some(ContextReasonCode::Superseded),
                _ => None,
            };
            if exclusion.is_none() && stale_at(tx, tenant_id, &target, at).await? {
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
            if best
                .get(&target_id)
                .is_some_and(|held| *held >= final_micros)
            {
                continue;
            }

            let mut path = frontier.path.clone();
            path.push(step);
            let candidate = ExpandedCandidate {
                snapshot: target.clone(),
                anchor_micros: frontier.anchor_micros,
                edge_weight_micros: edge_weight,
                hop_penalty_micros: hop_penalty,
                final_micros,
                path: path.clone(),
                exclusion,
            };
            if let Some(index) = expanded_index.get(&target_id).copied() {
                expansion.candidates[index] = candidate;
            } else {
                if expansion.candidates.len()
                    >= usize::try_from(configuration.max_expanded_candidates).unwrap_or(usize::MAX)
                {
                    push_degradation(&mut expansion.degraded, "graph_candidate_budget_exceeded");
                    break 'frontier;
                }
                let tokens = graph_candidate_tokens(&target);
                if expanded_tokens.saturating_add(tokens) > configuration.token_budget {
                    push_degradation(&mut expansion.degraded, "graph_token_budget_exceeded");
                    continue;
                }
                expanded_tokens = expanded_tokens.saturating_add(tokens);
                expanded_index.insert(target_id, expansion.candidates.len());
                expansion.candidates.push(candidate);
            }
            best.insert(target_id, final_micros);
            if exclusion.is_none() && hop < usize::from(configuration.max_hops) {
                let mut visited = frontier.visited.clone();
                visited.push(target_id);
                queue.push_back(GraphFrontier {
                    snapshot: target,
                    anchor_micros: frontier.anchor_micros,
                    edge_weight_micros: edge_weight,
                    path_rank_micros: final_micros,
                    path,
                    visited,
                });
            }
        }
    }
    Ok(expansion)
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

async fn graph_step_is_visible(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    step: &PlannedGraphStep,
) -> Result<bool> {
    let Some(relation) = knowledge::relation(&mut *tx, tenant_id, step.relation_id).await? else {
        return Ok(false);
    };
    if relation.relation_type != step.relation_type
        || relation.asserting_revision_id != step.asserting_revision_id
    {
        return Ok(false);
    }
    let asserting_item = if relation.source_item_id == step.from_item_id {
        step.from_item_id
    } else if relation.source_item_id == step.to_item_id {
        step.to_item_id
    } else {
        return Ok(false);
    };
    for (item_id, revision_id) in [
        (step.from_item_id, step.from_revision_id),
        (step.to_item_id, step.to_revision_id),
    ] {
        let Some(current) = knowledge::current(&mut *tx, tenant_id, item_id).await? else {
            return Ok(false);
        };
        if current.revision.id != revision_id {
            return Ok(false);
        }
    }
    for (item_id, revision_id) in [
        (step.from_item_id, step.from_revision_id),
        (step.to_item_id, step.to_revision_id),
        (asserting_item, step.asserting_revision_id),
    ] {
        if load_visible_revision(state, tx, tenant_id, item_id, revision_id, false)
            .await?
            .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) async fn merge_graph_expansion(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    candidates: &mut Vec<PlannedCandidate>,
    expansion: GraphExpansion,
    at: DateTime<Utc>,
) -> Result<bool> {
    let mut policy_exclusion = expansion.policy_exclusion;
    for candidate in candidates.iter_mut() {
        let Some(item_id) = candidate.item_id() else {
            continue;
        };
        let Some(warnings) = expansion.warnings.get(&item_id) else {
            continue;
        };
        for warning in warnings {
            if graph_step_is_visible(state, tx, tenant_id, warning).await? {
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
        let Some(current) =
            knowledge::current(&mut *tx, tenant_id, expanded.snapshot.item.id).await?
        else {
            continue;
        };
        if current.revision.id != expanded.snapshot.revision.id {
            continue;
        }
        let authorization =
            match crate::knowledge_api::authorize_snapshot(state, tx, tenant_id, &current).await {
                Ok(allowed) => audit::decision_context(Action::KnowledgeRead, &allowed),
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {
                    policy_exclusion = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
        let mut path_visible = true;
        for step in &expanded.path {
            if !graph_step_is_visible(state, tx, tenant_id, step).await? {
                path_visible = false;
                policy_exclusion = true;
                break;
            }
        }
        if !path_visible {
            continue;
        }
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
        candidates.push(PlannedCandidate {
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
            graph_path: expanded.path,
        });
    }
    rank_and_deduplicate(candidates);
    Ok(policy_exclusion)
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
