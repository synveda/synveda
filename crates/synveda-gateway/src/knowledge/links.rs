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
use synveda_types::{
    EntityLinkId, Error, KnowledgeItemId, KnowledgeRevisionId, Result, Sensitivity, TenantId,
};

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
    /// The source at the expected revision the link will pin, or `None` when
    /// the source has no such revision.
    source: Option<LinkEndpointItem>,
    target: LinkEndpointItem,
    target_revision_id: KnowledgeRevisionId,
}

impl AuthorizedLink {
    /// Refuses a new link the source scope's Configuration does not admit,
    /// and freezes the target revision the check read into the command. The
    /// source's class was read at the expected revision the effect pins, so
    /// neither end's class can change before the link is recorded. This runs
    /// after every target is authorised, so a refusal here reveals nothing
    /// about a source the caller cannot write.
    pub(super) fn admit(
        &self,
        configuration: &ConfigurationDocument,
        command: &mut KnowledgeCommand,
    ) -> Result<()> {
        let Some(link) = &self.proposed else {
            return Ok(());
        };
        let source = link.source.ok_or_else(|| Error::Conflict {
            message: "expected_revision_id is not a revision of the source item".to_owned(),
        })?;
        configuration.admit_entity_link(link.predicate, source, link.target)?;
        if let KnowledgeCommand::Link {
            target_revision_id, ..
        } = command
        {
            *target_revision_id = Some(link.target_revision_id);
        }
        Ok(())
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
            expected_revision_id,
            target_item_id,
            predicate,
            ..
        } => {
            let target = readable(state, tx, tenant_id, *target_item_id, || {
                missing_item(*target_item_id)
            })
            .await?;
            let source = store::mutation_target(&mut *tx, tenant_id, *source_item_id)
                .await?
                .ok_or_else(|| missing_item(*source_item_id))?;
            let asserting =
                store::revision(&mut *tx, tenant_id, *source_item_id, *expected_revision_id)
                    .await?
                    .map(|revision| -> Result<LinkEndpointItem> {
                        Ok(LinkEndpointItem {
                            knowledge_type: source.item.knowledge_type,
                            entity_class: revision.content.entity_class()?,
                        })
                    })
                    .transpose()?;
            Ok(Some(AuthorizedLink {
                target_sensitivity: target.revision.content.sensitivity,
                proposed: Some(ProposedLink {
                    predicate: *predicate,
                    source: asserting,
                    target: endpoint(&target)?,
                    target_revision_id: target.revision.id,
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
/// revision and the target at the revision admission read, so the classes
/// the allowlists and the endpoint rule checked still hold. Both ends must
/// also still be linkable and have room for another link; otherwise the
/// change is rejected. Both items are locked before the checks, so they
/// hold until the commit.
pub(super) async fn assert(
    tx: &mut PgConnection,
    link: &NewEntityLink<'_>,
    target_revision_id: KnowledgeRevisionId,
) -> Result<AppliedEffect> {
    let ends = [link.source_item_id, link.target_item_id];
    store::lock_items(tx, link.tenant_id, &ends).await?;
    let source = super::mutation_current(
        tx,
        link.tenant_id,
        link.source_item_id,
        link.asserting_revision_id,
    )
    .await?;
    let target =
        super::mutation_current(tx, link.tenant_id, link.target_item_id, target_revision_id)
            .await?;
    ensure_linkable(&source.item)?;
    ensure_linkable(&target.item)?;
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
