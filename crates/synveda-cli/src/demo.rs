//! Resumable PulseBoard product walkthrough (CPR-41, ADR-0100).
//!
//! This is deliberately an ordinary authenticated gateway client. It does not
//! open the store, provision identities, mint test tokens or run another
//! gateway. The private local receipt contains only responses from those
//! public APIs and fixed synthetic demo content; one-time invitation material
//! is stripped before persistence. Every product row is created through the
//! same HTTP/PDP/VedaFlow/audit path as the console and adapters.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::Utc;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::Api;
use crate::credentials;

const RECEIPT_VERSION: u32 = 1;
const RECEIPT_NAME: &str = "pulseboard-demo.json";
const MAX_CAPTURE_POLLS: usize = 120;

/// The three canonical product profiles. These are copied Configuration
/// documents, not runtime/deployment branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum DemoProfile {
    Personal,
    Team,
    Governed,
}

impl DemoProfile {
    const fn template(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Team => "team",
            Self::Governed => "enterprise",
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Team => "team",
            Self::Governed => "governed",
        }
    }
}

/// A private local resume receipt. Values are public API responses over fixed
/// synthetic demo data, with invitation tokens and accept URLs removed before
/// persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receipt {
    receipt_version: u32,
    run_id: String,
    generation: u32,
    profile: DemoProfile,
    gateway_url: String,
    actor_subject: String,
    state: String,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    resources: BTreeMap<String, Value>,
    #[serde(default)]
    notices: Vec<String>,
}

impl Receipt {
    fn new(
        previous_generation: u32,
        profile: DemoProfile,
        gateway_url: &str,
        actor_subject: &str,
    ) -> Result<Self, String> {
        let now = Utc::now().to_rfc3339();
        Ok(Self {
            receipt_version: RECEIPT_VERSION,
            run_id: random_hex(12)?,
            generation: previous_generation.saturating_add(1),
            profile,
            gateway_url: gateway_url.to_owned(),
            actor_subject: actor_subject.to_owned(),
            state: "starting".to_owned(),
            created_at: now.clone(),
            updated_at: now,
            resources: BTreeMap::new(),
            notices: Vec::new(),
        })
    }

    fn key(&self, step: &str) -> String {
        format!("pulseboard-demo-{}-{step}", self.run_id)
    }

    fn put(&mut self, name: &str, value: Value) -> Result<(), String> {
        self.resources.insert(name.to_owned(), value);
        self.updated_at = Utc::now().to_rfc3339();
        save_receipt(self)
    }

    fn notice(&mut self, notice: impl Into<String>) -> Result<(), String> {
        let notice = notice.into();
        if !self.notices.contains(&notice) {
            self.notices.push(notice);
        }
        self.updated_at = Utc::now().to_rfc3339();
        save_receipt(self)
    }

    fn resource(&self, name: &str) -> Option<&Value> {
        self.resources.get(name)
    }

    fn require_resource(&self, name: &str) -> Result<Value, String> {
        self.resource(name)
            .cloned()
            .ok_or_else(|| format!("demo receipt has no {name} response"))
    }
}

/// Create or resume the public-API walkthrough.
pub async fn start(
    profile: DemoProfile,
    credential_profile: &str,
    explicit_bob_profile: Option<&str>,
    json_output: bool,
) -> Result<(), String> {
    let (alice, _) = Api::connect(credential_profile).await?;
    let me = alice.get("/v1/me").await?;
    if me["principal"]["quarantined"].as_bool() != Some(false) {
        return Err(
            "the acting login is not provisioned; complete `synveda login` first".to_owned(),
        );
    }

    let mut receipt = begin_or_resume(profile, &alice)?;
    if receipt.state == "active" {
        return render(&receipt, json_output);
    }
    eprintln!(
        "PulseBoard {} demo as {} · trace {}",
        profile.as_str(),
        alice.subject,
        alice.trace_id()
    );

    ensure_workspace(&alice, &mut receipt).await?;
    ensure_configuration(&alice, &mut receipt).await?;
    ensure_project(&alice, &mut receipt).await?;
    ensure_repository(&alice, &mut receipt).await?;

    let bob = if profile == DemoProfile::Team {
        prepare_team_member(&alice, &mut receipt, explicit_bob_profile).await?
    } else {
        None
    };

    ensure_first_session(&alice, &mut receipt).await?;
    ensure_first_capture(&alice, &mut receipt).await?;
    decide_first_candidates(&alice, &mut receipt).await?;
    ensure_session_closed(
        &alice,
        &mut receipt,
        "first_session",
        "first_session_closed",
        "Captured and reviewed the initial PulseBoard learnings",
    )
    .await?;
    ensure_incident(&alice, &mut receipt).await?;

    let reuse_api = bob.as_ref().unwrap_or(&alice);
    ensure_reuse_context(reuse_api, &mut receipt).await?;
    ensure_correction(reuse_api, &mut receipt).await?;
    ensure_session_closed(
        reuse_api,
        &mut receipt,
        "reuse_session",
        "reuse_session_closed",
        "Reused project Knowledge and recorded the traceparent correction",
    )
    .await?;
    ensure_current_context(reuse_api, &mut receipt).await?;
    ensure_session_closed(
        reuse_api,
        &mut receipt,
        "current_session",
        "current_session_closed",
        "Verified the current request-correlation convention",
    )
    .await?;

    ensure_skill(&alice, &mut receipt).await?;
    ensure_tool(&alice, &mut receipt).await?;
    ensure_okf(&alice, &mut receipt).await?;
    ensure_okf_export(&alice, &mut receipt).await?;

    receipt.state = "active".to_owned();
    receipt.updated_at = Utc::now().to_rfc3339();
    save_receipt(&receipt)?;
    render(&receipt, json_output)
}

