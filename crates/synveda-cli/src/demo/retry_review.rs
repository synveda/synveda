//! Staged local ingestion-retry learning fixture (CPR-45, ADR-0100).
//!
//! This remains an ordinary authenticated API client. The seed boundary is
//! intentionally before capture and review so a presenter performs the
//! product transitions rather than watching a pre-completed story.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{Api, Origin};

use super::{knowledge_handle, object_id, poll_capture, required_str, write_private};

const FIXTURE: &str = "cpr45-retry-review-v1";
const RECEIPT_VERSION: u32 = 1;
const RECEIPT_NAME: &str = "retry-review-demo.json";
const WORKSPACE_SLUG: &str = "northstar-delivery-demo";
const PROJECT_SLUG: &str = "ingestion-api";
const SKILL_NAME: &str = "ingestion-retry-review";
const RETRY_RULE: &str = "Retried ingestion requests must reuse the original Idempotency-Key. While the original request is running, the retry returns 409; after completion, it replays the stored response without starting a second ingestion.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewState {
    Starting,
    Seeded,
    LearningPending,
    BindingPending,
    Verified,
}

impl fmt::Display for ReviewState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Starting => "starting",
            Self::Seeded => "seeded",
            Self::LearningPending => "learning_pending",
            Self::BindingPending => "binding_pending",
            Self::Verified => "verified",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IdentityReceipt {
    label: String,
    subject: String,
    credential_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReviewReceipt {
    receipt_version: u32,
    fixture: String,
    gateway_url: String,
    state: ReviewState,
    author: IdentityReceipt,
    reviewer: IdentityReceipt,
    viewer: IdentityReceipt,
    #[serde(default)]
    resources: BTreeMap<String, Value>,
}

impl ReviewReceipt {
    fn new(
        gateway_url: &str,
        author_profile: &str,
        author_subject: &str,
        reviewer_profile: &str,
        reviewer_subject: &str,
        viewer_profile: &str,
        viewer_subject: &str,
    ) -> Self {
        Self {
            receipt_version: RECEIPT_VERSION,
            fixture: FIXTURE.to_owned(),
            gateway_url: gateway_url.to_owned(),
            state: ReviewState::Starting,
            author: IdentityReceipt {
                label: "Avery Author".to_owned(),
                subject: author_subject.to_owned(),
                credential_profile: author_profile.to_owned(),
            },
            reviewer: IdentityReceipt {
                label: "Riley Reviewer".to_owned(),
                subject: reviewer_subject.to_owned(),
                credential_profile: reviewer_profile.to_owned(),
            },
            viewer: IdentityReceipt {
                label: "Vera Restricted Viewer".to_owned(),
                subject: viewer_subject.to_owned(),
                credential_profile: viewer_profile.to_owned(),
            },
            resources: BTreeMap::new(),
        }
    }

    fn key(&self, step: &str) -> String {
        format!("{FIXTURE}-{step}")
    }

    fn resource(&self, name: &str) -> Option<&Value> {
        self.resources.get(name)
    }

    fn require(&self, name: &str) -> Result<Value, String> {
        self.resource(name)
            .cloned()
            .ok_or_else(|| format!("retry-review receipt has no {name} response"))
    }

    fn record(&mut self, name: &str, value: Value) -> Result<(), String> {
        merge_resource(&mut self.resources, name, &value)?;
        self.resources.insert(name.to_owned(), value);
        save_receipt(self)
    }

    fn advance_state(&mut self, state: ReviewState) -> Result<(), String> {
        if state > self.state {
            self.state = state;
            save_receipt(self)?;
        }
        Ok(())
    }
}

/// Seed only the setup and source Session. Capture and review stay pending.
pub async fn seed(
    author_profile: &str,
    reviewer_profile: &str,
    viewer_profile: &str,
    confirm_target: &str,
    json_output: bool,
) -> Result<(), String> {
    let author = connect_person(author_profile, "author").await?;
    require_local_target(author.gateway(), confirm_target)?;
    let reviewer = connect_person_without_admission(reviewer_profile, "reviewer").await?;
    let viewer = connect_person_without_admission(viewer_profile, "restricted viewer").await?;
    require_same_gateway(&author, &reviewer, "reviewer")?;
    require_same_gateway(&author, &viewer, "restricted viewer")?;
    require_distinct_identities(&author, &reviewer, &viewer)?;

    let mut receipt = begin_or_resume(
        &author,
        author_profile,
        &reviewer,
        reviewer_profile,
        &viewer,
        viewer_profile,
    )?;
    ensure_workspace(&author, &mut receipt).await?;
    ensure_configuration(&author, &mut receipt).await?;
    ensure_project(&author, &mut receipt).await?;
    ensure_repository(&author, &mut receipt).await?;

    require_admitted(&reviewer, "reviewer").await?;
    require_admitted(&viewer, "restricted viewer").await?;
    ensure_grant(&author, &mut receipt, &reviewer.subject, "reviewer").await?;
    ensure_grant(&author, &mut receipt, &reviewer.subject, "administrator").await?;
    ensure_grant(&author, &mut receipt, &viewer.subject, "viewer").await?;
    verify_role_shape(&author, &receipt).await?;

    ensure_baseline_knowledge(&author, &mut receipt).await?;
    ensure_curator_rule(&author, &mut receipt).await?;
    ensure_skill_install(&author, &mut receipt).await?;
    ensure_source_session(&author, &mut receipt).await?;
    receipt.advance_state(ReviewState::Seeded)?;
    render_seed(&receipt, json_output)
}

/// Inspect the source Session through the public timeline and event routes.
pub async fn inspect(author_profile: &str, json_output: bool) -> Result<(), String> {
    let receipt = load_required_receipt()?;
    let author = connect_person(author_profile, "author").await?;
    require_receipt_actor(&receipt, &author)?;
    let session = receipt.require("source_session")?;
    let event = receipt.require("source_event")?;
    let session_id = required_str(&session, "id")?;
    let event_id = required_str(&event, "id")?;
    let live_session = author.get(&format!("/v1/sessions/{session_id}")).await?;
    let timeline = author
        .get(&format!("/v1/sessions/{session_id}/timeline"))
        .await?;
    let live_event = author
        .get(&format!("/v1/sessions/{session_id}/events/{event_id}"))
        .await?;
    if live_event["payload"]["text"].as_str() != Some(RETRY_RULE) {
        return Err(
            "the persisted synthetic source event no longer matches this fixture".to_owned(),
        );
    }
    let evidence = json!({
        "synthetic": true,
        "session": live_session,
        "timeline": timeline,
        "source_event": live_event,
    });
    if json_output {
        print_json(&evidence)
    } else {
        println!("Synthetic Session {session_id}");
        println!("    source event  {event_id}");
        println!("    finding       {RETRY_RULE}");
        println!("    next          capture it with `synveda demo retry-review capture ...`");
        Ok(())
    }
}

/// Freeze the Session, extract one candidate and accept it into review.
pub async fn capture(
    author_profile: &str,
    confirm_target: &str,
    json_output: bool,
) -> Result<(), String> {
    let mut receipt = load_required_receipt()?;
    let author = connect_person(author_profile, "author").await?;
    require_receipt_actor(&receipt, &author)?;
    require_local_target(author.gateway(), confirm_target)?;
    let session = receipt.require("source_session")?;
    let session_id = required_str(&session, "id")?;
    let event = receipt.require("source_event")?;
    let event_id = required_str(&event, "id")?;
    let _live_event = author
        .get(&format!("/v1/sessions/{session_id}/events/{event_id}"))
        .await?;

    let opened: Value = author
        .post_idempotent_as(
            &format!("/v1/sessions/{session_id}/capture-batches"),
            None,
            &receipt.key("capture"),
        )
        .await?;
    let batch = poll_capture(&author, &opened).await?;
    receipt.record("capture_batch", batch.clone())?;
    let batch_id = required_str(&batch, "id")?;
    let listing = author
        .get(&format!(
            "/v1/capture-candidates?batch_id={batch_id}&limit=100"
        ))
        .await?;
    receipt.record("capture_candidates", listing.clone())?;
    let candidates = listing["candidates"]
        .as_array()
        .ok_or_else(|| "capture response has no candidates array".to_owned())?;
    let mut matching = candidates.iter().filter(|candidate| {
        candidate["source_event_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(event_id)))
    });
    let candidate = matching
        .next()
        .ok_or_else(|| "deterministic capture did not produce the retry learning".to_owned())?;
    if matching.next().is_some() {
        return Err(
            "capture produced more than one candidate for the retry source event".to_owned(),
        );
    }

    let learning = if candidate["state"] == "pending" {
        let candidate_id = required_str(candidate, "id")?;
        let result: Value = author
            .post_idempotent_as(
                &format!("/v1/capture-candidates/{candidate_id}/accept"),
                Some(json!({
                    "knowledge_type": "decision",
                    "content": learning_content(),
                })),
                &receipt.key("accept-learning"),
            )
            .await?;
        knowledge_handle(&result)?
    } else {
        knowledge_handle(candidate)?
    };
    match learning["outcome"].as_str() {
        Some("pending_review" | "applied") => {}
        other => {
            return Err(format!(
                "retry learning returned governance outcome {other:?}"
            ));
        }
    }
    receipt.record("learning", learning)?;
    end_source_session(&author, &mut receipt).await?;
    receipt.advance_state(ReviewState::LearningPending)?;
    render_capture(&receipt, json_output)
}

/// Create the exact Skill binding once the prior proposals have been applied.
pub async fn bind_skill(
    author_profile: &str,
    confirm_target: &str,
    json_output: bool,
) -> Result<(), String> {
    let mut receipt = load_required_receipt()?;
    let author = connect_person(author_profile, "author").await?;
    require_receipt_actor(&receipt, &author)?;
    require_local_target(author.gateway(), confirm_target)?;

    let learning = receipt.require("learning")?;
    let knowledge_id = required_str(&learning, "id")?;
    author
        .get(&format!("/v1/knowledge/{knowledge_id}"))
        .await
        .map_err(|error| {
            format!(
                "the retry learning is not applied; review then apply proposal {} first: {error}",
                learning["change_id"].as_str().unwrap_or("unknown")
            )
        })?;
    let skill = refresh_skill_install(&author, &mut receipt).await?;
    let skill_id = required_str(&skill, "skill_id")?;
    let version_id = required_str(&skill, "version_id")?;
    author
        .get(&format!("/v1/skills/{skill_id}/versions/{version_id}"))
        .await
        .map_err(|error| {
            format!(
                "the Skill version is not applied; review then apply proposal {} first: {error}",
                skill["change_id"].as_str().unwrap_or("unknown")
            )
        })?;
    let project = receipt.require("project")?;
    let result: Value = author
        .post_idempotent_as(
            "/v1/skill-bindings",
            Some(json!({
                "scope_id": required_str(&project, "scope_id")?,
                "skill_id": skill_id,
                "pinned_version_id": version_id,
                "enabled": true,
            })),
            &receipt.key("skill-binding"),
        )
        .await?;
    match result["outcome"].as_str() {
        Some("pending_review" | "applied") => {}
        other => {
            return Err(format!(
                "Skill binding returned governance outcome {other:?}"
            ));
        }
    }
    receipt.record("skill_binding", result)?;
    receipt.advance_state(ReviewState::BindingPending)?;
    render_binding(&receipt, json_output)
}

/// Verify the applied artifacts, request authorised context, and show audit.
pub async fn verify(
    author_profile: &str,
    reviewer_profile: &str,
    confirm_target: &str,
    json_output: bool,
) -> Result<(), String> {
    let mut receipt = load_required_receipt()?;
    let author = connect_person(author_profile, "author").await?;
    require_receipt_actor(&receipt, &author)?;
    require_local_target(author.gateway(), confirm_target)?;
    let reviewer = connect_person(reviewer_profile, "reviewer").await?;
    require_same_gateway(&author, &reviewer, "reviewer")?;
    if reviewer.subject != receipt.reviewer.subject {
        return Err("the reviewer profile is not the identity recorded by the fixture".to_owned());
    }

    let learning = receipt.require("learning")?;
    let knowledge_id = required_str(&learning, "id")?;
    let knowledge = author.get(&format!("/v1/knowledge/{knowledge_id}")).await?;
    if knowledge["current_revision"]["body_markdown"].as_str() != Some(RETRY_RULE) {
        return Err(
            "the applied retry Knowledge revision does not match the reviewed learning".to_owned(),
        );
    }
    let revision_id = required_str(&knowledge["current_revision"], "id")?;
    let sources = author
        .get(&format!("/v1/knowledge/{knowledge_id}/sources"))
        .await?;
    let source_event = receipt.require("source_event")?;
    let source_event_id = required_str(&source_event, "id")?;
    if !sources["sources"].as_array().is_some_and(|entries| {
        entries.iter().any(|source| {
            source["source_type"] == "session_event"
                && source["session_event_id"].as_str() == Some(source_event_id)
        })
    }) {
        return Err("the applied retry revision lost its Session-event provenance".to_owned());
    }

    let skill = refresh_skill_install(&author, &mut receipt).await?;
    let skill_id = required_str(&skill, "skill_id")?;
    let version_id = required_str(&skill, "version_id")?;
    let skill_version = author
        .get(&format!("/v1/skills/{skill_id}/versions/{version_id}"))
        .await?;
    let binding_change = receipt.require("skill_binding")?;
    let binding_id = required_str(&binding_change, "binding_id")?;
    let binding = author
        .get(&format!("/v1/skill-bindings/{binding_id}"))
        .await?;
    if binding["enabled"] != true || binding["pinned_version_id"].as_str() != Some(version_id) {
        return Err("the reviewed Skill binding does not resolve the pinned version".to_owned());
    }
    let project = receipt.require("project")?;
    let available = reviewer
        .get(&format!(
            "/v1/skills/available?scope_id={}",
            required_str(&project, "scope_id")?
        ))
        .await?;
    if !available["skills"].as_array().is_some_and(|skills| {
        skills.iter().any(|entry| {
            entry["binding"]["id"].as_str() == Some(binding_id)
                && entry["version"]["id"].as_str() == Some(version_id)
        })
    }) {
        return Err("the reviewed Skill binding is not available at the project".to_owned());
    }

    let context_session = ensure_context_session(&reviewer, &mut receipt).await?;
    let context_session_id = required_str(&context_session, "id")?;
    let run: Value = reviewer
        .post_idempotent_as(
            &format!("/v1/sessions/{context_session_id}/context-runs"),
            Some(json!({
                "query": "retry ingestion 409 replay original Idempotency-Key",
                "budget_tokens": 768,
            })),
            &receipt.key("authorised-context"),
        )
        .await?;
    receipt.record("context_run", run.clone())?;
    let run_id = required_str(&run, "id")?;
    let context = reviewer.get(&format!("/v1/context-runs/{run_id}")).await?;
    if !context["selections"].as_array().is_some_and(|selections| {
        selections.iter().any(|selection| {
            selection["knowledge_item_id"].as_str() == Some(knowledge_id)
                && selection["knowledge_revision_id"].as_str() == Some(revision_id)
        })
    }) {
        return Err("authorised context did not select the reviewed retry revision".to_owned());
    }

    let knowledge_audit = author
        .get(&format!(
            "/v1/audit/events?artifact_family=knowledge&artifact_id={knowledge_id}&limit=100"
        ))
        .await?;
    let skill_audit = author
        .get(&format!(
            "/v1/audit/events?artifact_family=skill&artifact_id={binding_id}&limit=100"
        ))
        .await?;
    let session_audit = author
        .get(&format!(
            "/v1/audit/events?session_id={context_session_id}&limit=100"
        ))
        .await?;
    let context_audit = author
        .get(&format!(
            "/v1/audit/events?context_run_id={run_id}&limit=100"
        ))
        .await?;
    let chain = author.get("/v1/audit/verify").await?;
    for (label, page) in [
        ("Knowledge", &knowledge_audit),
        ("Skill binding", &skill_audit),
        ("Session", &session_audit),
        ("context", &context_audit),
    ] {
        if page["events"].as_array().is_none_or(Vec::is_empty) {
            return Err(format!("no {label} audit evidence was visible"));
        }
    }
    if chain["valid"].as_bool() != Some(true)
        || chain["head_seq"]
            .as_u64()
            .is_none_or(|sequence| sequence == 0)
    {
        return Err(
            "the audit chain verification did not return a valid non-empty head".to_owned(),
        );
    }
    let evidence = json!({
        "synthetic": true,
        "knowledge": knowledge,
        "provenance": sources,
        "skill_version": skill_version,
        "skill_binding": binding,
        "available_skills": available,
        "context": context,
        "audit": {
            "knowledge": knowledge_audit,
            "skill_binding": skill_audit,
            "session": session_audit,
            "context": context_audit,
            "chain": chain,
        }
    });
    receipt.record(
        "verification",
        json!({
            "knowledge_item_id": knowledge_id,
            "knowledge_revision_id": revision_id,
            "source_event_id": source_event_id,
            "skill_id": skill_id,
            "skill_version_id": version_id,
            "skill_binding_id": binding_id,
            "context_run_id": run_id,
            "audit_head_seq": evidence["audit"]["chain"]["head_seq"],
            "verified_at": Utc::now().to_rfc3339(),
        }),
    )?;
    receipt.advance_state(ReviewState::Verified)?;
    if json_output {
        print_json(&evidence)
    } else {
        println!("Retry-review fixture verified through public APIs");
        println!("    Knowledge      {knowledge_id} revision {revision_id}");
        println!("    provenance     Session event {source_event_id}");
        println!("    Skill          {skill_id} version {version_id}");
        println!("    binding        {binding_id}");
        println!("    context run    {run_id}");
        println!("    audit chain    verified");
        Ok(())
    }
}

/// Re-read the receipt's main addresses without mutating product state.
pub async fn status(author_profile: &str, json_output: bool) -> Result<(), String> {
    let receipt = load_required_receipt()?;
    let author = connect_person(author_profile, "author").await?;
    require_receipt_actor(&receipt, &author)?;
    let mut live = BTreeMap::new();
    for (name, path) in live_paths(&receipt)? {
        live.insert(
            name,
            match author.get(&path).await {
                Ok(value) => json!({"status": "visible", "value": value}),
                Err(error) => json!({"status": "unavailable", "reason": error}),
            },
        );
    }
    let output = json!({"receipt": receipt, "live": live});
    if json_output {
        print_json(&output)
    } else {
        println!("Retry-review fixture · {}", receipt.state);
        println!("    gateway  {}", receipt.gateway_url);
        println!("    receipt  {}", receipt_path()?.display());
        println!(
            "    visible  {} public resource(s)",
            live.values()
                .filter(|entry| entry["status"] == "visible")
                .count()
        );
        Ok(())
    }
}

async fn connect_person(profile: &str, label: &str) -> Result<Api, String> {
    let api = connect_person_without_admission(profile, label).await?;
    require_admitted(&api, label).await?;
    Ok(api)
}

async fn connect_person_without_admission(profile: &str, label: &str) -> Result<Api, String> {
    let (api, origin) = Api::connect(profile).await?;
    if !matches!(origin, Origin::Profile(_)) {
        return Err(format!(
            "the retry-review {label} must use a stored login profile; unset SYNVEDA_TOKEN"
        ));
    }
    Ok(api)
}

async fn require_admitted(api: &Api, label: &str) -> Result<(), String> {
    let me = api.get("/v1/me").await?;
    if me["principal"]["quarantined"].as_bool() != Some(false) {
        return Err(format!(
            "the retry-review {label} is not provisioned; complete its browser login first"
        ));
    }
    Ok(())
}

fn require_same_gateway(author: &Api, other: &Api, label: &str) -> Result<(), String> {
    if author.gateway() != other.gateway() {
        return Err(format!(
            "the {label} profile points at {}, not the author's {}",
            other.gateway(),
            author.gateway()
        ));
    }
    Ok(())
}

fn require_distinct_identities(author: &Api, reviewer: &Api, viewer: &Api) -> Result<(), String> {
    let subjects = [&author.subject, &reviewer.subject, &viewer.subject];
    if subjects.iter().collect::<BTreeSet<_>>().len() != subjects.len() {
        return Err(
            "author, reviewer and restricted viewer must be three distinct principals".to_owned(),
        );
    }
    Ok(())
}

fn require_local_target(gateway: &str, confirmation: &str) -> Result<(), String> {
    if gateway != confirmation {
        return Err(format!(
            "refusing demo mutation: --confirm-target must exactly equal the credential-bound gateway {gateway}"
        ));
    }
    let url = reqwest::Url::parse(gateway)
        .map_err(|_| "refusing demo mutation: the credential gateway is not a URL".to_owned())?;
    let local_host = matches!(
        url.host_str(),
        Some("app.synveda.test" | "localhost" | "127.0.0.1" | "::1")
    );
    if !matches!(url.scheme(), "http" | "https")
        || !local_host
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(
            "refusing demo mutation: retry-review is local-demo-only and accepts app.synveda.test or a loopback gateway"
                .to_owned(),
        );
    }
    Ok(())
}

fn begin_or_resume(
    author: &Api,
    author_profile: &str,
    reviewer: &Api,
    reviewer_profile: &str,
    viewer: &Api,
    viewer_profile: &str,
) -> Result<ReviewReceipt, String> {
    if let Some(receipt) = load_receipt()? {
        require_receipt_actor(&receipt, author)?;
        if receipt.reviewer.subject != reviewer.subject || receipt.viewer.subject != viewer.subject
        {
            return Err(
                "the retry-review receipt belongs to different reviewer/viewer principals; it will not be repointed"
                    .to_owned(),
            );
        }
        return Ok(receipt);
    }
    let receipt = ReviewReceipt::new(
        author.gateway(),
        author_profile,
        &author.subject,
        reviewer_profile,
        &reviewer.subject,
        viewer_profile,
        &viewer.subject,
    );
    save_receipt(&receipt)?;
    Ok(receipt)
}

fn require_receipt_actor(receipt: &ReviewReceipt, api: &Api) -> Result<(), String> {
    if receipt.gateway_url != api.gateway() || receipt.author.subject != api.subject {
        return Err(format!(
            "the retry-review receipt belongs to {} at {}; this login is {} at {}",
            receipt.author.subject,
            receipt.gateway_url,
            api.subject,
            api.gateway()
        ));
    }
    Ok(())
}

async fn ensure_workspace(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let value: Value = api
        .post_idempotent_as(
            "/v1/workspaces",
            Some(json!({
                "slug": WORKSPACE_SLUG,
                "display_name": "Northstar Delivery",
                "description": "Local synthetic walkthrough for governed ingestion learnings",
            })),
            &receipt.key("workspace"),
        )
        .await?;
    if value["slug"] != WORKSPACE_SLUG || value["status"] != "active" {
        return Err("the fixture workspace is not active with its expected slug".to_owned());
    }
    receipt.record("workspace", value.clone())?;
    let live = api
        .get(&format!("/v1/workspaces/{}", required_str(&value, "id")?))
        .await?;
    if live["slug"] != WORKSPACE_SLUG || live["status"] != "active" {
        return Err("the live fixture workspace was changed or archived".to_owned());
    }
    Ok(())
}

async fn ensure_configuration(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let templates = api.get("/v1/configuration-templates").await?;
    let template = templates["templates"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["name"] == "team"))
        .ok_or_else(|| "gateway did not offer the canonical team Configuration".to_owned())?;
    let workspace = receipt.require("workspace")?;
    let configuration: Value = api
        .post_idempotent_as(
            "/v1/configurations",
            Some(json!({
                "governing_scope_id": required_str(&workspace, "scope_id")?,
                "name": "Northstar local demo profile",
                "document": template["document"],
                "source_template": "team",
            })),
            &receipt.key("configuration"),
        )
        .await?;
    require_applied(&configuration, "canonical team Configuration")?;
    receipt.record("configuration", configuration.clone())?;
    let binding: Value = api
        .post_idempotent_as(
            "/v1/configuration-bindings",
            Some(json!({
                "scope_id": required_str(&workspace, "scope_id")?,
                "artifact_id": required_str(&configuration, "artifact_id")?,
                "pinned_version_id": required_str(&configuration, "version_id")?,
                "enabled": true,
            })),
            &receipt.key("configuration-binding"),
        )
        .await?;
    require_applied(&binding, "canonical team Configuration binding")?;
    receipt.record("configuration_binding", binding)
}

