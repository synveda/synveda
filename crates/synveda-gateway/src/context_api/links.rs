//! Business links on recall hits (ADR-0164 Amendment 2). A hit shows a link
//! only when the caller may read its other end, that end is current Knowledge
//! at the query instant, and the source's scope still enables the predicate.
//! Every limit applies to links that passed those checks, so hidden links
//! leave no trace.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgConnection;
use synveda_store::{configuration, entity_links};
use synveda_types::configuration::ConfigurationDocument;
use synveda_types::context::ContextGraphDirection;
use synveda_types::knowledge::{EntityLink, EntityPredicate};
use synveda_types::{EntityLinkId, KnowledgeItemId, Result, ScopeId, TenantId};
use utoipa::ToSchema;

use super::ContextKnowledgeView;
use crate::app::AppState;
use crate::knowledge_api::ReadableItems;
use crate::workspaces::string_enum;

/// Most links one hit shows.
const MAX_HIT_LINKS: usize = 16;
/// Links a hit decides per round: one more than it shows, so a hit whose
/// first chunk passes in full knows it is truncated.
const CHUNK: usize = MAX_HIT_LINKS + 1;

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

/// What decides whether a stored link shows on a hit.
struct LinkFilter<'a> {
    readable: ReadableItems<'a>,
    configurations: SourceConfigurations,
    at: DateTime<Utc>,
}

/// The effective Configuration of each source scope consulted.
struct SourceConfigurations {
    tenant_id: TenantId,
    documents: HashMap<ScopeId, ConfigurationDocument>,
}

/// One hit's stored links and how far through them it has got.
struct HitWalk<'v> {
    hit: &'v mut ContextKnowledgeView,
    links: Vec<EntityLink>,
    next: usize,
}

impl HitWalk<'_> {
    fn chunk(&self) -> &[EntityLink] {
        let end = (self.next + CHUNK).min(self.links.len());
        &self.links[self.next..end]
    }

    fn is_done(&self) -> bool {
        self.hit.links_truncated || self.next >= self.links.len()
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
    let hit_ids: Vec<KnowledgeItemId> = items.iter().map(|item| item.knowledge.id).collect();
    if hit_ids.is_empty() {
        return Ok(());
    }
    let mut stored = entity_links::for_items(&mut *tx, tenant_id, &hit_ids, at, None).await?;
    let mut walks: Vec<HitWalk<'_>> = items
        .iter_mut()
        .filter_map(|hit| {
            let links = stored.remove(&hit.knowledge.id)?;
            Some(HitWalk {
                hit,
                links,
                next: 0,
            })
        })
        .collect();
    let mut filter = LinkFilter {
        readable: ReadableItems::new(state, tenant_id),
        configurations: SourceConfigurations {
            tenant_id,
            documents: HashMap::new(),
        },
        at,
    };
    // A round decides the next chunk of every hit still short of its limit
    // together, so a query takes as many rounds as its deepest hit needs.
    while !walks.is_empty() {
        let others: Vec<KnowledgeItemId> = walks
            .iter()
            .flat_map(|walk| {
                let hit = walk.hit.knowledge.id;
                walk.chunk().iter().map(move |link| link.other_end(hit))
            })
            .collect();
        filter.readable.decide(tx, others).await?;
        for walk in &mut walks {
            filter.advance(tx, walk).await?;
        }
        walks.retain(|walk| !walk.is_done());
    }
    Ok(())
}

impl LinkFilter<'_> {
    /// Shows the links in the walk's next chunk that pass, up to the limit.
    async fn advance(&mut self, tx: &mut PgConnection, walk: &mut HitWalk<'_>) -> Result<()> {
        let end = (walk.next + CHUNK).min(walk.links.len());
        for link in &walk.links[walk.next..end] {
            let Some(view) = self.view(tx, walk.hit, link).await? else {
                continue;
            };
            if walk.hit.links.len() == MAX_HIT_LINKS {
                walk.hit.links_truncated = true;
                break;
            }
            walk.hit.links.push(view);
        }
        walk.next = end;
        Ok(())
    }

    /// The link as the hit shows it, or `None` when it does not show.
    async fn view(
        &mut self,
        tx: &mut PgConnection,
        hit: &ContextKnowledgeView,
        link: &EntityLink,
    ) -> Result<Option<RecallLinkView>> {
        let hit_id = hit.knowledge.id;
        let Some(other) = self.readable.readable(link.other_end(hit_id)) else {
            return Ok(None);
        };
        let item = &other.item;
        if !super::holds_at(
            item.lifecycle_state,
            other.valid_from,
            other.valid_to,
            self.at,
        ) {
            return Ok(None);
        }
        let (direction, source_scope) = if link.source_item_id == hit_id {
            (ContextGraphDirection::Outbound, hit.knowledge.scope_id)
        } else {
            (ContextGraphDirection::Inbound, item.scope_id)
        };
        if !self
            .configurations
            .enable(tx, source_scope, link.predicate)
            .await?
        {
            return Ok(None);
        }
        Ok(Some(RecallLinkView {
            link_id: link.id,
            predicate: link.predicate.as_str().to_owned(),
            direction: direction.as_str().to_owned(),
            item_id: item.id,
            knowledge_type: item.knowledge_type.as_str().to_owned(),
            entity_class: other.entity_class.map(|class| class.as_str().to_owned()),
            title: other.title.clone(),
        }))
    }
}

impl SourceConfigurations {
    /// Whether the scope still enables the predicate, so emptying the
    /// vocabulary takes links out of recall.
    async fn enable(
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