/// Read the local address book and verify its main resources through public
/// API reads. Missing/denied resources are reported without guessing.
pub async fn status(credential_profile: &str, json_output: bool) -> Result<(), String> {
    let receipt = load_receipt()?.ok_or_else(|| {
        "no PulseBoard demo receipt; run `synveda demo start --profile personal`".to_owned()
    })?;
    let (api, _) = Api::connect(credential_profile).await?;
    require_same_actor(&receipt, &api)?;

    let mut live = BTreeMap::new();
    for (name, prefix) in [
        ("workspace", "/v1/workspaces/"),
        ("project", "/v1/projects/"),
        ("first_session", "/v1/sessions/"),
        ("reuse_session", "/v1/sessions/"),
        ("current_session", "/v1/sessions/"),
        ("webhook_knowledge", "/v1/knowledge/"),
        ("request_id_knowledge", "/v1/knowledge/"),
        ("replacement_knowledge", "/v1/knowledge/"),
        ("private_knowledge", "/v1/knowledge/"),
        ("incident_knowledge", "/v1/knowledge/"),
        ("reuse_context", "/v1/context-runs/"),
        ("current_context", "/v1/context-runs/"),
    ] {
        if let Some(value) = receipt.resource(name)
            && let Some(id) = object_id(value)
        {
            let result = api.get(&format!("{prefix}{id}")).await;
            live.insert(
                name.to_owned(),
                match result {
                    Ok(value) => json!({"status": "visible", "value": value}),
                    Err(error) => json!({"status": "unavailable", "reason": error}),
                },
            );
        }
    }

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"receipt": receipt, "live": live}))
                .map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    render_status(&receipt, &live)
}

/// Archive only what the receipt names. Historical evidence is deliberately
/// retained; this is not a database reset in disguise.
pub async fn reset(credential_profile: &str, force: bool) -> Result<(), String> {
    if !force {
        return Err(
            "this archives the active PulseBoard workspace and its private demo Knowledge; \
             immutable revisions, proposals and audit history remain. Re-run with:\n\n    \
             synveda demo reset --force"
                .to_owned(),
        );
    }
    let mut receipt = load_receipt()?.ok_or_else(|| "no PulseBoard demo receipt".to_owned())?;
    if receipt.state == "reset" {
        println!(
            "PulseBoard demo generation {} is already reset",
            receipt.generation
        );
        return Ok(());
    }
    let (api, _) = Api::connect(credential_profile).await?;
    require_same_actor(&receipt, &api)?;

    if let Some(invite) = receipt.resource("bob_invite")
        && let (Some(workspace), Some(invite_id)) = (
            receipt.resource("workspace").and_then(object_id),
            invite.get("invite").and_then(object_id),
        )
    {
        let listed = api
            .get(&format!("/v1/workspaces/{workspace}/invites"))
            .await?;
        if listed["invites"].as_array().is_some_and(|invites| {
            invites.iter().any(|candidate| {
                object_id(candidate) == Some(invite_id) && candidate["status"] == "pending"
            })
        }) {
            api.delete(&format!("/v1/workspaces/{workspace}/invites/{invite_id}"))
                .await?;
        }
    }

    for name in ["private_knowledge", "incident_knowledge"] {
        if let Some(value) = receipt.resource(name)
            && let Some(item) = object_id(value)
        {
            let current = api.get(&format!("/v1/knowledge/{item}")).await?;
            if current["lifecycle_state"] != "archived" && current["lifecycle_state"] != "erased" {
                let revision = required_str(&current["current_revision"], "id")?;
                api.delete_with_body(
                    &format!("/v1/knowledge/{item}"),
                    json!({
                        "mode": "archive",
                        "expected_revision_id": revision,
                        "reason": "PulseBoard demo reset"
                    }),
                )
                .await?;
            }
        }
    }

    if let Some(project) = receipt.resource("project")
        && let Some(id) = object_id(project)
    {
        let current = api.get(&format!("/v1/projects/{id}")).await?;
        if current["status"] != "archived" {
            let revision = current["revision"]
                .as_i64()
                .ok_or_else(|| format!("project {id} response has no revision"))?;
            api.patch(
                &format!("/v1/projects/{id}"),
                json!({"expected_revision": revision, "status": "archived"}),
            )
            .await?;
        }
    }
    if let Some(workspace) = receipt.resource("workspace")
        && let Some(id) = object_id(workspace)
    {
        let current = api.get(&format!("/v1/workspaces/{id}")).await?;
        if current["status"] != "archived" {
            let revision = current["revision"]
                .as_i64()
                .ok_or_else(|| format!("workspace {id} response has no revision"))?;
            api.patch(
                &format!("/v1/workspaces/{id}"),
                json!({"expected_revision": revision, "status": "archived"}),
            )
            .await?;
        }
    }

    receipt.state = "reset".to_owned();
    receipt.updated_at = Utc::now().to_rfc3339();
    save_receipt(&receipt)?;
    println!(
        "PulseBoard demo generation {} archived; immutable product and audit history remains",
        receipt.generation
    );
    Ok(())
}