async fn ensure_project(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let workspace = receipt.require("workspace")?;
    let workspace_id = required_str(&workspace, "id")?;
    let value: Value = api
        .post_idempotent_as(
            &format!("/v1/workspaces/{workspace_id}/projects"),
            Some(json!({
                "slug": PROJECT_SLUG,
                "display_name": "Ingestion API",
                "description": "Northstar's request ingestion service",
            })),
            &receipt.key("project"),
        )
        .await?;
    if value["slug"] != PROJECT_SLUG || value["workspace_id"].as_str() != Some(workspace_id) {
        return Err("the fixture project does not match its expected workspace".to_owned());
    }
    receipt.record("project", value.clone())?;
    let live = api
        .get(&format!("/v1/projects/{}", required_str(&value, "id")?))
        .await?;
    if live["slug"] != PROJECT_SLUG || live["status"] != "active" {
        return Err("the live fixture project was changed or archived".to_owned());
    }
    Ok(())
}

async fn ensure_repository(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let project = receipt.require("project")?;
    let project_id = required_str(&project, "id")?;
    let value: Value = api
        .post_idempotent_as(
            &format!("/v1/projects/{project_id}/repositories"),
            Some(json!({
                "remote_uri": "https://github.com/northstar-demo/ingestion-api.git",
                "default_branch": "main",
                "metadata": {"fixture": FIXTURE, "synthetic": true},
            })),
            &receipt.key("repository"),
        )
        .await?;
    if value["canonical_uri"] != "https://github.com/northstar-demo/ingestion-api" {
        return Err("the fixture repository canonical URI changed".to_owned());
    }
    receipt.record("repository", value)
}

