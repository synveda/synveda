//! ADPT-4: actual language clients and captured Claude hooks, ordinary tenant
//! roles and policy packs. This is gateway acceptance, not a live IdP/client claim.
use super::*;
use opentelemetry::trace::TracerProvider as _;
use tokio::io::AsyncWriteExt as _;
use tracing_subscriber::prelude::*;

const ALLOWED: &str = "Interoperability allowed context marker";
const PENDING: &str = "Interoperability unpublished proposal marker";

#[tokio::test]
#[ignore = "requires built CLI/Claude/SDKs and Python dependencies; run make interop-acceptance"]
async fn shared_authenticated_harness_python_typescript_workflow() {
    let _serial = serial().await;
    let world = world()
        .await
        .expect("interop acceptance requires the exact-role Docker fixture");
    telemetry::install_propagator();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("sdk-acceptance")));
    let _guard = tracing::subscriber::set_default(subscriber);
    let cli = CliSync::start(&world).await;
    let (skill, version) = install_skill(&world).await;
    let (status, binding) = call(
        &world.app,
        Method::POST,
        "/v1/skill-bindings",
        &world.alice,
        Some(json!({"scope_id": world.project, "skill_id": skill,
            "pinned_version_id": version, "enabled": true})),
        Some("interop-bind"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    approve_and_apply(
        &world,
        binding["change_id"].as_str().expect("binding change"),
    )
    .await;
    let (status, knowledge) = call(&world.app, Method::POST, "/v1/knowledge", &world.alice,
        Some(json!({"scope_id": world.project, "project_id": world.project_id, "knowledge_type": "convention",
            "content": {"title": "Interoperability", "summary": ALLOWED, "body_markdown": ALLOWED,
                "confidence_permille": 950, "sensitivity": "internal"}})), Some("interop-seed")).await;
    assert_eq!(status, StatusCode::CREATED, "{knowledge}");
    assert_eq!(knowledge["outcome"], "applied");
    let (denied, auditor) = isolation_and_review(&world).await;
    let (_, me) = call(&world.app, Method::GET, "/v1/me", &world.alice, None, None).await;
    let project = me["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .find(|project| project["id"] == world.project_id.to_string())
        .expect("selected project");
    let workspace = project["workspace_id"].as_str().expect("workspace");
    let session = start_harness(&world, &cli, workspace).await;
    let scenario = json!({"gateway": cli.url, "session_id": session,
        "scope_id": world.project, "project_id": world.project_id, "workspace_id": workspace,
        "skill_id": skill, "version_id": version, "knowledge_id": knowledge["knowledge_item_id"],
        "denied_session_id": denied, "allowed_marker": ALLOWED, "proposal_marker": PENDING,
        "query": "Interoperability", "run_key": world.tenant.id.to_string()});
    std::fs::write(cli.root.join("scenario.json"), scenario.to_string()).expect("scenario file");
    private_token(&cli.root.join("member.token"), &world.alice);
    private_token(&cli.root.join("auditor.token"), &auditor);
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (program, script) in [
        ("node".to_owned(), "sdks/typescript/dist/workflow.mjs"),
        (
            std::env::var("SYNVEDA_PYTHON").unwrap_or_else(|_| "python3".to_owned()),
            "sdks/python/workflow.py",
        ),
    ] {
        let mut command = tokio::process::Command::new(program);
        command
            .arg(repo.join(script))
            .arg(cli.root.join("scenario.json"))
            .env("SYNVEDA_TOKEN_FILE", cli.root.join("member.token"))
            .env("SYNVEDA_AUDITOR_TOKEN_FILE", cli.root.join("auditor.token"))
            .env("PYTHONPATH", repo.join("sdks/python"))
            .kill_on_drop(true);
        let result = run(&mut command).await;
        assert_eq!(result["session_id"], session);
        assert!(result["proposal_id"].is_string());
    }
    // Content checks and chain verification cover every row, including the
    // proposal and deny paths outside the Session-filtered SDK page.
    let (_, audit) = call(
        &world.app,
        Method::GET,
        "/v1/audit/events?limit=1000",
        &auditor,
        None,
        None,
    )
    .await;
    assert!(
        audit["events"]
            .as_array()
            .is_some_and(|events| !events.is_empty()),
        "{audit}"
    );
    assert!(!audit.to_string().contains(ALLOWED));
    assert!(!audit.to_string().contains(PENDING));
    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("audit transaction");
    let chain = synveda_audit::verify(&mut tx, world.tenant.id)
        .await
        .expect("verify chain");
    assert!(matches!(
        chain,
        synveda_audit::ChainVerification::Valid { .. }
    ));
    tx.commit().await.expect("finish audit read");
}

async fn isolation_and_review(world: &World) -> (String, String) {
    let mut tx = rls::begin_tenant_tx(&world.pool, world.tenant.id)
        .await
        .expect("fixture transaction");
    let root = scopes::ensure_tenant_root(&mut tx, world.tenant.id)
        .await
        .expect("tenant root");
    let foreign = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: world.tenant.id,
            slug: "unrelated-workspace".to_owned(),
            display_name: "Unrelated workspace".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("foreign workspace");
    // Reuse the maintained strict pack to make SDK Knowledge writes proposals.
    configuration_support::bind_tenant_pack(
        &mut tx,
        world.tenant.id,
        synveda_policy::REGULATED_STRICT,
    )
    .await;
    tx.commit()
        .await
        .expect("commit fixture placement and policy");
    user(&world.pool, world.tenant.id, "auditor").await;
    user(&world.pool, world.tenant.id, "other-member").await;
    grant(
        &world.pool,
        world.tenant.id,
        root.id,
        "auditor",
        RoleKey::Administrator,
    )
    .await;
    grant(
        &world.pool,
        world.tenant.id,
        foreign.scope_id,
        "other-member",
        RoleKey::Member,
    )
    .await;
    let token = issue("other-member", world.tenant.id);
    let (status, session) = call(
        &world.app,
        Method::POST,
        "/v1/sessions",
        &token,
        Some(json!({"workspace_id": foreign.id, "client_name": "sdk-fixture"})),
        Some("foreign-open"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    (
        session["id"].as_str().expect("foreign Session").to_owned(),
        issue("auditor", world.tenant.id),
    )
}

async fn start_harness(world: &World, cli: &CliSync, workspace: &str) -> String {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes =
        std::fs::read(repo.join("adapters/claude-code/fixtures/hooks/session-start-startup.json"))
            .expect("authentic Claude fixture");
    let mut frame: Value = serde_json::from_slice(&bytes).expect("captured JSON");
    frame["session_id"] = json!(world.tenant.id.to_string());
    frame["cwd"] = json!(cli.root);
    frame["transcript_path"] = json!(cli.root.join("transcript.jsonl"));
    let mut command = tokio::process::Command::new("node");
    command
        .arg(repo.join("adapters/claude-code/dist/hook.mjs"))
        .arg("session-start")
        .env("SYNVEDA_TOKEN", &world.alice)
        .env("SYNVEDA_GATEWAY", &cli.url)
        .env("SYNVEDA_WORKSPACE", workspace)
        .env("SYNVEDA_PROJECT", world.project_id.to_string())
        .env("XDG_STATE_HOME", cli.root.join("state"))
        .env("XDG_CONFIG_HOME", cli.root.join("config"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().expect("built Claude hook");
    child
        .stdin
        .take()
        .expect("hook stdin")
        .write_all(frame.to_string().as_bytes())
        .await
        .expect("captured frame");
    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("hook deadline")
        .expect("hook output");
    assert!(output.status.success());
    let output: Value = serde_json::from_slice(&output.stdout).expect("hook context JSON");
    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("delivered context");
    assert!(context.contains(ALLOWED), "{context}");
    context
        .lines()
        .next()
        .expect("Session handoff")
        .strip_prefix("Synveda Session ID: ")
        .and_then(|line| line.split_once('.'))
        .expect("explicit Session ID")
        .0
        .to_owned()
}

fn private_token(path: &std::path::Path, token: &str) {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .expect("private fixture token")
        .write_all(token.as_bytes())
        .expect("write fixture token");
}

async fn run(command: &mut tokio::process::Command) -> Value {
    let output = tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .expect("SDK deadline")
        .expect("run SDK example");
    assert!(
        output.status.success(),
        "SDK example failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("content-free SDK result")
}