fn begin_or_resume(profile: DemoProfile, api: &Api) -> Result<Receipt, String> {
    let previous = load_receipt()?;
    if let Some(receipt) = previous.as_ref()
        && receipt.state != "reset"
    {
        require_same_actor(receipt, api)?;
        if receipt.profile != profile {
            return Err(format!(
                "an active {} demo already exists; run `synveda demo reset --force` before selecting {}",
                receipt.profile.as_str(),
                profile.as_str()
            ));
        }
        return Ok(receipt.clone());
    }
    let generation = previous.as_ref().map_or(0, |value| value.generation);
    let receipt = Receipt::new(generation, profile, api.gateway(), &api.subject)?;
    save_receipt(&receipt)?;
    Ok(receipt)
}

fn require_same_actor(receipt: &Receipt, api: &Api) -> Result<(), String> {
    if receipt.gateway_url != api.gateway() || receipt.actor_subject != api.subject {
        return Err(format!(
            "the demo receipt belongs to {} at {}; this login is {} at {}",
            receipt.actor_subject,
            receipt.gateway_url,
            api.subject,
            api.gateway()
        ));
    }
    Ok(())
}

async fn ensure_workspace(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("workspace").is_some() {
        return Ok(());
    }
    let slug = if receipt.generation == 1 {
        "pulseboard-demo".to_owned()
    } else {
        format!("pulseboard-demo-{}", receipt.generation)
    };
    let value: Value = api
        .post_idempotent_as(
            "/v1/workspaces",
            Some(json!({
                "slug": slug,
                "display_name": "PulseBoard",
                "description": "Synveda's governed context-platform walkthrough"
            })),
            &receipt.key("workspace"),
        )
        .await?;
    receipt.put("workspace", value)
}

async fn ensure_configuration(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("configuration").is_none() {
        let templates = api.get("/v1/configuration-templates").await?;
        let template = templates["templates"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["name"] == receipt.profile.template())
            })
            .ok_or_else(|| {
                format!(
                    "gateway did not offer canonical {} Configuration",
                    receipt.profile.template()
                )
            })?;
        let workspace = receipt.require_resource("workspace")?;
        let scope = required_str(&workspace, "scope_id")?;
        let result: Value = api
            .post_idempotent_as(
                "/v1/configurations",
                Some(json!({
                    "governing_scope_id": scope,
                    "name": format!("PulseBoard {} profile", receipt.profile.as_str()),
                    "document": template["document"],
                    "source_template": receipt.profile.template(),
                })),
                &receipt.key("configuration"),
            )
            .await?;
        receipt.put("configuration", result)?;
    }
    let configuration = receipt.require_resource("configuration")?;
    require_applied(&configuration, "first canonical Configuration")?;

    if receipt.resource("configuration_binding").is_none() {
        let workspace = receipt.require_resource("workspace")?;
        let result: Value = api
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
        receipt.put("configuration_binding", result)?;
    }
    require_applied(
        &receipt.require_resource("configuration_binding")?,
        "first canonical Configuration binding",
    )
}

async fn ensure_project(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("project").is_some() {
        return Ok(());
    }
    let workspace = receipt.require_resource("workspace")?;
    let id = required_str(&workspace, "id")?;
    let value: Value = api
        .post_idempotent_as(
            &format!("/v1/workspaces/{id}/projects"),
            Some(json!({
                "slug": "delivery-api",
                "display_name": "Delivery API",
                "description": "Webhook delivery, request correlation and release operations"
            })),
            &receipt.key("project"),
        )
        .await?;
    receipt.put("project", value)
}

async fn ensure_repository(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("repository").is_some() {
        return Ok(());
    }
    let project = receipt.require_resource("project")?;
    let id = required_str(&project, "id")?;
    let value: Value = api
        .post_idempotent_as(
            &format!("/v1/projects/{id}/repositories"),
            Some(json!({
                "remote_uri": "https://github.com/pulseboard/delivery-api.git",
                "default_branch": "main",
                "metadata": {"demo": "pulseboard", "scenario": "CPR-41"}
            })),
            &receipt.key("repository"),
        )
        .await?;
    receipt.put("repository", value)
}

async fn prepare_team_member(
    alice: &Api,
    receipt: &mut Receipt,
    explicit_profile: Option<&str>,
) -> Result<Option<Api>, String> {
    let bob_profile = explicit_profile.map(ToOwned::to_owned).or_else(|| {
        credentials::load()
            .ok()
            .filter(|set| set.profiles.contains_key("bob"))
            .map(|_| "bob".to_owned())
    });
    if let Some(profile) = bob_profile {
        let (bob, _) = Api::connect(&profile).await?;
        if bob.gateway() != alice.gateway() {
            return Err(format!(
                "Bob profile {profile:?} points at {}, not {}",
                bob.gateway(),
                alice.gateway()
            ));
        }
        if bob.subject == alice.subject {
            return Err("Bob's credential profile resolves to Alice; a teammate must be a distinct principal".to_owned());
        }
        let me = bob.get("/v1/me").await?;
        if me["principal"]["quarantined"].as_bool() != Some(false) {
            return Err(format!("Bob profile {profile:?} is not provisioned"));
        }
        if receipt.resource("bob_member").is_none() {
            let project = receipt.require_resource("project")?;
            let id = required_str(&project, "id")?;
            let value: Value = alice
                .post_idempotent_as(
                    &format!("/v1/projects/{id}/members"),
                    Some(json!({"principal_id": bob.subject, "role": "member"})),
                    &receipt.key("bob-member"),
                )
                .await?;
            receipt.put("bob_member", value)?;
        }
        receipt.put(
            "bob_principal",
            json!({"subject": bob.subject, "credential_profile": profile}),
        )?;
        return Ok(Some(bob));
    }

    if receipt.resource("bob_invite").is_none() {
        let workspace = receipt.require_resource("workspace")?;
        let id = required_str(&workspace, "id")?;
        let created: Value = alice
            .post_idempotent_as(
                &format!("/v1/workspaces/{id}/invites"),
                Some(json!({
                    "role": "member",
                    "email": "bob@pulseboard.example",
                    "expires_in_secs": 604800
                })),
                &receipt.key("bob-invite"),
            )
            .await?;
        let token = created["token"].as_str().unwrap_or("<not returned>");
        let accept_url = created["accept_url"].as_str().unwrap_or("<not returned>");
        println!("Bob invitation (shown once): {token}");
        println!("Accept through Bob's own login: {accept_url}");
        receipt.put("bob_invite", json!({"invite": created["invite"]}))?;
    } else {
        receipt.notice(
            "Bob's one-time invitation token was already returned and is not stored; revoke/reset and issue another if it was lost",
        )?;
    }
    receipt.notice(
        "No distinct Bob credential was available; clean-session reuse runs as Alice and no teammate claim is made",
    )?;
    Ok(None)
}