async fn ensure_grant(
    api: &Api,
    receipt: &mut ReviewReceipt,
    subject: &str,
    role: &str,
) -> Result<(), String> {
    let project = receipt.require("project")?;
    let project_id = required_str(&project, "id")?;
    let value: Value = api
        .post_idempotent_as(
            &format!("/v1/projects/{project_id}/members"),
            Some(json!({"principal_id": subject, "role": role})),
            &receipt.key(&format!("grant-{role}")),
        )
        .await?;
    if value["principal_id"].as_str() != Some(subject) || value["role"].as_str() != Some(role) {
        return Err(format!(
            "the {role} grant response names another principal or role"
        ));
    }
    receipt.record(&format!("grant_{role}"), value)
}

async fn verify_role_shape(api: &Api, receipt: &ReviewReceipt) -> Result<(), String> {
    let project = receipt.require("project")?;
    let project_id = required_str(&project, "id")?;
    let listing = api
        .get(&format!("/v1/projects/{project_id}/members"))
        .await?;
    let members = listing["members"]
        .as_array()
        .ok_or_else(|| "project membership response has no members".to_owned())?;
    let roles_for = |subject: &str| {
        members
            .iter()
            .filter(|member| member["principal_id"].as_str() == Some(subject))
            .filter_map(|member| member["role"].as_str())
            .collect::<BTreeSet<_>>()
    };
    if !roles_for(&receipt.author.subject).contains("administrator") {
        return Err(
            "Avery Author does not inherit the existing administrator grant at this project"
                .to_owned(),
        );
    }
    let reviewer_roles = roles_for(&receipt.reviewer.subject);
    if !reviewer_roles.contains("reviewer") || !reviewer_roles.contains("administrator") {
        return Err(
            "Riley Reviewer does not hold the existing reviewer and administrator grants"
                .to_owned(),
        );
    }
    let viewer_roles = roles_for(&receipt.viewer.subject);
    if viewer_roles != BTreeSet::from(["viewer"]) {
        return Err(format!(
            "Vera Restricted Viewer must hold only the viewer grant at this project, found {viewer_roles:?}"
        ));
    }
    Ok(())
}

