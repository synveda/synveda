//! Business links seen from Knowledge items (ADR-0164 Amendments 2 and 3).
//! Recall shows a hit's links and graph expansion follows them. Either way
//! a link counts only when the caller may read its other end, that end
//! passes the caller's own test, and the source's scope still enables the
//! predicate. Expansion also needs the revision that asserted the link to be
//! readable, as the graph step naming it is. Every limit applies to links
//! that passed, so hidden links leave no trace.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgConnection;
use synveda_store::knowledge::KnowledgeHead;
use synveda_store::{configuration, entity_links};
use synveda_types::configuration::ConfigurationDocument;
use synveda_types::context::ContextGraphDirection;
use synveda_types::knowledge::{EntityLink, EntityPredicate};
use synveda_types::{
    EntityLinkId, KnowledgeItemId, KnowledgeRevisionId, Result, ScopeId, Sensitivity, TenantId,
};
use utoipa::ToSchema;

use super::{AssertingRevisions, ContextKnowledgeView};
use crate::app::AppState;
use crate::knowledge_api::ReadableItems;
use crate::workspaces::string_enum;

/// Most links one hit shows.
const MAX_HIT_LINKS: usize = 16;

fn direction_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        ContextGraphDirection::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

/// One current business link on a recall hit. Its other end passed its own
/// read decision.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RecallLinkView {
    /// Stable link id.
    #[schema(value_type = String, format = "uuid")]
    pub link_id: EntityLinkId,
    /// Compact IRI of the predicate.
    #[schema(schema_with = crate::knowledge_api::predicate_schema)]
    pub predicate: String,
    /// `outbound` when the hit asserts the link, `inbound` when it points to
    /// the hit.
    #[schema(schema_with = direction_schema)]
    pub direction: String,
    /// The item at the other end.
    #[schema(value_type = String, format = "uuid")]
    pub item_id: KnowledgeItemId,
    /// Its Knowledge type.
    pub knowledge_type: String,
    /// Compact IRI of its declared entity class.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(schema_with = crate::knowledge_api::entity_class_schema)]
    pub entity_class: Option<String>,
    /// Its current title.
    pub title: String,
}

/// A link that passed every check, seen from the item it was walked from.
pub(super) struct VisibleLink {
    pub(super) link: EntityLink,
    pub(super) direction: ContextGraphDirection,
    /// The current head of the other end.
    pub(super) other: KnowledgeHead,
}

/// One item's links that passed, oldest first.
#[derive(Default)]
pub(super) struct WalkedLinks {
    pub(super) links: Vec<VisibleLink>,
    /// Whether more links would pass than the limit allowed.
    pub(super) truncated: bool,
}

/// Where a walk starts: an item at its current revision, which the caller
/// has authorised, and the scope whose Configuration governs the links it
/// asserts.
#[derive(Clone, Copy)]
pub(super) struct WalkStart {
    pub(super) item_id: KnowledgeItemId,
    pub(super) revision_id: KnowledgeRevisionId,
    pub(super) scope_id: ScopeId,
}

/// Walks items' business links valid at one instant, as recorded now. It
/// keeps its read decisions and Configuration documents across walks.
pub(super) struct LinkWalker<'a> {
    tenant_id: TenantId,
    readable: ReadableItems<'a>,
    configurations: SourceConfigurations,
    at: DateTime<Utc>,
}

/// One item's stored links and how far through them its walk has got.
struct Walk {
    start: WalkStart,
    links: Vec<EntityLink>,
    next: usize,
    walked: WalkedLinks,
}

impl Walk {
    /// Where the next round stops: one link past the limit, so a walk whose
    /// first round passes in full knows it is truncated.
    fn round_end(&self, limit: usize) -> usize {
        (self.next + limit + 1).min(self.links.len())
    }

    fn is_done(&self) -> bool {
        self.walked.truncated || self.next >= self.links.len()
    }
}