async fn ensure_first_session(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("first_session").is_none() {
        let session = open_session(
            api,
            receipt,
            "first-session",
            "Capture PulseBoard conventions",
        )
        .await?;
        receipt.put("first_session", session)?;
    }
    if receipt.resource("first_events").is_none() {
        let session = receipt.require_resource("first_session")?;
        let id = required_str(&session, "id")?;
        let occurred = Utc::now().to_rfc3339();
        let statements = [
            "Webhook deliveries are deduplicated by provider event ID.",
            "Public requests currently use X-Request-Id.",
            "I prefer my local quick-test command to be just test-fast.",
            "The cafe beside the office closes at four.",
        ];
        let events = statements
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "event_type": "message.user",
                    "client_event_id": format!("{}-first-{index}", receipt.run_id),
                    "occurred_at": occurred,
                    "payload": {"text": text}
                })
            })
            .collect::<Vec<_>>();
        let value = api
            .post(
                &format!("/v1/sessions/{id}/events"),
                Some(json!({"events": events})),
            )
            .await?;
        receipt.put("first_events", value)?;
    }
    Ok(())
}

async fn ensure_first_capture(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("first_capture").is_none() {
        let session = receipt.require_resource("first_session")?;
        let id = required_str(&session, "id")?;
        let batch: Value = api
            .post_idempotent_as(
                &format!("/v1/sessions/{id}/capture-batches"),
                None,
                &receipt.key("first-capture"),
            )
            .await?;
        receipt.put("first_capture", batch)?;
    }
    let batch = poll_capture(api, &receipt.require_resource("first_capture")?).await?;
    receipt.put("first_capture", batch)?;
    if receipt.resource("first_candidates").is_none() {
        let capture = receipt.require_resource("first_capture")?;
        let id = required_str(&capture, "id")?;
        let value = api
            .get(&format!("/v1/capture-candidates?batch_id={id}&limit=100"))
            .await?;
        receipt.put("first_candidates", value)?;
    }
    Ok(())
}

async fn decide_first_candidates(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    let listing = receipt.require_resource("first_candidates")?;
    let candidates = listing["candidates"]
        .as_array()
        .ok_or_else(|| "capture candidate listing has no candidates array".to_owned())?;
    for (needle, result_name, action, body) in [
        (
            "provider event ID",
            "webhook_knowledge",
            "accept",
            json!({}),
        ),
        ("X-Request-Id", "request_id_knowledge", "accept", json!({})),
        ("test-fast", "private_knowledge", "accept", json!({})),
        (
            "cafe beside the office",
            "dismissed_candidate",
            "dismiss",
            json!({"reason": "incidental detail, not durable project Knowledge"}),
        ),
    ] {
        if receipt.resource(result_name).is_some() {
            continue;
        }
        let Some(candidate) = candidates.iter().find(|candidate| {
            candidate["content"]["body_markdown"]
                .as_str()
                .is_some_and(|text| text.contains(needle))
        }) else {
            receipt.notice(format!(
                "extractor did not produce the exact {needle:?} candidate; it remains a manual New Learnings decision"
            ))?;
            continue;
        };
        let id = required_str(candidate, "id")?;
        let result: Value = api
            .post_idempotent_as(
                &format!("/v1/capture-candidates/{id}/{action}"),
                Some(body),
                &receipt.key(result_name),
            )
            .await?;
        let stored = if action == "accept" {
            knowledge_handle(&result).unwrap_or_else(|| result.clone())
        } else {
            result
        };
        receipt.put(result_name, stored)?;
    }
    Ok(())
}

async fn ensure_incident(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("incident_knowledge").is_some() {
        return Ok(());
    }
    let project = receipt.require_resource("project")?;
    let repository = receipt.require_resource("repository")?;
    let result: Value = api
        .post_idempotent_as(
            "/v1/knowledge",
            Some(json!({
                "scope_id": required_str(&project, "scope_id")?,
                "project_id": required_str(&project, "id")?,
                "knowledge_type": "episode",
                "origin": "authored",
                "content": content(
                    "2026-08 webhook replay incident",
                    "A provider retry exposed a missing event-ID uniqueness constraint. The incident was resolved by rejecting duplicate provider event IDs before delivery.",
                    "Historical webhook retry incident and resolution",
                    &["incident", "webhooks", "reliability"],
                    940,
                ),
                "sources": [{
                    "source_type": "repository",
                    "scope_id": required_str(&project, "scope_id")?,
                    "locator": required_str(&repository, "canonical_uri")?,
                    "source_revision": "pulseboard-demo-v1",
                    "metadata": {"path": "docs/incidents/2026-08-webhook-replay.md"}
                }]
            })),
            &receipt.key("incident-knowledge"),
        )
        .await?;
    receipt.put(
        "incident_knowledge",
        knowledge_handle(&result).unwrap_or(result),
    )
}