async fn ensure_baseline_knowledge(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let project = receipt.require("project")?;
    let repository = receipt.require("repository")?;
    let value: Value = api
        .post_idempotent_as(
            "/v1/knowledge",
            Some(json!({
                "scope_id": required_str(&project, "scope_id")?,
                "project_id": required_str(&project, "id")?,
                "knowledge_type": "convention",
                "origin": "authored",
                "content": {
                    "title": "Ingestion idempotency baseline",
                    "body_markdown": "Every ingestion request carries an Idempotency-Key scoped to its project. The service stores the key and canonical request hash before work begins.",
                    "summary": "Ingestion requests establish their idempotency record before work.",
                    "tags": ["ingestion", "idempotency", "reliability"],
                    "sensitivity": "internal",
                    "confidence_permille": 980,
                    "verification_metadata": {"method": "reviewed repository baseline"},
                    "metadata": {"fixture": FIXTURE, "synthetic": true},
                },
                "sources": [{
                    "source_type": "repository",
                    "scope_id": required_str(&project, "scope_id")?,
                    "locator": required_str(&repository, "canonical_uri")?,
                    "source_revision": "northstar-demo-baseline-v1",
                    "metadata": {"path": "docs/ingestion/idempotency.md", "fixture": FIXTURE},
                }],
            })),
            &receipt.key("baseline-knowledge"),
        )
        .await?;
    require_applied(&value, "approved baseline Knowledge")?;
    let item_id = required_str(&value, "knowledge_item_id")?;
    let revision_id = required_str(&value, "revision_id")?;
    let live = api.get(&format!("/v1/knowledge/{item_id}")).await?;
    if live["current_revision"]["id"].as_str() != Some(revision_id) {
        return Err(
            "the approved baseline Knowledge head no longer matches the fixture".to_owned(),
        );
    }
    let sources = api.get(&format!("/v1/knowledge/{item_id}/sources")).await?;
    if !sources["sources"].as_array().is_some_and(|entries| {
        entries.iter().any(|source| {
            source["source_type"] == "repository"
                && source["source_revision"] == "northstar-demo-baseline-v1"
        })
    }) {
        return Err("the approved baseline Knowledge has no repository provenance".to_owned());
    }
    receipt.record("baseline_knowledge", value)?;
    receipt.record("baseline_provenance", sources)
}