impl<'a> LinkWalker<'a> {
    /// A walker whose decisions treat any item above `ceiling` as unreadable.
    pub(super) fn new(
        state: &'a AppState,
        tenant_id: TenantId,
        ceiling: Option<Sensitivity>,
        at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            readable: ReadableItems::new(state, tenant_id, ceiling),
            configurations: SourceConfigurations::new(tenant_id),
            at,
        }
    }

    /// Each start's links that pass, at most `limit` each. `admits` is the
    /// caller's test of the other end's head. With `asserters`, as graph
    /// expansion walks, a link also needs a readable asserting revision, so
    /// the step it becomes holds when rechecked and cannot displace a path
    /// that would. A start with no stored link has no entry.
    pub(super) async fn walk(
        &mut self,
        tx: &mut PgConnection,
        starts: &[WalkStart],
        limit: usize,
        admits: impl Fn(&KnowledgeHead) -> bool,
        mut asserters: Option<&mut AssertingRevisions<'_>>,
    ) -> Result<HashMap<KnowledgeItemId, WalkedLinks>> {
        let ids: Vec<KnowledgeItemId> = starts.iter().map(|start| start.item_id).collect();
        let mut walked = HashMap::new();
        if ids.is_empty() {
            return Ok(walked);
        }
        let mut stored =
            entity_links::for_items(&mut *tx, self.tenant_id, &ids, self.at, None).await?;
        let mut walks: Vec<Walk> = starts
            .iter()
            .filter_map(|start| {
                let links = stored.remove(&start.item_id)?;
                Some(Walk {
                    start: *start,
                    links,
                    next: 0,
                    walked: WalkedLinks::default(),
                })
            })
            .collect();
        // A round decides the next links of every walk still short of its
        // limit together, so the walks take as many rounds as the deepest
        // needs.
        while !walks.is_empty() {
            let others: Vec<KnowledgeItemId> = walks
                .iter()
                .flat_map(|walk| {
                    let from = walk.start.item_id;
                    walk.links[walk.next..walk.round_end(limit)]
                        .iter()
                        .map(move |link| link.other_end(from))
                })
                .collect();
            self.readable.decide(tx, others).await?;
            for walk in &mut walks {
                let asserters = asserters.as_deref_mut();
                self.advance(tx, walk, limit, &admits, asserters).await?;
            }
            let done = walks.extract_if(.., |walk| walk.is_done());
            walked.extend(done.map(|walk| (walk.start.item_id, walk.walked)));
        }
        Ok(walked)
    }

    /// Takes the links in the walk's next round that pass, up to the limit.
    async fn advance(
        &mut self,
        tx: &mut PgConnection,
        walk: &mut Walk,
        limit: usize,
        admits: &impl Fn(&KnowledgeHead) -> bool,
        mut asserters: Option<&mut AssertingRevisions<'_>>,
    ) -> Result<()> {
        let end = walk.round_end(limit);
        for link in &walk.links[walk.next..end] {
            let checked = self.check(tx, walk.start, link, admits, asserters.as_deref_mut());
            let Some(visible) = checked.await? else {
                continue;
            };
            if walk.walked.links.len() == limit {
                walk.walked.truncated = true;
                break;
            }
            walk.walked.links.push(visible);
        }
        walk.next = end;
        Ok(())
    }

    /// The link as seen from `start`, or `None` when it does not pass.
    async fn check(
        &mut self,
        tx: &mut PgConnection,
        start: WalkStart,
        link: &EntityLink,
        admits: &impl Fn(&KnowledgeHead) -> bool,
        asserters: Option<&mut AssertingRevisions<'_>>,
    ) -> Result<Option<VisibleLink>> {
        let Some(other) = self.readable.readable(link.other_end(start.item_id)) else {
            return Ok(None);
        };
        if !admits(other) {
            return Ok(None);
        }
        let (direction, source_scope) = if link.source_item_id == start.item_id {
            (ContextGraphDirection::Outbound, start.scope_id)
        } else {
            (ContextGraphDirection::Inbound, other.item.scope_id)
        };
        let other = other.clone();
        if !self
            .configurations
            .enable(tx, source_scope, link.predicate)
            .await?
        {
            return Ok(None);
        }
        if let Some(asserters) = asserters {
            let ends = [
                (start.item_id, start.revision_id),
                (other.item.id, other.item.current_revision_id),
            ];
            let asserting = link.asserting_revision_id;
            if !asserters
                .readable(tx, link.source_item_id, asserting, ends)
                .await?
            {
                return Ok(None);
            }
        }
        Ok(Some(VisibleLink {
            link: link.clone(),
            direction,
            other,
        }))
    }
}

/// Puts each hit's current business links on it, at most [`MAX_HIT_LINKS`]
/// each, oldest first.
pub(super) async fn attach(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    items: &mut [ContextKnowledgeView],
    at: DateTime<Utc>,
) -> Result<()> {
    let starts: Vec<WalkStart> = items
        .iter()
        .map(|item| WalkStart {
            item_id: item.knowledge.id,
            revision_id: item.knowledge.current_revision.id,
            scope_id: item.knowledge.scope_id,
        })
        .collect();
    let admits = |other: &KnowledgeHead| {
        super::holds_at(
            other.item.lifecycle_state,
            other.valid_from,
            other.valid_to,
            at,
        )
    };
    let mut walker = LinkWalker::new(state, tenant_id, None, at);
    let mut walked = walker
        .walk(tx, &starts, MAX_HIT_LINKS, admits, None)
        .await?;
    for hit in items {
        let Some(walked) = walked.remove(&hit.knowledge.id) else {
            continue;
        };
        hit.links = walked.links.into_iter().map(recall_view).collect();
        hit.links_truncated = walked.truncated;
    }
    Ok(())
}

fn recall_view(visible: VisibleLink) -> RecallLinkView {
    let item = &visible.other.item;
    RecallLinkView {
        link_id: visible.link.id,
        predicate: visible.link.predicate.as_str().to_owned(),
        direction: visible.direction.as_str().to_owned(),
        item_id: item.id,
        knowledge_type: item.knowledge_type.as_str().to_owned(),
        entity_class: visible
            .other
            .entity_class
            .map(|class| class.as_str().to_owned()),
        title: visible.other.title,
    }
}

/// The effective Configuration of each source scope consulted.
pub(super) struct SourceConfigurations {
    tenant_id: TenantId,
    documents: HashMap<ScopeId, ConfigurationDocument>,
}

impl SourceConfigurations {
    pub(super) fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            documents: HashMap::new(),
        }
    }

    /// Whether the scope still enables the predicate, so emptying the
    /// vocabulary takes links out of recall and traversal.
    pub(super) async fn enable(
        &mut self,
        tx: &mut PgConnection,
        scope_id: ScopeId,
        predicate: EntityPredicate,
    ) -> Result<bool> {
        let document = match self.documents.entry(scope_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let effective = configuration::effective_at_scope(tx, self.tenant_id, scope_id);
                entry.insert(effective.await?.document)
            }
        };
        Ok(document.permits_entity_predicate(predicate))
    }
}