async fn ensure_reuse_context(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("reuse_session").is_none() {
        let session = open_session(
            api,
            receipt,
            "reuse-session",
            "Reuse project Knowledge in a clean session",
        )
        .await?;
        receipt.put("reuse_session", session)?;
    }
    if receipt.resource("reuse_context").is_none() {
        let session = receipt.require_resource("reuse_session")?;
        let id = required_str(&session, "id")?;
        let run: Value = api
            .post_idempotent_as(
                &format!("/v1/sessions/{id}/context-runs"),
                Some(json!({
                    "query": "provider event ID X-Request-Id quick-test webhook reliability",
                    "budget_tokens": 700
                })),
                &receipt.key("reuse-context"),
            )
            .await?;
        if api.subject != receipt.actor_subject
            && run["rendered"]
                .as_str()
                .is_some_and(|text| text.contains("test-fast"))
        {
            return Err("Bob's context leaked Alice's private quick-test preference".to_owned());
        }
        receipt.put("reuse_context", run)?;
    }
    Ok(())
}

async fn ensure_correction(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("correction_events").is_none() {
        let session = receipt.require_resource("reuse_session")?;
        let id = required_str(&session, "id")?;
        let value = api
            .post(
                &format!("/v1/sessions/{id}/events"),
                Some(json!({"events": [{
                    "event_type": "message.user",
                    "client_event_id": format!("{}-correction", receipt.run_id),
                    "occurred_at": Utc::now().to_rfc3339(),
                    "payload": {"text": "We decided traceparent replaces X-Request-Id for public requests."}
                }]})),
            )
            .await?;
        receipt.put("correction_events", value)?;
    }
    if receipt.resource("correction_capture").is_none() {
        let session = receipt.require_resource("reuse_session")?;
        let id = required_str(&session, "id")?;
        let batch: Value = api
            .post_idempotent_as(
                &format!("/v1/sessions/{id}/capture-batches"),
                None,
                &receipt.key("correction-capture"),
            )
            .await?;
        receipt.put("correction_capture", batch)?;
    }
    let batch = poll_capture(api, &receipt.require_resource("correction_capture")?).await?;
    receipt.put("correction_capture", batch)?;
    if receipt.resource("correction_candidates").is_none() {
        let capture = receipt.require_resource("correction_capture")?;
        let id = required_str(&capture, "id")?;
        let listing = api
            .get(&format!("/v1/capture-candidates?batch_id={id}&limit=100"))
            .await?;
        receipt.put("correction_candidates", listing)?;
    }
    if receipt.resource("replacement_knowledge").is_some() {
        return Ok(());
    }
    let Some(old) = receipt.resource("request_id_knowledge") else {
        receipt.notice("The original request-ID candidate is awaiting manual review, so supersession remains a New Learnings action")?;
        return Ok(());
    };
    let (Some(old_id), Some(old_revision)) = (
        object_id(old).map(ToOwned::to_owned),
        old.get("revision_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    ) else {
        receipt.notice("The original request-ID change is pending review; correction cannot supersede an item that is not published")?;
        return Ok(());
    };
    let candidates = receipt.require_resource("correction_candidates")?;
    let Some(candidate) = candidates["candidates"].as_array().and_then(|entries| {
        entries.iter().find(|candidate| {
            candidate["content"]["body_markdown"]
                .as_str()
                .is_some_and(|text| text.contains("traceparent"))
        })
    }) else {
        receipt.notice("Extractor did not produce the exact traceparent correction; no automatic replacement was attempted")?;
        return Ok(());
    };
    let project = receipt.require_resource("project")?;
    let candidate_id = required_str(candidate, "id")?;
    let result: Value = api
        .post_idempotent_as(
            &format!("/v1/capture-candidates/{candidate_id}/replace"),
            Some(json!({
                "item_id": old_id,
                "expected_revision_id": old_revision,
                "replacement": {
                    "scope_id": required_str(&project, "scope_id")?,
                    "project_id": required_str(&project, "id")?,
                    "knowledge_type": "convention",
                    "content": content(
                        "Current request correlation convention",
                        "PulseBoard public requests use the W3C traceparent header.",
                        "traceparent is the current public-request correlation header",
                        &["http", "observability", "convention"],
                        980,
                    )
                }
            })),
            &receipt.key("replace-request-id"),
        )
        .await?;
    receipt.put(
        "replacement_knowledge",
        knowledge_handle(&result).unwrap_or(result),
    )
}

async fn ensure_current_context(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("current_session").is_none() {
        let session = open_session(
            api,
            receipt,
            "current-session",
            "Use the current request-correlation convention",
        )
        .await?;
        receipt.put("current_session", session)?;
    }
    if receipt.resource("current_context").is_none() {
        let session = receipt.require_resource("current_session")?;
        let id = required_str(&session, "id")?;
        let run: Value = api
            .post_idempotent_as(
                &format!("/v1/sessions/{id}/context-runs"),
                Some(json!({"query": "traceparent X-Request-Id request correlation", "budget_tokens": 512})),
                &receipt.key("current-context"),
            )
            .await?;
        if receipt.resource("replacement_knowledge").is_some()
            && run["rendered"]
                .as_str()
                .is_some_and(|text| text.contains("X-Request-Id"))
        {
            return Err(
                "the clean current session selected the superseded X-Request-Id convention"
                    .to_owned(),
            );
        }
        receipt.put("current_context", run)?;
    }
    Ok(())
}

async fn ensure_skill(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("release_skill").is_none() {
        let project = receipt.require_resource("project")?;
        let result: Value = api
            .post_idempotent_as(
                "/v1/skills",
                Some(json!({
                    "governing_scope_id": required_str(&project, "scope_id")?,
                    "name": "pulseboard-release",
                    "sensitivity": "internal",
                    "files": [{
                        "path": "SKILL.md",
                        "content": "---\nname: pulseboard-release\ndescription: Prepare and verify a PulseBoard release from an exact project revision.\nlicense: Apache-2.0\ncompatibility: Requires git and cargo.\nmetadata:\n  version: 1.0.0\nallowed-tools: Read Bash(git diff *) Bash(cargo test *)\n---\n\n# PulseBoard release\n\n1. Inspect the exact diff.\n2. Run the governed release checks.\n3. Report the revision and evidence.\n\nDeclared tools are metadata, never authorisation.\n"
                    }],
                    "provenance": {
                        "kind": "authored",
                        "reference": "pulseboard-demo/release-skill",
                        "revision": "1.0.0",
                        "metadata": {"demo": "CPR-41"}
                    }
                })),
                &receipt.key("release-skill"),
            )
            .await?;
        receipt.put("release_skill", result)?;
    }
    let skill = receipt.require_resource("release_skill")?;
    if skill["outcome"] == "applied" && receipt.resource("release_skill_binding").is_none() {
        let project = receipt.require_resource("project")?;
        let result: Value = api
            .post_idempotent_as(
                "/v1/skill-bindings",
                Some(json!({
                    "scope_id": required_str(&project, "scope_id")?,
                    "skill_id": required_str(&skill, "skill_id")?,
                    "pinned_version_id": required_str(&skill, "version_id")?,
                    "enabled": true
                })),
                &receipt.key("release-skill-binding"),
            )
            .await?;
        receipt.put("release_skill_binding", result)?;
    } else if skill["outcome"] == "pending_review" {
        receipt.notice("Release Skill installation is in Advanced > Reviews; no unreviewed version was advertised or pinned")?;
    }
    Ok(())
}

async fn ensure_tool(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("tool_server").is_none() {
        let project = receipt.require_resource("project")?;
        let result: Value = api
            .post_idempotent_as(
                "/v1/tool-servers",
                Some(json!({
                    "governing_scope_id": required_str(&project, "scope_id")?,
                    "name": "pulseboard-issues",
                    "descriptor": {
                        "source_kind": "manifest",
                        "source_reference": "pulseboard-demo/mcp-server.json",
                        "transport": "streamable_http",
                        "endpoint": "https://mcp.pulseboard.example/mcp",
                        "authentication": "none",
                        "requested_permissions": ["issues:read"],
                        "metadata": {"demo": "CPR-41"}
                    },
                    "capabilities": {
                        "protocol_version": "2026-07-28",
                        "server_info": {"name": "pulseboard", "version": "1.0.0"},
                        "tools": [{
                            "name": "lookup_issue",
                            "description": "Read one PulseBoard issue",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"issue_id": {"type": "string"}},
                                "required": ["issue_id"]
                            }
                        }],
                        "resources": [{"uri": "repo://pulseboard/runbooks", "name": "runbooks"}],
                        "prompts": [{"name": "triage", "description": "Triage an incident", "arguments": []}],
                        "extensions": {"fixture": "CPR-41"}
                    }
                })),
                &receipt.key("tool-server"),
            )
            .await?;
        receipt.put("tool_server", result)?;
    }
    let server = receipt.require_resource("tool_server")?;
    if server["outcome"] == "applied" && receipt.resource("tool_binding").is_none() {
        let project = receipt.require_resource("project")?;
        let result: Value = api
            .post_idempotent_as(
                "/v1/tool-bindings",
                Some(json!({
                    "project_id": required_str(&project, "id")?,
                    "server_id": required_str(&server, "server_id")?,
                    "version_id": required_str(&server, "version_id")?,
                    "state": "enabled"
                })),
                &receipt.key("tool-binding"),
            )
            .await?;
        receipt.put("tool_binding", result)?;
    } else if server["outcome"] == "pending_review" {
        receipt.notice("MCP server version is quarantined in Advanced > Reviews; it was not silently bound to the project")?;
    }
    Ok(())
}