async fn ensure_curator_rule(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let project = receipt.require("project")?;
    let scope_id = required_str(&project, "scope_id")?;
    let expected = format!("knowledge/* @{}\n", receipt.reviewer.subject);
    let path = format!("/v1/admin/scopes/{scope_id}/curators");
    let before = api.get(&path).await?;
    match before["source"].as_str() {
        None => {
            let committed = api
                .put(
                    &path,
                    json!({
                        "source": expected,
                        "message": "CPR-45 local demo: require Riley Reviewer for Knowledge changes",
                    }),
                )
                .await?;
            receipt.record("curator_commit", committed)?;
        }
        Some(source) if source == expected && before["effective_at"].as_str() == Some(scope_id) => {
        }
        Some(_) => {
            return Err(
                "the project already has a different effective curator file; retry-review will not overwrite it"
                    .to_owned(),
            );
        }
    }
    let current = api.get(&path).await?;
    if current["source"].as_str() != Some(expected.as_str())
        || current["effective_at"].as_str() != Some(scope_id)
    {
        return Err(
            "the exact retry-review curator rule is not effective at the project".to_owned(),
        );
    }
    receipt.record("curator_rule", current)
}

async fn ensure_skill_install(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let project = receipt.require("project")?;
    let repository = receipt.require("repository")?;
    let reference = format!(
        "{}#skills/{SKILL_NAME}",
        required_str(&repository, "canonical_uri")?
    );
    let value: Value = api
        .post_idempotent_as(
            "/v1/skills",
            Some(json!({
                "governing_scope_id": required_str(&project, "scope_id")?,
                "name": SKILL_NAME,
                "sensitivity": "internal",
                "files": [{
                    "path": "SKILL.md",
                    "content": "---\nname: ingestion-retry-review\ndescription: Verify ingestion retry behaviour against an exact idempotency contract. Use when changing request admission or replay handling.\nlicense: Apache-2.0\ncompatibility: Requires an HTTP client that can set Idempotency-Key.\nmetadata:\n  version: 1.0.0\n---\n\n# Ingestion retry review\n\n## Steps\n\n1. Send an ingestion request with a stable Idempotency-Key.\n2. Retry while the original is running and require a conflict response.\n3. Retry after completion and require the stored response without a second ingestion.\n4. Report the exact request hash and response evidence; do not log payload content.\n",
                }],
                "provenance": {
                    "kind": "authored",
                    "reference": reference,
                    "revision": "northstar-demo-skill-v1",
                    "metadata": {"fixture": FIXTURE, "synthetic": true},
                },
            })),
            &receipt.key("skill-install"),
        )
        .await?;
    match value["outcome"].as_str() {
        Some("pending_review" | "applied") => {}
        other => {
            return Err(format!(
                "Skill install returned governance outcome {other:?}"
            ));
        }
    }
    for field in ["change_id", "skill_id", "version_id"] {
        required_str(&value, field)?;
    }
    receipt.record("skill_install", value)
}

