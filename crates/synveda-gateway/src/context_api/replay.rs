//! Current disclosure proof for an immutable saved delivery (ADR-0141).
//!
//! Optional traces are not complete lifecycle lineage. A saved contribution
//! without exact dependency proof is refused without replacing its receipt.

use std::collections::HashMap;

use chrono::Utc;
use serde_json::Value;
use sqlx::PgConnection;
use synveda_policy::Action;
use synveda_store::{configuration, knowledge, sessions};
use synveda_types::capture::CaptureCandidateState;
use synveda_types::configuration::ConfigurationContextChannel;
use synveda_types::session::{ContextRun, SessionEventType};
use synveda_types::{
    CaptureCandidateId, ContextCandidateId, ContextGraphStep, Error, KnowledgeRevisionId, Result,
    TenantId, TraceRetentionMode,
};

// Exceeds the current planner's combined bounded pools. Corrupt or future
// larger saved rows must not turn replay into an unbounded authorization pass.
const MAX_SAVED_SELECTIONS: usize = 1024;

pub(super) async fn admissible(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    run: &ContextRun,
) -> Result<bool> {
    if run.rendered.len() > synveda_retrieval::MAX_RENDERED_CONTEXT_BYTES
        || run.block_hash != blake3::hash(run.rendered.as_bytes()).to_hex().as_str()
        || run.selection_count < 0
        || run.selection_count as usize > MAX_SAVED_SELECTIONS
        || run.restart_event_ids.len() > 16
    {
        return Ok(false);
    }
    if run.rendered.is_empty()
        && run.entry_count == 0
        && run.skills.as_array().is_some_and(Vec::is_empty)
    {
        return Ok(true);
    }
    let Some(manifest) = super::dependencies::Manifest::load(tx, tenant, run).await? else {
        return Ok(false);
    };
    let effective = configuration::effective_at_scope(tx, tenant, run.scope_id).await?;
    if effective.content_hash != run.configuration_hash
        || !manifest.restart_matches(state, tx, tenant, run).await?
        || !manifest.authored_matches(state, tx, tenant, run).await?
    {
        return Ok(false);
    }
    let Some(disclosed) = disclosed_sources(&run.rendered) else {
        return Ok(false);
    };
    if disclosed.len() != manifest.knowledge.len() + manifest.candidates.len() {
        return Ok(false);
    }
    let at = Utc::now();
    for step in manifest.graph {
        for (item, revision) in [
            (step.from_item_id, step.from_revision_id),
            (step.to_item_id, step.to_revision_id),
        ] {
            let Some(current) = knowledge::current(&mut *tx, tenant, item).await? else {
                return Ok(false);
            };
            if current.revision.id != revision || !super::current_active_at(&current, at) {
                return Ok(false);
            }
        }
        let held = ContextGraphStep {
            tenant_id: tenant,
            context_run_id: run.id,
            context_candidate_id: ContextCandidateId::new(),
            ordinal: 0,
            hop: 1,
            relation_id: Some(step.relation_id),
            relation_hash: step.relation_hash,
            relation_type: step.relation_type,
            direction: step.direction,
            from_item_id: Some(step.from_item_id),
            from_revision_id: Some(step.from_revision_id),
            to_item_id: Some(step.to_item_id),
            to_revision_id: Some(step.to_revision_id),
            asserting_revision_id: Some(step.asserting_revision_id),
            from_content_hash: step.from_content_hash,
            to_content_hash: step.to_content_hash,
            edge_weight_micros: step.edge_weight_micros,
            supporting: step.supporting,
            created_at: run.created_at,
        };
        let (path, _) =
            super::stored_graph_path_view(state, tx, tenant, TraceRetentionMode::Full, vec![held])
                .await?;
        if path.is_none() {
            return Ok(false);
        }
    }
    for selection in manifest.knowledge {
        if !effective
            .document
            .context
            .permits(ConfigurationContextChannel::CurrentKnowledge)
        {
            return Ok(false);
        }
        let Some(current) = knowledge::current(&mut *tx, tenant, selection.item_id).await? else {
            return Ok(false);
        };
        if current.revision.id != selection.revision_id
            || current.revision.content_hash != selection.content_hash
            || !super::current_active_at(&current, at)
            || super::graph::stale_at(tx, tenant, &current, at).await?
        {
            return Ok(false);
        }
        let Some(visible) = super::load_visible_revision(
            state,
            tx,
            tenant,
            selection.item_id,
            selection.revision_id,
            true,
        )
        .await?
        else {
            return Ok(false);
        };
        if !selection
            .source_ids
            .iter()
            .all(|id| visible.sources.iter().any(|source| source.id == *id))
        {
            return Ok(false);
        }
        let permitted: Vec<_> = visible.sources.iter().map(super::source_line).collect();
        if !sources_still_visible(
            &disclosed,
            EvidenceKey::Knowledge(selection.revision_id),
            &permitted,
        ) {
            return Ok(false);
        }
    }
    for selection in manifest.candidates {
        if !effective
            .document
            .context
            .permits(ConfigurationContextChannel::UnreviewedCandidates)
        {
            return Ok(false);
        }
        let Some(candidate) = super::load_visible_capture_candidate(
            state,
            tx,
            tenant,
            selection.candidate_id,
            &selection.content_hash,
            true,
        )
        .await?
        else {
            return Ok(false);
        };
        if candidate.state != CaptureCandidateState::Pending
            || candidate.source_event_ids != selection.source_event_ids
            || candidate.source_artifact_ids != selection.source_artifact_ids
        {
            return Ok(false);
        }
        let permitted: Vec<_> = candidate
            .source_event_ids
            .iter()
            .map(|id| format!("session-event:{id}"))
            .chain(
                candidate
                    .source_artifact_ids
                    .iter()
                    .map(|id| format!("import-artifact:{id}")),
            )
            .collect();
        if !sources_still_visible(
            &disclosed,
            EvidenceKey::Candidate(selection.candidate_id),
            &permitted,
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

// The original block contains one JSON line per selected input, so source
// descriptors actually disclosed can be checked without requiring access to
// independently hidden evidence supporting an approved shared conclusion.
#[derive(Debug, PartialEq, Eq, Hash)]
enum EvidenceKey {
    Knowledge(KnowledgeRevisionId),
    Candidate(CaptureCandidateId),
}

fn disclosed_sources(rendered: &str) -> Option<HashMap<EvidenceKey, Vec<String>>> {
    let mut disclosed = HashMap::new();
    for line in rendered
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .filter(|line| line.starts_with('{'))
    {
        let entry: Value = serde_json::from_str(line).ok()?;
        let key = match entry["kind"].as_str()? {
            "published_knowledge" => {
                EvidenceKey::Knowledge(entry["knowledge_revision_id"].as_str()?.parse().ok()?)
            }
            "unreviewed_candidate" => {
                EvidenceKey::Candidate(entry["capture_candidate_id"].as_str()?.parse().ok()?)
            }
            "checkpoint_user_excerpt" | "recent_user_event" => continue,
            _ => return None,
        };
        if disclosed.len() >= MAX_SAVED_SELECTIONS {
            return None;
        }
        let sources = entry["sources"]
            .as_array()?
            .iter()
            .map(|source| source.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        if disclosed.insert(key, sources).is_some() {
            return None;
        }
    }
    Some(disclosed)
}

fn sources_still_visible(
    disclosed: &HashMap<EvidenceKey, Vec<String>>,
    key: EvidenceKey,
    permitted: &[String],
) -> bool {
    disclosed.get(&key).is_some_and(|sources| {
        sources
            .iter()
            .all(|source| permitted.iter().any(|held| held == source))
    })
}

pub(super) async fn restart_available(
    state: &crate::app::AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    run: &ContextRun,
) -> Result<bool> {
    if run.checkpoint_event_id.is_none() && run.restart_event_ids.is_empty() {
        return Ok(true);
    }
    match crate::sessions::load(state, tx, tenant, run.session_id, Action::SessionRead).await {
        Ok(_) => {}
        Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => return Ok(false),
        Err(error) => return Err(error),
    }
    if let Some(id) = run.checkpoint_event_id
        && !crate::sessions::checkpoint::source_available(tx, tenant, run.session_id, id).await?
    {
        return Ok(false);
    }
    for id in &run.restart_event_ids {
        let event = sessions::event(&mut *tx, tenant, run.session_id, *id).await?;
        if !event.is_some_and(|event| event.event_type == SessionEventType::MessageUser)
            || !sessions::withheld_event_ids(&mut *tx, tenant, run.session_id, &[*id])
                .await?
                .is_empty()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{EvidenceKey, disclosed_sources, sources_still_visible};
    use synveda_types::KnowledgeRevisionId;

    #[test]
    fn proves_only_disclosed_sources_and_refuses_missing_or_ambiguous_entries() {
        let id = KnowledgeRevisionId::new();
        let rendered = format!(
            "- {{\"kind\":\"published_knowledge\",\"knowledge_revision_id\":\"{id}\",\"sources\":[\"manual:public\"]}}\n"
        );
        let disclosed = disclosed_sources(&rendered).unwrap();
        assert!(sources_still_visible(
            &disclosed,
            EvidenceKey::Knowledge(id),
            &["manual:public".to_owned()]
        ));
        assert!(!sources_still_visible(
            &disclosed,
            EvidenceKey::Knowledge(id),
            &[]
        ));
        assert!(!sources_still_visible(
            &disclosed,
            EvidenceKey::Knowledge(KnowledgeRevisionId::new()),
            &[]
        ));
        assert!(disclosed_sources(&rendered.repeat(2)).is_none());
    }
}
