//! MEM-8 private producer proof. Never returned as diagnostic trace detail.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
use synveda_retrieval::{
    ComposeRequest, ComposedBlock, CountMethod, TokenCounter, compose_authored,
};
use synveda_store::{context_dependencies, sessions};
use synveda_types::{
    CaptureCandidateId, ContextPackChunkId, EntryTier, Error, ImportArtifactId, KnowledgeItemId,
    KnowledgeRevisionId, KnowledgeSourceId, Result, ScopeId, Sensitivity, SessionEventId,
    SkillBindingId, SkillVersionId, TenantId,
};

use super::{PlannedCandidate, PlannedGraphStep, PlannedPayload};

const MAX_MANIFEST_BYTES: usize = 512 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    schema_version: u8,
    block_hash: String,
    configuration_hash: String,
    pub(super) knowledge: Vec<KnowledgeDependency>,
    pub(super) candidates: Vec<CandidateDependency>,
    pub(super) graph: Vec<PlannedGraphStep>,
    checkpoint: Option<EventDependency>,
    tail: Vec<EventDependency>,
    restart_contributed: bool,
    restart_source_event_ids: Vec<SessionEventId>,
    authored: Vec<AuthoredDependency>,
    channels: Vec<ChannelDependency>,
    skills: Vec<SkillDependency>,
    authored_budget: u32,
    sensitivity_ceiling: Option<Sensitivity>,
    counter: Counter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) handoff: Option<synveda_types::task_state::SessionHandoff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    receiver_checkout_hash: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KnowledgeDependency {
    pub(super) item_id: KnowledgeItemId,
    pub(super) revision_id: KnowledgeRevisionId,
    pub(super) content_hash: String,
    pub(super) source_ids: Vec<KnowledgeSourceId>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateDependency {
    pub(super) candidate_id: CaptureCandidateId,
    pub(super) content_hash: String,
    pub(super) source_event_ids: Vec<SessionEventId>,
    pub(super) source_artifact_ids: Vec<ImportArtifactId>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventDependency {
    event_id: SessionEventId,
    payload_hash: String,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredDependency {
    chunk_id: ContextPackChunkId,
    scope_id: ScopeId,
    document_hash: String,
    content_hash: String,
    delivered_line_hash: String,
    tier: EntryTier,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelDependency {
    scope_id: ScopeId,
    commit: String,
    pinned: bool,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillDependency {
    binding_id: SkillBindingId,
    version_id: SkillVersionId,
    scope_id: ScopeId,
    bundle_digest: String,
    object_hash: String,
    sensitivity: Sensitivity,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Counter {
    Legacy,
    O200k,
}

pub(super) struct Inputs<'a> {
    pub block_hash: &'a str,
    pub configuration_hash: &'a str,
    pub candidates: &'a [PlannedCandidate],
    pub authored: &'a ComposedBlock,
    pub checkpoint: Option<SessionEventId>,
    pub tail: &'a [SessionEventId],
    pub restart_contributed: bool,
    pub restart_source_event_ids: &'a [SessionEventId],
    pub authored_budget: u32,
    pub sensitivity_ceiling: Option<Sensitivity>,
    pub counter: &'a TokenCounter,
    pub handoff: Option<synveda_types::task_state::SessionHandoff>,
    pub receiver_checkout_hash: Option<String>,
}

impl Manifest {
    pub(super) fn has_restart_ancestry(&self) -> bool {
        self.restart_contributed
            || self.handoff.is_some()
            || self.checkpoint.is_some()
            || !self.tail.is_empty()
    }
    pub(super) async fn from_plan(
        tx: &mut PgConnection,
        tenant: TenantId,
        session: synveda_types::SessionId,
        inputs: Inputs<'_>,
    ) -> Result<Self> {
        let mut knowledge = Vec::new();
        let mut candidates = Vec::new();
        let mut graph = Vec::new();
        for candidate in inputs
            .candidates
            .iter()
            .filter(|entry| entry.selected_tokens.is_some())
        {
            match &candidate.payload {
                PlannedPayload::Knowledge(snapshot) => knowledge.push(KnowledgeDependency {
                    item_id: snapshot.item.id,
                    revision_id: snapshot.revision.id,
                    content_hash: snapshot.revision.content_hash.clone(),
                    source_ids: candidate.sources.iter().map(|source| source.id).collect(),
                }),
                PlannedPayload::Unreviewed(candidate) => candidates.push(CandidateDependency {
                    candidate_id: candidate.id,
                    content_hash: candidate.content_hash.clone(),
                    source_event_ids: candidate.source_event_ids.clone(),
                    source_artifact_ids: candidate.source_artifact_ids.clone(),
                }),
            }
            graph.extend(candidate.graph_path.iter().cloned());
        }
        let mut checkpoint = None;
        let mut tail = Vec::new();
        for (id, is_checkpoint) in inputs
            .checkpoint
            .into_iter()
            .map(|id| (id, true))
            .chain(inputs.tail.iter().map(|id| (*id, false)))
        {
            let event = sessions::event(&mut *tx, tenant, session, id)
                .await?
                .ok_or_else(source_changed)?;
            let dependency = EventDependency {
                event_id: id,
                payload_hash: event.payload_hash,
            };
            if is_checkpoint {
                checkpoint = Some(dependency);
            } else {
                tail.push(dependency);
            }
        }
        let manifest = Self {
            schema_version: 3,
            block_hash: inputs.block_hash.to_owned(),
            configuration_hash: inputs.configuration_hash.to_owned(),
            knowledge,
            candidates,
            graph,
            checkpoint,
            tail,
            restart_contributed: inputs.restart_contributed,
            restart_source_event_ids: inputs.restart_source_event_ids.to_vec(),
            authored: authored_dependencies(inputs.authored),
            channels: channel_dependencies(inputs.authored),
            skills: skill_dependencies(inputs.authored),
            authored_budget: inputs.authored_budget,
            sensitivity_ceiling: inputs.sensitivity_ceiling,
            counter: if inputs.counter.method() == CountMethod::LegacyEstimate {
                Counter::Legacy
            } else {
                Counter::O200k
            },
            handoff: inputs.handoff,
            receiver_checkout_hash: inputs.receiver_checkout_hash,
        };
        manifest.encode()?;
        Ok(manifest)
    }

    fn encode(&self) -> Result<(Value, String)> {
        if !matches!(self.schema_version, 2 | 3)
            || (self.schema_version == 2
                && (self.handoff.is_some() || self.receiver_checkout_hash.is_some()))
            || (self.handoff.is_some()
                && (!self.restart_contributed
                    || self.checkpoint.is_some()
                    || !self.tail.is_empty()))
            || self.knowledge.len() + self.candidates.len() > 1024
            || self.graph.len() > 2048
            || self.authored.len() > 1024
            || self.channels.len() > 64
            || self.skills.len() > 32
            || self.tail.len() > 16
            || self.restart_source_event_ids.len() > 256
            || self
                .knowledge
                .iter()
                .any(|entry| entry.source_ids.len() > 64)
            || self.candidates.iter().any(|entry| {
                entry.source_event_ids.len() > 64 || entry.source_artifact_ids.len() > 64
            })
        {
            return Err(limit_exceeded());
        }
        let hashes = [self.block_hash.as_str(), self.configuration_hash.as_str()]
            .into_iter()
            .chain(
                self.knowledge
                    .iter()
                    .map(|entry| entry.content_hash.as_str()),
            )
            .chain(
                self.candidates
                    .iter()
                    .map(|entry| entry.content_hash.as_str()),
            )
            .chain(self.graph.iter().flat_map(|entry| {
                [
                    entry.relation_hash.as_str(),
                    entry.from_content_hash.as_str(),
                    entry.to_content_hash.as_str(),
                ]
            }))
            .chain(
                self.checkpoint
                    .iter()
                    .chain(&self.tail)
                    .map(|entry| entry.payload_hash.as_str()),
            )
            .chain(self.authored.iter().flat_map(|entry| {
                [
                    entry.document_hash.as_str(),
                    entry.content_hash.as_str(),
                    entry.delivered_line_hash.as_str(),
                ]
            }))
            .chain(self.channels.iter().map(|entry| entry.commit.as_str()))
            .chain(
                self.skills
                    .iter()
                    .flat_map(|entry| [entry.bundle_digest.as_str(), entry.object_hash.as_str()]),
            );
        let hashes = hashes
            .chain(self.handoff.iter().map(|h| h.payload_hash.as_str()))
            .chain(self.receiver_checkout_hash.iter().map(String::as_str));
        if hashes.into_iter().any(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        }) {
            return Err(limit_exceeded());
        }
        let value = serde_json::to_value(self).map_err(|_| limit_exceeded())?;
        // All values are typed addresses, digests, integers and closed enums.
        // Pretty encoding bounds PostgreSQL's spaced JSONB encoding as well.
        if serde_json::to_string_pretty(&value)
            .map_err(|_| limit_exceeded())?
            .len()
            > MAX_MANIFEST_BYTES
        {
            return Err(limit_exceeded());
        }
        Ok((value.clone(), manifest_hash(&value)))
    }

    pub(super) async fn persist(
        &self,
        tx: &mut PgConnection,
        tenant: TenantId,
        run: synveda_types::ContextRunId,
    ) -> Result<()> {
        let (value, hash) = self.encode()?;
        context_dependencies::record(
            tx,
            tenant,
            run,
            &hash,
            &value,
            i32::from(self.schema_version),
        )
        .await
    }

    pub(super) async fn load(
        tx: &mut PgConnection,
        tenant: TenantId,
        run: &synveda_types::session::ContextRun,
    ) -> Result<Option<Self>> {
        let Some(stored) = context_dependencies::for_run(&mut *tx, tenant, run.id).await? else {
            return Ok(None);
        };
        if !matches!(stored.dependency_version, 2 | 3)
            || manifest_hash(&stored.manifest) != stored.manifest_hash
        {
            return Ok(None);
        }
        let Ok(manifest) = serde_json::from_value::<Self>(stored.manifest) else {
            return Ok(None);
        };
        if manifest.encode().is_err()
            || i32::from(manifest.schema_version) != stored.dependency_version
            || manifest.block_hash != run.block_hash
            || manifest.configuration_hash != run.configuration_hash
            || manifest.knowledge.len() + manifest.candidates.len()
                != run.selection_count.max(0) as usize
            || manifest.knowledge.len()
                + manifest.candidates.len()
                + manifest.authored.len()
                + usize::from(manifest.restart_contributed)
                != run.entry_count.max(0) as usize
            || manifest.skills.len() != run.skills.as_array().map_or(usize::MAX, Vec::len)
            || manifest.checkpoint.as_ref().map(|entry| entry.event_id) != run.checkpoint_event_id
            || manifest
                .tail
                .iter()
                .map(|entry| entry.event_id)
                .collect::<Vec<_>>()
                != run.restart_event_ids
        {
            return Ok(None);
        }
        Ok(Some(manifest))
    }

    pub(super) async fn restart_matches(
        &self,
        state: &crate::app::AppState,
        tx: &mut PgConnection,
        tenant: TenantId,
        run: &synveda_types::session::ContextRun,
    ) -> Result<bool> {
        if let Some(handoff) = &self.handoff {
            return crate::sessions::checkpoint::handoff_available(
                state,
                tx,
                tenant,
                handoff,
                self.sensitivity_ceiling,
            )
            .await;
        }
        if self.restart_contributed {
            match crate::sessions::load(
                state,
                tx,
                tenant,
                run.session_id,
                synveda_policy::Action::SessionRead,
            )
            .await
            {
                Ok(_) => {}
                Err(Error::NotFound { .. } | Error::PolicyDenied { .. }) => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        for id in &self.restart_source_event_ids {
            let Some(event) = sessions::event(&mut *tx, tenant, run.session_id, *id).await? else {
                return Ok(false);
            };
            if !synveda_store::task_state::within_ceiling(tx, tenant, *id, self.sensitivity_ceiling)
                .await?
            {
                return Ok(false);
            }
            if event.event_type == synveda_types::session::SessionEventType::TaskState
                && crate::sessions::task_state::verified(state, tx, tenant, run.session_id, &event)
                    .await?
                    .is_none()
            {
                return Ok(false);
            }
        }
        for entry in self.checkpoint.iter().chain(&self.tail) {
            if sessions::event(&mut *tx, tenant, run.session_id, entry.event_id)
                .await?
                .is_none_or(|event| event.payload_hash != entry.payload_hash)
            {
                return Ok(false);
            }
        }
        super::replay::restart_available(state, tx, tenant, run).await
    }

    pub(super) async fn authored_matches(
        &self,
        state: &crate::app::AppState,
        tx: &mut PgConnection,
        tenant: TenantId,
        run: &synveda_types::session::ContextRun,
    ) -> Result<bool> {
        let prepared = super::prepare_context_in_tx(state, tx, tenant, run.session_id).await?;
        if prepared.configuration.content_hash != self.configuration_hash
            || run.tokens < 0
            || run.tokens as u32 > prepared.plan.budget_tokens
        {
            return Ok(false);
        }
        if self.authored.is_empty() && self.skills.is_empty() {
            return Ok(true);
        }
        let counter = match self.counter {
            Counter::Legacy => TokenCounter::legacy(),
            Counter::O200k => TokenCounter::o200k(true)?,
        };
        let mut request =
            ComposeRequest::new(prepared.plan.scopes, self.authored_budget, run.as_of)
                .with_counter(counter);
        if let Some(ceiling) = self.sensitivity_ceiling {
            request = request.narrowed_to(ceiling);
        }
        let current = compose_authored(tx, tenant, &request).await?;
        Ok(self.authored == authored_dependencies(&current)
            && self.channels == channel_dependencies(&current)
            && self.skills == skill_dependencies(&current))
    }
}

fn authored_dependencies(block: &ComposedBlock) -> Vec<AuthoredDependency> {
    block
        .entries
        .iter()
        .map(|entry| AuthoredDependency {
            chunk_id: entry.chunk_id,
            scope_id: entry.scope_id,
            document_hash: entry.document_hash.clone(),
            content_hash: entry.content_hash.clone(),
            delivered_line_hash: entry.delivered_line_hash.clone(),
            tier: entry.tier,
        })
        .collect()
}
fn channel_dependencies(block: &ComposedBlock) -> Vec<ChannelDependency> {
    block
        .channels
        .iter()
        .filter(|channel| {
            block
                .entries
                .iter()
                .any(|entry| entry.scope_id == channel.scope_id)
        })
        .map(|channel| ChannelDependency {
            scope_id: channel.scope_id,
            commit: channel.commit.clone(),
            pinned: channel.pinned,
        })
        .collect()
}
fn skill_dependencies(block: &ComposedBlock) -> Vec<SkillDependency> {
    block
        .skills
        .iter()
        .map(|skill| SkillDependency {
            binding_id: skill.binding_id,
            version_id: skill.version_id,
            scope_id: skill.scope_id,
            bundle_digest: skill.bundle_digest.clone(),
            object_hash: skill.object_hash.clone(),
            sensitivity: skill.sensitivity,
        })
        .collect()
}
fn manifest_hash(value: &Value) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"synveda.context.delivery.v1\0");
    hash.update(
        synveda_types::json::canonicalise(value)
            .to_string()
            .as_bytes(),
    );
    hash.finalize().to_hex().to_string()
}
fn limit_exceeded() -> Error {
    Error::Invalid { message: "Context delivery dependency proof exceeds its supported bounds; narrow the task or budget".to_owned() }
}
fn source_changed() -> Error {
    Error::Conflict {
        message:
            "Context evidence changed during composition; request current context with a new key"
                .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty() -> Value {
        json!({"schema_version":2, "block_hash":"a".repeat(64), "configuration_hash":"b".repeat(64),
            "knowledge":[], "candidates":[], "graph":[], "checkpoint":null, "tail":[],
            "restart_contributed":false,"restart_source_event_ids":[], "authored":[], "channels":[], "skills":[],
            "authored_budget":1500, "sensitivity_ceiling":null, "counter":"legacy"})
    }

    #[test]
    fn retained_v2_proof_keeps_its_exact_encoding_and_digest() {
        let old = empty();
        let manifest: Manifest = serde_json::from_value(old.clone()).unwrap();
        let (encoded, digest) = manifest.encode().unwrap();
        assert_eq!(encoded, old);
        assert_eq!(manifest_hash(&encoded), manifest_hash(&old));
        assert_eq!(digest, manifest_hash(&old));
        assert!(encoded.get("handoff").is_none());
        assert!(encoded.get("receiver_checkout_hash").is_none());
    }

    #[test]
    fn internal_lineage_refuses_plaintext_unknown_methods_and_exhausted_bounds() {
        let base = empty();
        assert!(
            serde_json::from_value::<Manifest>(base.clone())
                .unwrap()
                .encode()
                .is_ok()
        );
        let mut value = base.clone();
        value["body_markdown"] = json!("a private statement must never enter this index");
        assert!(serde_json::from_value::<Manifest>(value).is_err());
        let mut value = base.clone();
        value["counter"] = json!("external_model");
        assert!(serde_json::from_value::<Manifest>(value).is_err());
        for (key, mutation) in [
            ("schema_version", json!(4)),
            ("block_hash", json!("plaintext is not a digest")),
            (
                "tail",
                json!(vec![
                    json!({"event_id":SessionEventId::new(), "payload_hash":"c".repeat(64)});
                    17
                ]),
            ),
        ] {
            let mut value = base.clone();
            value[key] = mutation;
            assert!(
                serde_json::from_value::<Manifest>(value)
                    .unwrap()
                    .encode()
                    .is_err()
            );
        }
        let mut reordered = serde_json::Map::new();
        for (key, value) in base.as_object().unwrap().iter().rev() {
            reordered.insert(key.clone(), value.clone());
        }
        assert_eq!(
            manifest_hash(&base),
            manifest_hash(&Value::Object(reordered))
        );
    }
}