async fn refresh_skill_install(api: &Api, receipt: &mut ReviewReceipt) -> Result<Value, String> {
    ensure_skill_install(api, receipt).await?;
    receipt.require("skill_install")
}

async fn ensure_source_session(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let workspace = receipt.require("workspace")?;
    let project = receipt.require("project")?;
    let repository = receipt.require("repository")?;
    let session: Value = api
        .post_idempotent_as(
            "/v1/sessions",
            Some(json!({
                "workspace_id": required_str(&workspace, "id")?,
                "project_id": required_str(&project, "id")?,
                "repository_id": required_str(&repository, "id")?,
                "client_name": "synveda-demo",
                "client_version": env!("CARGO_PKG_VERSION"),
                "external_session_id": FIXTURE,
                "agent_name": "Synthetic Northstar delivery assistant",
                "model_name": "deterministic-manual-capture",
                "branch": "main",
                "task_summary": "Synthetic replay: determine ingestion retry behaviour",
                "metadata": {"fixture": FIXTURE, "synthetic": true, "replay": true},
            })),
            &receipt.key("source-session"),
        )
        .await?;
    receipt.record("source_session", session.clone())?;
    let session_id = required_str(&session, "id")?;
    let client_event_id = format!("{FIXTURE}-finding");
    let event = match receipt.resource("source_event") {
        Some(recorded) => {
            let event_id = required_str(recorded, "id")?;
            api.get(&format!("/v1/sessions/{session_id}/events/{event_id}"))
                .await?
        }
        None => {
            let response = api
                .post(
                    &format!("/v1/sessions/{session_id}/events"),
                    Some(json!({"events": [{
                        "event_type": "message.assistant",
                        "client_event_id": client_event_id.clone(),
                        "occurred_at": "2026-09-10T09:00:00Z",
                        "payload": {"text": RETRY_RULE, "synthetic": true, "replay": true},
                    }]})),
                )
                .await?;
            let entry = response["events"]
                .as_array()
                .and_then(|entries| entries.first())
                .ok_or_else(|| "source Session append returned no event".to_owned())?;
            if !matches!(entry["outcome"].as_str(), Some("appended" | "duplicate")) {
                return Err(format!(
                    "source Session event returned outcome {:?}",
                    entry["outcome"]
                ));
            }
            entry
                .get("event")
                .filter(|value| value.is_object())
                .cloned()
                .ok_or_else(|| "source Session append returned no persisted event".to_owned())?
        }
    };
    if event["session_id"].as_str() != Some(session_id)
        || event["client_event_id"].as_str() != Some(client_event_id.as_str())
        || event["payload"]["text"].as_str() != Some(RETRY_RULE)
        || event["payload"]["synthetic"].as_bool() != Some(true)
        || event["payload"]["replay"].as_bool() != Some(true)
    {
        return Err("source Session replay resolved to different event content".to_owned());
    }
    receipt.record("source_event", event)
}

