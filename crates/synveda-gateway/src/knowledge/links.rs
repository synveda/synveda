//! Business-link commands (ADR-0164). Both ends are authorised for reading
//! before anything about them is checked, and an absent end is refused
//! exactly like an unreadable one, so a refusal reveals neither.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_store::entity_links::{self, NewEntityLink};
use synveda_store::knowledge::{self as store, KnowledgeSnapshot};
use synveda_types::configuration::ConfigurationDocument;
use synveda_types::knowledge::{
    EntityPredicate, KnowledgeCommand, KnowledgeItem, KnowledgeLifecycleState, LinkEndpointItem,
};
use synveda_types::{EntityLinkId, Error, KnowledgeItemId, Result, Sensitivity, TenantId};

use super::{AppliedEffect, missing_item};
use crate::app::AppState;

/// What a link command's authorisation learned about its ends.
pub(super) struct AuthorizedLink {
    /// The target's classification, which the review requirement covers.
    pub(super) target_sensitivity: Sensitivity,
    proposed: Option<ProposedLink>,
}

/// The facts a new link's admission checks.
struct ProposedLink {
    predicate: EntityPredicate,
    source: LinkEndpointItem,
    target: LinkEndpointItem,
}

impl AuthorizedLink {
    /// Refuses a new link the source scope's Configuration does not admit.
    pub(super) fn admit(&self, configuration: &ConfigurationDocument) -> Result<()> {
        self.proposed.as_ref().map_or(Ok(()), |link| {
            configuration.admit_entity_link(link.predicate, link.source, link.target)
        })
    }
}

/// Authorises reading the ends a link command names. Writing the source is
/// authorised with every other command target. Lifecycle and an open link
/// are left to the effect, which rejects a change they no longer allow.
pub(super) async fn authorize(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    command: &KnowledgeCommand,
) -> Result<Option<AuthorizedLink>> {
    match command {
        KnowledgeCommand::Link {
            source_item_id,
            target_item_id,
            predicate,
            ..
        } => {
            let target = readable(state, tx, tenant_id, *target_item_id, || {
                missing_item(*target_item_id)
            })
            .await?;
            let source = store::current(&mut *tx, tenant_id, *source_item_id)
                .await?
                .ok_or_else(|| missing_item(*source_item_id))?;
            Ok(Some(AuthorizedLink {
                target_sensitivity: target.revision.content.sensitivity,
                proposed: Some(ProposedLink {
                    predicate: *predicate,
                    source: endpoint(&source)?,
                    target: endpoint(&target)?,
                }),
            }))
        }
        KnowledgeCommand::EndLink {
            link_id,
            source_item_id,
            target_item_id,
            valid_to,
            ..
        } => {
            let hidden = || link_not_found(*link_id);
            let link = entity_links::get(&mut *tx, tenant_id, *link_id)
                .await?
                .filter(|link| {
                    link.source_item_id == *source_item_id && link.target_item_id == *target_item_id
                })
                .ok_or_else(hidden)?;
            readable(state, tx, tenant_id, link.source_item_id, hidden).await?;
            let target = readable(state, tx, tenant_id, link.target_item_id, hidden).await?;
            if *valid_to <= link.valid_from {
                return Err(Error::Invalid {
                    message: "a business link must end after it starts".to_owned(),
                });
            }
            Ok(Some(AuthorizedLink {
                target_sensitivity: target.revision.content.sensitivity,
                proposed: None,
            }))
        }
        _ => Ok(None),
    }
}

/// Records a reviewed link. The source must still be at the asserting
/// revision the author inspected, and both ends must still be current and
/// fit the predicate; otherwise the change is rejected.
pub(super) async fn assert(
    tx: &mut PgConnection,
    link: &NewEntityLink<'_>,
) -> Result<AppliedEffect> {
    let ends = [link.source_item_id, link.target_item_id];
    store::lock_items_shared(tx, link.tenant_id, &ends).await?;
    let source = super::current(
        tx,
        link.tenant_id,
        link.source_item_id,
        link.asserting_revision_id,
    )
    .await?;
    let target = store::current(&mut *tx, link.tenant_id, link.target_item_id)
        .await?
        .ok_or_else(|| missing_item(link.target_item_id))?;
    ensure_linkable(&source.item)?;
    ensure_linkable(&target.item)?;
    if !link
        .predicate
        .admits(endpoint(&source)?, endpoint(&target)?)
    {
        return Err(Error::Conflict {
            message: format!("{} no longer joins these Knowledge items", link.predicate),
        });
    }
    entity_links::insert(tx, link).await?;
    // A link writes no revision; the change names the revision asserting it.
    Ok(AppliedEffect::item(
        link.source_item_id,
        link.asserting_revision_id,
    ))
}

/// Records a reviewed end, unless the link is gone or already ended.
pub(super) async fn end(
    tx: &mut PgConnection,
    tenant_id: TenantId,
    link_id: EntityLinkId,
    valid_to: DateTime<Utc>,
    actor_subject: &str,
) -> Result<AppliedEffect> {
    let link = entity_links::get(&mut *tx, tenant_id, link_id)
        .await?
        .ok_or_else(|| link_not_found(link_id))?;
    if !entity_links::end(tx, tenant_id, link_id, valid_to, actor_subject).await? {
        return Err(Error::Conflict {
            message: format!("business link {link_id} has already ended"),
        });
    }
    Ok(AppliedEffect::item(
        link.source_item_id,
        link.asserting_revision_id,
    ))
}

pub(crate) fn link_not_found(link_id: EntityLinkId) -> Error {
    Error::NotFound {
        entity: format!("business link {link_id}"),
    }
}

async fn readable(
    state: &AppState,
    tx: &mut PgConnection,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
    hidden: impl Fn() -> Error,
) -> Result<KnowledgeSnapshot> {
    crate::knowledge_api::readable_snapshot(state, tx, tenant_id, item_id)
        .await?
        .ok_or_else(hidden)
}

fn endpoint(snapshot: &KnowledgeSnapshot) -> Result<LinkEndpointItem> {
    Ok(LinkEndpointItem {
        knowledge_type: snapshot.item.knowledge_type,
        entity_class: snapshot.revision.content.entity_class()?,
    })
}

fn ensure_linkable(item: &KnowledgeItem) -> Result<()> {
    if matches!(
        item.lifecycle_state,
        KnowledgeLifecycleState::Active
            | KnowledgeLifecycleState::Stale
            | KnowledgeLifecycleState::Transitional
    ) {
        return Ok(());
    }
    Err(Error::Conflict {
        message: format!(
            "Knowledge item {} is {}; only active, stale or transitional Knowledge can be linked",
            item.id, item.lifecycle_state
        ),
    })
}