async fn ensure_okf(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("okf_import").is_none() {
        let project = receipt.require_resource("project")?;
        let markdown = "---\ntype: troubleshooting\ntitle: Recover a webhook delivery backlog\nsummary: Drain by provider event ID without replaying acknowledged deliveries.\ntags:\n  - webhooks\n  - troubleshooting\nx-pulseboard-owner: delivery-api\n---\n\nPause new delivery claims, preserve provider event IDs, and replay only entries without an acknowledgement.\n";
        let request = json!({
            "source_kind": "directory",
            "source_locator": "pulseboard-troubleshooting-okf-v0.2",
            "encoding": "entries",
            "entries": [{
                "logical_path": "knowledge/webhook-backlog.md",
                "kind": "file",
                "content_base64": STANDARD.encode(markdown.as_bytes())
            }]
        });
        let result: Value = api
            .post_idempotent_as(
                &format!("/v1/projects/{}/okf/imports", required_str(&project, "id")?),
                Some(request),
                &receipt.key("okf-import"),
            )
            .await?;
        receipt.put("okf_import", result)?;
    }
    if receipt.resource("okf_capture").is_none() {
        let import = receipt.require_resource("okf_import")?;
        let job = required_str(&import["job"], "id")?;
        let result: Value = api
            .post_idempotent_as(
                &format!("/v1/okf/imports/{job}/materialize"),
                None,
                &receipt.key("okf-materialize"),
            )
            .await?;
        receipt.put("okf_capture", result)?;
    }
    Ok(())
}

