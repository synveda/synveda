//! Knowledge admission applies the effective policy pack's redaction ladder.
#[path = "support/context_world.rs"]
mod context_world;
use context_world::*;
use synveda_types::json::canonicalise;
use synveda_types::{RedactionConfig, RedactionMode};

const EMAIL: &str = "dana.seeded@example.com";
const TOKEN: &str = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
const UUID: &str = "01a13be5-0012-7345-8bcd-0123456789ab";
const TIMESTAMP: &str = "2026-10-14T01:02:03.123456789Z";

async fn use_redaction(world: &World, name: &str, redaction: RedactionConfig) {
    world
        .install_policy(
            name,
            1,
            BLANKET,
            PackConfig {
                redaction: Some(redaction),
                ..PackConfig::default()
            },
        )
        .await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin redaction Configuration");
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, name).await;
    tx.commit().await.expect("commit redaction Configuration");
}

fn seeded_content(title: &str) -> Value {
    json!({
        "title": format!("{title} {EMAIL}"),
        "body_markdown": format!("Escalate to {EMAIL}."),
        "summary": format!("Owner {EMAIL}"),
        "tags": [format!("owner {EMAIL}"), "Runbook"],
        "sensitivity": "internal",
        "confidence_permille": 950,
        "verification_metadata": {"verifier": EMAIL},
        "metadata": {
            "nested": [{"contact": EMAIL}],
            "reference": UUID,
            "observed_at": TIMESTAMP,
        },
    })
}

fn seeded_body(world: &World, content: Value) -> Value {
    json!({
        "scope_id": world.project_scope,
        "project_id": world.project_id,
        "knowledge_type": "convention",
        "origin": "authored",
        "content": content,
        "sources": [{
            "scope_id": world.project_scope,
            "source_type": "document",
            "locator": format!("mailbox/{EMAIL}/runbook"),
            "source_revision": UUID,
            "metadata": {"author": EMAIL, "import_job_id": UUID},
        }],
    })
}

async fn stored_text(world: &World) -> String {
    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant_id).await;
    let mut text = String::new();
    for table in [
        "knowledge_revisions",
        "knowledge_sources",
        "knowledge_changes",
    ] {
        let rows: Vec<String> = sqlx::query_scalar(&format!(
            "select to_jsonb(row)::text from {table} row where tenant_id = $1"
        ))
        .bind(world.tenant_id.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .expect("read stored Knowledge");
        text.push_str(&rows.join("\n"));
    }
    tx.commit().await.expect("commit storage sweep");
    text
}

async fn change_hashes_match_payloads(world: &World) {
    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant_id).await;
    let rows: Vec<(Value, String)> =
        sqlx::query_as("select payload, payload_hash from knowledge_changes where tenant_id = $1")
            .bind(world.tenant_id.as_uuid())
            .fetch_all(&mut *tx)
            .await
            .expect("read change payloads");
    tx.commit().await.expect("commit change read");
    assert!(!rows.is_empty());
    for (payload, stored) in rows {
        let bytes = serde_json::to_vec(&canonicalise(&payload)).expect("encode payload");
        assert_eq!(blake3::hash(&bytes).to_hex().to_string(), stored);
    }
}

async fn quarantine_events(world: &World) -> Vec<synveda_audit::StoredEvent> {
    audit_events(world)
        .await
        .into_iter()
        .filter(|event| event.action == "knowledge.quarantined")
        .collect()
}

#[tokio::test]
async fn seeded_pii_is_scrubbed_from_every_field_before_hashing() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (status, created) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some("admission-pii"),
        Some(seeded_body(&world, seeded_content("Escalation"))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    let item = created["knowledge_item_id"].as_str().expect("item id");

    let (status, read) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let revision = &read["current_revision"];
    assert_eq!(revision["title"], "Escalation [REDACTED:email]");
    assert_eq!(revision["body_markdown"], "Escalate to [REDACTED:email].");
    assert_eq!(revision["summary"], "Owner [REDACTED:email]");
    assert_eq!(
        revision["tags"],
        json!(["owner [redacted:email]", "runbook"]),
        "a scrubbed tag stays canonical"
    );
    assert_eq!(
        revision["verification_metadata"]["verifier"],
        "[REDACTED:email]"
    );
    assert_eq!(
        revision["metadata"]["nested"][0]["contact"],
        "[REDACTED:email]"
    );
    assert_eq!(revision["metadata"]["reference"], UUID);
    assert_eq!(revision["metadata"]["observed_at"], TIMESTAMP);

    let (status, sources) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}/sources"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sources}");
    let source = &sources["sources"][0];
    assert_eq!(source["locator"], "mailbox/[REDACTED:email]/runbook");
    assert_eq!(source["source_revision"], UUID);
    assert_eq!(source["metadata"]["author"], "[REDACTED:email]");
    assert_eq!(source["metadata"]["import_job_id"], UUID);

    assert!(!stored_text(&world).await.contains(EMAIL));
    change_hashes_match_payloads(&world).await;
    assert!(quarantine_events(&world).await.is_empty());
}

