//! Business links as graph edges (ADR-0164 Amendment 3). Every link carries
//! one fixed weight, so ranking stays deterministic.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_store::entity_links;
use synveda_store::knowledge::KnowledgeSnapshot;
use synveda_types::context::ContextGraphDirection;
use synveda_types::knowledge::{EntityLink, EntityPredicate};
use synveda_types::{EntityLinkId, KnowledgeItemId, Result, TenantId};

use super::super::{PlannedEdge, PlannedGraphStep};

/// The weight of every business link: a reviewed claim, ranked with
/// `supersedes` and below the evidence relations.
const LINK_WEIGHT_MICROS: i32 = 500_000;

/// The step from `from` across `link` to `to`.
pub(super) fn link_step(
    link: &EntityLink,
    direction: ContextGraphDirection,
    from: &KnowledgeSnapshot,
    to: &KnowledgeSnapshot,
) -> PlannedGraphStep {
    let relation_hash = blake3::hash(
        format!(
            "entity-link|{}|{}|{}|{}|{}|{}|{}|{}",
            link.id,
            link.predicate,
            direction,
            from.revision.id,
            from.revision.content_hash,
            to.revision.id,
            to.revision.content_hash,
            link.asserting_revision_id,
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    PlannedGraphStep {
        edge: PlannedEdge::EntityLink {
            id: link.id,
            predicate: link.predicate,
        },
        relation_hash,
        direction,
        from_item_id: from.item.id,
        from_revision_id: from.revision.id,
        to_item_id: to.item.id,
        to_revision_id: to.revision.id,
        asserting_revision_id: link.asserting_revision_id,
        from_content_hash: from.revision.content_hash.clone(),
        to_content_hash: to.revision.content_hash.clone(),
        edge_weight_micros: LINK_WEIGHT_MICROS,
        supporting: true,
    }
}

/// The source of the link a step names, when that link is still recorded
/// with the step's predicate and asserting revision between the step's two
/// ends. With `holding_at`, the link must also be valid then.
pub(super) async fn recorded_source(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    link_id: EntityLinkId,
    predicate: EntityPredicate,
    step: &PlannedGraphStep,
    holding_at: Option<DateTime<Utc>>,
) -> Result<Option<KnowledgeItemId>> {
    let Some(link) = entity_links::get(&mut *tx, tenant_id, link_id).await? else {
        return Ok(None);
    };
    let joins_ends = [step.from_item_id, step.to_item_id]
        .iter()
        .all(|end| [link.source_item_id, link.target_item_id].contains(end));
    let holds = holding_at.is_none_or(|at| link.is_valid_at(at));
    Ok((link.predicate == predicate
        && link.asserting_revision_id == step.asserting_revision_id
        && joins_ends
        && holds)
        .then_some(link.source_item_id))
}