async fn ensure_okf_export(api: &Api, receipt: &mut Receipt) -> Result<(), String> {
    if receipt.resource("okf_export").is_some() {
        return Ok(());
    }
    let project = receipt.require_resource("project")?;
    let result: Value = api
        .post_idempotent_as(
            &format!("/v1/projects/{}/okf/exports", required_str(&project, "id")?),
            Some(json!({"item_ids": []})),
            &receipt.key("okf-export"),
        )
        .await?;
    let summary = json!({
        "format": result["format"],
        "version": result["version"],
        "content_hash": result["content_hash"],
        "file_count": result["files"].as_array().map_or(0, Vec::len),
    });
    receipt.put("okf_export", summary)
}

async fn open_session(
    api: &Api,
    receipt: &Receipt,
    step: &str,
    task: &str,
) -> Result<Value, String> {
    let workspace = receipt.require_resource("workspace")?;
    let project = receipt.require_resource("project")?;
    let repository = receipt.require_resource("repository")?;
    api.post_idempotent_as(
        "/v1/sessions",
        Some(json!({
            "workspace_id": required_str(&workspace, "id")?,
            "project_id": required_str(&project, "id")?,
            "repository_id": required_str(&repository, "id")?,
            "client_name": "synveda-demo",
            "client_version": env!("CARGO_PKG_VERSION"),
            "external_session_id": format!("{}-{step}", receipt.run_id),
            "agent_name": "PulseBoard walkthrough",
            "model_name": "demo-client",
            "branch": "main",
            "task_summary": task,
            "metadata": {"demo": "pulseboard", "profile": receipt.profile.as_str()}
        })),
        &receipt.key(step),
    )
    .await
}

/// Close a demo run through the same forward-only two-phase lifecycle used by
/// adapters. A public read makes receipt publication loss resumable without
/// trying to transition an already-closed session again.
async fn ensure_session_closed(
    api: &Api,
    receipt: &mut Receipt,
    session_resource: &str,
    closure_resource: &str,
    task_summary: &str,
) -> Result<(), String> {
    if receipt.resource(closure_resource).is_some() {
        return Ok(());
    }
    let session = receipt.require_resource(session_resource)?;
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
                    "task_summary": task_summary,
                    "end_reason": "PulseBoard walkthrough step completed"
                })),
            )
            .await?;
    }
    if current["status"] != "ended" {
        return Err(format!(
            "demo session {id} cannot be completed from status {:?}",
            current["status"]
        ));
    }
    receipt.put(
        closure_resource,
        json!({
            "id": id,
            "status": "ended",
            "ended_at": current["ended_at"],
        }),
    )
}

async fn poll_capture(api: &Api, batch: &Value) -> Result<Value, String> {
    let id = required_str(batch, "id")?;
    for _ in 0..MAX_CAPTURE_POLLS {
        let current = api.get(&format!("/v1/capture-batches/{id}")).await?;
        match current["state"].as_str() {
            Some("completed") => return Ok(current),
            Some("failed") => {
                return Err(format!(
                    "capture batch {id} failed ({})",
                    current["error_code"].as_str().unwrap_or("unknown")
                ));
            }
            Some("pending" | "running") => tokio::time::sleep(Duration::from_millis(500)).await,
            other => return Err(format!("capture batch {id} returned state {other:?}")),
        }
    }
    Err(format!(
        "capture batch {id} did not complete within {} seconds",
        MAX_CAPTURE_POLLS / 2
    ))
}

fn knowledge_handle(result: &Value) -> Option<Value> {
    let candidate = result.get("candidate").unwrap_or(result);
    let item_id = candidate
        .get("resulting_knowledge_item_id")
        .or_else(|| candidate.get("knowledge_item_id"))?
        .as_str()?;
    let revision_id = candidate
        .get("resulting_revision_id")
        .or_else(|| candidate.get("revision_id"))?
        .as_str()?;
    Some(json!({
        "id": item_id,
        "revision_id": revision_id,
        "change_id": candidate.get("resulting_change_id").or_else(|| candidate.get("change_id")),
        "outcome": candidate.get("resulting_outcome").or_else(|| candidate.get("outcome")),
    }))
}

fn content(title: &str, body: &str, summary: &str, tags: &[&str], confidence: u16) -> Value {
    json!({
        "title": title,
        "body_markdown": body,
        "summary": summary,
        "tags": tags,
        "sensitivity": "internal",
        "confidence_permille": confidence,
        "verification_metadata": {"method": "pulseboard-demo"},
        "metadata": {"demo": "CPR-41"}
    })
}