async fn end_source_session(api: &Api, receipt: &mut ReviewReceipt) -> Result<(), String> {
    let session = receipt.require("source_session")?;
    let id = required_str(&session, "id")?;
    let mut current = api.get(&format!("/v1/sessions/{id}")).await?;
    if current["status"] == "active" {
        current = api
            .post(
                &format!("/v1/sessions/{id}/end"),
                Some(json!({"status": "ending"})),
            )
            .await?;
    }
    if current["status"] == "ending" {
        current = api
            .post(
                &format!("/v1/sessions/{id}/end"),
                Some(json!({
                    "status": "ended",
                    "task_summary": "Synthetic ingestion retry finding captured for human review",
                    "end_reason": "CPR-45 local demo capture completed",
                })),
            )
            .await?;
    }
    if current["status"] != "ended" {
        return Err(format!(
            "source Session {id} cannot end from {:?}",
            current["status"]
        ));
    }
    receipt.record("source_session_closed", current)
}

async fn ensure_context_session(
    reviewer: &Api,
    receipt: &mut ReviewReceipt,
) -> Result<Value, String> {
    let workspace = receipt.require("workspace")?;
    let project = receipt.require("project")?;
    let repository = receipt.require("repository")?;
    let session: Value = reviewer
        .post_idempotent_as(
            "/v1/sessions",
            Some(json!({
                "workspace_id": required_str(&workspace, "id")?,
                "project_id": required_str(&project, "id")?,
                "repository_id": required_str(&repository, "id")?,
                "client_name": "synveda-demo",
                "client_version": env!("CARGO_PKG_VERSION"),
                "external_session_id": format!("{FIXTURE}-context"),
                "agent_name": "Northstar context consumer",
                "model_name": "deterministic-context-demo",
                "branch": "main",
                "task_summary": "Request reviewed retry guidance",
                "metadata": {"fixture": FIXTURE, "synthetic": true, "replay": true},
            })),
            &receipt.key("context-session"),
        )
        .await?;
    receipt.record("context_session", session.clone())?;
    Ok(session)
}

fn learning_content() -> Value {
    json!({
        "title": "Retry behaviour for ingestion requests",
        "body_markdown": RETRY_RULE,
        "summary": "Reuse the original idempotency key; conflict while running and replay after completion.",
        "tags": ["ingestion", "idempotency", "retries"],
        "sensitivity": "internal",
        "confidence_permille": 930,
        "verification_metadata": {"method": "synthetic session finding pending human review"},
        "metadata": {"fixture": FIXTURE, "synthetic": true, "replay": true},
    })
}

fn require_applied(value: &Value, label: &str) -> Result<(), String> {
    match value["outcome"].as_str() {
        Some("applied") => Ok(()),
        Some(outcome) => Err(format!(
            "{label} returned {outcome}; use this fixture on a fresh local demo tenant so the canonical first Configuration can apply"
        )),
        None => Err(format!("{label} response has no governance outcome")),
    }
}

fn merge_resource(
    resources: &mut BTreeMap<String, Value>,
    name: &str,
    value: &Value,
) -> Result<(), String> {
    let Some(existing) = resources.get(name) else {
        return Ok(());
    };
    let old_id = stable_resource_id(existing);
    let new_id = stable_resource_id(value);
    if old_id.is_some() && new_id.is_some() && old_id != new_id {
        return Err(format!(
            "retry-review {name} resolved to a different resource; the receipt will not be repointed"
        ));
    }
    Ok(())
}

fn stable_resource_id(value: &Value) -> Option<&str> {
    object_id(value)
        .or_else(|| value.get("change_id").and_then(Value::as_str))
        .or_else(|| value.get("knowledge_item_id").and_then(Value::as_str))
}

fn live_paths(receipt: &ReviewReceipt) -> Result<Vec<(String, String)>, String> {
    let mut paths = Vec::new();
    for (name, prefix) in [
        ("workspace", "/v1/workspaces/"),
        ("project", "/v1/projects/"),
        ("source_session", "/v1/sessions/"),
        ("capture_batch", "/v1/capture-batches/"),
        ("context_session", "/v1/sessions/"),
        ("context_run", "/v1/context-runs/"),
    ] {
        if let Some(value) = receipt.resource(name)
            && let Some(id) = object_id(value)
        {
            paths.push((name.to_owned(), format!("{prefix}{id}")));
        }
    }
    for name in ["baseline_knowledge", "learning"] {
        if let Some(value) = receipt.resource(name) {
            let id = value
                .get("knowledge_item_id")
                .or_else(|| value.get("id"))
                .and_then(Value::as_str);
            if let Some(id) = id {
                paths.push((name.to_owned(), format!("/v1/knowledge/{id}")));
                if name == "learning" {
                    paths.push((
                        "learning_provenance".to_owned(),
                        format!("/v1/knowledge/{id}/sources"),
                    ));
                }
            }
        }
    }
    if let Some(value) = receipt.resource("skill_install")
        && let Some(id) = value.get("skill_id").and_then(Value::as_str)
    {
        paths.push(("skill_install".to_owned(), format!("/v1/skills/{id}")));
    }
    if let Some(value) = receipt.resource("skill_binding")
        && let Some(id) = value.get("binding_id").and_then(Value::as_str)
    {
        paths.push((
            "skill_binding".to_owned(),
            format!("/v1/skill-bindings/{id}"),
        ));
    }
    if receipt.resource("verification").is_some() {
        paths.push(("audit_chain".to_owned(), "/v1/audit/verify".to_owned()));
    }
    Ok(paths)
}

