//! Read decisions on many Knowledge items at once. Each item gets the
//! decision [`authorize_snapshot`](super::authorize_snapshot) gives it, from
//! the same parts: its historical evidence rechecked when its revision
//! carries some, then a Knowledge read at its sensitivity. The decision input
//! is gathered once per scope and project rather than once per item. A
//! request's sensitivity ceiling is applied first, so an item above it is
//! not decided at all.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use sqlx::PgConnection;
use synveda_store::knowledge::{self as store, KnowledgeHead};
use synveda_types::{KnowledgeItemId, ProjectId, Result, ScopeId, Sensitivity, TenantId};

use super::{authorize_historical, hide_refusal, read_input, with_historical_evidence};
use crate::app::AppState;
use crate::authz::{self, DecisionInput};

/// The read decisions one request has taken, so an item is decided once
/// however many links reach it.
pub(crate) struct ReadableItems<'a> {
    state: &'a AppState,
    tenant_id: TenantId,
    /// The request's sensitivity ceiling. An item above it is not decided and
    /// reads as unreadable.
    ceiling: Option<Sensitivity>,
    /// `None` for an absent item and for one the caller may not read alike.
    decided: HashMap<KnowledgeItemId, Option<KnowledgeHead>>,
    /// `None` when nothing at the placement can be read.
    inputs: HashMap<(ScopeId, Option<ProjectId>), Option<DecisionInput>>,
}

impl<'a> ReadableItems<'a> {
    /// Decisions that treat any item above `ceiling` as unreadable.
    pub(crate) fn new(
        state: &'a AppState,
        tenant_id: TenantId,
        ceiling: Option<Sensitivity>,
    ) -> Self {
        Self {
            state,
            tenant_id,
            ceiling,
            decided: HashMap::new(),
            inputs: HashMap::new(),
        }
    }

    /// Decides each of the items not decided yet.
    pub(crate) async fn decide(
        &mut self,
        tx: &mut PgConnection,
        item_ids: impl IntoIterator<Item = KnowledgeItemId>,
    ) -> Result<()> {
        let mut pending: Vec<KnowledgeItemId> = item_ids
            .into_iter()
            .filter(|id| !self.decided.contains_key(id))
            .collect();
        pending.sort_unstable();
        pending.dedup();
        if pending.is_empty() {
            return Ok(());
        }
        self.decided.extend(pending.iter().map(|id| (*id, None)));
        let mut heads = store::current_heads(&mut *tx, self.tenant_id, &pending).await?;
        if let Some(ceiling) = self.ceiling {
            heads.retain(|head| head.sensitivity <= ceiling);
        }
        let revisions: Vec<uuid::Uuid> = heads
            .iter()
            .map(|head| head.item.current_revision_id.as_uuid())
            .collect();
        let with_evidence = with_historical_evidence(tx, self.tenant_id, &revisions).await?;
        for head in heads {
            let revision = head.item.current_revision_id.as_uuid();
            if with_evidence.contains(&revision) {
                let recheck = authorize_historical(
                    self.state,
                    tx,
                    self.tenant_id,
                    "knowledge_revision",
                    revision,
                );
                if hide_refusal(recheck.await)?.is_none() {
                    continue;
                }
            }
            if self.permits(tx, &head).await? {
                self.decided.insert(head.item.id, Some(head));
            }
        }
        Ok(())
    }

    /// The current head of a decided item the caller may read.
    pub(crate) fn readable(&self, item_id: KnowledgeItemId) -> Option<&KnowledgeHead> {
        self.decided.get(&item_id).and_then(Option::as_ref)
    }

    async fn permits(&mut self, tx: &mut PgConnection, head: &KnowledgeHead) -> Result<bool> {
        let item = &head.item;
        let input = match self.inputs.entry((item.scope_id, item.project_id)) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let gathered = read_input(self.state, tx, self.tenant_id, item).await;
                entry.insert(hide_refusal(gathered)?)
            }
        };
        let Some(input) = input else {
            return Ok(false);
        };
        let decision = authz::decide_knowledge_read_of(
            self.state,
            input,
            item.id,
            item.scope_id,
            head.sensitivity,
        );
        Ok(hide_refusal(decision)?.is_some())
    }
}