fn require_applied(value: &Value, what: &str) -> Result<(), String> {
    match value["outcome"].as_str() {
        Some("applied") => Ok(()),
        Some(outcome) => Err(format!(
            "{what} returned {outcome}; the walkthrough will not pretend that profile is effective"
        )),
        None => Err(format!("{what} response has no governance outcome")),
    }
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("response has no string field {field:?}: {value}"))
}

fn object_id(value: &Value) -> Option<&str> {
    value.get("id").and_then(Value::as_str)
}

fn render(receipt: &Receipt, json_output: bool) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(receipt).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    println!();
    println!(
        "PulseBoard {} demo generation {} is {}",
        receipt.profile.as_str(),
        receipt.generation,
        receipt.state
    );
    for (label, key) in [
        ("workspace", "workspace"),
        ("project", "project"),
        ("capture batch", "first_capture"),
        ("current context", "current_context"),
        ("release Skill", "release_skill"),
        ("MCP server", "tool_server"),
        ("OKF import", "okf_import"),
    ] {
        if let Some(value) = receipt.resource(key) {
            let id = object_id(value)
                .or_else(|| value.get("change_id").and_then(Value::as_str))
                .or_else(|| value.get("job").and_then(object_id))
                .unwrap_or("recorded");
            let outcome = value["outcome"]
                .as_str()
                .map_or_else(String::new, |value| format!(" · {value}"));
            println!("    {label:<16} {id}{outcome}");
        }
    }
    if let Some(context) = receipt.resource("current_context") {
        let model = context["embedding_model"].as_str().unwrap_or("unknown");
        let degradation = context["degradation_mode"].as_str().unwrap_or("none");
        println!("    retrieval       embedding={model} · degradation={degradation}");
        if model.contains("deterministic") {
            println!(
                "    semantic        unavailable: deterministic hash is labelled lexical-only"
            );
            println!(
                "                    re-run the deployment with `synveda init --embedder tei`"
            );
        }
    }
    for notice in &receipt.notices {
        println!("    notice          {notice}");
    }
    println!();
    println!(
        "Open {}/console/",
        receipt.gateway_url.trim_end_matches('/')
    );
    println!("    New Learnings   /console/new-learnings");
    println!("    Knowledge       /console/knowledge");
    println!("    Context trace   /console/context-runs");
    println!("    Skills          /console/skills");
    println!("    Tools           /console/tools");
    println!("    OKF             /console/projects/<project>/imports");
    Ok(())
}

fn render_status(receipt: &Receipt, live: &BTreeMap<String, Value>) -> Result<(), String> {
    println!(
        "PulseBoard {} generation {} · {} · {} visible public resource(s)",
        receipt.profile.as_str(),
        receipt.generation,
        receipt.state,
        live.values()
            .filter(|value| value["status"] == "visible")
            .count()
    );
    println!("    actor    {}", receipt.actor_subject);
    println!("    gateway  {}", receipt.gateway_url);
    println!("    receipt  {}", receipt_path()?.display());
    for notice in &receipt.notices {
        println!("    notice   {notice}");
    }
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

fn load_receipt() -> Result<Option<Receipt>, String> {
    let path = receipt_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let receipt: Receipt =
        serde_json::from_str(&raw).map_err(|error| format!("parse {}: {error}", path.display()))?;
    if receipt.receipt_version != RECEIPT_VERSION {
        return Err(format!(
            "{} carries receipt version {}; this build reads {RECEIPT_VERSION}",
            path.display(),
            receipt.receipt_version
        ));
    }
    Ok(Some(receipt))
}

fn save_receipt(receipt: &Receipt) -> Result<(), String> {
    let path = receipt_path()?;
    let directory = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    #[cfg(unix)]
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("restrict {}: {error}", directory.display()))?;
    let body = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("encode demo receipt: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    write_private(&temporary, &body)?;
    fs::rename(&temporary, &path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("publish {}: {error}", path.display())
    })
}

#[cfg(unix)]
fn write_private(path: &Path, body: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    file.write_all(body)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("write {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, body: &[u8]) -> Result<(), String> {
    fs::write(path, body).map_err(|error| format!("write {}: {error}", path.display()))
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut value = vec![0_u8; bytes];
    getrandom::fill(&mut value).map_err(|error| format!("system CSPRNG unavailable: {error}"))?;
    Ok(value.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_configuration_templates_not_runtime_modes() {
        assert_eq!(DemoProfile::Personal.template(), "personal");
        assert_eq!(DemoProfile::Team.template(), "team");
        assert_eq!(DemoProfile::Governed.template(), "enterprise");
    }

    #[test]
    fn receipt_never_persists_an_invitation_token_or_secret() {
        let source = include_str!("demo.rs");
        assert!(source.contains("created[\"token\"]"));
        assert!(source.contains("json!({\"invite\": created[\"invite\"]})"));
        for forbidden in [
            concat!("synveda_", "store"),
            concat!("sql", "x"),
            concat!("DATABASE", "_URL"),
            concat!("SYNVEDA_DEV_", "JWT_SECRET"),
        ] {
            assert!(
                !source.contains(forbidden),
                "demo client contains {forbidden}"
            );
        }
    }

    #[test]
    fn reset_needs_force_and_never_calls_database_reset() {
        let source = include_str!("demo.rs");
        assert!(source.contains("if !force"));
        assert!(!source.contains(concat!("reset", "::reset")));
        assert!(!source.contains(concat!("/v1/", "observe")));
        assert!(!source.contains(concat!("/v1/", "inject")));
        assert!(!source.contains(concat!("/v1/", "recall")));
    }
}