fn render_seed(receipt: &ReviewReceipt, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(receipt);
    }
    let session = receipt.require("source_session")?;
    let baseline = receipt.require("baseline_knowledge")?;
    let skill = receipt.require("skill_install")?;
    println!("Northstar retry-review fixture is {}", receipt.state);
    println!(
        "    author      {} ({})",
        receipt.author.label, receipt.author.subject
    );
    println!(
        "    reviewer    {} ({}) · reviewer + administrator grants",
        receipt.reviewer.label, receipt.reviewer.subject
    );
    println!(
        "    viewer      {} ({}) · viewer grant only",
        receipt.viewer.label, receipt.viewer.subject
    );
    println!(
        "    Session     {} · synthetic, {}",
        required_str(&session, "id")?,
        if receipt.resource("capture_batch").is_some() {
            "capture already recorded"
        } else {
            "not yet captured"
        }
    );
    println!(
        "    baseline    {} revision {} · approved with repository provenance",
        required_str(&baseline, "knowledge_item_id")?,
        required_str(&baseline, "revision_id")?
    );
    println!(
        "    Skill       proposal {} · {}",
        required_str(&skill, "change_id")?,
        required_str(&skill, "outcome")?
    );
    if receipt.resource("capture_batch").is_some() {
        println!(
            "    next        continue from the recorded {} stage",
            receipt.state
        );
    } else {
        println!("    next        inspect the Session; capture remains a presenter action");
    }
    Ok(())
}

fn render_capture(receipt: &ReviewReceipt, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(receipt);
    }
    let learning = receipt.require("learning")?;
    let skill = receipt.require("skill_install")?;
    println!("Synthetic Session captured; two explicit reviews are ready");
    println!(
        "    learning proposal  {} · {}",
        required_str(&learning, "change_id")?,
        required_str(&learning, "outcome")?
    );
    println!(
        "    Skill proposal     {} · {}",
        required_str(&skill, "change_id")?,
        required_str(&skill, "outcome")?
    );
    println!(
        "    next               inspect as Riley, try the denied viewer approval, then approve as Riley and apply as Avery"
    );
    Ok(())
}

fn render_binding(receipt: &ReviewReceipt, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(receipt);
    }
    let binding = receipt.require("skill_binding")?;
    println!("Skill binding proposal is ready");
    println!("    proposal  {}", required_str(&binding, "change_id")?);
    println!("    binding   {}", required_str(&binding, "binding_id")?);
    println!("    outcome   {}", required_str(&binding, "outcome")?);
    println!("    next      review as Riley, apply as Avery, then run verify");
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn receipt_path() -> Result<PathBuf, String> {
    let base = match std::env::var("XDG_STATE_HOME") {
        Ok(value) if value.starts_with('/') => PathBuf::from(value),
        _ => {
            let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_owned())?;
            PathBuf::from(home).join(".local").join("state")
        }
    };
    Ok(base.join("synveda").join(RECEIPT_NAME))
}

fn load_required_receipt() -> Result<ReviewReceipt, String> {
    load_receipt()?.ok_or_else(|| {
        "no retry-review fixture receipt; run `synveda demo retry-review seed` first".to_owned()
    })
}

fn load_receipt() -> Result<Option<ReviewReceipt>, String> {
    let path = receipt_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let receipt: ReviewReceipt =
        serde_json::from_str(&raw).map_err(|error| format!("parse {}: {error}", path.display()))?;
    if receipt.receipt_version != RECEIPT_VERSION || receipt.fixture != FIXTURE {
        return Err(format!(
            "{} is not a current retry-review receipt",
            path.display()
        ));
    }
    Ok(Some(receipt))
}

fn save_receipt(receipt: &ReviewReceipt) -> Result<(), String> {
    let path = receipt_path()?;
    let directory = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("restrict {}: {error}", directory.display()))?;
    }
    let body = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("encode retry-review receipt: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    write_private(&temporary, &body)?;
    fs::rename(&temporary, &path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("publish {}: {error}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_target_requires_an_exact_confirmation_and_refuses_remote_hosts() {
        assert!(
            require_local_target(
                "http://app.synveda.test:8080",
                "http://app.synveda.test:8080"
            )
            .is_ok()
        );
        assert!(require_local_target("http://127.0.0.1:8080", "http://127.0.0.1:8080").is_ok());
        assert!(
            require_local_target("http://app.synveda.test:8080", "http://127.0.0.1:8080").is_err()
        );
        let error =
            require_local_target("https://synveda.example.com", "https://synveda.example.com")
                .expect_err("a remote deployment must be refused");
        assert!(error.contains("local-demo-only"));
    }

    #[test]
    fn first_and_second_resource_observations_converge_but_repointing_is_refused() {
        let mut resources = BTreeMap::new();
        let first = json!({"id": "first", "status": "active"});
        merge_resource(&mut resources, "workspace", &first).expect("first run");
        resources.insert("workspace".to_owned(), first.clone());
        merge_resource(&mut resources, "workspace", &first).expect("second run");
        assert!(
            merge_resource(
                &mut resources,
                "workspace",
                &json!({"id": "other", "status": "active"})
            )
            .is_err()
        );
    }

    #[test]
    fn fixture_keys_and_source_event_are_stable_across_retries() {
        let receipt = ReviewReceipt::new(
            "http://app.synveda.test:8080",
            "author",
            "author-subject",
            "reviewer",
            "reviewer-subject",
            "viewer",
            "viewer-subject",
        );
        assert_eq!(receipt.key("workspace"), "cpr45-retry-review-v1-workspace");
        assert_eq!(receipt.key("workspace"), receipt.key("workspace"));
        assert!(ReviewState::Starting < ReviewState::Seeded);
        assert!(ReviewState::Seeded < ReviewState::LearningPending);
        assert!(ReviewState::LearningPending < ReviewState::BindingPending);
        assert!(ReviewState::BindingPending < ReviewState::Verified);
        assert!(RETRY_RULE.contains("original Idempotency-Key"));
        assert!(RETRY_RULE.contains("returns 409"));
        assert!(RETRY_RULE.contains("replays the stored response"));
    }

    #[test]
    fn fixture_client_has_no_store_or_token_bypass() {
        let source = include_str!("retry_review.rs");
        for forbidden in [
            concat!("synveda_", "store"),
            concat!("sql", "x"),
            concat!("DATABASE", "_URL"),
            concat!("SYNVEDA_DEV_", "JWT_SECRET"),
        ] {
            assert!(!source.contains(forbidden), "fixture contains {forbidden}");
        }
    }
}
