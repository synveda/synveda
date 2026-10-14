//! MEM-9 real Git → ordinary source operations → Capture → Knowledge.
#![cfg(unix)]
#[path = "support/context_world.rs"]
mod context_world;
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
    world.state.git_sources = Some(Arc::clone(&reader));
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
async fn refresh(world: &World, fixture: &Fixture, source: &str, key: &str) -> Value {
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
        Arc::clone(&fixture.reader),
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
        .find(|c| c["content"]["title"] == "Iris retry policy")
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
        Arc::clone(&fixture.reader),
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
        Arc::clone(&fixture.reader),
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