#[tokio::test]
async fn edit_and_verify_pass_through_the_same_ladder() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (item, revision) = create_knowledge(
        &world,
        "admission-edit-source",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Clean runbook",
        "No personal data here.",
        None,
    )
    .await;
    let (status, edited) = call(
        &world.app,
        "PATCH",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        Some("admission-edit"),
        Some(json!({
            "expected_revision_id": revision,
            "content": seeded_content("Edited"),
            "sources": [{"scope_id": world.project_scope, "source_type": "manual"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{edited}");
    let edited_revision = edited["revision_id"].as_str().expect("edited revision");
    let (status, verified) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{item}/verify"),
        &world.alice_token,
        Some("admission-verify"),
        Some(json!({
            "expected_revision_id": edited_revision,
            "verification_metadata": {"verifier": EMAIL, "verified_at": TIMESTAMP},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{verified}");
    let (status, read) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(
        read["current_revision"]["verification_metadata"],
        json!({"verifier": "[REDACTED:email]", "verified_at": TIMESTAMP})
    );
    assert!(!stored_text(&world).await.contains(EMAIL));
    change_hashes_match_payloads(&world).await;
}

#[tokio::test]
async fn a_credential_is_refused_with_rule_ids_and_chained_under_quarantine() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let mut content = seeded_content("Credential");
    content["body_markdown"] = json!(format!("Authenticate with {TOKEN}."));
    let (status, refused) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some("admission-secret"),
        Some(seeded_body(&world, content)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let text = refused.to_string();
    assert!(text.contains("redaction scanner"), "{text}");
    assert!(text.contains("quarantine"), "{text}");
    assert!(!text.contains(TOKEN), "{text}");
    assert!(!text.contains(EMAIL), "{text}");

    let stored = stored_text(&world).await;
    assert!(!stored.contains(TOKEN));
    assert!(!stored.contains(EMAIL));

    let events = quarantine_events(&world).await;
    assert_eq!(events.len(), 1, "{events:?}");
    let payload = events[0].payload.to_string();
    assert!(
        payload.contains("\"disposition\":\"quarantine\""),
        "{payload}"
    );
    assert!(payload.contains("\"command\":\"create\""), "{payload}");
    assert!(!payload.contains(TOKEN), "{payload}");
    assert!(!payload.contains(EMAIL), "{payload}");
}

#[tokio::test]
async fn the_refusal_comes_after_the_policy_decision() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let mut content = seeded_content("Unauthorised");
    content["body_markdown"] = json!(format!("Authenticate with {TOKEN}."));
    let (status, refused) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.mallory_token,
        Some("admission-outsider"),
        Some(seeded_body(&world, content)),
    )
    .await;
    assert_ne!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_ne!(status, StatusCode::CREATED, "{refused}");
    assert!(
        !refused.to_string().contains("redaction scanner"),
        "{refused}"
    );
    assert!(quarantine_events(&world).await.is_empty());
}

#[tokio::test]
async fn the_effective_pack_decides_each_rung() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let refuse = |key: &'static str, mode: &'static str| {
        let body = seeded_body(&world, seeded_content("Laddered"));
        let world = &world;
        async move {
            let (status, refused) = call(
                &world.app,
                "POST",
                "/v1/knowledge",
                &world.alice_token,
                Some(key),
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
            let text = refused.to_string();
            assert!(text.contains(mode), "{text}");
            assert!(text.contains("email"), "{text}");
            assert!(!text.contains(EMAIL), "{text}");
        }
    };

    use_redaction(
        &world,
        "admission-pii-quarantine",
        RedactionConfig {
            secrets: RedactionMode::Quarantine,
            pii: RedactionMode::Quarantine,
        },
    )
    .await;
    refuse("admission-rung-quarantine", "quarantine").await;
    assert_eq!(quarantine_events(&world).await.len(), 1);

    use_redaction(
        &world,
        "admission-pii-deny",
        RedactionConfig {
            secrets: RedactionMode::Deny,
            pii: RedactionMode::Deny,
        },
    )
    .await;
    refuse("admission-rung-deny", "deny").await;
    assert_eq!(
        quarantine_events(&world).await.len(),
        1,
        "deny refuses without chaining a quarantine"
    );
    assert!(!stored_text(&world).await.contains(EMAIL));

    use_redaction(
        &world,
        "admission-redact-all",
        RedactionConfig {
            secrets: RedactionMode::Redact,
            pii: RedactionMode::Redact,
        },
    )
    .await;
    let (status, created) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some("admission-rung-redact"),
        Some(seeded_body(&world, seeded_content("Scrubbed"))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(!stored_text(&world).await.contains(EMAIL));

    let mut content = seeded_content("Credential");
    content["body_markdown"] = json!(format!("Authenticate with {TOKEN}."));
    let (status, refused) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some("admission-rung-secret-floor"),
        Some(seeded_body(&world, content)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(!refused.to_string().contains(TOKEN), "{refused}");
    assert!(!stored_text(&world).await.contains(TOKEN));
    assert_eq!(quarantine_events(&world).await.len(), 2);
}

#[tokio::test]
async fn a_scrubbed_title_that_outgrows_its_bound_is_refused() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let mut content = seeded_content("Bound");
    let tail = " a@b.co";
    content["title"] = json!(format!("{}{tail}", "x".repeat(300 - tail.len())));
    let (status, refused) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some("admission-bound"),
        Some(seeded_body(&world, content)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused.to_string().contains("title"), "{refused}");
}
