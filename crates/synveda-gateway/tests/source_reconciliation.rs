//! Real Git → ordinary source operations → Capture → Knowledge.
#![cfg(unix)]
#[path = "support/context_world.rs"]
mod context_world;
#[path = "support/procedure_world.rs"]
mod procedure_world;
#[path = "support/source_https.rs"]
mod source_https;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use context_world::*;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use synveda_git::LocalSourceReader;
use synveda_types::source::GitSourceDescriptor;
use synveda_types::{ImportJobId, ProjectId, RepositoryId};

fn git(directory: &Path, args: &[&str]) {
    let result = Command::new("git")
        .args([
            "-c",
            "user.name=Fictional fixture",
            "-c",
            "user.email=fixture@example.test",
        ])
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(result.status.success(), "fixture Git command failed");
}
fn retry(cap: u8) -> String {
    format!(
        "---\ntype: Procedure\ntitle: Iris retry policy\ndescription: Approved fictional retry procedure.\n---\n\nThe fictional Iris retry cap is {cap} attempts.\n"
    )
}
struct Fixture {
    _root: tempfile::TempDir,
    work: PathBuf,
    bare: PathBuf,
    reader: Arc<LocalSourceReader>,
    repository: RepositoryId,
}
async fn fixture(world: &mut World) -> Fixture {
    let (status,repository)=call(&world.app,"POST",&format!("/v1/projects/{}/repositories",world.project_id),&world.alice_token,Some("source-fixture-repository"),
        Some(json!({"remote_uri":"https://example.test/iris.git","default_branch":"main","metadata":{}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    let repository: RepositoryId = repository["id"].as_str().unwrap().parse().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let reader = Arc::new(LocalSourceReader::new(root.clone()).unwrap());
    let bare = reader
        .repository(world.tenant_id, world.project_scope, "fixture")
        .unwrap();
    std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
    for directory in [
        root.join(world.tenant_id.to_string()),
        bare.parent().unwrap().to_owned(),
    ] {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    git(&work, &["init", "--template=", "--initial-branch=main"]);
    std::fs::create_dir(work.join("procedures")).unwrap();
    std::fs::create_dir(work.join("conventions")).unwrap();
    std::fs::write(work.join("procedures/retries.md"), retry(3)).unwrap();
    std::fs::write(work.join("conventions/trace.md"),"---\ntype: Convention\ntitle: Iris tracing\ndescription: Independent fictional tracing.\n---\n\nUse traceparent for Iris requests.\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-m", "fixture"]);
    git(&work, &["branch", "dev"]);
    git(
        &root,
        &[
            "clone",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    std::fs::set_permissions(&bare, std::fs::Permissions::from_mode(0o700)).unwrap();
    world.state.git_sources = Some(Arc::new(synveda_git::SourceReaders::new(
        Some((*reader).clone()),
        None,
    )));
    world.app = router(world.state.clone());
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::bind_pack(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        synveda_policy::STANDARD,
    )
    .await;
    configuration_support::set_git_sources(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        vec![
            descriptor(repository, "source-a", "main"),
            descriptor(repository, "source-b", "dev"),
        ],
    )
    .await;
    tx.commit().await.unwrap();
    Fixture {
        _root: temp,
        work,
        bare,
        reader,
        repository,
    }
}
fn descriptor(repository_id: RepositoryId, source: &str, branch: &str) -> GitSourceDescriptor {
    GitSourceDescriptor {
        source_id: source.into(),
        repository_id,
        read_target: "fixture".into(),
        git_ref: format!("refs/heads/{branch}"),
        allowed_paths: vec![
            "conventions/trace.md".into(),
            "procedures/retries.md".into(),
        ],
        enabled: true,
    }
}
fn change(fixture: &Fixture, branch: &str, cap: u8) {
    git(&fixture.work, &["checkout", branch]);
    std::fs::write(fixture.work.join("procedures/retries.md"), retry(cap)).unwrap();
    git(&fixture.work, &["add", "procedures/retries.md"]);
    git(&fixture.work, &["commit", "-m", "source change"]);
    git(
        &fixture.work,
        &[
            "push",
            fixture.bare.to_str().unwrap(),
            &format!("{branch}:{branch}"),
        ],
    );
}
async fn refresh(world: &World, _fixture: &Fixture, source: &str, key: &str) -> Value {
    refresh_source(world, source, key).await
}
async fn refresh_source(world: &World, source: &str, key: &str) -> Value {
    let path = format!("/v1/projects/{}/sources/{source}/refresh", world.project_id);
    let (status, requested) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some(key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{requested}");
    let runtime = synveda_gateway::source_reconciliation::Runtime::new(
        world.state.pool.clone(),
        Arc::clone(&world.state.pdp),
        Arc::clone(world.state.git_sources.as_ref().unwrap()),
        Arc::clone(&world.state.keys),
    );
    assert!(
        synveda_gateway::source_reconciliation::execute_page(&runtime, world.tenant_id)
            .await
            .unwrap()
    );
    let (status, view) = call(
        &world.app,
        "GET",
        &format!("/v1/operations/{}", requested["id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["state"], "succeeded", "{view}");
    let (status, replay) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some(key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay, view);
    view
}
async fn job(world: &World, receipt: &Value) -> Value {
    let (status, job) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/okf/imports/{}",
            receipt["import_job_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{job}");
    job
}
fn artifact(job: &Value, path: &str) -> Value {
    job["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["logical_path"] == path)
        .unwrap()
        .clone()
}
fn retry_mapping(job: &Value) -> Value {
    let artifact = artifact(job, "procedures/retries.md");
    job["mappings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["artifact_id"] == artifact["id"])
        .unwrap()
        .clone()
}
async fn accept(world: &World, job: &Value, title: &str, key: &str) -> Value {
    let (status, batch) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/capture-candidates?batch_id={}",
            job["job"]["capture_batch_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    let candidate = batch["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["content"]["title"] == title)
        .unwrap();
    let (status, accepted) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/accept",
            candidate["id"].as_str().unwrap()
        ),
        &world.alice_token,
        Some(key),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    let mut candidate = accepted["candidate"].clone();
    if candidate["resulting_outcome"] == "pending_review" {
        apply_change(world, candidate["resulting_change_id"].as_str().unwrap()).await;
        let (status, item) = call(
            &world.app,
            "GET",
            &format!(
                "/v1/knowledge/{}",
                candidate["resulting_knowledge_item_id"].as_str().unwrap()
            ),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{item}");
        candidate["resulting_revision_id"] = item["current_revision"]["id"].clone();
    }
    candidate
}
async fn apply_change(world: &World, change: &str) {
    let path = format!("/v1/proposals/{change}");
    let (status, detail) = call(&world.app, "GET", &path, &world.bob_token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let (status, approved) = call(
        &world.app,
        "POST",
        &format!("{path}/approve"),
        &world.bob_token,
        None,
        Some(json!({"expected_commit":detail["commit"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let (status, applied) = call(
        &world.app,
        "POST",
        &format!("{path}/apply"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
}

async fn replace(world: &World, job: &Value, prior: &Value, key: &str) -> Value {
    replace_named(world, job, prior, "Iris retry policy", key).await
}
async fn replace_named(world: &World, job: &Value, prior: &Value, title: &str, key: &str) -> Value {
    let (status, batch) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/capture-candidates?batch_id={}",
            job["job"]["capture_batch_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    let candidate = batch["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["content"]["title"] == title)
        .unwrap();
    let (status, result) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/replace",
            candidate["id"].as_str().unwrap()
        ),
        &world.alice_token,
        Some(key),
        Some(json!({"item_id":prior["resulting_knowledge_item_id"],
        "expected_revision_id":prior["resulting_revision_id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    let mut result = result["candidate"].clone();
    if result["resulting_outcome"] == "pending_review" {
        apply_change(world, result["resulting_change_id"].as_str().unwrap()).await;
        let (status, item) = call(
            &world.app,
            "GET",
            &format!(
                "/v1/knowledge/{}",
                result["resulting_knowledge_item_id"].as_str().unwrap()
            ),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{item}");
        result["resulting_revision_id"] = item["current_revision"]["id"].clone();
    }
    result
}

#[tokio::test]
async fn force_push_of_historical_bytes_reviews_against_the_new_approved_head() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let fixture = fixture(&mut world).await;
    let first = refresh(&world, &fixture, "source-a", "reversion-first").await;
    let original = accept(
        &world,
        &job(&world, &first).await,
        "Iris retry policy",
        "reversion-accept",
    )
    .await;
    change(&fixture, "main", 5);
    let five = refresh(&world, &fixture, "source-a", "reversion-five").await;
    let approved = replace(
        &world,
        &job(&world, &five).await,
        &original,
        "reversion-replace",
    )
    .await;
    git(
        &fixture.work,
        &[
            "reset",
            "--hard",
            first["pinned_revision"].as_str().unwrap(),
        ],
    );
    git(
        &fixture.work,
        &[
            "push",
            "--force",
            fixture.bare.to_str().unwrap(),
            "main:main",
        ],
    );
    let reverted = refresh(&world, &fixture, "source-a", "reversion-three").await;
    let review = job(&world, &reverted).await;
    assert_ne!(review["job"]["id"], first["import_job_id"]);
    assert_eq!(retry_mapping(&review)["classification"], "update");
    assert_eq!(
        retry_mapping(&review)["matched_revision_id"],
        approved["resulting_revision_id"]
    );
    assert!(managed_changed(&world, &approved).await);
    let repeated = refresh(&world, &fixture, "source-a", "reversion-repeat").await;
    assert_eq!(
        repeated["import_job_id"], reverted["import_job_id"],
        "same compared head must reuse its review"
    );
    let restored = replace(&world, &review, &approved, "reversion-restore").await;
    assert!(!managed_changed(&world, &restored).await);
    let unchanged = refresh(&world, &fixture, "source-a", "reversion-unchanged").await;
    assert!(unchanged["import_job_id"].is_null());
}

#[tokio::test]
async fn restarted_attempt_keeps_the_committed_pin_and_rolls_back_incomplete_work() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let fixture = fixture(&mut world).await;
    let before = audit_events(&world).await.len();
    for suffix in ["", "/source-a"] {
        let (status, body) = call(
            &world.app,
            "GET",
            &format!("/v1/projects/{}/sources{suffix}", world.project_id),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let audit = audit_events(&world).await;
    assert_eq!(
        audit[before..]
            .iter()
            .filter(|event| event.payload["op"] == "sources"
                && event.payload["authz"]["action"] == "source.read"
                && event.outcome == "allow")
            .count(),
        2
    );
    let path = format!("/v1/projects/{}/sources/source-a/refresh", world.project_id);
    let (left, right) = tokio::join!(
        call(
            &world.app,
            "POST",
            &path,
            &world.alice_token,
            Some("crash-pin"),
            None
        ),
        call(
            &world.app,
            "POST",
            &path,
            &world.alice_token,
            Some("crash-pin"),
            None
        )
    );
    assert!(
        matches!(left.0, StatusCode::OK | StatusCode::ACCEPTED),
        "{left:?}"
    );
    assert!(
        matches!(right.0, StatusCode::OK | StatusCode::ACCEPTED),
        "{right:?}"
    );
    assert_eq!(left.1["id"], right.1["id"]);
    let id = left.1["id"].as_str().unwrap().parse().unwrap();
    let pin = fixture
        .reader
        .resolve(
            world.tenant_id,
            world.project_scope,
            "fixture",
            "refs/heads/main",
        )
        .await
        .unwrap();
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let claim =
        synveda_store::source_reconciliation::claim(&mut tx, world.tenant_id, "crashed-fixture")
            .await
            .unwrap()
            .unwrap();
    synveda_store::source_reconciliation::fence(
        &mut tx,
        world.tenant_id,
        id,
        claim.request.attempts,
        "crashed-fixture",
    )
    .await
    .unwrap();
    synveda_store::source_reconciliation::pin(&mut tx, world.tenant_id, id, &pin.commit, &pin.tree)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    synveda_store::source_reconciliation::fence(
        &mut tx,
        world.tenant_id,
        id,
        claim.request.attempts,
        "crashed-fixture",
    )
    .await
    .unwrap();
    let target = synveda_store::source_reconciliation::request(&mut tx, world.tenant_id, id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        synveda_store::source_reconciliation::complete(
            &mut tx,
            world.tenant_id,
            &target,
            None,
            &[
                synveda_store::source_reconciliation::ObservedPath {
                    path: "conventions/trace.md".to_owned(),
                    state: "missing",
                    blob_oid: None,
                    content_hash: None,
                    artifact_id: None,
                },
                synveda_store::source_reconciliation::ObservedPath {
                    path: "conventions/trace.md".to_owned(),
                    state: "missing",
                    blob_oid: None,
                    content_hash: None,
                    artifact_id: None,
                },
            ]
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let head = synveda_store::source_reconciliation::head(
        &mut tx,
        world.tenant_id,
        world.project_id.parse().unwrap(),
        "source-a",
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        head.observation_id.is_none(),
        "partial source work must not advance the cursor"
    );
    let count: i64 = sqlx::query_scalar(
        "select count(*) from source_observations where tenant_id=$1 and operation_id=$2",
    )
    .bind(world.tenant_id.as_uuid())
    .bind(id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(
        count, 0,
        "rollback must remove the partially inserted observation"
    );
    tx.commit().await.unwrap();
    change(&fixture, "main", 5);
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    sqlx::query("update durable_operations set lease_expires_at=clock_timestamp()-interval '1 second' where tenant_id=$1 and id=$2")
        .bind(world.tenant_id.as_uuid()).bind(id.as_uuid()).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let runtime = synveda_gateway::source_reconciliation::Runtime::new(
        world.state.pool.clone(),
        Arc::clone(&world.state.pdp),
        Arc::clone(world.state.git_sources.as_ref().unwrap()),
        Arc::clone(&world.state.keys),
    );
    assert!(
        synveda_gateway::source_reconciliation::execute_page(&runtime, world.tenant_id)
            .await
            .unwrap()
    );
    let (status, receipt) = call(
        &world.app,
        "GET",
        &format!("/v1/operations/{id}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["attempts"], 2);
    assert_eq!(receipt["pinned_revision"], pin.commit);
    assert!(
        job(&world, &receipt)
            .await
            .to_string()
            .contains("3 attempts")
    );
    let changed = refresh(&world, &fixture, "source-a", "crash-next").await;
    assert!(
        job(&world, &changed)
            .await
            .to_string()
            .contains("5 attempts")
    );
}
async fn managed_changed(world: &World, candidate: &Value) -> bool {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let evidence = synveda_store::knowledge_freshness::evidence(
        &mut tx,
        world.tenant_id,
        candidate["resulting_revision_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        Some(world.project_id.parse::<ProjectId>().unwrap()),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    evidence.managed_source_changed
}

#[tokio::test]
async fn real_git_reconciliation_separates_refs_targets_freshness_and_physical_aliases() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let fixture = fixture(&mut world).await;
    let first = refresh(&world, &fixture, "source-a", "source-a-initial").await;
    let a = job(&world, &first).await;
    let retry_item = accept(&world, &a, "Iris retry policy", "source-accept-retry").await;
    let clean = accept(&world, &a, "Iris tracing", "source-accept-clean").await;
    let second = refresh(&world, &fixture, "source-b", "source-b-initial").await;
    let b = job(&world, &second).await;
    assert_ne!(a["job"]["id"], b["job"]["id"]);
    assert_ne!(
        artifact(&a, "procedures/retries.md")["id"],
        artifact(&b, "procedures/retries.md")["id"]
    );
    change(&fixture, "dev", 5);
    let changed_b = refresh(&world, &fixture, "source-b", "source-b-five").await;
    assert_eq!(
        retry_mapping(&job(&world, &changed_b).await)["classification"],
        "addition"
    );
    assert!(
        !managed_changed(&world, &retry_item).await,
        "a different configured ref must not stale A"
    );
    assert!(!managed_changed(&world, &clean).await);
    change(&fixture, "main", 5);
    let changed_a = refresh(&world, &fixture, "source-a", "source-a-five").await;
    let a5 = job(&world, &changed_a).await;
    assert_eq!(retry_mapping(&a5)["classification"], "update");
    assert_eq!(
        retry_mapping(&a5)["matched_item_id"],
        retry_item["resulting_knowledge_item_id"]
    );
    assert!(managed_changed(&world, &retry_item).await);
    assert!(
        !managed_changed(&world, &clean).await,
        "unrelated path must retain freshness"
    );
    change(&fixture, "main", 7);
    let changed_again = refresh(&world, &fixture, "source-a", "source-a-seven").await;
    let a7 = job(&world, &changed_again).await;
    assert_eq!(
        retry_mapping(&a7)["matched_item_id"],
        retry_item["resulting_knowledge_item_id"],
        "unaccepted intermediate candidates must not lose the approved target"
    );
    // A source alias spans duplicate managed jobs, while freshness is source-specific.
    let source = artifact(&a, "procedures/retries.md");
    let (status,removed)=call(&world.app,"POST",&format!("/v1/okf/imports/{}/artifacts/{}/withdraw",a["job"]["id"].as_str().unwrap(),source["id"].as_str().unwrap()),
        &world.alice_token,Some("managed-source-withdraw"),Some(json!({"expected_content_hash":source["content_hash"],"reason":"Retire exact fictional source bytes"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    if removed["outcome"] == "pending_review" {
        apply_change(&world, removed["change_id"].as_str().unwrap()).await;
    }
    let (status, b_view) = call(
        &world.app,
        "GET",
        &format!("/v1/okf/imports/{}", b["job"]["id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{b_view}");
    assert!(!b_view.to_string().contains("3 attempts"));
    change(&fixture, "main", 3);
    let suppressed = refresh(&world, &fixture, "source-a", "source-a-suppressed").await;
    assert_eq!(suppressed["import_job_id"], Value::Null);
    let (status, clean_view) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/knowledge/{}",
            clean["resulting_knowledge_item_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{clean_view}");
    assert!(clean_view.to_string().contains("traceparent"));
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let id: ImportJobId = a7["job"]["id"].as_str().unwrap().parse().unwrap();
    let stored = synveda_store::imports::get_job(&mut *tx, world.tenant_id, id)
        .await
        .unwrap()
        .unwrap();
    synveda_store::artifact_control::admit_job(&mut tx, world.tenant_id, &stored)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn unchanged_deletion_and_force_push_observations_do_not_duplicate_candidates() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let fixture = fixture(&mut world).await;
    let first = refresh(&world, &fixture, "source-a", "source-observation-initial").await;
    let item = accept(
        &world,
        &job(&world, &first).await,
        "Iris retry policy",
        "source-observation-accept",
    )
    .await;
    std::fs::write(
        fixture.work.join("unrelated.md"),
        "An unrelated repository file.",
    )
    .unwrap();
    git(&fixture.work, &["add", "unrelated.md"]);
    git(&fixture.work, &["commit", "-m", "unrelated"]);
    git(
        &fixture.work,
        &["push", fixture.bare.to_str().unwrap(), "main:main"],
    );
    let unrelated = refresh(&world, &fixture, "source-a", "source-observation-unrelated").await;
    assert_eq!(unrelated["import_job_id"], Value::Null);
    assert!(!managed_changed(&world, &item).await);
    std::fs::remove_file(fixture.work.join("procedures/retries.md")).unwrap();
    git(&fixture.work, &["add", "-A"]);
    git(&fixture.work, &["commit", "-m", "deletion"]);
    git(
        &fixture.work,
        &["push", fixture.bare.to_str().unwrap(), "main:main"],
    );
    let deletion = refresh(&world, &fixture, "source-a", "source-observation-deletion").await;
    assert_eq!(deletion["import_job_id"], Value::Null);
    assert!(managed_changed(&world, &item).await);
    git(
        &fixture.work,
        &[
            "reset",
            "--hard",
            first["pinned_revision"].as_str().unwrap(),
        ],
    );
    git(
        &fixture.work,
        &[
            "push",
            "--force",
            fixture.bare.to_str().unwrap(),
            "main:main",
        ],
    );
    let restored = refresh(
        &world,
        &fixture,
        "source-a",
        "source-observation-force-push",
    )
    .await;
    assert_eq!(
        restored["import_job_id"],
        Value::Null,
        "an already admitted source/hash must not mint another proposal"
    );
    assert!(!managed_changed(&world, &item).await);
}

#[tokio::test]
async fn missing_objects_cancel_disconnect_and_denial_preserve_successful_cursor() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let fixture = fixture(&mut world).await;
    let first = refresh(&world, &fixture, "source-a", "source-failure-initial").await;
    git(&fixture.work, &["checkout", "main"]);
    std::fs::write(fixture.work.join("procedures/retries.md"), retry(5)).unwrap();
    git(&fixture.work, &["add", "."]);
    git(&fixture.work, &["commit", "-m", "missing blob fixture"]);
    git(
        &fixture.work,
        &["push", fixture.bare.to_str().unwrap(), "main:main"],
    );
    let pin = fixture
        .reader
        .resolve(
            world.tenant_id,
            world.project_scope,
            "fixture",
            "refs/heads/main",
        )
        .await
        .unwrap();
    let snapshot = fixture
        .reader
        .read_at(
            world.tenant_id,
            world.project_scope,
            "fixture",
            &pin,
            &["procedures/retries.md".into()],
        )
        .await
        .unwrap();
    let blob = snapshot.files[0].blob_oid.as_ref().unwrap();
    std::fs::remove_file(
        fixture
            .bare
            .join("objects")
            .join(&blob[..2])
            .join(&blob[2..]),
    )
    .unwrap();
    let path = format!("/v1/projects/{}/sources/source-a/refresh", world.project_id);
    let (status, pending) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("source-failure-missing"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    let runtime = synveda_gateway::source_reconciliation::Runtime::new(
        world.state.pool.clone(),
        Arc::clone(&world.state.pdp),
        Arc::clone(world.state.git_sources.as_ref().unwrap()),
        Arc::clone(&world.state.keys),
    );
    assert!(
        synveda_gateway::source_reconciliation::execute_page(&runtime, world.tenant_id)
            .await
            .is_err()
    );
    let (status, source) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{}/sources/source-a", world.project_id),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{source}");
    assert_eq!(source["observation_id"], first["observation_id"]);
    assert_eq!(source["operation"]["state"], "failed");
    assert_eq!(
        source["operation"]["error_code"],
        "source_snapshot_incomplete"
    );
    let (status, cancelled) = call(
        &world.app,
        "POST",
        &format!("/v1/operations/{}/cancel", pending["id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["state"], "cancelled");
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let mut disabled = descriptor(fixture.repository, "source-a", "main");
    disabled.enabled = false;
    configuration_support::set_git_sources(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        vec![disabled, descriptor(fixture.repository, "source-b", "dev")],
    )
    .await;
    tx.commit().await.unwrap();
    let (status, refused) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("source-failure-disconnected"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let denied_policy = format!(
        "{BLANKET}\nforbid (principal, action == Synveda::Action::\"SourceRead\", resource);"
    );
    world
        .install_policy(
            "source-read-denied",
            1,
            &denied_policy,
            PackConfig::default(),
        )
        .await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::set_source_policy(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        "source-read-denied",
    )
    .await;
    tx.commit().await.unwrap();
    let known = format!("/v1/projects/{}/sources/source-a", world.project_id);
    let absent = format!("/v1/projects/{}/sources/missing-source", world.project_id);
    let denied = call(&world.app, "GET", &known, &world.alice_token, None, None).await;
    assert_eq!(denied.0, StatusCode::NOT_FOUND);
    assert_eq!(
        denied,
        call(&world.app, "GET", &absent, &world.alice_token, None, None).await
    );
    assert!(audit_events(&world).await.iter().any(|entry| entry.action
        == synveda_audit::AuditAction::AuthzDecision.as_str()
        && entry.outcome == synveda_audit::Outcome::Deny.as_str()));
}

#[tokio::test]
async fn public_upload_cannot_supply_managed_source_provenance() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let fixture = fixture(&mut world).await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let repository = synveda_store::repositories::get(
        &mut *tx,
        world.tenant_id,
        world.project_id.parse().unwrap(),
        fixture.repository,
    )
    .await
    .unwrap()
    .unwrap();
    tx.commit().await.unwrap();
    let pin = fixture
        .reader
        .resolve(
            world.tenant_id,
            world.project_scope,
            "fixture",
            "refs/heads/main",
        )
        .await
        .unwrap();
    let entries: Vec<_>=["conventions/trace.md","procedures/retries.md"].iter().map(|path|
        json!({"logical_path":path,"kind":"file","content_base64":STANDARD.encode(std::fs::read(fixture.work.join(path)).unwrap())})).collect();
    let (status,uploaded)=call(&world.app,"POST",&format!("/v1/projects/{}/okf/imports",world.project_id),&world.alice_token,Some("source-unmanaged-upload"),
        Some(json!({"source_kind":"git","source_locator":repository.canonical_uri,"source_revision":pin.commit,"encoding":"entries","entries":entries}))).await;
    assert_eq!(status, StatusCode::CREATED, "{uploaded}");
    let managed = refresh(&world, &fixture, "source-a", "source-after-upload").await;
    let managed_job = job(&world, &managed).await;
    assert_eq!(
        uploaded["job"]["bundle_digest"],
        managed_job["job"]["bundle_digest"]
    );
    assert_ne!(uploaded["job"]["id"], managed_job["job"]["id"]);
    assert_ne!(
        artifact(&uploaded, "procedures/retries.md")["id"],
        artifact(&managed_job, "procedures/retries.md")["id"]
    );
}

async fn attach_remote(world: &World, name: &str, key: &str) -> RepositoryId {
    let (status,repository)=call(&world.app,"POST",&format!("/v1/projects/{}/repositories",world.project_id),
        &world.alice_token,Some(key),Some(json!({"remote_uri":format!("https://github.com/fixture-owner/{name}"),"default_branch":"main","metadata":{}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    repository["id"].as_str().unwrap().parse().unwrap()
}
async fn put_read_secret(world: &World, id: synveda_types::TenantSecretId, format: &str) {
    let key = world
        .state
        .keys
        .sealing_key(
            &world.state.pool,
            synveda_crypto::KeyScope::Tenant(world.tenant_id),
        )
        .await
        .unwrap();
    let document = serde_json::to_vec(
        &json!({"format":format,"repository_id":123,"token":source_https::TOKEN}),
    )
    .unwrap();
    let sealed = key
        .seal(
            synveda_crypto::Purpose::TenantSecret,
            synveda_crypto::RowKey::Uuid(id.as_uuid()),
            &document,
        )
        .unwrap();
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    synveda_store::tenant_secrets::put(
        &mut tx,
        id,
        world.tenant_id,
        world.project_scope,
        synveda_types::secret::TenantSecretKind::SourceRead,
        "source.read.fixture",
        Some("github"),
        key.version(),
        &sealed,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}
async fn configure_remote(
    world: &mut World,
    https: &source_https::Fixture,
    secret: synveda_types::TenantSecretId,
    repository: RepositoryId,
    renamed: bool,
) {
    world.state.git_sources = Some(Arc::new(synveda_git::SourceReaders::new(
        None,
        Some(https.reader(world.tenant_id, world.project_scope, secret, renamed)),
    )));
    world.app = router(world.state.clone());
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::set_git_sources(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        vec![GitSourceDescriptor {
            source_id: "private-source".to_owned(),
            repository_id: repository,
            read_target: "private-fixture".to_owned(),
            git_ref: "refs/heads/main".to_owned(),
            allowed_paths: vec!["missing.md".to_owned(), "procedures/retries.md".to_owned()],
            enabled: true,
        }],
    )
    .await;
    tx.commit().await.unwrap();
}
async fn remote_fixture(
    world: &mut World,
) -> (
    source_https::Fixture,
    synveda_types::TenantSecretId,
    RepositoryId,
) {
    world.state.keys = Arc::new(synveda_store::keys::KeyRing::new(
        synveda_crypto::Kms::Local(
            synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:fictional-source-fixture")
                .unwrap(),
        ),
    ));
    world
        .state
        .keys
        .provision(
            &world.state.pool,
            synveda_crypto::KeyScope::Tenant(world.tenant_id),
        )
        .await
        .unwrap();
    world.app = router(world.state.clone());
    let https = source_https::Fixture::new().await;
    let repository = attach_remote(world, "source", "private-source-attachment").await;
    let secret = synveda_types::TenantSecretId::new();
    put_read_secret(world, secret, "synveda-github-source-credential-v1").await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::bind_pack(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        synveda_policy::STANDARD,
    )
    .await;
    configuration_support::set_github_exports(&mut tx, world.tenant_id, world.project_scope, true)
        .await;
    tx.commit().await.unwrap();
    configure_remote(world, &https, secret, repository, false).await;
    (https, secret, repository)
}
fn runtime(world: &World) -> synveda_gateway::source_reconciliation::Runtime {
    synveda_gateway::source_reconciliation::Runtime::new(
        world.state.pool.clone(),
        Arc::clone(&world.state.pdp),
        Arc::clone(world.state.git_sources.as_ref().unwrap()),
        Arc::clone(&world.state.keys),
    )
}
async fn queue_remote(world: &World, key: &str) -> Value {
    let (status, body) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/projects/{}/sources/private-source/refresh",
            world.project_id
        ),
        &world.alice_token,
        Some(key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body
}
async fn operation(world: &World, receipt: &Value) -> Value {
    let (status, body) = call(
        &world.app,
        "GET",
        &format!("/v1/operations/{}", receipt["id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn remote_status(world: &World) -> Value {
    let (status, body) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{}/sources/private-source", world.project_id),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn private_https_complete_objects_and_rename_preserve_review_and_suppression_identity() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let (https, secret, _repository) = remote_fixture(&mut world).await;
    let first = refresh_source(&world, "private-source", "https-initial").await;
    assert_eq!(first["pinned_revision"], https.pin.commit);
    let original = job(&world, &first).await;
    let _approved = accept(&world, &original, "Iris retry policy", "https-accept").await;
    https.set_mode(source_https::Mode::MovedRef);
    let changed = refresh_source(&world, "private-source", "https-five").await;
    assert_eq!(
        retry_mapping(&job(&world, &changed).await)["classification"],
        "update"
    );
    let renamed = attach_remote(&world, "renamed", "https-renamed-attachment").await;
    configure_remote(&mut world, &https, secret, renamed, true).await;
    https.set_mode(source_https::Mode::RenamedMovedRef);
    let repeated = refresh_source(&world, "private-source", "https-renamed-five").await;
    assert_eq!(
        changed["import_job_id"], repeated["import_job_id"],
        "rename must reuse the exact pending review"
    );
    let source = artifact(&original, "procedures/retries.md");
    let (status,removed)=call(&world.app,"POST",&format!("/v1/okf/imports/{}/artifacts/{}/withdraw",original["job"]["id"].as_str().unwrap(),source["id"].as_str().unwrap()),
        &world.alice_token,Some("https-withdraw"),Some(json!({"expected_content_hash":source["content_hash"],"reason":"Retire exact fictional source bytes"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    if removed["outcome"] == "pending_review" {
        apply_change(&world, removed["change_id"].as_str().unwrap()).await;
    }
    https.set_mode(source_https::Mode::Renamed);
    let suppressed = refresh_source(&world, "private-source", "https-renamed-suppressed").await;
    assert!(suppressed["import_job_id"].is_null());
    let before = https.requests.lock().unwrap().len();
    let same = refresh_source(&world, "private-source", "https-renamed-suppressed-repeat").await;
    assert!(same["import_job_id"].is_null());
    assert!(
        !https.requests.lock().unwrap()[before..]
            .iter()
            .any(|request| request.contains("/git/blobs/")),
        "known suppressed bytes must not be fetched"
    );
    assert!(
        !remote_status(&world)
            .await
            .to_string()
            .contains(source_https::TOKEN)
    );
}

#[tokio::test]
async fn private_https_errors_never_advance_the_last_complete_cursor_or_infer_deletion() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let (https, _, _) = remote_fixture(&mut world).await;
    let first = refresh_source(&world, "private-source", "https-error-initial").await;
    for (index, mode) in [
        source_https::Mode::Truncated,
        source_https::Mode::TamperedTree,
        source_https::Mode::TamperedBlob,
        source_https::Mode::MissingBlob,
        source_https::Mode::Outage,
        source_https::Mode::Public,
        source_https::Mode::Replaced,
        source_https::Mode::Redirect,
    ]
    .into_iter()
    .enumerate()
    {
        https.set_mode(mode);
        let queued = queue_remote(&world, &format!("https-error-{index}")).await;
        assert!(
            synveda_gateway::source_reconciliation::execute_page(&runtime(&world), world.tenant_id)
                .await
                .is_err()
        );
        let current = operation(&world, &queued).await;
        assert!(
            matches!(current["state"].as_str(), Some("failed" | "blocked")),
            "{current}"
        );
        assert!(current["observation_id"].is_null());
        assert_eq!(
            remote_status(&world).await["observation_id"],
            first["observation_id"]
        );
        let (status, cancelled) = call(
            &world.app,
            "POST",
            &format!("/v1/operations/{}/cancel", queued["id"].as_str().unwrap()),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
    }
    assert!(https.requests.lock().unwrap().iter().all(|request| {
        request.starts_with("GET /repos/fixture-owner/source") && !request.contains('?')
    }));
}

#[tokio::test]
async fn private_https_rotation_and_revocation_during_fetch_fence_all_source_work() {
    let _guard = serial().await;
    for revoke in [false, true] {
        let Some(mut world) = admitted_world().await else {
            return;
        };
        let (https, secret, _) = remote_fixture(&mut world).await;
        https.set_mode(source_https::Mode::PauseBlob);
        let queued = queue_remote(
            &world,
            if revoke {
                "https-revoke"
            } else {
                "https-rotate"
            },
        )
        .await;
        let worker = runtime(&world);
        let tenant = world.tenant_id;
        let execution = tokio::spawn(async move {
            synveda_gateway::source_reconciliation::execute_page(&worker, tenant).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), https.reached.notified())
            .await
            .unwrap();
        // This mutation must complete while HTTPS is paused: provider waits hold no admission gate.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            if revoke {
                let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
                    .await
                    .unwrap();
                synveda_store::tenant_secrets::revoke(&mut tx, world.tenant_id, secret)
                    .await
                    .unwrap()
                    .unwrap();
                tx.commit().await.unwrap();
            } else {
                put_read_secret(&world, secret, "synveda-github-source-credential-v1").await;
            }
        })
        .await
        .unwrap();
        https.release.notify_one();
        assert!(execution.await.unwrap().is_err());
        assert_eq!(operation(&world, &queued).await["state"], "blocked");
        assert!(remote_status(&world).await["observation_id"].is_null());
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .unwrap();
        let jobs:i64=sqlx::query_scalar("select count(*) from import_jobs where tenant_id=$1 and managed_source_id='private-source'")
            .bind(world.tenant_id.as_uuid()).fetch_one(&mut *tx).await.unwrap();
        assert_eq!(jobs, 0);
        tx.commit().await.unwrap();
        if revoke {
            put_read_secret(&world, secret, "synveda-github-source-credential-v1").await;
        }
        https.set_mode(source_https::Mode::Normal);
        let recovered = refresh_source(&world, "private-source", "https-custody-recovered").await;
        assert_eq!(recovered["state"], "succeeded");
    }
}

#[tokio::test]
async fn transient_key_opening_retries_the_same_frozen_request_without_any_http() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let (https, _, _) = remote_fixture(&mut world).await;
    let queued = queue_remote(&world, "https-key-retry").await;
    let disabled = Arc::new(synveda_store::keys::KeyRing::new(
        synveda_crypto::Kms::Disabled,
    ));
    let worker = synveda_gateway::source_reconciliation::Runtime::new(
        world.state.pool.clone(),
        Arc::clone(&world.state.pdp),
        Arc::clone(world.state.git_sources.as_ref().unwrap()),
        disabled,
    );
    assert!(matches!(
        synveda_gateway::source_reconciliation::execute_page(&worker, world.tenant_id).await,
        Err(synveda_types::Error::Dependency { .. })
    ));
    assert_eq!(operation(&world, &queued).await["state"], "failed");
    assert!(https.requests.lock().unwrap().is_empty());
    assert!(remote_status(&world).await["observation_id"].is_null());
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    assert!(
        synveda_gateway::source_reconciliation::execute_page(&runtime(&world), world.tenant_id)
            .await
            .unwrap()
    );
    let succeeded = operation(&world, &queued).await;
    assert_eq!(succeeded["state"], "succeeded");
    assert_eq!(succeeded["attempts"], 2);
}

#[tokio::test]
async fn detached_legacy_uri_history_cannot_be_rebound_into_another_provider_root() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let local = fixture(&mut world).await;
    let legacy = attach_remote(&world, "renamed", "legacy-uri-source").await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::set_git_sources(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        vec![descriptor(legacy, "source-a", "main")],
    )
    .await;
    tx.commit().await.unwrap();
    let original = refresh(&world, &local, "source-a", "legacy-root-first").await;
    let legacy_job = job(&world, &original).await;
    let source = artifact(&legacy_job, "procedures/retries.md");
    let (status,removed)=call(&world.app,"POST",&format!("/v1/okf/imports/{}/artifacts/{}/withdraw",legacy_job["job"]["id"].as_str().unwrap(),source["id"].as_str().unwrap()),
        &world.alice_token,Some("legacy-root-withdraw"),Some(json!({"expected_content_hash":source["content_hash"],"reason":"Preserve withdrawal across detached identity"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    if removed["outcome"] == "pending_review" {
        apply_change(&world, removed["change_id"].as_str().unwrap()).await;
    }
    let (status, body) = call(
        &world.app,
        "DELETE",
        &format!("/v1/projects/{}/repositories/{legacy}", world.project_id),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (https, secret, _) = remote_fixture(&mut world).await;
    let first = refresh_source(&world, "private-source", "provider-root-first").await;
    let reattached = attach_remote(&world, "renamed", "legacy-uri-reattached").await;
    assert_ne!(legacy, reattached);
    configure_remote(&mut world, &https, secret, reattached, true).await;
    https.set_mode(source_https::Mode::Renamed);
    let (status, refused) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/projects/{}/sources/private-source/refresh",
            world.project_id
        ),
        &world.alice_token,
        Some("ambiguous-root-refused"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(
        remote_status(&world).await["observation_id"],
        first["observation_id"]
    );
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let bound = synveda_store::source_reconciliation::github_identity_bound(
        &mut tx,
        world.tenant_id,
        world.project_id.parse().unwrap(),
        reattached,
        123,
    )
    .await
    .unwrap();
    assert!(!bound, "failed rebinding must not retain a new alias");
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn export_credentials_and_disallowed_providers_cannot_authorize_inbound_reads() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let (https, secret, _) = remote_fixture(&mut world).await;
    put_read_secret(&world, secret, "synveda-github-export-credential-v1").await;
    let queued = queue_remote(&world, "wrong-credential-purpose").await;
    assert!(
        synveda_gateway::source_reconciliation::execute_page(&runtime(&world), world.tenant_id)
            .await
            .is_err()
    );
    assert_eq!(operation(&world, &queued).await["state"], "blocked");
    assert!(https.requests.lock().unwrap().is_empty());
    put_read_secret(&world, secret, "synveda-github-source-credential-v1").await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::set_github_exports(&mut tx, world.tenant_id, world.project_scope, false)
        .await;
    tx.commit().await.unwrap();
    let (status, refused) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/projects/{}/sources/private-source/refresh",
            world.project_id
        ),
        &world.alice_token,
        Some("provider-narrowing"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert!(https.requests.lock().unwrap().is_empty());
}

fn iris_digest(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}
async fn iris_item(world: &World, item: &str) -> Value {
    let (status, v) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v
}
async fn iris_state(world: &World, item: &Value, key: &str) -> Value {
    let text = item["current_revision"]["body_markdown"].as_str().unwrap();
    let (status,v)=call(&world.app,"POST",&format!("/v1/sessions/{}/task-state",world.alice_session),&world.alice_token,None,
        Some(json!({"schema_version":1,"client_event_id":key,"expected_previous":null,"statements":[{"kind":"constraint","evidence":{"source_kind":"knowledge_revision","item_id":item["id"],"revision_id":item["current_revision"]["id"],"content_hash":item["current_revision"]["content_hash"],"span":{"start":0,"end":text.len()}}}]}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v["event"].clone()
}
async fn iris_cli_preview(world: &World, state: &Value, required: &Value) -> Value {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = world.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let handoff = format!(
        "{}@{}@{}",
        world.alice_session,
        state["id"].as_str().unwrap(),
        state["payload_hash"].as_str().unwrap()
    );
    let revision = format!(
        "{}@{}",
        required["id"].as_str().unwrap(),
        required["current_revision"]["id"].as_str().unwrap()
    );
    let binary = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/synveda");
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(binary)
            .args([
                "context",
                "preview",
                &world.bob_session,
                "--handoff",
                &handoff,
                "--require-knowledge",
                &revision,
                "--tokenizer-encoding",
                "o200k_base",
                "--budget-tokens",
                "2600",
                "--json",
            ])
            .env("SYNVEDA_GATEWAY", url)
            .env("SYNVEDA_TOKEN", &world.bob_token)
            .env("XDG_STATE_HOME", temp.path())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    server.abort();
    let output = output
        .unwrap()
        .expect("build the actual CLI before the Iris demonstration");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
// This closed local fixture interprets only the reviewed retry cap, delay and
// unknown-commit guard. It never executes arbitrary source instructions.
fn iris_retry_case(body: &str, unknown: bool, ack_after_ms: u32) -> Value {
    let cap = if body.contains("5 attempts") { 5 } else { 3 };
    let delay = if body.contains("200 ms") { 200 } else { 0 };
    if unknown {
        return json!({"passed":body.contains("Never retry an unknown commit outcome"),"attempts":0,"observed":"unknown_commit"});
    }
    let mut elapsed = 0;
    for attempt in 1..=cap {
        elapsed += delay;
        if elapsed >= ack_after_ms {
            return json!({"passed":true,"attempts":attempt,"elapsed_ms":elapsed});
        }
    }
    json!({"passed":false,"attempts":cap,"elapsed_ms":elapsed})
}

#[tokio::test]
async fn iris_domain_context_lifecycle_is_governed_end_to_end() {
    let started = Instant::now();
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::ERROR)
        .try_init();
    let fixture = fixture(&mut world).await;
    let files = [
        ("entities/delivery.md", "Iris Delivery"),
        ("entities/provider-event.md", "Iris ProviderEvent"),
        (
            "definitions/acknowledged-rate.md",
            "Iris acknowledged delivery rate",
        ),
        (
            "conventions/applicability.md",
            "Iris applicability and relationships",
        ),
        (
            "decisions/unknown-commit.md",
            "Iris unknown commit decision",
        ),
        ("procedures/retries.md", "Iris retry policy"),
    ];
    let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demos/fixtures/iris-domain");
    let now = chrono::Utc::now();
    let generated = now.to_rfc3339();
    let due = (now + chrono::Duration::days(14)).to_rfc3339();
    for (path, _) in files {
        let dest = fixture.work.join(path);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let source = std::fs::read_to_string(package.join(path))
            .unwrap()
            .replace("2026-10-14T00:00:00Z", &generated)
            .replace("2027-01-01T00:00:00Z", &due);
        std::fs::write(dest, source).unwrap();
    }
    git(&fixture.work, &["add", "."]);
    git(&fixture.work, &["commit", "-m", "owned Iris domain"]);
    git(
        &fixture.work,
        &["push", fixture.bare.to_str().unwrap(), "main:main"],
    );
    let mut d = descriptor(fixture.repository, "source-a", "main");
    d.allowed_paths = files.iter().map(|(path, _)| (*path).to_owned()).collect();
    d.allowed_paths.sort();
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::set_git_sources(&mut tx, world.tenant_id, world.project_scope, vec![d])
        .await;
    tx.commit().await.unwrap();
    let receipt = refresh_source(&world, "source-a", "iris-domain-first").await;
    let first = job(&world, &receipt).await;
    assert_eq!(first["artifacts"].as_array().unwrap().len(), 6);
    let mut items = Vec::new();
    for (i, (_, title)) in files.iter().enumerate() {
        let accepted = accept(&world, &first, title, &format!("iris-publish-{i}")).await;
        items.push(
            iris_item(
                &world,
                accepted["resulting_knowledge_item_id"].as_str().unwrap(),
            )
            .await,
        );
    }
    let required:Vec<_>=[2,4,5].iter().map(|i|json!({"item_id":items[*i]["id"],"revision_id":items[*i]["current_revision"]["id"]})).collect();
    let (status, run) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/context-runs", world.alice_session),
        &world.alice_token,
        Some("iris-domain-delivery"),
        Some(json!({"budget_tokens":2600,"required_knowledge_revisions":required})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let rendered = run["rendered"].as_str().unwrap();
    for fact in [
        "percentage",
        "UTC event day",
        "provider event time",
        "Exclude sandbox",
        "unknown commit outcome",
        "3 attempts",
    ] {
        assert!(rendered.contains(fact), "lost domain qualifier: {fact}");
    }
    let handoff = iris_state(&world, &items[5], "iris-domain-state").await;
    let cli = iris_cli_preview(&world, &handoff, &items[2]).await;
    assert!(cli.to_string().contains("UTC event day"));
    assert!(cli.to_string().contains("unknown commit outcome"));
    let metric_path = fixture.work.join(files[2].0);
    let old_metric = std::fs::read_to_string(&metric_path).unwrap();
    std::fs::write(
        &metric_path,
        old_metric.replace("Exclude sandbox events", "Include sandbox events"),
    )
    .unwrap();
    let retry_path = fixture.work.join(files[5].0);
    let old_retry = std::fs::read_to_string(&retry_path).unwrap();
    std::fs::write(&retry_path, old_retry.replace("3 attempts", "5 attempts")).unwrap();
    git(&fixture.work, &["add", "."]);
    git(
        &fixture.work,
        &["commit", "-m", "owned definitions changed"],
    );
    git(
        &fixture.work,
        &["push", fixture.bare.to_str().unwrap(), "main:main"],
    );
    let refreshed = refresh_source(&world, "source-a", "iris-domain-changed").await;
    let changed = job(&world, &refreshed).await;
    assert_eq!(
        changed["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["classification"] == "update")
            .count(),
        2
    );
    let current_metric = iris_item(&world, items[2]["id"].as_str().unwrap()).await;
    assert_eq!(
        current_metric["current_revision"]["id"],
        items[2]["current_revision"]["id"]
    );
    assert!(
        current_metric["current_revision"]["body_markdown"]
            .as_str()
            .unwrap()
            .contains("Exclude sandbox")
    );
    assert!(
        current_metric["current_revision"]["freshness_reasons"]
            .to_string()
            .contains("source_freshness"),
        "{current_metric}"
    );
    let clean = iris_item(&world, items[0]["id"].as_str().unwrap()).await;
    assert!(
        !clean["current_revision"]["freshness_reasons"]
            .to_string()
            .contains("source_freshness")
    );
    let health = detail(&world, &world.alice_token, run["id"].as_str().unwrap()).await;
    let freshness = health["health"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["dimension"] == "freshness")
        .unwrap();
    assert_eq!(freshness["status"], "needs_attention", "{health}");
    assert_eq!(freshness["evidence"]["source_id"], "source-a");
    assert_eq!(freshness["evidence"]["definition_matches"], true);
    assert_eq!(freshness["evidence"]["changed"], true);
    let metric_prior = json!({"resulting_knowledge_item_id":items[2]["id"],"resulting_revision_id":items[2]["current_revision"]["id"]});
    let corrected_metric = replace_named(
        &world,
        &changed,
        &metric_prior,
        "Iris acknowledged delivery rate",
        "iris-metric-reviewed",
    )
    .await;
    let corrected_metric = iris_item(
        &world,
        corrected_metric["resulting_knowledge_item_id"]
            .as_str()
            .unwrap(),
    )
    .await;
    assert!(
        corrected_metric["current_revision"]["body_markdown"]
            .as_str()
            .unwrap()
            .contains("Include sandbox")
    );
    let old_procedure = json!({"resulting_knowledge_item_id":items[5]["id"],"resulting_revision_id":items[5]["current_revision"]["id"]});
    let five = replace_named(
        &world,
        &changed,
        &old_procedure,
        "Iris retry policy",
        "iris-retry-review",
    )
    .await;
    let approved = iris_item(
        &world,
        five["resulting_knowledge_item_id"].as_str().unwrap(),
    )
    .await;
    let procedure = json!({"item_id":approved["id"],"revision_id":approved["current_revision"]["id"],"content_hash":approved["current_revision"]["content_hash"]});
    let l = procedure_world::setup_world(world, Some(procedure)).await;
    let baseline = approved["current_revision"]["body_markdown"]
        .as_str()
        .unwrap();
    let observed = iris_retry_case(baseline, false, 120);
    assert_eq!(observed["passed"], false);
    let mut body = procedure_world::outcome_body(&l, "iris-executed-retry");
    body["environment"]["environment_digest"] = json!(iris_digest(
        &json!({"harness":"closed-iris-retry-v1","tree":refreshed["pinned_revision"]})
    ));
    body["result_digest"] = json!(iris_digest(&observed));
    let (status, outcome) = call(
        &l.world.app,
        "POST",
        &format!("/v1/sessions/{}/outcomes", l.world.alice_session),
        &l.runner,
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{outcome}");
    let (_, candidate) = procedure_world::candidate(&l, &outcome["event"]).await;
    let attempt = procedure_world::freeze(&l, &candidate).await;
    let content = candidate["content"]["body_markdown"].as_str().unwrap();
    let mut results = Vec::new();
    for case in l.suite["cases"].as_array().unwrap() {
        let unknown = case["case_id"] == "held-out-unknown-commit";
        let before = iris_retry_case(baseline, unknown, 350);
        let after = iris_retry_case(content, unknown, 350);
        assert_eq!(after["passed"], true);
        results.push(json!({"case_id":case["case_id"],"digest":case["digest"],"baseline":if before["passed"]==true{"passed"}else{"failed"},"candidate":"passed","result_digest":iris_digest(&json!({"baseline":before,"candidate":after}))}));
    }
    let (status,qualified)=call(&l.world.app,"POST",&format!("/v1/procedure-evaluations/{}/results",attempt["attempt_id"].as_str().unwrap()),&l.evaluator,None,
        Some(json!({"candidate_hash":attempt["candidate_hash"],"specification_hash":attempt["specification_hash"],"results":results}))).await;
    assert_eq!(status, StatusCode::OK, "{qualified}");
    assert_eq!(qualified["qualified"], true);
    let (status, pending) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/procedure-submit",
            candidate["id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        Some("iris-qualified-improvement"),
        Some(json!({"candidate_hash":candidate["content_hash"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["candidate"]["resulting_outcome"], "pending_review");
    procedure_world::review(
        &l,
        pending["candidate"]["resulting_change_id"]
            .as_str()
            .unwrap(),
    )
    .await;
    let learned = iris_item(&l.world, l.procedure["item_id"].as_str().unwrap()).await;
    assert_eq!(
        learned["current_revision"]["content_hash"],
        candidate["content_hash"]
    );
    // A new Session owns the replacement head; the previous Session snapshot
    // deliberately cannot silently fall back after exact-head advancement.
    let (status,new_session)=call(&l.world.app,"POST","/v1/sessions",&l.world.alice_token,Some("iris-new-state-session"),Some(json!({"workspace_id":l.world.workspace_id,"project_id":l.world.project_id,"client_name":"iris-fixture"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{new_session}");
    let mut world = l.world;
    world.alice_session = new_session["id"].as_str().unwrap().to_owned();
    let new_state = iris_state(&world, &learned, "iris-learned-state").await;
    let handoff_request = json!({"budget_tokens":2600,"handoff":{"source_session_id":world.alice_session,"snapshot_event_id":new_state["id"],"payload_hash":new_state["payload_hash"]}});
    let receiver = format!("/v1/sessions/{}/context-runs", world.bob_session);
    let (status, delivered) = call(
        &world.app,
        "POST",
        &receiver,
        &world.bob_token,
        Some("iris-final-handoff"),
        Some(handoff_request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{delivered}");
    assert!(delivered["rendered"].as_str().unwrap().contains("200 ms"));
    let source = artifact(&changed, "procedures/retries.md");
    let (status,removal)=call(&world.app,"POST",&format!("/v1/okf/imports/{}/artifacts/{}/forget",changed["job"]["id"].as_str().unwrap(),source["id"].as_str().unwrap()),&world.alice_token,Some("iris-forget-retry"),Some(json!({"expected_content_hash":source["content_hash"],"reason":"Retire owned Iris retry source"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removal}");
    if removal["outcome"] == "pending_review" {
        apply_change(&world, removal["change_id"].as_str().unwrap()).await;
    }
    for key in ["iris-final-handoff", "iris-after-forget"] {
        let (status, denied) = call(
            &world.app,
            "POST",
            &receiver,
            &world.bob_token,
            Some(key),
            Some(handoff_request.clone()),
        )
        .await;
        assert!(
            matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
            "{denied}"
        );
        assert!(!denied.to_string().contains("200 ms"));
    }
    let (status, control) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/okf/imports/{}/artifacts/{}/source-control",
            changed["job"]["id"].as_str().unwrap(),
            source["id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{control}");
    for _ in 0..30 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "iris-owned-cleanup",
        )
        .await
        .unwrap()
        {
            break;
        }
    }
    let (status, operation) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/operations/{}",
            control["operation_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operation}");
    assert_eq!(operation["cleanup_state"], "succeeded");
    let unchanged = refresh_source(&world, "source-a", "iris-no-resurrection").await;
    assert!(unchanged["import_job_id"].is_null(), "{unchanged}");
    let survivor = iris_item(&world, items[0]["id"].as_str().unwrap()).await;
    assert_eq!(
        survivor["current_revision"]["id"],
        items[0]["current_revision"]["id"]
    );
    if let Ok(path) = std::env::var("SYNVEDA_IRIS_REPORT") {
        let report = json!({"schema_version":1,"scope":"owned_iris_fixture","source_revision":refreshed["pinned_revision"],"source_items":6,"required_revisions":3,
            "cli_preview":{"tokens":cli["tokens"],"token_count_kind":cli["token_count_kind"],"encoding":"o200k_base","budget_tokens":cli["budget_tokens"],"rendered_hash":cli["rendered_hash"]},
            "fixed_cases_passed":2,"cleanup_state":operation["cleanup_state"],"independent_survivor":true,"elapsed_ms":started.elapsed().as_millis(),"provider_usage":"unavailable","cost":"unavailable","native_vendor_execution":"not_run"});
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!(
        "Iris: six source-backed items, separate review, real CLI preview, fixed local runner cases, exact-root forgetting and independent survivor; provider usage/cost unavailable"
    );
}

#[tokio::test]
async fn health_definition_drift_cannot_hide_a_later_changed_source_or_disclose_denied_sources() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    let fixture = fixture(&mut world).await;
    let a = refresh_source(&world, "source-a", "health-a").await;
    let a = accept(
        &world,
        &job(&world, &a).await,
        "Iris retry policy",
        "health-a-accept",
    )
    .await;
    let b = refresh_source(&world, "source-b", "health-b").await;
    let b = accept(
        &world,
        &job(&world, &b).await,
        "Iris tracing",
        "health-b-accept",
    )
    .await;
    let (status,run)=call(&world.app,"POST",&format!("/v1/sessions/{}/context-runs",world.alice_session),&world.alice_token,Some("health-two-sources"),Some(json!({"query":"Iris retry policy","budget_tokens":2000,"required_knowledge_revisions":[{"item_id":a["resulting_knowledge_item_id"],"revision_id":a["resulting_revision_id"]},{"item_id":b["resulting_knowledge_item_id"],"revision_id":b["resulting_revision_id"]}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    // Trace identities are minted before ranking. Freeze their actual public
    // order, then make the first source unknown and the later source changed.
    let before = detail(&world, &world.alice_token, run["id"].as_str().unwrap()).await;
    let first_a = before["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["knowledge_revision_id"] == a["resulting_revision_id"])
        .unwrap();
    let first_b = before["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["knowledge_revision_id"] == b["resulting_revision_id"])
        .unwrap();
    let (drift_id, drift_branch, changed_id) = if first_a < first_b {
        ("source-a", "main", "source-b")
    } else {
        ("source-b", "dev", "source-a")
    };
    let mut drifted = descriptor(fixture.repository, drift_id, drift_branch);
    drifted.allowed_paths.push("missing.md".into());
    drifted.allowed_paths.sort();
    let mut sources = vec![
        descriptor(fixture.repository, "source-a", "main"),
        descriptor(fixture.repository, "source-b", "dev"),
    ];
    *sources
        .iter_mut()
        .find(|d| d.source_id == drift_id)
        .unwrap() = drifted;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::set_git_sources(&mut tx, world.tenant_id, world.project_scope, sources)
        .await;
    tx.commit().await.unwrap();
    if changed_id == "source-a" {
        change(&fixture, "main", 5);
    } else {
        git(&fixture.work, &["checkout", "dev"]);
        std::fs::write(fixture.work.join("conventions/trace.md"),"---\ntype: Convention\ntitle: Iris tracing\ndescription: Changed fictional tracing.\n---\n\nUse W3C traceparent and a span id for Iris requests.\n").unwrap();
        git(&fixture.work, &["add", "."]);
        git(
            &fixture.work,
            &["commit", "-m", "owned health trace change"],
        );
        git(
            &fixture.work,
            &["push", fixture.bare.to_str().unwrap(), "dev:dev"],
        );
    }
    refresh_source(&world, changed_id, "health-current-changed").await;
    let inspected = detail(&world, &world.alice_token, run["id"].as_str().unwrap()).await;
    let first = inspected["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["knowledge_revision_id"] == a["resulting_revision_id"])
        .unwrap();
    let second = inspected["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["knowledge_revision_id"] == b["resulting_revision_id"])
        .unwrap();
    assert!(
        (first < second) == (drift_id == "source-a"),
        "the unknown source must precede the attention source in this regression"
    );
    let freshness = inspected["health"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["dimension"] == "freshness")
        .unwrap();
    assert_eq!(freshness["status"], "needs_attention", "{inspected}");
    assert_eq!(freshness["evidence"]["source_id"], changed_id);
    assert_eq!(freshness["evidence"]["definition_matches"], true);
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let mut sources = synveda_store::configuration::effective_at_scope(
        &mut tx,
        world.tenant_id,
        world.project_scope,
    )
    .await
    .unwrap()
    .document
    .git_sources;
    let path = if changed_id == "source-a" {
        "procedures/retries.md"
    } else {
        "conventions/trace.md"
    };
    sources
        .iter_mut()
        .find(|d| d.source_id == changed_id)
        .unwrap()
        .allowed_paths
        .retain(|p| p != path);
    configuration_support::set_git_sources(&mut tx, world.tenant_id, world.project_scope, sources)
        .await;
    tx.commit().await.unwrap();
    refresh_source(&world, changed_id, "health-narrower-coverage").await;
    let narrowed = detail(&world, &world.alice_token, run["id"].as_str().unwrap()).await;
    let freshness = narrowed["health"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["dimension"] == "freshness")
        .unwrap();
    assert_eq!(freshness["status"], "unknown", "{narrowed}");
    assert!(freshness["evidence"]["path_state"].is_null());
    world.install_policy("health-source-deny",1,&format!("{BLANKET}\nforbid (principal, action == Synveda::Action::\"SourceRead\", resource);"),PackConfig::default()).await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::set_source_policy(
        &mut tx,
        world.tenant_id,
        world.project_scope,
        "health-source-deny",
    )
    .await;
    tx.commit().await.unwrap();
    let hidden = detail(&world, &world.alice_token, run["id"].as_str().unwrap()).await;
    assert!(!hidden["health"].to_string().contains("source-a"));
    assert!(!hidden["health"].to_string().contains("source-b"));
}
