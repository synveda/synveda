//! Dormant pre-CPR-45 `synveda init` lifecycle (OPS-1, ADR-0055).
//!
//! The public entrypoint is unconditionally closed before profile discovery,
//! Compose, secret-file or database work while the canonical Docker reference
//! lifecycle is cut over to deployment-owned Keycloak and database authority.
//! The retained private implementation compiles so its useful boundary code can
//! be reused deliberately, but it is not an operator path or compatibility mode.
//!
//! The intended design is defined as much by what it *does not* do. It applies
//! migrations and admits a tenant, both of which are pre-existing audited
//! break-glass paths with no governed surface and no operator to run them
//! yet (ADR-0055 decision 1). It writes no scope, no identity, no grant and no
//! Knowledge. Those arrive the way a customer's do:
//!
//!   * the operator's **identity**, their own `principal` scope and — because
//!     `init` puts them in the `synveda-admins` IdP group — an
//!     **`administrator` grant at the tenant root**, on the first `synveda
//!     login`, inside AUTH-2's provisioning transaction and chained under the
//!     operator's own subject (CPR-7, ADR-0074 decision 4);
//!   * **workspaces, projects and org units** through the governed surfaces —
//!     `POST /v1/workspaces` and `synveda scope create`, a PDP decision each;
//!   * **sessions, capture candidates and Knowledge** through the generated
//!     public application API, with publication passing through VedaFlow.
//!
//! There is no placement convention and no role binding left for it to write:
//! CPR-7 deleted both, so an identity's scope is its own and everything else
//! is a grant (ADR-0074 decision 3).
//!
//! That is not fastidiousness. An installer runs once, as root-equivalent,
//! before anybody is watching, and whatever it writes becomes the tenant's
//! history — so it is the single worst place in this product to keep a
//! shortcut past the PDP. It is also why the
//! bootstrap this replaces needed a *second gateway* running with
//! `SYNVEDA_DEV_JWT_SECRET` just to create three nodes, and why nothing
//! here does: the gateway starts once, in OIDC mode, and the dev secret
//! never appears on the install path (ADR-0010 decision 3).

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synveda_types::{TenantId, TenantStatus};
use zeroize::Zeroizing;

/// The bundled dev IdP, as `deploy/compose/docker-compose.yml` publishes
/// it. Dev-only credentials; the API key is the one in
/// `deploy/compose/rauthy/config.toml`, which exists precisely so a
/// bootstrap has something to authenticate with before a human does.
const RAUTHY_URL: &str = "http://localhost:8100";
const RAUTHY_ISSUER: &str = "http://localhost:8100/auth/v1/";
const RAUTHY_API_KEY: &str =
    "API-Key synveda-dev$6xxmjZD7Wqe9zWN1fWzOW1jA4uxAkFQ9rYlVFpxBzVgJ0xEj2KWSLiaRTZzKV1oz";
const GATEWAY_URL: &str = "http://127.0.0.1:8120";
/// How to get an installed release, in the one place every message that
/// suggests it reads from. A raw GitHub URL rather than a vanity domain:
/// three of these messages pointed at `synveda.dev`, which does not
/// resolve, and an error that names a dead URL is worse than one that names
/// none (OPS-8).
const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh";
/// The one IdP group the product reads: its members are granted
/// `administrator` at the tenant root on every login (CPR-7, ADR-0074
/// decision 4, over AUTHZ-3's tenant-wide `org-admin` binding).
const ADMIN_GROUP: &str = "synveda-admins";

/// What one `init` was asked for.
pub struct Plan {
    pub slug: String,
    pub name: String,
    pub embedder: String,
    pub issuer: Option<String>,
    pub dry_run: bool,
}

/// Local-only credential for the operator created in the bundled IdP. It is
/// printed by `init`; it is not a production or customer secret.
const OPERATOR_PASSWORD: &str = "Synveda-Demo-Passw0rd!";

const DEFAULT_DATABASE_URL: &str = "postgres://synveda:synveda-dev@localhost:5432/synveda";
const MAX_DATABASE_URL_FILE_BYTES: u64 = 1_048_576;

struct InitDatabasePlan {
    roles: synveda_store::runtime_role::DatabaseRoles,
    roles_json: String,
    migrator_url: Zeroizing<String>,
    gateway_url: Zeroizing<String>,
    worker_url: Zeroizing<String>,
}

/// Settings owned by worker/bootstrap planes that a transitional host gateway
/// must not inherit from the operator shell. `Command::env_remove` records the
/// removal in the child environment without mutating the parent process.
const HOST_GATEWAY_REMOVED_ENV: &[&str] = &[
    "DATABASE_URL_FILE",
    "SYNVEDA_GATEWAY_DATABASE_URL",
    "SYNVEDA_GATEWAY_DATABASE_URL_FILE",
    "SYNVEDA_WORKER_DATABASE_URL",
    "SYNVEDA_WORKER_DATABASE_URL_FILE",
    "SYNVEDA_DATABASE_ROLES_FILE",
    "SYNVEDA_OIDC_ISSUERS_FILE",
    "SYNVEDA_DEV_JWT_SECRET",
    "SYNVEDA_DEV_JWT_SECRET_FILE",
    "SYNVEDA_KMS_KEY_FILE",
    "SYNVEDA_WORKER_DB_MAX_CONNECTIONS",
    "SYNVEDA_WORKER_LISTEN_ADDR",
    "SYNVEDA_WORKER_SHUTDOWN_SECS",
    "SYNVEDA_RELAXATION_SWEEP_SECS",
    "SYNVEDA_DIRECTORY_SYNC",
    "SYNVEDA_DIRECTORY_SYNC_INTERVAL_SECS",
    "SYNVEDA_KNOWLEDGE_EMBED_POLL_MS",
    "SYNVEDA_KNOWLEDGE_EMBED_BATCH",
    "SYNVEDA_CAPTURE_LEASE_OWNER",
    "SYNVEDA_CAPTURE_POLL_MS",
    "SYNVEDA_CAPTURE_LEASE_SECS",
    "SYNVEDA_CAPTURE_BATCHES_PER_TENANT",
    "SYNVEDA_EXTRACTOR",
    "SYNVEDA_EXTRACTOR_MODEL",
    "SYNVEDA_ANTHROPIC_BASE_URL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_API_KEY_FILE",
    "SYNVEDA_VLLM_BASE_URL",
];

pub async fn init(_plan: Plan) -> Result<(), String> {
    reference_cutover_gate()
}

#[allow(dead_code)]
async fn init_after_cutover(plan: Plan) -> Result<(), String> {
    let started = Instant::now();
    let profile = Profile::discover()?;
    profile.check_version()?;
    let compose_file = profile.compose_file();
    if !compose_file.exists() {
        return Err(format!(
            "no compose file at {} — the {} is incomplete",
            compose_file.display(),
            profile.describe(),
        ));
    }

    let bundled = plan.issuer.is_none();
    let issuer = plan
        .issuer
        .clone()
        .unwrap_or_else(|| RAUTHY_ISSUER.to_owned());

    // Database authority is deployment-owned. Resolve the same explicit
    // migrator/gateway/worker/role plan before dry-run or any Compose command;
    // init never infers or provisions PostgreSQL principals.
    let database = init_database_plan()?;

    if plan.dry_run {
        return dry_run(&plan, &profile, &issuer, bundled, &database);
    }

    // ── 1. the stack ────────────────────────────────────────────────────
    //
    // TEI is started only when it is the embedder. The default path never
    // downloads BGE-M3, which is the difference between an installer that
    // finishes in the acceptance criterion's ten minutes and one that
    // finishes when a 2.3 GB download does (ADR-0055 decision 5).
    let mut services = vec!["postgres", "jaeger"];
    if bundled {
        services.push("rauthy");
    }
    if plan.embedder == "tei" {
        services.push("tei");
    }
    step(1, "starting the stack");
    println!("    {}", profile.describe());
    println!("    {}", services.join(", "));
    // The architecture-correct TEI image, when this machine needs one and
    // the operator has not already chosen. Set on the compose invocation
    // rather than exported, so it cannot outlive the command that needed it.
    let mut environment: Vec<(&str, &str)> = Vec::new();
    if plan.embedder == "tei"
        && let Some(image) = tei_image()
    {
        environment.push(("SYNVEDA_TEI_IMAGE", image));
    }
    compose_with_env(
        &compose_file,
        &[&["up", "--detach", "--wait"], &services[..]].concat(),
        &environment,
    )?;

    // ── 2. schema ───────────────────────────────────────────────────────
    step(2, "applying migrations");
    let migrator_options =
        synveda_store::database_url::parse("DATABASE_URL", &database.migrator_url)
            .map_err(|error| error.to_string())?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(migrator_options)
        .await
        .map_err(|err| {
            format!(
                "connect to {}: {err}",
                redacted_database_url(&database.migrator_url)
            )
        })?;
    // The epoch guard before the migrator (CPR-2, ADR-0069), so that an
    // operator re-running `init` over a database from before the
    // context-platform cut is told to reset it — as itself, rather than
    // wrapped in "apply migrations: storage:" three layers down. It also
    // precedes cluster-role convergence: a refused database is not mutated.
    synveda_store::epoch::preflight(&pool)
        .await
        .map_err(|refusal| refusal.to_string())?;
    let schema = synveda_store::migrate_reporting(&pool, &database.roles)
        .await
        .map_err(|err| format!("apply migrations: {err}"))?;
    println!(
        "    schema epoch {}, baseline revision {} at migration {}",
        schema.epoch, schema.baseline_revision, schema.migration_head
    );
    let migrator_database = synveda_store::runtime_role::database_identity(&pool)
        .await
        .map_err(|error| format!("verify migrator database target: {error}"))?;
    let gateway_database = verify_runtime_database_url(
        "SYNVEDA_GATEWAY_DATABASE_URL",
        &database.gateway_url,
        database.roles.gateway(),
        &database.roles,
    )
    .await?;
    let worker_database = verify_runtime_database_url(
        "SYNVEDA_WORKER_DATABASE_URL",
        &database.worker_url,
        database.roles.worker(),
        &database.roles,
    )
    .await?;
    require_one_database_target(&migrator_database, &gateway_database, &worker_database)?;
    println!(
        "    gateway role {}: LOGIN, RLS-enforced, synveda_app member",
        database.roles.gateway()
    );
    println!(
        "    worker role {}: LOGIN, RLS-enforced, synveda_app member",
        database.roles.worker()
    );

    // ── 3. the tenant ───────────────────────────────────────────────────
    //
    // The one row this command writes that a governed surface does not
    // already own, and it audits itself as break-glass exactly as
    // `synveda tenant create` does — because it *is* that path.
    step(3, "admitting the tenant");
    // Scanned rather than queried by slug: `active` is the pack
    // refresher's own iteration set and is documented as fine at
    // admissible tenant counts, and an installer that runs once has no
    // business adding a query — and therefore a `.sqlx` cache entry — for
    // a lookup the store already answers.
    let admitted = synveda_store::tenants::active(&pool)
        .await
        .map_err(|err| format!("list tenants: {err}"))?
        .into_iter()
        .find(|tenant| tenant.slug == plan.slug);
    let tenant_id = match admitted {
        Some(existing) => {
            println!("    {} ({}) — already admitted", existing.slug, existing.id);
            existing.id
        }
        None => {
            let tenant = crate::create_tenant(
                &pool,
                &database.roles,
                &plan.slug,
                &plan.name,
                TenantStatus::Active,
            )
            .await
            .map_err(|err| {
                format!(
                    "admit tenant `{}`: {err}\n\
                         (a suspended tenant holds its slug but is not listed as active — \
                         `synveda tenant create --slug <other>` or resume that one)",
                    plan.slug
                )
            })?;
            println!("    {} ({})", tenant.slug, tenant.id);
            tenant.id
        }
    };

    // ── 4. the issuer ───────────────────────────────────────────────────
    step(4, "configuring the issuer");
    if bundled {
        converge_rauthy(&plan).await?;
        println!("    bundled Rauthy: client `synveda`, group `{ADMIN_GROUP}`");
    } else {
        println!("    {issuer} — external, nothing created in your directory");
        print_external_issuer_instructions(&issuer);
    }

    // ── 5. the product processes ────────────────────────────────────────
    step(5, "starting the gateway and worker");
    let kek = ensure_kek(&profile)?;
    // Beside the compose file, which is where compose's `env_file` looks
    // and where its own variable substitution reads a `.env` from.
    let env_file = compose_file.with_file_name(".env");
    write_env_file(&env_file, &plan, tenant_id, &issuer, &kek, Some(&database))?;
    println!("    configuration: {}", env_file.display());
    let mut product_services = vec!["worker"];
    if containerised(&plan) {
        product_services.push("gateway");
    }
    // The worker never contacts the bundled localhost issuer. It can
    // therefore use the product container even during the temporary Rauthy
    // host-gateway shape, preserving one worker process contract.
    let mut up = vec!["up", "--detach", "--wait"];
    if profile.may_build() {
        up.push("--build");
    }
    up.extend(product_services);
    compose(&compose_file, &up)?;
    println!("    worker: container healthy");

    let gateway = if containerised(&plan) {
        // `--build` only where there is something to build from. A release
        // bundle's compose file has no build context by construction, and
        // asking compose to build one is an error rather than a no-op.
        // Compose owns liveness here: `--wait` already gates on the
        // container's own health check, and there is no host pid to watch.
        None
    } else {
        Some(start_host_gateway(
            &profile, &plan, tenant_id, &issuer, &kek, &database,
        )?)
    };
    wait_for_health(
        &format!("{GATEWAY_URL}/healthz"),
        Duration::from_secs(90),
        gateway.as_ref(),
    )
    .await?;
    println!("    {GATEWAY_URL} healthy");

    let elapsed = started.elapsed();
    println!();
    println!("synveda: initialised in {}s", elapsed.as_secs());
    println!();
    println!("Next, and this is where the workspace starts to exist:");
    println!();
    println!("    synveda login --gateway {GATEWAY_URL}");
    println!();
    println!("That first login provisions your identity and your own scope, and — because");
    println!("you are in the `{ADMIN_GROUP}` group — grants you `administrator` at the root");
    println!(
        "of tenant `{}`, all of it audited under your own subject",
        plan.slug
    );
    println!("rather than an installer's. Then:");
    println!();
    println!("    synveda scope tree");
    println!("    # create and bind a governed personal/team/enterprise Configuration");
    println!("    # under Advanced > Configuration, then create a workspace and project");
    if plan.embedder == "deterministic" {
        println!();
        println!("Embedder: deterministic (hash). Retrieval works and is exact for the");
        println!("lexical leg; semantic similarity is not meaningful. `--embedder tei`");
        println!("serves BGE-M3. Knowledge embeddings retain their model and dimension,");
        println!("and a model change converges a separately labelled index sidecar.");
    }
    Ok(())
}

fn reference_cutover_gate() -> Result<(), String> {
    Err(
        "synveda init is unavailable during the CPR-45 Docker reference cutover; it refuses before profile discovery, Compose, secret-file or database mutation. Validate the additive topology with `make compose-config`; a turnkey init returns only after clean-volume Keycloak, exact-issuer and database-authority acceptance passes"
            .to_owned(),
    )
}

fn dry_run(
    plan: &Plan,
    profile: &Profile,
    issuer: &str,
    bundled: bool,
    _database: &InitDatabasePlan,
) -> Result<(), String> {
    println!("synveda init --dry-run");
    println!();
    println!("  profile        {}", profile.describe());
    println!("  compose file   {}", profile.compose_file().display());
    println!(
        "  gateway        {}",
        match gateway_binary(profile) {
            Ok(path) => path.display().to_string(),
            Err(_) => "none found — see the message `synveda init` would print".to_owned(),
        }
    );
    println!(
        "  console        {}",
        match profile.console_dir() {
            Some(path) => path.display().to_string(),
            None => "no bundle — /console/ would 404 (ADR-0056 decision 1)".to_owned(),
        }
    );
    println!("  tenant         {} ({})", plan.slug, plan.name);
    println!("  embedder       {}", plan.embedder);
    println!(
        "  issuer         {issuer}{}",
        if bundled {
            " (bundled Rauthy)"
        } else {
            " (external)"
        }
    );
    println!();
    println!(
        "  would start    postgres, jaeger{}, gateway, worker{}",
        if bundled { ", rauthy" } else { "" },
        if plan.embedder == "tei" { ", tei" } else { "" },
    );
    println!("  would write    migrations and one tenant row");
    println!("  would verify   explicit migrator, gateway and worker database principals");
    println!("  would NOT      write any scope, identity, grant, Configuration or Knowledge");
    Ok(())
}

fn print_external_issuer_instructions(issuer: &str) {
    println!();
    println!("    Register this client in your directory, then re-run `synveda init`:");
    println!();
    println!("      client_id      synveda");
    println!("      type           public (PKCE S256), no client secret");
    println!("      redirect_uri   {GATEWAY_URL}/auth/callback");
    println!("      grants         authorization_code, refresh_token");
    println!("      scopes         openid, profile, email, groups");
    println!("      issuer         {issuer}");
    println!();
    println!("    One group claim is read: a `{ADMIN_GROUP}` member is granted");
    println!("    `administrator` at the tenant root on every login. There is no");
    println!("    placement convention — everybody arrives at their own scope and");
    println!("    reaches anything else through a grant. This command configures an");
    println!("    issuer; it does not synchronise a directory.");
    println!();
    println!("    For joiners, movers and leavers, issue a provisioning credential");
    println!("    once the instance is up:");
    println!();
    println!("      synveda scim token issue --label entra");
    println!();
    println!("    then point your IdP at {GATEWAY_URL}/scim/v2. For Entra, also set");
    println!("    `external_id_claim` to `oid` on the issuer — its `sub` is pairwise");
    println!("    per application and will not match what its provisioning agent sends.");
}

// ── the bundled IdP ─────────────────────────────────────────────────────

/// Converges the client, the admin group and the operator in the bundled
/// Rauthy. Desired state, not creation: a second `init` must change
/// nothing (ADR-0055 decision 7).
async fn converge_rauthy(plan: &Plan) -> Result<(), String> {
    let http = idp_client()?;
    wait_for_health(
        &format!("{RAUTHY_URL}/auth/v1/.well-known/openid-configuration"),
        Duration::from_secs(120),
        // Rauthy is compose's container, not a process this CLI spawned.
        None,
    )
    .await?;

    let client = json!({
        "id": "synveda",
        "name": "Synveda Gateway",
        "enabled": true,
        "confidential": false,
        "redirect_uris": [format!("{GATEWAY_URL}/auth/callback")],
        "post_logout_redirect_uris": [],
        // `refresh_token` is how Rauthy grants what ADR-0027 decision 6
        // needs: a login the CLI can keep alive without holding client
        // credentials. It does not advertise `offline_access` in
        // discovery, so the gateway never asks for that scope.
        "flows_enabled": ["authorization_code", "refresh_token"],
        "access_token_alg": "RS256",
        "id_token_alg": "RS256",
        "auth_code_lifetime": 60,
        "access_token_lifetime": 1800,
        "scopes": ["openid", "email", "profile", "groups"],
        "default_scopes": ["openid"],
        "challenges": ["S256"],
        "force_mfa": false,
    });
    if idp_get(&http, "/auth/v1/clients/synveda").await?.is_none() {
        idp_send(&http, reqwest::Method::POST, "/auth/v1/clients", &client).await?;
    }
    idp_send(
        &http,
        reqwest::Method::PUT,
        "/auth/v1/clients/synveda",
        &client,
    )
    .await?;

    ensure_group(&http, ADMIN_GROUP).await?;
    let operator = operator_email(&plan.slug);
    ensure_user(&http, &operator, "Synveda", "Operator", ADMIN_GROUP).await?;
    println!("    operator: {operator}  (password {OPERATOR_PASSWORD})");
    Ok(())
}

fn idp_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| format!("build the HTTP client: {err}"))
}

async fn ensure_group(http: &reqwest::Client, group: &str) -> Result<(), String> {
    let existing = idp_get(http, "/auth/v1/groups")
        .await?
        .unwrap_or(Value::Null);
    let present = existing
        .as_array()
        .is_some_and(|groups| groups.iter().any(|g| g["name"] == json!(group)));
    if !present {
        idp_send(
            http,
            reqwest::Method::POST,
            "/auth/v1/groups",
            &json!({"group": group}),
        )
        .await?;
    }
    Ok(())
}

/// Creates or converges one IdP user.
///
/// The password is set on creation and **not** on convergence: Rauthy
/// refuses a password it has seen in the last three, which is exactly the
/// state a second `init` is in, and a re-run that fails on its own
/// idempotence is not idempotent (ADR-0055 decision 7).
///
/// **That fallback is only correct while this command is the sole writer of
/// the address.** It reads "Rauthy would not take the password" as "the
/// password is already the one I want", and those are the same fact only if
/// nothing else has set a different one in between. The slug-derived
/// [`operator_email`] makes the address deployment-local. Deleting and
/// recreating the operator would mint a new `sub` and orphan the
/// `administrator` grant keyed by that subject (ADR-0072 decision 4).
async fn ensure_user(
    http: &reqwest::Client,
    email: &str,
    given: &str,
    family: &str,
    group: &str,
) -> Result<(), String> {
    let users = idp_get(http, "/auth/v1/users")
        .await?
        .unwrap_or(Value::Null);
    let existing = users.as_array().and_then(|users| {
        users
            .iter()
            .find(|user| user["email"] == json!(email))
            .and_then(|user| user["id"].as_str().map(str::to_owned))
    });
    let state = json!({
        "email": email,
        "given_name": given,
        "family_name": family,
        "language": "en",
        "roles": [],
        "groups": [group],
        "enabled": true,
        "email_verified": true,
    });
    let id = match existing {
        Some(id) => id,
        None => {
            // Created *without* a password, then given one by the update
            // below. A create that carries one leaves Rauthy with a user
            // whose credentials do not authenticate — measured, and the
            // sequence ADPT-1's estate has always used.
            idp_send(http, reqwest::Method::POST, "/auth/v1/users", &state).await?;
            idp_get(http, "/auth/v1/users")
                .await?
                .unwrap_or(Value::Null)
                .as_array()
                .and_then(|users| {
                    users
                        .iter()
                        .find(|user| user["email"] == json!(email))
                        .and_then(|user| user["id"].as_str().map(str::to_owned))
                })
                .ok_or_else(|| format!("the bundled IdP created {email} but does not list it"))?
        }
    };
    // Desired state, twice: with the password, and — if that is refused —
    // without it. Rauthy declines a password it has seen in the last
    // three, which is exactly the state a second `init` is in, and a
    // re-run that fails on its own idempotence is not idempotent
    // (ADR-0055 decision 7).
    let mut with_password = state.clone();
    with_password["password"] = json!(OPERATOR_PASSWORD);
    let path = format!("/auth/v1/users/{id}");
    if idp_send(http, reqwest::Method::PUT, &path, &with_password)
        .await
        .is_err()
    {
        idp_send(http, reqwest::Method::PUT, &path, &state).await?;
    }
    Ok(())
}

/// `GET path`, with `None` for a 404 — the shape "is this already there".
async fn idp_get(http: &reqwest::Client, path: &str) -> Result<Option<Value>, String> {
    let response = http
        .get(format!("{RAUTHY_URL}{path}"))
        .header("Authorization", RAUTHY_API_KEY)
        .send()
        .await
        .map_err(|err| format!("GET {RAUTHY_URL}{path}: {err}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(idp_refusal("GET", path, status, &body));
    }
    Ok(serde_json::from_str(&body).ok())
}

async fn idp_send(
    http: &reqwest::Client,
    method: reqwest::Method,
    path: &str,
    body: &Value,
) -> Result<(), String> {
    let response = http
        .request(method.clone(), format!("{RAUTHY_URL}{path}"))
        .header("Authorization", RAUTHY_API_KEY)
        .json(body)
        .send()
        .await
        .map_err(|err| format!("{method} {RAUTHY_URL}{path}: {err}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    Err(idp_refusal(method.as_str(), path, status, &text))
}

fn idp_refusal(method: &str, path: &str, status: reqwest::StatusCode, body: &str) -> String {
    let detail = body.trim();
    if detail.is_empty() {
        format!("{method} {path}: the bundled IdP answered {status}")
    } else {
        format!("{method} {path}: the bundled IdP answered {status}: {detail}")
    }
}

/// The operator's login for a bundled-IdP deployment. Derived from the
/// tenant slug so two deployments on one laptop do not share an operator.
fn operator_email(slug: &str) -> String {
    format!("operator@{slug}.localhost")
}

// ── compose, environment, waiting ───────────────────────────────────────

/// Whether the gateway runs as a container or as a host process
/// (ADR-0055 decision 9).
///
/// The container is the better deployment shape and it is what OPS-2 will
/// ship — but it cannot serve the **bundled** IdP, and the reason is not
/// fixable by configuration. An OIDC issuer identifier is one URL that
/// both the browser and the gateway must reach, compared byte-for-byte
/// against the discovery document and the `iss` claim (`IssuerConfig`
/// carries no separate discovery URL, deliberately). The bundled Rauthy's
/// is `http://localhost:8100/auth/v1/`, and RFC 6761 makes every resolver
/// answer `localhost` — and any `*.localhost` name — with the *container's
/// own* loopback, ahead of DNS and ahead of `/etc/hosts`. A container
/// therefore cannot be made to reach it: not with `extra_hosts`, not with
/// `host-gateway` (measured: the address is right and the name still
/// resolves to 127.0.0.1), not with a network alias.
///
/// A real issuer has no such problem — an Entra or Okta URL resolves the
/// same everywhere — so `--issuer` runs the container, and the bundled
/// profile runs the binary on the host, which is also where every demo in
/// this repository has always run it.
fn containerised(plan: &Plan) -> bool {
    plan.issuer.is_some()
}

/// Starts the gateway as a detached host process and waits for it.
///
/// Deliberately small: a pidfile, a log, and a process group of its own so
/// that Ctrl-C on the installer does not take the deployment down with it.
/// An installer for a single-node profile should not be a service manager,
/// and on the day it needs to be, the answer is a launchd/systemd unit
/// rather than more of this function.
fn start_host_gateway(
    profile: &Profile,
    plan: &Plan,
    tenant_id: TenantId,
    issuer: &str,
    kek: &str,
    database: &InitDatabasePlan,
) -> Result<Started, String> {
    let state = profile.state_dir();
    std::fs::create_dir_all(&state).map_err(|err| format!("create {}: {err}", state.display()))?;
    let pidfile = state.join("gateway.pid");
    let logfile = state.join("gateway.log");
    let configfile = state.join("gateway.config");

    let issuers = issuers_json(tenant_id, issuer);
    // What this gateway would be started with. Convergence compares it
    // against what the running one *was* started with, because "already
    // running" is not the same as "running what you just configured" —
    // a re-run that admits a different tenant and then leaves the old
    // process up serves the old tenant, silently, and the login that
    // follows lands somewhere nobody asked for. Measured the hard way.
    // The KEK joins the fingerprint as a *hash*, never as itself: this
    // string is written to `gateway.config` beside the pidfile, and a
    // second copy of the key on disk is a second place to lose it from.
    // Not a cryptographic requirement — it only has to change when the key
    // changes, so that a deployment which was running without one restarts
    // to pick it up instead of reporting "already running with this
    // configuration" and leaving the console unusable.
    // And the **binary**, because an upgrade changes neither the
    // configuration nor the liveness this function was written to compare.
    // `install.sh` replaces `bin/synveda-gateway` and the next `init`
    // reported "already running with this configuration", healthy, while the
    // *previous release* kept serving — an upgrade that looks like it worked
    // and did not. Measured on the 0.1.0 → 0.1.1 upgrade: the process had
    // been running for two hours and the binary under it was minutes old.
    //
    // Modification time and length rather than a digest of the file: this
    // runs on every `init` and the gateway is tens of megabytes, so the
    // question is "did this artefact change" rather than "which artefact is
    // this". Reinstalling moves both.
    let stamp = binary_stamp(profile);
    let desired = format!(
        "{issuers}\n{}\n{GATEWAY_URL}\nkek:{}\ndatabase:{}\ngateway:{stamp}\n",
        plan.embedder,
        secret_fingerprint(kek),
        secret_fingerprint(&database.gateway_url),
    );
    match (
        running_gateway(&pidfile),
        std::fs::read_to_string(&configfile).ok(),
    ) {
        (Some(pid), Some(current)) if current == desired => {
            println!("    already running with this configuration (pid {pid})");
            return Ok(Started {
                pid,
                logfile: logfile.clone(),
            });
        }
        (Some(pid), _) => {
            println!("    configuration changed — restarting (pid {pid})");
            let _ = Command::new("kill").arg(pid.to_string()).status();
            // Wait for it to let go of the port rather than guess how long
            // that takes. A fixed sleep is wrong in both directions: too
            // short and the replacement cannot bind, too long and every
            // restart pays for the worst case. This mattered more once the
            // pre-flight below became a refusal — a gateway still shutting
            // down would have been reported as a stranger holding the port.
            wait_for_port_release(GATEWAY_URL, Duration::from_secs(10));
        }
        (None, _) => {}
    }

    // Nothing of ours holds the port, so it has to be free before we spawn
    // into it. When it is not, the occupant is another deployment — most
    // often the *installed* release while this is a checkout, a pair
    // ADR-0065 decision 5 deliberately lets coexist, or the reverse.
    //
    // This is a pre-flight rather than a liveness check because liveness
    // loses the race. The gateway completes database/schema, identity and
    // key-plane setup before it binds,
    // so the child is still alive for the first second — while the *stranger*
    // answers `/healthz`
    // immediately. Measured: `init` printed `pid 51544`, `healthy` and
    // `initialised in 6s`, exit 0, for a process that died with
    // `AddrInUse` and never bound anything. Watching our own pid narrows
    // that window; only asking for the port closes it.
    port_available(GATEWAY_URL)?;

    let binary = gateway_binary(profile)?;
    let log = std::fs::File::create(&logfile)
        .map_err(|err| format!("create {}: {err}", logfile.display()))?;
    let errors = log
        .try_clone()
        .map_err(|err| format!("open {}: {err}", logfile.display()))?;

    let mut command = Command::new(&binary);
    command
        .current_dir(profile.working_dir())
        .env("DATABASE_URL", &*database.gateway_url)
        .env("SYNVEDA_DATABASE_ROLES", &database.roles_json)
        .env("SYNVEDA_OIDC_ISSUERS", &issuers)
        .env("SYNVEDA_PUBLIC_URL", GATEWAY_URL)
        .env("SYNVEDA_LISTEN_ADDR", "127.0.0.1:8120")
        .env("SYNVEDA_EMBEDDER", &plan.embedder)
        // The key plane (TEN-4, ADR-0064). Without this the gateway boots
        // `Kms::Disabled` and the console cannot be signed in to, because a
        // console session seals its tokens under the deployment key.
        .env("SYNVEDA_KMS_KEY", kek)
        .stdout(log)
        .stderr(errors);
    sanitize_host_gateway_environment(&mut command);
    if plan.embedder == "tei" {
        command.env("SYNVEDA_TEI_URL", "http://localhost:8110");
    }
    // The console bundle (CNSL-1). The image has set this since ADR-0056 and
    // the host process never did, so it fell back to `console/dist` relative
    // to the working directory — which resolves only inside a checkout where
    // somebody has run `pnpm --filter @synveda/console build`. Every default
    // install has therefore been serving a 404 at `/console/`; a missing
    // bundle staying a 404 rather than a boot failure is what kept it quiet.
    if let Some(console) = profile.console_dir() {
        command.env("SYNVEDA_CONSOLE_DIR", &console);
        println!("    console: {}", console.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|err| format!("start {}: {err}", binary.display()))?;
    std::fs::write(&pidfile, child.id().to_string())
        .map_err(|err| format!("write {}: {err}", pidfile.display()))?;
    // Written after the spawn, so a failed start leaves no record claiming
    // this configuration is live.
    std::fs::write(&configfile, &desired)
        .map_err(|err| format!("write {}: {err}", configfile.display()))?;
    println!("    pid {} · log {}", child.id(), logfile.display());
    Ok(Started {
        pid: child.id(),
        logfile,
    })
}

fn sanitize_host_gateway_environment(command: &mut Command) {
    for setting in HOST_GATEWAY_REMOVED_ENV {
        command.env_remove(setting);
    }
}

/// A host gateway this `init` is responsible for: the pid to watch while
/// waiting for health, and the log to quote when it turns out to be gone.
struct Started {
    pid: u32,
    logfile: PathBuf,
}

impl Started {
    /// `Ok` while the process exists; otherwise an error carrying the reason
    /// out of its own log. "The gateway exited" on its own sends somebody to
    /// open a file we already know the path of, and the line that explains
    /// it is the last one in there.
    fn still_alive(&self) -> Result<(), String> {
        if pid_alive(self.pid) {
            return Ok(());
        }
        Err(format!(
            "the gateway (pid {}) exited while starting up:\n{}",
            self.pid,
            log_tail(&self.logfile, 8)
        ))
    }
}

/// Refuses when something that is not this deployment already holds the
/// gateway's port.
///
/// Binding it ourselves and letting go is the whole test: it needs no
/// `lsof`, no `/proc`, and it answers the question the health check cannot
/// — whether the port is *ours to take*, rather than whether somebody is
/// answering on it.
fn port_available(url: &str) -> Result<(), String> {
    let authority = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    match std::net::TcpListener::bind(authority) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => Err(format!(
            "{authority} is already in use.\n\
             \n\
             This deployment has no gateway of its own running, so the port\n\
             belongs to something else — most often the other half of a\n\
             checkout / installed-release pair, which share it by default.\n\
             Stop whatever holds it (its pid is in that deployment's\n\
             `data/gateway.pid`) and run this again.\n\
             \n\
             Starting anyway is worse than refusing: the new gateway exits with\n\
             `AddrInUse` while the old one keeps answering /healthz, which reads\n\
             as an init that worked."
        )),
        Err(err) => Err(format!("check whether {authority} is free: {err}")),
    }
}

/// Waits for a gateway we just signalled to release the port.
///
/// Silent about failure on purpose: if the port is still held when the
/// budget runs out, the pre-flight that follows says so properly, and says
/// it once.
fn wait_for_port_release(url: &str, budget: Duration) {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if port_available(url).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Does this pid exist and may we signal it — `kill -0`.
///
/// Silenced, because the answer we want is the exit status: `kill -0` on a
/// pid that is gone writes "No such process" to stderr, and a stale pidfile
/// is the ordinary case here rather than a fault. It printed that line in
/// the middle of an otherwise clean `init`, where it read as the failure
/// instead of as the normal way of finding out.
fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The last `lines` lines of a log, indented, for an error that would
/// otherwise report only that a process is missing. `Address already in
/// use` is the line this exists to put in front of somebody.
fn log_tail(path: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return format!("  (no log at {})", path.display());
    };
    let tail: Vec<&str> = text.lines().rev().take(lines).collect();
    if tail.is_empty() {
        return format!("  ({} is empty)", path.display());
    }
    tail.into_iter()
        .rev()
        .map(|line| format!("  {line}\n"))
        .collect()
}

/// The pid in `pidfile`, if that process is alive.
fn running_gateway(pidfile: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pidfile).ok()?.trim().parse().ok()?;
    pid_alive(pid).then_some(pid)
}

/// Stops the host gateway this deployment started, if one is running
/// (CPR-2, ADR-0069). `Some(pid)` when something was signalled.
///
/// `synveda reset` needs this before it destroys the database: a gateway left
/// running would hold connections against a database that is about to stop
/// existing, and — worse — would keep serving from in-process caches (the
/// scope chains, the policy packs) that describe rows nobody can read any
/// more. `DROP DATABASE ... WITH (FORCE)` would evict its connections and
/// leave the process alive and confidently wrong.
///
/// Deliberately only the host process. [`stop_compose_product_processes`]
/// owns the paired container gateway/worker lifecycle.
pub fn stop_host_gateway() -> Option<u32> {
    let pidfile = state_dir()?.join("gateway.pid");
    let pid = running_gateway(&pidfile)?;
    let _ = Command::new("kill").arg(pid.to_string()).status();
    wait_for_port_release(GATEWAY_URL, Duration::from_secs(10));
    Some(pid)
}

/// The compose file of the deployment this CLI would drive, or `None` where no
/// profile resolves. Named so that a message can tell somebody which compose
/// file is theirs rather than making them work it out from which shape of
/// install they have.
pub fn compose_file_if_any() -> Option<PathBuf> {
    Profile::discover()
        .ok()
        .map(|profile| profile.compose_file())
}

/// Stops the Compose-owned gateway and worker before destructive local
/// database maintenance. The worker receives its normal 75-second SIGTERM
/// cancellation/join budget inside Compose's slightly larger stop bound.
pub fn stop_compose_product_processes(compose_file: &Path) -> Result<(), String> {
    compose(
        compose_file,
        &["stop", "--timeout", "90", "gateway", "worker"],
    )
}

/// The state directory of the deployment this CLI would drive — the pidfile,
/// the log, `kms.key` — or `None` where no profile resolves at all.
///
/// `None` is ordinary rather than an error: somebody pointing `DATABASE_URL`
/// at their own Postgres has no compose profile, and the commands that ask
/// this are the ones that can carry on without one.
pub fn state_dir() -> Option<PathBuf> {
    Profile::discover().ok().map(|profile| profile.state_dir())
}

/// What the gateway binary is, for the convergence fingerprint.
///
/// Modification time and length rather than a digest: this runs on every
/// `init` and the gateway is tens of megabytes, so the question is "did this
/// artefact change" rather than "which artefact is this". Reinstalling moves
/// both. `none` when there is no binary yet, which is a checkout before
/// anybody has run `cargo build`.
fn binary_stamp(profile: &Profile) -> String {
    gateway_binary(profile)
        .ok()
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|meta| {
            let modified = meta
                .modified()
                .ok()
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_secs());
            format!("{}:{modified}", meta.len())
        })
        .unwrap_or_else(|| "none".to_owned())
}

/// Finds a gateway to run. In a checkout that is a `cargo build` away and
/// always was; in a release bundle it is a file the installer unpacked, which
/// is the whole of what OPS-8 changed here.
fn gateway_binary(profile: &Profile) -> Result<PathBuf, String> {
    let candidates = profile.gateway_candidates();
    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }
    let looked = candidates
        .iter()
        .map(|path| format!("\n\x20 {}", path.display()))
        .collect::<String>();
    match profile {
        Profile::Checkout { .. } => Err(format!(
            "no synveda-gateway binary. Looked in:{looked}\n\
             \n\
             Build one with `cargo build -p synveda-gateway` and re-run \
             `synveda init`."
        )),
        Profile::Bundle { .. } => Err(format!(
            "no synveda-gateway binary. Looked in:{looked}\n\
             \n\
             The installer unpacks it; a profile without one is a partial \
             install. Re-run:\n\
             \n\
             \x20 {INSTALL_COMMAND}"
        )),
    }
}

/// `docker compose -f <file> <args...>`, inheriting stdout/stderr so a
/// slow image pull is visible rather than silent.
fn compose(compose_file: &Path, args: &[&str]) -> Result<(), String> {
    compose_with_env(compose_file, args, &[])
}

/// The same, with variables compose substitutes into the file. Passed on
/// the child rather than exported into this process, so a value chosen for
/// one invocation cannot leak into the next.
fn compose_with_env(
    compose_file: &Path,
    args: &[&str],
    environment: &[(&str, &str)],
) -> Result<(), String> {
    let status = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .args(args)
        .envs(environment.iter().copied())
        .status()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                "docker is not on PATH — install Docker Desktop or OrbStack and start it".to_owned()
            }
            _ => format!("run docker compose: {err}"),
        })?;
    if !status.success() {
        return Err(format!(
            "docker compose {} failed ({status}) — is the Docker daemon running?",
            args.join(" ")
        ));
    }
    Ok(())
}

/// The gateway's configuration, as the env file compose reads. Written
/// rather than passed so that `docker compose up gateway` afterwards — by
/// an operator, by a restart, by OPS-2's successor — brings up the same
/// deployment without `init` having to be re-run.
fn write_env_file(
    path: &Path,
    plan: &Plan,
    tenant_id: TenantId,
    issuer: &str,
    kek: &str,
    database: Option<&InitDatabasePlan>,
) -> Result<(), String> {
    // One line, no quotes: quoting the JSON value makes those quote bytes
    // part of the value delivered to the gateway. This transitional raw
    // env-file handoff is not accepted for arbitrary secret characters; the
    // CPR-45 file-secret cutover replaces it.
    let issuers = issuers_json(tenant_id, issuer);
    let tei = if plan.embedder == "tei" {
        "SYNVEDA_TEI_URL=http://tei:80\n"
    } else {
        ""
    };
    let runtime_databases = match database {
        Some(database) => Zeroizing::new(format!(
            "SYNVEDA_GATEWAY_DATABASE_URL={}\n\
             SYNVEDA_WORKER_DATABASE_URL={}\n\
             SYNVEDA_DATABASE_ROLES={}\n",
            database.gateway_url.as_str(),
            database.worker_url.as_str(),
            database.roles_json,
        )),
        None => Zeroizing::new(String::new()),
    };
    let body = Zeroizing::new(format!(
        "# Written by `synveda init` (CPR-36, ADR-0095). Regenerate by re-running it.\n\
         # Local deployment material. Runtime behaviour comes from governed Configuration.\n\
         SYNVEDA_TENANT_ID={tenant_id}\n\
         SYNVEDA_OIDC_ISSUERS={issuers}\n\
         SYNVEDA_PUBLIC_URL={GATEWAY_URL}\n\
         SYNVEDA_LISTEN_ADDR=0.0.0.0:8120\n\
         SYNVEDA_EMBEDDER={}\n\
         SYNVEDA_KMS_KEY={kek}\n\
         {}{tei}",
        plan.embedder,
        runtime_databases.as_str(),
    ));
    write_private_file_atomically(path, body.as_bytes())
}

static PRIVATE_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Replaces one local secret file without ever creating the destination with
/// permissive permissions or exposing a partially written value. The
/// temporary lives beside the destination, so the final rename is one
/// filesystem operation. Its name is not a security boundary: `create_new`
/// and the private deployment-state directory are.
fn write_private_file_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} must have a UTF-8 file name", path.display()))?;
    if let Ok(metadata) = std::fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(format!(
            "{} must be a regular non-symlink file",
            path.display()
        ));
    }

    let mut opened = None;
    for _ in 0..64 {
        let sequence = PRIVATE_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{file_name}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => {
                opened = Some((temporary, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "create private file for {}: {error}",
                    path.display()
                ));
            }
        }
    }
    let Some((temporary, mut file)) = opened else {
        return Err(format!(
            "create private file for {}: temporary-name attempts exhausted",
            path.display()
        ));
    };
    let result = (|| -> Result<(), String> {
        file.write_all(contents)
            .map_err(|error| format!("write private file for {}: {error}", path.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync private file for {}: {error}", path.display()))?;
        drop(file);
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("replace {} atomically: {error}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|error| format!("inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.permissions().mode() & 0o777 != 0o600
            {
                return Err(format!(
                    "{} was not installed as a regular mode-0600 file",
                    path.display()
                ));
            }
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("sync {}: {error}", parent.display()))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| format!("create {}: {error}", path.display()))?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "{} must be a private non-symlink directory",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("restrict {} to 0700: {error}", path.display()))?;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        if metadata.mode() & 0o7777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.gid() != rustix::process::getegid().as_raw()
        {
            return Err(format!(
                "{} must be a private process-owned directory",
                path.display()
            ));
        }
    }
    Ok(())
}

fn read_private_file(path: &Path, maximum_bytes: u64) -> Result<String, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let path_metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(format!(
            "{} must be a regular non-symlink file",
            path.display()
        ));
    }
    let mut file =
        open_nonblocking_read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect {}: {error}", path.display()))?;
    if !opened_metadata.is_file() || opened_metadata.len() > maximum_bytes {
        return Err(format!("{} exceeds its private-file bound", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let parent_metadata = std::fs::symlink_metadata(parent)
            .map_err(|error| format!("inspect {}: {error}", parent.display()))?;
        if parent_metadata.file_type().is_symlink()
            || !parent_metadata.is_dir()
            || parent_metadata.mode() & 0o7777 != 0o700
            || parent_metadata.uid() != rustix::process::geteuid().as_raw()
            || parent_metadata.gid() != rustix::process::getegid().as_raw()
            || opened_metadata.mode() & 0o7777 != 0o600
            || opened_metadata.uid() != rustix::process::geteuid().as_raw()
            || opened_metadata.gid() != rustix::process::getegid().as_raw()
            || opened_metadata.nlink() != 1
            || opened_metadata.dev() != path_metadata.dev()
            || opened_metadata.ino() != path_metadata.ino()
        {
            return Err(format!(
                "{} must remain one private process-owned regular file",
                path.display()
            ));
        }
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(opened_metadata.len().min(maximum_bytes))
            .map_err(|_| format!("{} exceeds its private-file bound", path.display()))?,
    );
    std::io::Read::by_ref(&mut file)
        .take(maximum_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(format!("{} exceeds its private-file bound", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let opened_after = file
            .metadata()
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        let path_after = std::fs::symlink_metadata(path)
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        let snapshot = |metadata: &std::fs::Metadata| {
            (
                metadata.dev(),
                metadata.ino(),
                metadata.mode(),
                metadata.uid(),
                metadata.gid(),
                metadata.nlink(),
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            )
        };
        if snapshot(&opened_metadata) != snapshot(&opened_after)
            || path_after.file_type().is_symlink()
            || snapshot(&opened_after) != snapshot(&path_after)
            || bytes.len() as u64 != opened_metadata.len()
        {
            return Err(format!("{} changed while it was read", path.display()));
        }
    }
    String::from_utf8(bytes).map_err(|_| format!("{} must contain valid UTF-8", path.display()))
}

/// The gateway's `SYNVEDA_OIDC_ISSUERS`, in one place so the host process
/// and the container cannot be configured differently.
fn issuers_json(tenant_id: TenantId, issuer: &str) -> String {
    json!([{
        "issuer": issuer,
        "client_id": "synveda",
        "audience": "synveda-api",
        "tenant": {"static": {"tenant_id": tenant_id}},
        "login_scopes": ["openid", "profile", "email", "groups"],
    }])
    .to_string()
}

/// Polls a health endpoint until it answers or the budget runs out — and,
/// when we started the process ourselves, until *that process* stops
/// existing.
///
/// Asking the port whether something answers is a different question from
/// asking whether the gateway this `init` started is serving, and the two
/// diverge exactly where it matters. Measured on a machine holding a release
/// install and a checkout at once: `init` from the checkout spawned a
/// gateway that died in milliseconds with `AddrInUse`, because the installed
/// deployment already held 8120 — and then the *installed* gateway answered
/// `/healthz`, so `init` printed `pid 51544`, `healthy` and `initialised in
/// 6s`, and exited 0. The pid it named had never lived long enough to bind.
///
/// This is the same shape as the upgrade that left the previous release
/// serving (ADR-0065 amendment 5) and the console that returned 200 while
/// nobody could sign in (amendment 4): a check standing one layer shallower
/// than the claim it is made to support. The liveness test goes *before* the
/// probe so a dead process is reported rather than a stranger's success, and
/// again *after* a success, so the answer cannot have come from a gateway
/// that outlived ours by the width of one HTTP round trip.
async fn wait_for_health(
    url: &str,
    budget: Duration,
    started: Option<&Started>,
) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("build the HTTP client: {err}"))?;
    let deadline = Instant::now() + budget;
    let mut last = String::new();
    while Instant::now() < deadline {
        if let Some(started) = started {
            started.still_alive()?;
        }
        match http.get(url).send().await {
            Ok(response) if response.status().is_success() => {
                if let Some(started) = started {
                    started.still_alive()?;
                }
                return Ok(());
            }
            Ok(response) => last = format!("HTTP {}", response.status()),
            Err(err) => last = err.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!(
        "{url} did not become healthy within {}s (last: {last})",
        budget.as_secs()
    ))
}

/// The deployment's key-encryption key: the operator's if they set one,
/// otherwise this deployment's own, minted once and kept.
///
/// Without a KEK the gateway boots with `Kms::Disabled` and warns that
/// "console sessions and per-tenant secrets are unavailable" — and then
/// **signing in to the console is impossible**, because a console session
/// seals its tokens under the deployment-scope key (TEN-4, ADR-0064) and
/// `/auth/callback` fails with `not found: encryption key for deployment`.
///
/// That is what shipped: CNSL-1 built the console before TEN-4 sealed its
/// sessions, TEN-4 made it need a key, and nothing minted one. The console
/// served its bundle on every install and nobody could get past the login.
///
/// Minting one here rather than making the operator do it is the same
/// judgement ADR-0055 decision 5 makes about the embedder: an installer's
/// job is to leave a working deployment, and a key the product needs for
/// its own admin surface is not a decision worth interrupting an install
/// for. An operator who *has* opinions sets `SYNVEDA_KMS_KEY` and this
/// defers to it, exactly as `DATABASE_URL` works.
///
/// The file sits in the state directory at `0600`. That is the same
/// posture ADR-0064 already states — the KEK lives in deployment
/// configuration, so this defends a dumped table and a stolen archive
/// rather than somebody who can read this machine.
fn ensure_kek(profile: &Profile) -> Result<String, String> {
    let state = profile.state_dir();
    ensure_private_directory(&state)?;
    let path = state.join("kms.key");

    let configured = std::env::var("SYNVEDA_KMS_KEY")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| canonical_local_kek("SYNVEDA_KMS_KEY", &value).map(Zeroizing::new))
        .transpose()?;
    let persisted = match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            let existing = Zeroizing::new(read_private_file(&path, 65)?);
            Some(Zeroizing::new(canonical_local_kek(
                "the local KMS key file",
                existing.as_str(),
            )?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("inspect {}: {error}", path.display())),
    };
    match (configured, persisted) {
        (Some(configured), Some(persisted)) => {
            if configured.as_str() != persisted.as_str() {
                return Err("SYNVEDA_KMS_KEY does not match the persisted local KMS key".to_owned());
            }
            println!("    key plane: SYNVEDA_KMS_KEY matches {}", path.display());
            return Ok(configured.to_string());
        }
        (Some(configured), None) => {
            write_private_file_atomically(&path, format!("{}\n", configured.as_str()).as_bytes())?;
            println!(
                "    key plane: persisted SYNVEDA_KMS_KEY at {}",
                path.display()
            );
            println!("    ** back this file up. Every tenant key in this database is");
            println!("       wrapped by it, and losing it is losing them. **");
            return Ok(configured.to_string());
        }
        (None, Some(persisted)) => {
            println!("    key plane: {}", path.display());
            return Ok(persisted.to_string());
        }
        (None, None) => {}
    }

    let key = synveda_crypto::DataKey::generate()
        .map_err(|err| format!("mint a key-encryption key: {err}"))?;
    let hex = key.to_hex().to_string();
    write_private_file_atomically(&path, format!("{hex}\n").as_bytes())?;
    println!("    key plane: minted {}", path.display());
    println!("    ** back this file up. Every tenant key in this database is");
    println!("       wrapped by it, and losing it is losing them. **");
    Ok(hex)
}

fn canonical_local_kek(label: &str, value: &str) -> Result<String, String> {
    let key = synveda_crypto::DataKey::from_hex(value)
        .map_err(|_| format!("{label} must contain one 32-byte hexadecimal key"))?;
    Ok(key.to_hex().to_string())
}

/// The selected bootstrap database and whether an operator supplied it.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DatabaseUrlSetting {
    pub(crate) value: String,
    pub(crate) explicitly_configured: bool,
}

/// `DATABASE_URL`, `DATABASE_URL_FILE`, or the single-node profile's own
/// Postgres. Shared with `main::connect` so direct binaries and containers
/// cannot disagree about which database this deployment is.
pub(crate) fn database_url() -> Result<DatabaseUrlSetting, String> {
    resolve_database_url(
        std::env::var_os("DATABASE_URL"),
        std::env::var_os("DATABASE_URL_FILE"),
    )
}

/// Reads one sensitive CLI setting from `NAME` or `NAME_FILE` without ever
/// putting its value in an error. This is the direct-binary half of the same
/// mounted-secret contract used by gateway and worker configuration.
pub(crate) fn sensitive_setting(name: &str) -> Result<Option<String>, String> {
    let file_name = format!("{name}_FILE");
    match (std::env::var_os(name), std::env::var_os(&file_name)) {
        (Some(_), Some(_)) => Err(format!(
            "{name} and {file_name} are mutually exclusive; configure exactly one"
        )),
        (Some(value), None) => value
            .into_string()
            .map(Some)
            .map_err(|_| format!("{name} must be valid UTF-8")),
        (None, Some(path)) => read_database_url_file(&file_name, Path::new(&path)).map(Some),
        (None, None) => Ok(None),
    }
}

/// Closed deployment database-role contract used by migration, preflight and
/// every long-running product process.
pub(crate) fn database_roles() -> Result<synveda_store::runtime_role::DatabaseRoles, String> {
    database_roles_with_json().map(|(roles, _)| roles)
}

fn database_roles_with_json() -> Result<(synveda_store::runtime_role::DatabaseRoles, String), String>
{
    let direct = std::env::var_os("SYNVEDA_DATABASE_ROLES");
    let file = std::env::var_os("SYNVEDA_DATABASE_ROLES_FILE");
    let value = match (direct, file) {
        (Some(_), Some(_)) => {
            return Err(
                "SYNVEDA_DATABASE_ROLES and SYNVEDA_DATABASE_ROLES_FILE are mutually exclusive"
                    .to_owned(),
            );
        }
        (Some(value), None) => value
            .into_string()
            .map_err(|_| "SYNVEDA_DATABASE_ROLES must be valid UTF-8".to_owned())?,
        (None, Some(path)) => {
            read_database_url_file("SYNVEDA_DATABASE_ROLES_FILE", Path::new(&path))?
        }
        (None, None) => {
            return Err(
                "SYNVEDA_DATABASE_ROLES or SYNVEDA_DATABASE_ROLES_FILE is required".to_owned(),
            );
        }
    };
    if value.len() > 4_096 {
        return Err("SYNVEDA_DATABASE_ROLES exceeds the 4096-byte startup bound".to_owned());
    }
    let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(&value)
        .map_err(|error| error.to_string())?;
    let compact = serde_json::from_str::<Value>(&value)
        .map_err(|_| "SYNVEDA_DATABASE_ROLES is not valid JSON".to_owned())?
        .to_string();
    Ok((roles, compact))
}

fn init_database_plan() -> Result<InitDatabasePlan, String> {
    let migrator = database_url()?;
    if !migrator.explicitly_configured {
        return Err(
            "synveda init requires explicit DATABASE_URL or DATABASE_URL_FILE for the pre-provisioned migrator; database authority is never inferred or provisioned by init"
                .to_owned(),
        );
    }
    let gateway = resolve_required_database_url(
        "SYNVEDA_GATEWAY_DATABASE_URL",
        "SYNVEDA_GATEWAY_DATABASE_URL_FILE",
        std::env::var_os("SYNVEDA_GATEWAY_DATABASE_URL"),
        std::env::var_os("SYNVEDA_GATEWAY_DATABASE_URL_FILE"),
    )?;
    let worker = resolve_required_database_url(
        "SYNVEDA_WORKER_DATABASE_URL",
        "SYNVEDA_WORKER_DATABASE_URL_FILE",
        std::env::var_os("SYNVEDA_WORKER_DATABASE_URL"),
        std::env::var_os("SYNVEDA_WORKER_DATABASE_URL_FILE"),
    )?;
    let (roles, roles_json) = database_roles_with_json()?;
    validate_init_database_plan(roles, roles_json, migrator.value, gateway, worker)
}

fn resolve_required_database_url(
    direct_setting: &str,
    file_setting: &str,
    direct: Option<OsString>,
    file: Option<OsString>,
) -> Result<String, String> {
    match (direct, file) {
        (Some(_), Some(_)) => Err(format!(
            "{direct_setting} and {file_setting} are mutually exclusive; configure exactly one"
        )),
        (Some(value), None) => value
            .into_string()
            .map_err(|_| format!("{direct_setting} must be valid UTF-8")),
        (None, Some(path)) => read_database_url_file(file_setting, Path::new(&path)),
        (None, None) => Err(format!(
            "{direct_setting} or {file_setting} is required for synveda init"
        )),
    }
}

fn validate_init_database_plan(
    roles: synveda_store::runtime_role::DatabaseRoles,
    roles_json: String,
    migrator_url: String,
    gateway_url: String,
    worker_url: String,
) -> Result<InitDatabasePlan, String> {
    let migrator = synveda_store::database_url::parse("DATABASE_URL", &migrator_url)
        .map_err(|error| error.to_string())?;
    let gateway = synveda_store::database_url::parse("SYNVEDA_GATEWAY_DATABASE_URL", &gateway_url)
        .map_err(|error| error.to_string())?;
    let worker = synveda_store::database_url::parse("SYNVEDA_WORKER_DATABASE_URL", &worker_url)
        .map_err(|error| error.to_string())?;
    if migrator.get_username() != roles.migrator()
        || gateway.get_username() != roles.gateway()
        || worker.get_username() != roles.worker()
    {
        return Err(
            "the database role contract must exactly match the migrator, gateway and worker URL principals"
                .to_owned(),
        );
    }
    let one_target = |candidate: &sqlx::postgres::PgConnectOptions| {
        candidate.get_host() == migrator.get_host()
            && candidate.get_port() == migrator.get_port()
            && candidate.get_socket() == migrator.get_socket()
            && candidate.get_database() == migrator.get_database()
    };
    if !one_target(&gateway) || !one_target(&worker) {
        return Err(
            "DATABASE_URL and the gateway and worker database URLs must declare one exact PostgreSQL host, port, socket and database"
                .to_owned(),
        );
    }
    let target_database = migrator.get_database().ok_or_else(|| {
        "DATABASE_URL must name the PostgreSQL database managed by synveda init".to_owned()
    })?;
    if roles
        .forbidden_databases()
        .iter()
        .any(|forbidden| forbidden == target_database)
    {
        return Err(
            "the init database target cannot be one of the configured isolated peer databases"
                .to_owned(),
        );
    }
    Ok(InitDatabasePlan {
        roles,
        roles_json,
        migrator_url: Zeroizing::new(migrator_url),
        gateway_url: Zeroizing::new(gateway_url),
        worker_url: Zeroizing::new(worker_url),
    })
}

fn resolve_database_url(
    direct: Option<OsString>,
    file: Option<OsString>,
) -> Result<DatabaseUrlSetting, String> {
    match (direct, file) {
        (Some(_), Some(_)) => Err(
            "DATABASE_URL and DATABASE_URL_FILE are mutually exclusive; configure exactly one"
                .to_owned(),
        ),
        (Some(value), None) => value
            .into_string()
            .map(|value| DatabaseUrlSetting {
                value,
                explicitly_configured: true,
            })
            .map_err(|_| "DATABASE_URL must be valid UTF-8".to_owned()),
        (None, Some(path)) => {
            read_database_url_file("DATABASE_URL_FILE", Path::new(&path)).map(|value| {
                DatabaseUrlSetting {
                    value,
                    explicitly_configured: true,
                }
            })
        }
        (None, None) => Ok(DatabaseUrlSetting {
            value: DEFAULT_DATABASE_URL.to_owned(),
            explicitly_configured: false,
        }),
    }
}

pub(crate) fn read_database_url_file(setting: &str, path: &Path) -> Result<String, String> {
    if path.as_os_str().is_empty() {
        return Err(format!("{setting} must name a file"));
    }
    let file = open_nonblocking_read(path).map_err(|_| format!("{setting} cannot be read"))?;
    let metadata = file
        .metadata()
        .map_err(|_| format!("{setting} cannot be read"))?;
    if !metadata.is_file() {
        return Err(format!("{setting} must name a regular file"));
    }
    if metadata.len() > MAX_DATABASE_URL_FILE_BYTES {
        return Err(format!(
            "{setting} exceeds the {MAX_DATABASE_URL_FILE_BYTES} byte startup bound"
        ));
    }
    let capacity = usize::try_from(metadata.len().min(MAX_DATABASE_URL_FILE_BYTES + 1))
        .map_err(|_| format!("{setting} exceeds the startup bound"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_DATABASE_URL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{setting} cannot be read"))?;
    if bytes.len() as u64 > MAX_DATABASE_URL_FILE_BYTES {
        return Err(format!(
            "{setting} exceeds the {MAX_DATABASE_URL_FILE_BYTES} byte startup bound"
        ));
    }
    let mut value =
        String::from_utf8(bytes).map_err(|_| format!("{setting} must contain valid UTF-8"))?;
    if let Some(stripped) = value.strip_suffix("\r\n") {
        value.truncate(stripped.len());
    } else if value.ends_with('\n') {
        value.pop();
    }
    if value.contains('\0') {
        return Err(format!("{setting} must not contain NUL bytes"));
    }
    Ok(value)
}

fn open_nonblocking_read(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Open first, then classify the descriptor. O_NONBLOCK makes FIFOs
        // and device-like paths observable without waiting for a writer,
        // while following Kubernetes' projected-secret symlink to its regular
        // immutable target remains supported.
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    options.open(path)
}

/// A credential-safe rendering for operator errors. `DATABASE_URL` is an
/// admin boundary and must not be echoed into a terminal transcript or CI log.
pub(crate) fn redacted_database_url(value: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(value) else {
        return "configured PostgreSQL database".to_owned();
    };
    if parsed.password().is_some() {
        let _ = parsed.set_password(Some("REDACTED"));
    }
    // PostgreSQL accepts credentials in URL query parameters too. None of
    // the query or fragment is needed to identify the failed target, so drop
    // both instead of trying to maintain an incomplete secret-key vocabulary.
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

/// A stable, content-sensitive value suitable for the non-secret deployment
/// convergence file. A credential rotation must restart the gateway, while
/// the credential itself must never be copied beside the pidfile.
fn secret_fingerprint(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
async fn verify_runtime_role(
    pool: &sqlx::PgPool,
    role: &str,
    database_roles: &synveda_store::runtime_role::DatabaseRoles,
) -> Result<(), String> {
    synveda_store::runtime_role::verify_capability_role(pool, database_roles)
        .await
        .map_err(|error| format!("verify synveda_app capability role: {error}"))?;
    let facts = sqlx::query!(
        r#"select rolcanlogin as "can_login!", rolinherit as "inherits!",
                  rolsuper as "superuser!", rolcreatedb as "create_db!",
                  rolcreaterole as "create_role!", rolreplication as "replication!",
                  rolbypassrls as "bypass_rls!",
                  pg_has_role($1, 'synveda_app', 'member') as "app_member!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as membership
                      join pg_catalog.pg_roles as granted_role
                        on granted_role.oid = membership.roleid
                     where membership.member = roles.oid
                       and granted_role.rolname = 'synveda_app'
                       and not membership.admin_option
                       and membership.inherit_option
                       and membership.set_option
                  ) as "app_membership_safe!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as membership
                      join pg_catalog.pg_roles as granted_role
                        on granted_role.oid = membership.roleid
                     where membership.member = roles.oid
                       and granted_role.rolname = 'synveda_app'
                       and (
                         membership.admin_option
                         or not membership.inherit_option
                         or not membership.set_option
                       )
                  ) as "app_membership_unsafe!",
                  exists (
                    select 1 from pg_catalog.pg_database
                     where datdba = roles.oid
                  ) as "database_owner!",
                  exists (
                    select 1 from pg_catalog.pg_namespace
                     where nspowner = roles.oid
                  ) as "schema_owner!",
                  exists (
                    select 1 from pg_catalog.pg_class as objects
                     where objects.relowner = roles.oid
                    union all
                    select 1 from pg_catalog.pg_proc as routines
                     where routines.proowner = roles.oid
                    union all
                    select 1 from pg_catalog.pg_type as data_type
                     where data_type.typowner = roles.oid
                  ) as "application_object_owner!",
                  exists (
                    select 1
                      from pg_catalog.pg_auth_members as memberships
                      join pg_catalog.pg_roles as granted_roles
                        on granted_roles.oid = memberships.roleid
                     where memberships.member = roles.oid
                       and granted_roles.rolname <> 'synveda_app'
                  ) as "unexpected_membership!",
                  exists (
                    select 1 from pg_catalog.pg_auth_members as memberships
                     where memberships.roleid = roles.oid
                       and (
                         not memberships.admin_option
                         or memberships.inherit_option
                         or memberships.set_option
                       )
                  ) as "unsafe_inbound_membership!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = current_database()
                       and acl.grantee = roles.oid
                       and acl.privilege_type = 'CONNECT'
                       and not acl.is_grantable
                  ) as "direct_connect!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = current_database()
                       and acl.grantee = roles.oid
                       and (
                         acl.privilege_type <> 'CONNECT'
                         or acl.is_grantable
                       )
                  ) as "unsafe_direct_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname = current_database()
                       and acl.grantee = 0
                  ) as "public_database_acl!",
                  exists (
                    select 1
                      from pg_catalog.pg_database as database,
                           lateral pg_catalog.aclexplode(
                             coalesce(
                               database.datacl,
                               pg_catalog.acldefault('d', database.datdba)
                             )
                           ) as acl
                     where database.datname <> current_database()
                       and acl.grantee = roles.oid
                  ) as "other_database_acl!"
             from pg_catalog.pg_roles as roles
            where rolname = $1"#,
        role,
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("verify runtime database login {role}: {error}"))?
    .ok_or_else(|| format!("runtime database login {role} is not provisioned"))?;
    if !facts.can_login
        || !facts.inherits
        || facts.superuser
        || facts.create_db
        || facts.create_role
        || facts.replication
        || facts.bypass_rls
        || !facts.app_member
        || !facts.app_membership_safe
        || facts.app_membership_unsafe
        || facts.database_owner
        || facts.schema_owner
        || facts.application_object_owner
        || facts.unexpected_membership
        || facts.unsafe_inbound_membership
        || !facts.direct_connect
        || facts.unsafe_direct_database_acl
        || facts.public_database_acl
        || facts.other_database_acl
    {
        return Err(format!(
            "runtime database login {role} is not an exact inheriting, non-elevated synveda_app login with only direct CONNECT on the selected database, no PUBLIC or other-database ACL, no privilege-bearing inbound membership, and no database, schema, relation, routine or type ownership"
        ));
    }
    Ok(())
}

/// Connects the resolved runtime DSN and proves the session, role, schema and
/// target it will actually give the process. Looking up a same-named role
/// through the bootstrap pool is insufficient when an explicitly configured
/// runtime URL may name another host or database.
async fn verify_runtime_database_url(
    setting: &str,
    runtime_url: &str,
    role: &str,
    database_roles: &synveda_store::runtime_role::DatabaseRoles,
) -> Result<synveda_store::runtime_role::DatabaseIdentity, String> {
    let connect_options = synveda_store::database_url::parse(setting, runtime_url)
        .map_err(|_| format!("{setting} is not a valid PostgreSQL URL"))?;
    tokio::time::timeout(Duration::from_secs(15), async move {
        let runtime = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(connect_options)
            .await
            .map_err(|_| {
                format!(
                    "connect through {setting} ({}) failed",
                    redacted_database_url(runtime_url)
                )
            })?;
        let result = async {
            synveda_store::epoch::verify(&runtime)
                .await
                .map_err(|error| format!("{setting} schema check failed: {error}"))?;
            synveda_store::runtime_role::verify(&runtime, role, database_roles)
                .await
                .map_err(|error| format!("{setting} runtime-role check failed: {error}"))?;
            synveda_store::runtime_role::database_identity(&runtime)
                .await
                .map_err(|error| format!("{setting} database-target check failed: {error}"))
        }
        .await;
        runtime.close().await;
        result
    })
    .await
    .map_err(|_| format!("{setting} verification exceeded the 15-second startup bound"))?
}

fn require_one_database_target(
    bootstrap: &synveda_store::runtime_role::DatabaseIdentity,
    gateway: &synveda_store::runtime_role::DatabaseIdentity,
    worker: &synveda_store::runtime_role::DatabaseIdentity,
) -> Result<(), String> {
    if bootstrap != gateway || bootstrap != worker {
        return Err("DATABASE_URL, SYNVEDA_GATEWAY_DATABASE_URL and \
             SYNVEDA_WORKER_DATABASE_URL must target one live PostgreSQL primary instance \
             and database"
            .to_owned());
    }
    Ok(())
}

/// Everything `init` needs that is not inside this binary: the compose file,
/// a gateway to run, a console bundle to serve, and somewhere to keep the
/// pidfile. ADR-0055 decision 6 said a released binary would carry its own
/// profile; OPS-8 is that release, and this is what carrying it means.
///
/// Two shapes. A **checkout** is a source tree — the contributor's case, and
/// the only one that existed before OPS-8. A **bundle** is what `install.sh`
/// unpacks under `$SYNVEDA_HOME` (default `~/.synveda`) on a machine with no
/// source and no Rust toolchain.
enum Profile {
    Checkout {
        root: PathBuf,
    },
    Bundle {
        home: PathBuf,
        /// The release this bundle was packaged at, from its `version` file.
        version: String,
    },
}

impl Profile {
    /// Explicit beats a checkout beats an installed bundle (ADR-0065
    /// decision 4). The middle rung is the one that matters: a contributor
    /// who has *also* installed a release must get the tree they are
    /// editing, because the reverse is a debugging session that lies.
    fn discover() -> Result<Self, String> {
        if let Some(file) = std::env::var("SYNVEDA_COMPOSE_FILE")
            .ok()
            .filter(|value| !value.is_empty())
        {
            return Self::from_explicit_compose_file(&PathBuf::from(file));
        }

        let cwd = std::env::current_dir()
            .map_err(|err| format!("resolve the working directory: {err}"))?;
        if cwd.join("deploy/compose/docker-compose.yml").is_file() {
            return Ok(Self::Checkout { root: cwd });
        }

        let home = synveda_home()?;
        if home.join("profile/docker-compose.yml").is_file() {
            let version = read_bundle_version(&home.join("profile"))?;
            return Ok(Self::Bundle { home, version });
        }

        Err(format!(
            "no Synveda profile to install from. Three places were looked in:\n\
             \x20 SYNVEDA_COMPOSE_FILE — unset\n\
             \x20 a checkout           — no deploy/compose/docker-compose.yml under {}\n\
             \x20 an installed release — no profile under {}\n\
             \n\
             Install one with:  {INSTALL_COMMAND}",
            cwd.display(),
            home.display(),
        ))
    }

    /// `SYNVEDA_COMPOSE_FILE`, which predates OPS-8 and keeps working. It
    /// can now name either shape: a bundle's directory holds a `version`
    /// file, and a checkout's compose file is three levels below its root.
    fn from_explicit_compose_file(path: &Path) -> Result<Self, String> {
        let directory = path.parent().ok_or_else(|| {
            "SYNVEDA_COMPOSE_FILE must be a path to a compose file, not a directory".to_owned()
        })?;
        if directory.join("version").is_file() {
            return Ok(Self::Bundle {
                home: directory.parent().unwrap_or(directory).to_path_buf(),
                version: read_bundle_version(directory)?,
            });
        }
        directory
            .parent()
            .and_then(Path::parent)
            .map(|root| Self::Checkout {
                root: root.to_path_buf(),
            })
            .ok_or_else(|| {
                format!(
                    "SYNVEDA_COMPOSE_FILE names {}, which is neither a release bundle \
                     (no `version` file beside it) nor <root>/deploy/compose/<file> in a checkout",
                    path.display()
                )
            })
    }

    fn compose_file(&self) -> PathBuf {
        match self {
            Self::Checkout { root } => root.join("deploy/compose/docker-compose.yml"),
            Self::Bundle { home, .. } => home.join("profile/docker-compose.yml"),
        }
    }

    /// The gateway's pidfile, log and rendered configuration. Beside the
    /// compose file in a checkout, because that is where `data/` has always
    /// been; under the install root in a bundle, so that unpacking a new
    /// profile over an old one does not delete the record of what is running.
    fn state_dir(&self) -> PathBuf {
        match self {
            Self::Checkout { root } => root.join("data"),
            Self::Bundle { home, .. } => home.join("data"),
        }
    }

    /// The directory the gateway process is given as its own. A checkout so
    /// that relative paths behave as they do under `cargo run`; the install
    /// root otherwise.
    fn working_dir(&self) -> PathBuf {
        match self {
            Self::Checkout { root } => root.clone(),
            Self::Bundle { home, .. } => home.clone(),
        }
    }

    /// The console bundle CNSL-1 serves from `/console/`, if this profile
    /// has one. `None` is not an error — ADR-0056 decision 1 makes a missing
    /// bundle a 404 rather than a boot failure, because a static asset must
    /// not be a dependency of the audit log.
    fn console_dir(&self) -> Option<PathBuf> {
        let candidate = match self {
            Self::Checkout { root } => root.join("console/dist"),
            Self::Bundle { home, .. } => home.join("console"),
        };
        candidate.join("index.html").is_file().then_some(candidate)
    }

    /// Where to look for a gateway to run, in order.
    fn gateway_candidates(&self) -> Vec<PathBuf> {
        match self {
            Self::Checkout { root } => vec![
                root.join("target/release/synveda-gateway"),
                root.join("target/debug/synveda-gateway"),
            ],
            Self::Bundle { home, .. } => {
                let mut candidates = vec![home.join("bin/synveda-gateway")];
                // Beside the CLI that is running, which is where somebody
                // who unpacked the archive by hand would have put both.
                if let Some(beside) = std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(|dir| dir.join("synveda-gateway")))
                {
                    candidates.push(beside);
                }
                candidates
            }
        }
    }

    /// Whether `docker compose up gateway` may build. Only a checkout can:
    /// a bundle's compose file has no build context, by construction
    /// (ADR-0065 decision 3, asserted by `scripts/package-release.sh`).
    fn may_build(&self) -> bool {
        matches!(self, Self::Checkout { .. })
    }

    fn describe(&self) -> String {
        match self {
            Self::Checkout { root } => format!("checkout at {}", root.display()),
            Self::Bundle { home, version } => {
                format!("release {version} installed at {}", home.display())
            }
        }
    }

    /// A bundle packaged at one release, driven by a CLI from another, is
    /// the failure ADR-0065 decision 5 exists to prevent: it presents as a
    /// service that will not start or an environment variable the gateway
    /// does not read, and both look like product bugs. A checkout is exempt
    /// — its profile and its binaries come from the same tree by
    /// construction, and a contributor's `git status` is the check.
    fn check_version(&self) -> Result<(), String> {
        let Self::Bundle { home, version } = self else {
            return Ok(());
        };
        let cli = env!("CARGO_PKG_VERSION");
        if version == cli {
            return Ok(());
        }
        Err(format!(
            "this CLI is {cli} and the installed profile at {} is {version}.\n\
             They ship together and are not mixed — re-run the installer:\n\
             \n\
             \x20 {INSTALL_COMMAND}",
            home.join("profile").display(),
        ))
    }
}

/// Where an installed release lives. `SYNVEDA_HOME` wins so that a demo, a
/// second deployment or a CI job can keep its own, which is exactly what
/// `demos/ops-8-release-install.sh` does.
pub fn synveda_home() -> Result<PathBuf, String> {
    if let Some(home) = std::env::var("SYNVEDA_HOME")
        .ok()
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(home));
    }
    std::env::var("HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".synveda"))
        .ok_or_else(|| "neither SYNVEDA_HOME nor HOME is set, so there is nowhere to look for an installed release".to_owned())
}

fn read_bundle_version(profile_dir: &Path) -> Result<String, String> {
    let path = profile_dir.join("version");
    let raw =
        std::fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let version = raw.trim().to_owned();
    if version.is_empty() {
        return Err(format!("{} is empty", path.display()));
    }
    Ok(version)
}

/// The TEI image for this machine's architecture.
///
/// Upstream publishes two builds and versions only one of them, so the
/// arm64 side is pinned by commit; the two serve the same model at the same
/// dimension and agree to float32 rounding (deploy/compose/docker-compose.yml
/// has the measurement). The Makefile has done this for `make dev-up` since
/// FND-2 and `init` did not, so `init --embedder tei` on an Apple Silicon
/// laptop — half of what OPS-8 ships binaries for — pulled the amd64 image
/// and ran the embedder under emulation.
fn tei_image() -> Option<&'static str> {
    tei_image_for(
        std::env::consts::ARCH,
        std::env::var("SYNVEDA_TEI_IMAGE").is_ok_and(|value| !value.is_empty()),
    )
}

/// The decision, with both inputs passed in so it can be tested on the
/// architecture that is not this one.
fn tei_image_for(arch: &str, chosen_by_operator: bool) -> Option<&'static str> {
    if chosen_by_operator {
        return None; // an explicit choice wins, and needs no help from us
    }
    match arch {
        "aarch64" | "arm" => {
            Some("ghcr.io/huggingface/text-embeddings-inference:cpu-arm64-sha-4150561")
        }
        _ => None, // the compose default is the amd64 release
    }
}

fn step(number: u8, what: &str) {
    println!("==> {number}. {what}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory of this test's own. Named for the test so two
    /// running in parallel cannot share one, which the pid alone does not
    /// guarantee.
    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("synveda-init-{what}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_test_roles(forbidden_databases: &[&str]) -> synveda_store::runtime_role::DatabaseRoles {
        synveda_store::runtime_role::DatabaseRoles::new(
            "migrator".to_owned(),
            "gateway".to_owned(),
            "worker".to_owned(),
            vec!["administrator".to_owned()],
            Vec::new(),
            forbidden_databases
                .iter()
                .map(|database| (*database).to_owned())
                .collect(),
            Vec::new(),
        )
        .expect("init test roles")
    }

    fn private_test_database_url(setting: &str) -> Option<String> {
        let path = std::env::var_os(setting)?;
        Some(
            read_database_url_file(setting, Path::new(&path))
                .expect("read isolated test database URL"),
        )
    }

    async fn tenant_admission_footprint(pool: &sqlx::PgPool, tenant_id: TenantId) -> [i64; 4] {
        let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant_id)
            .await
            .expect("open tenant-scoped footprint transaction");
        let row = sqlx::query!(
            r#"
            select (select count(*) from public.tenants
                     where id = $1) as "tenants!",
                   (select count(*) from public.scopes
                     where tenant_id = $1) as "scopes!",
                   (select count(*) from public.scope_grants
                     where tenant_id = $1) as "grants!",
                   (select count(*) from public.audit_log
                     where tenant_id = $1) as "audits!"
            "#,
            tenant_id.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("measure tenant-admission footprint");
        tx.rollback()
            .await
            .expect("roll back footprint transaction");
        [row.tenants, row.scopes, row.grants, row.audits]
    }

    struct TestSchemaEpoch {
        epoch: i32,
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    async fn test_schema_epoch(pool: &sqlx::PgPool) -> TestSchemaEpoch {
        let row = sqlx::query!(
            r#"
            select epoch as "epoch!",
                   updated_at as "updated_at!"
              from public.schema_metadata
             where id
            "#,
        )
        .fetch_one(pool)
        .await
        .expect("snapshot test schema epoch");
        TestSchemaEpoch {
            epoch: row.epoch,
            updated_at: row.updated_at,
        }
    }

    async fn set_test_schema_epoch(pool: &sqlx::PgPool, state: &TestSchemaEpoch) {
        sqlx::query!(
            r#"
            update public.schema_metadata
               set epoch = $1,
                   updated_at = $2
             where id
            "#,
            state.epoch,
            state.updated_at,
        )
        .execute(pool)
        .await
        .expect("set test schema epoch");
    }

    /// A gateway that died on startup names the pid *and* the reason.
    ///
    /// The failure this covers: a checkout `init` beside an installed one
    /// spawned a gateway that exited in milliseconds with `AddrInUse`,
    /// the installed gateway answered `/healthz`, and `init` reported
    /// `pid 51544`, `healthy` and exit 0 for a process that had never
    /// bound the port. Liveness is asked of the pid we started, and the
    /// answer carries the log line that explains it.
    #[test]
    fn a_gateway_that_exited_reports_the_reason_from_its_own_log() {
        let dir = scratch("exited");
        let logfile = dir.join("gateway.log");
        std::fs::write(
            &logfile,
            "INFO gateway starting\n\
             Error: Os { code: 48, kind: AddrInUse, message: \"Address already in use\" }\n",
        )
        .unwrap();

        // A pid that has certainly exited: our own child, waited on.
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        let started = Started {
            pid: child.id(),
            logfile,
        };

        let err = started.still_alive().unwrap_err();
        assert!(err.contains(&format!("pid {}", child.id())), "{err}");
        assert!(
            err.contains("Address already in use"),
            "the reason has to come with the failure, not be left in a file: {err}"
        );
    }

    /// A port somebody else holds is refused before anything is spawned.
    ///
    /// The pre-flight rather than the liveness check is what closes this,
    /// and the reason is timing: the gateway completes database/schema,
    /// identity and key-plane setup before it binds, so a child that is doomed to
    /// `AddrInUse` is still alive for the first second — while the process
    /// already on the port answers `/healthz` at once. Watching our own pid
    /// narrows the window; asking for the port closes it.
    #[test]
    fn a_port_somebody_else_holds_is_refused() {
        // An ephemeral port, held for the length of the test, standing in
        // for the other deployment's gateway.
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", held.local_addr().unwrap());

        let err = port_available(&url).unwrap_err();
        assert!(err.contains("already in use"), "{err}");
        assert!(
            err.contains("as an init that worked"),
            "the refusal has to say why starting anyway is worse, not just that it stopped: {err}"
        );

        // The other direction, so the check cannot pass by always refusing.
        // Port 0 is "any free port", which always binds — asserting on a
        // port we just released instead would be a race against every other
        // test in this binary, since the kernel is free to hand the number
        // straight to one of them. It failed that way once here.
        assert!(port_available("http://127.0.0.1:0").is_ok());
    }

    #[test]
    fn a_host_gateway_does_not_inherit_worker_or_ambiguous_file_settings() {
        let mut command = Command::new("synveda-gateway");
        command
            .env("DATABASE_URL", "gateway-dsn")
            .env("SYNVEDA_OIDC_ISSUERS", "issuer-json")
            .env("SYNVEDA_KMS_KEY", "gateway-key")
            .env("SYNVEDA_KMS_KEY_REF_FILE", "/operator/key-ref");
        for setting in HOST_GATEWAY_REMOVED_ENV {
            command.env(setting, "must-not-reach-gateway");
        }

        sanitize_host_gateway_environment(&mut command);

        for setting in HOST_GATEWAY_REMOVED_ENV {
            let inherited = command
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(setting))
                .map(|(_, value)| value);
            assert!(
                matches!(inherited, Some(None)),
                "{setting} was not removed from the host gateway environment"
            );
        }
        for retained in [
            "DATABASE_URL",
            "SYNVEDA_OIDC_ISSUERS",
            "SYNVEDA_KMS_KEY",
            "SYNVEDA_KMS_KEY_REF_FILE",
        ] {
            let value = command
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(retained))
                .and_then(|(_, value)| value);
            assert!(value.is_some(), "{retained} was removed unexpectedly");
        }
    }

    /// The other direction, so the check cannot pass by always failing.
    #[test]
    fn a_live_gateway_is_alive() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let started = Started {
            pid: child.id(),
            logfile: scratch("alive").join("gateway.log"),
        };
        let alive = started.still_alive();
        let _ = child.kill();
        let _ = child.wait();
        assert!(alive.is_ok(), "{alive:?}");
    }

    /// `log_tail` is the error's evidence, so it has to survive the states a
    /// log is actually in — including not existing, which is what a gateway
    /// that failed before opening one leaves behind.
    #[test]
    fn log_tail_quotes_the_end_and_survives_a_missing_file() {
        let dir = scratch("logtail");
        let logfile = dir.join("gateway.log");
        std::fs::write(&logfile, "one\ntwo\nthree\nfour\n").unwrap();
        let tail = log_tail(&logfile, 2);
        assert!(tail.contains("three") && tail.contains("four"), "{tail}");
        assert!(!tail.contains("one"), "took more than asked for: {tail}");
        // Order preserved, not reversed by the way the last lines are taken.
        assert!(tail.find("three") < tail.find("four"), "{tail}");

        let missing = log_tail(&dir.join("absent.log"), 4);
        assert!(missing.contains("no log at"), "{missing}");

        std::fs::write(&logfile, "").unwrap();
        assert!(log_tail(&logfile, 4).contains("is empty"));
    }

    /// Lays out an installed release the way `install.sh` does, minus the
    /// binaries' contents.
    fn installed_bundle(home: &Path, version: &str) {
        std::fs::create_dir_all(home.join("profile/rauthy")).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("profile/docker-compose.yml"), "name: synveda\n").unwrap();
        std::fs::write(home.join("profile/version"), format!("{version}\n")).unwrap();
        std::fs::write(home.join("bin/synveda-gateway"), "").unwrap();
    }

    #[test]
    fn an_installed_bundle_resolves_every_path_init_needs() {
        let home = scratch("bundle");
        installed_bundle(&home, "9.9.9");
        let profile = Profile::from_explicit_compose_file(&home.join("profile/docker-compose.yml"))
            .expect("a bundle");

        assert_eq!(
            profile.compose_file(),
            home.join("profile/docker-compose.yml")
        );
        // State lives under the install root, not inside the profile, so
        // unpacking a new profile over an old one does not delete the
        // record of what is running.
        assert_eq!(profile.state_dir(), home.join("data"));
        assert_eq!(
            gateway_binary(&profile).unwrap(),
            home.join("bin/synveda-gateway")
        );
        // A bundle's compose file has no build context, by construction.
        assert!(!profile.may_build());
        assert!(profile.describe().contains("9.9.9"));
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_checkouts_compose_file_still_resolves_the_way_it_always_did() {
        let root = scratch("checkout");
        std::fs::create_dir_all(root.join("deploy/compose")).unwrap();
        std::fs::create_dir_all(root.join("target/release")).unwrap();
        std::fs::write(
            root.join("deploy/compose/docker-compose.yml"),
            "name: synveda\n",
        )
        .unwrap();
        std::fs::write(root.join("target/release/synveda-gateway"), "").unwrap();

        let profile =
            Profile::from_explicit_compose_file(&root.join("deploy/compose/docker-compose.yml"))
                .expect("a checkout");

        assert_eq!(
            profile.compose_file(),
            root.join("deploy/compose/docker-compose.yml")
        );
        assert_eq!(profile.state_dir(), root.join("data"));
        assert_eq!(
            gateway_binary(&profile).unwrap(),
            root.join("target/release/synveda-gateway")
        );
        // Only a source tree may be asked to build one.
        assert!(profile.may_build());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_bundle_from_another_release_is_refused_rather_than_run() {
        let home = scratch("mismatch");
        installed_bundle(&home, "0.0.1-not-this-one");
        let profile = Profile::from_explicit_compose_file(&home.join("profile/docker-compose.yml"))
            .expect("a bundle");

        // The failure this prevents does not look like a version problem:
        // it looks like a service that will not start, or a variable the
        // gateway does not read (ADR-0065 decision 5).
        let refusal = profile.check_version().expect_err("a version mismatch");
        assert!(refusal.contains("0.0.1-not-this-one"), "{refusal}");
        assert!(refusal.contains(env!("CARGO_PKG_VERSION")), "{refusal}");

        // And the same bundle at this CLI's own version is fine.
        installed_bundle(&home, env!("CARGO_PKG_VERSION"));
        Profile::from_explicit_compose_file(&home.join("profile/docker-compose.yml"))
            .unwrap()
            .check_version()
            .expect("matching versions");

        // A checkout is exempt: its profile and its binaries come from one
        // tree, and `git status` is the check.
        Profile::Checkout { root: home.clone() }
            .check_version()
            .expect("a checkout is never version-checked");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_console_bundle_is_found_when_built_and_absent_otherwise() {
        let home = scratch("console");
        installed_bundle(&home, env!("CARGO_PKG_VERSION"));
        let profile = Profile::from_explicit_compose_file(&home.join("profile/docker-compose.yml"))
            .expect("a bundle");

        // No bundle is not an error — ADR-0056 decision 1 makes it a 404,
        // because a static asset must not be a dependency of the audit log.
        assert_eq!(profile.console_dir(), None);

        // An empty directory is not a bundle either. The gateway serves
        // `index.html`, so that is the file that decides.
        std::fs::create_dir_all(home.join("console")).unwrap();
        assert_eq!(profile.console_dir(), None);

        std::fs::write(home.join("console/index.html"), "<!doctype html>").unwrap();
        assert_eq!(profile.console_dir(), Some(home.join("console")));
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_deployment_gets_a_key_plane_and_keeps_the_same_one() {
        // Without this the gateway boots `Kms::Disabled` and **the console
        // cannot be signed in to**: a console session seals its tokens
        // under the deployment key, so `/auth/callback` fails with
        // "not found: encryption key for deployment". That is what shipped
        // — CNSL-1 built the console before TEN-4 sealed its sessions, and
        // nothing minted the key it started needing.
        let home = scratch("kek");
        let profile = Profile::Bundle {
            home: home.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };

        let first = ensure_kek(&profile).expect("a minted key");
        assert_eq!(first.len(), 64, "a 32-byte key as hex: {first}");
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()), "{first}");

        // Idempotent: a second `init` must not mint a new one, or every
        // re-run orphans every tenant key the previous one wrapped.
        let second = ensure_kek(&profile).expect("the same key");
        assert_eq!(first, second, "init minted a second KEK over the first");

        let path = home.join("data/kms.key");
        assert!(path.is_file(), "no key file at {}", path.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "a world-readable KEK looks like a KEK");
        }
        std::fs::remove_dir_all(&home).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_local_key_file_is_private_bounded_and_key_shaped() {
        use std::os::unix::fs::symlink;

        let home = scratch("kek-refusals");
        let profile = Profile::Bundle {
            home: home.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let state = home.join("data");
        ensure_private_directory(&state).expect("private state directory");
        let path = state.join("kms.key");

        write_private_file_atomically(&path, b"not-a-key\n").expect("malformed key fixture");
        let malformed = ensure_kek(&profile).expect_err("malformed existing key must be refused");
        assert!(malformed.contains("32-byte hexadecimal key"), "{malformed}");
        assert!(!malformed.contains("not-a-key"), "{malformed}");

        write_private_file_atomically(&path, &[b'a'; 66]).expect("oversized key fixture");
        let oversized = ensure_kek(&profile).expect_err("oversized existing key must be refused");
        assert!(oversized.contains("private-file bound"), "{oversized}");

        std::fs::remove_file(&path).expect("remove owned fixture");
        let target = state.join("attacker-key");
        write_private_file_atomically(&target, format!("{}\n", "ab".repeat(32)).as_bytes())
            .expect("symlink target fixture");
        symlink(&target, &path).expect("key symlink fixture");
        let linked = ensure_kek(&profile).expect_err("linked existing key must be refused");
        assert!(linked.contains("regular non-symlink"), "{linked}");

        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn private_file_replacement_is_complete_and_idempotent() {
        let dir = scratch("private-file-replacement");
        let path = dir.join("secret");
        write_private_file_atomically(&path, b"first\n").expect("first private write");
        write_private_file_atomically(&path, b"second\n").expect("replacement private write");
        assert_eq!(std::fs::read(&path).expect("read replacement"), b"second\n");
        let temporary_prefix = format!(
            ".{}.{}.",
            path.file_name().and_then(|name| name.to_str()).unwrap(),
            std::process::id()
        );
        assert!(
            std::fs::read_dir(&dir)
                .expect("list private-write fixture")
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&temporary_prefix)),
            "atomic replacement retained a temporary file"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_env_file_carries_the_key_the_container_path_needs() {
        let dir = scratch("kek-env");
        let path = dir.join(".env");
        let kek = "ab".repeat(32);
        write_env_file(
            &path,
            &Plan {
                slug: "acme".to_owned(),
                name: "ACME".to_owned(),
                embedder: "deterministic".to_owned(),
                issuer: None,
                dry_run: false,
            },
            TenantId::new(),
            RAUTHY_ISSUER,
            &kek,
            None,
        )
        .unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains(&format!("SYNVEDA_KMS_KEY={kek}")),
            "the compose gateway would boot without a key plane: {written}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "this file now holds the KEK");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_env_file_uses_the_validated_runtime_authorities() {
        const ROLES_JSON: &str = r#"{"migrator":"migrator","gateway":"gateway","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["identity"],"isolated_peer_roles":[]}"#;
        let dir = scratch("database-authorities-env");
        let path = dir.join(".env");
        let database = validate_init_database_plan(
            init_test_roles(&["identity"]),
            ROLES_JSON.to_owned(),
            "postgres://migrator:migrator-secret@db.example.test/synveda".to_owned(),
            "postgres://gateway:gateway-secret@db.example.test/synveda".to_owned(),
            "postgres://worker:worker-secret@db.example.test/synveda".to_owned(),
        )
        .expect("one validated authority plan");
        write_env_file(
            &path,
            &Plan {
                slug: "acme".to_owned(),
                name: "ACME".to_owned(),
                embedder: "deterministic".to_owned(),
                issuer: None,
                dry_run: false,
            },
            TenantId::new(),
            RAUTHY_ISSUER,
            &"00".repeat(32),
            Some(&database),
        )
        .expect("write resolved runtime authorities");

        let written = std::fs::read_to_string(&path).expect("read generated env file");
        assert!(
            written.contains(
                "SYNVEDA_GATEWAY_DATABASE_URL=postgres://gateway:gateway-secret@db.example.test/synveda\n"
            ),
            "gateway authority was not preserved"
        );
        assert!(
            written.contains(
                "SYNVEDA_WORKER_DATABASE_URL=postgres://worker:worker-secret@db.example.test/synveda\n"
            ),
            "worker authority was not preserved"
        );
        assert!(
            written.contains(&format!("SYNVEDA_DATABASE_ROLES={ROLES_JSON}\n")),
            "role contract was not preserved"
        );
        assert!(
            !written.contains("migrator-secret"),
            "the migrator credential must never enter runtime configuration"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_upgraded_binary_is_a_changed_configuration() {
        // The upgrade path's own failure, measured on 0.1.0 → 0.1.1:
        // `install.sh` replaced `bin/synveda-gateway`, the next `init`
        // reported "already running with this configuration" and healthy,
        // and the *previous release* kept serving. Convergence compared the
        // configuration and the liveness, and an upgrade changes neither.
        let home = scratch("upgrade");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        let binary = home.join("bin/synveda-gateway");
        let profile = Profile::Bundle {
            home: home.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };

        std::fs::write(&binary, b"the 0.1.0 gateway").unwrap();
        let before = binary_stamp(&profile);

        // A reinstall: different bytes, and a modification time that moves.
        std::fs::write(&binary, b"the 0.1.1 gateway, which is longer").unwrap();
        let after = binary_stamp(&profile);
        assert_ne!(
            before, after,
            "a replaced gateway must change the fingerprint, or `init` leaves \
             the previous release serving and calls it healthy"
        );

        // And an absent binary is a stable answer rather than a panic: the
        // checkout path reaches here before anybody has run `cargo build`.
        std::fs::remove_file(&binary).unwrap();
        assert_eq!(binary_stamp(&profile), "none");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn the_tei_image_follows_the_architecture_the_makefile_has_always_followed() {
        // Apple Silicon is half of what OPS-8 ships binaries for, and
        // upstream versions only the amd64 tag — so arm64 is pinned by
        // commit and amd64 falls through to the compose default.
        assert_eq!(
            tei_image_for("aarch64", false),
            Some("ghcr.io/huggingface/text-embeddings-inference:cpu-arm64-sha-4150561")
        );
        assert_eq!(tei_image_for("x86_64", false), None);
        // An operator who pinned an image keeps it on every architecture.
        assert_eq!(tei_image_for("aarch64", true), None);
        assert_eq!(tei_image_for("x86_64", true), None);
    }

    #[test]
    fn the_env_file_carries_json_without_quotes_compose_would_keep() {
        let dir = std::env::temp_dir().join(format!("synveda-init-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        let tenant = TenantId::new();
        write_env_file(
            &path,
            &Plan {
                slug: "acme".to_owned(),
                name: "ACME".to_owned(),
                embedder: "deterministic".to_owned(),
                issuer: None,
                dry_run: false,
            },
            tenant,
            RAUTHY_ISSUER,
            "00".repeat(32).as_str(),
            None,
        )
        .unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let line = written
            .lines()
            .find(|line| line.starts_with("SYNVEDA_OIDC_ISSUERS="))
            .expect("the issuer line");
        let value = line.trim_start_matches("SYNVEDA_OIDC_ISSUERS=");
        assert!(
            !value.starts_with('\''),
            "compose keeps outer quotes: {value}"
        );
        assert!(
            !value.starts_with('"'),
            "compose keeps outer quotes: {value}"
        );
        // It has to parse as what the gateway's own config reader expects.
        let parsed: Value = serde_json::from_str(value).expect("valid JSON on one line");
        assert_eq!(parsed[0]["client_id"], json!("synveda"));
        assert_eq!(parsed[0]["audience"], json!("synveda-api"));
        assert_eq!(parsed[0]["tenant"]["static"]["tenant_id"], json!(tenant));
        assert!(!value.contains('\n'), "one line only");
        // The default path must not point the gateway at an embedder it
        // was not asked to start (ADR-0055 decision 5).
        assert!(written.contains("SYNVEDA_EMBEDDER=deterministic"));
        assert!(!written.contains("SYNVEDA_TEI_URL"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_install_path_never_sets_a_dev_jwt_secret() {
        let dir = std::env::temp_dir().join(format!("synveda-init-jwt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        write_env_file(
            &path,
            &Plan {
                slug: "acme".to_owned(),
                name: "ACME".to_owned(),
                embedder: "tei".to_owned(),
                issuer: None,
                dry_run: false,
            },
            TenantId::new(),
            RAUTHY_ISSUER,
            "00".repeat(32).as_str(),
            None,
        )
        .unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        // ADR-0010 decision 3 and ADR-0055 decision 2: the two auth modes
        // never coexist, and the installer is the place that used to need
        // them to (a second gateway, purely to seed three nodes).
        assert!(
            !written.contains("SYNVEDA_DEV_JWT_SECRET"),
            "the install path must not carry a dev secret: {written}"
        );
        assert!(written.contains("SYNVEDA_TEI_URL=http://tei:80"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_deployments_on_one_laptop_do_not_share_an_operator() {
        assert_ne!(operator_email("acme"), operator_email("globex"));
    }

    #[test]
    fn database_errors_redact_passwords() {
        let rendered = redacted_database_url(
            "postgres://admin:user-secret@example.test/synveda?sslmode=require&password=query-secret#fragment-secret",
        );
        assert!(
            !rendered.contains("secret"),
            "credential leaked: {rendered}"
        );
        assert!(
            rendered.contains("REDACTED"),
            "redaction absent: {rendered}"
        );
        assert!(!rendered.contains('?'), "query retained: {rendered}");
        assert!(!rendered.contains('#'), "fragment retained: {rendered}");
    }

    #[test]
    fn database_url_direct_file_resolution_is_bounded_and_content_free() {
        let implicit = resolve_database_url(None, None).unwrap();
        assert_eq!(implicit.value, DEFAULT_DATABASE_URL);
        assert!(!implicit.explicitly_configured);

        let direct = resolve_database_url(Some(OsString::from("direct-value")), None).unwrap();
        assert_eq!(direct.value, "direct-value");
        assert!(direct.explicitly_configured);

        let empty = resolve_database_url(Some(OsString::new()), None).unwrap();
        assert!(empty.value.is_empty());
        assert!(empty.explicitly_configured);

        let dir = scratch("database-url-file");
        let path = dir.join("database-url");
        std::fs::write(&path, "postgres://app:secret@db.example.test/synveda\r\n").unwrap();
        let from_file = resolve_database_url(None, Some(path.clone().into_os_string())).unwrap();
        assert_eq!(
            from_file.value,
            "postgres://app:secret@db.example.test/synveda"
        );
        assert!(from_file.explicitly_configured);

        std::fs::write(&path, "\n").unwrap();
        let empty_file = resolve_database_url(None, Some(path.clone().into_os_string())).unwrap();
        assert!(empty_file.value.is_empty());
        assert!(empty_file.explicitly_configured);

        const SENTINEL: &str = "SYNVEDA_DATABASE_URL_FILE_SECRET";
        let ambiguity = resolve_database_url(
            Some(OsString::from(SENTINEL)),
            Some(dir.join(SENTINEL).into_os_string()),
        )
        .expect_err("direct and file settings must be refused before either is read");
        assert_eq!(
            ambiguity,
            "DATABASE_URL and DATABASE_URL_FILE are mutually exclusive; configure exactly one"
        );
        assert!(!ambiguity.contains(SENTINEL), "{ambiguity}");

        for (file_name, bytes, expected) in [
            (
                "invalid-utf8",
                vec![0xff],
                "DATABASE_URL_FILE must contain valid UTF-8",
            ),
            (
                "nul",
                format!("postgres://app:{SENTINEL}@db.example.test/synveda\0").into_bytes(),
                "DATABASE_URL_FILE must not contain NUL bytes",
            ),
            (
                "oversized",
                vec![b'x'; (MAX_DATABASE_URL_FILE_BYTES + 1) as usize],
                "DATABASE_URL_FILE exceeds the 1048576 byte startup bound",
            ),
        ] {
            let invalid = dir.join(file_name);
            std::fs::write(&invalid, bytes).unwrap();
            let error = resolve_database_url(None, Some(invalid.into_os_string()))
                .expect_err("invalid file must be refused");
            assert_eq!(error, expected);
            assert!(!error.contains(SENTINEL), "{error}");
        }

        let directory_error = resolve_database_url(None, Some(dir.clone().into_os_string()))
            .expect_err("directory must not be treated as a setting file");
        assert_eq!(
            directory_error,
            "DATABASE_URL_FILE must name a regular file"
        );
        let missing = dir.join(SENTINEL);
        let missing_error = resolve_database_url(None, Some(missing.into_os_string()))
            .expect_err("missing file must be refused");
        assert_eq!(missing_error, "DATABASE_URL_FILE cannot be read");
        assert!(!missing_error.contains(SENTINEL), "{missing_error}");
        assert_eq!(
            resolve_database_url(None, Some(OsString::new()))
                .expect_err("empty file path must be refused"),
            "DATABASE_URL_FILE must name a file"
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn database_url_direct_non_utf8_is_content_free() {
        use std::os::unix::ffi::OsStringExt as _;

        let error = resolve_database_url(Some(OsString::from_vec(vec![0xff])), None)
            .expect_err("non-UTF-8 direct setting must be refused");
        assert_eq!(error, "DATABASE_URL must be valid UTF-8");
    }

    #[cfg(unix)]
    #[test]
    fn database_url_file_refuses_a_fifo_without_waiting_for_a_writer() {
        let dir = scratch("database-url-fifo");
        let path = dir.join("database-url");
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("create FIFO fixture");
        assert!(status.success(), "mkfifo failed");

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read_database_url_file("DATABASE_URL_FILE", &path));
        });
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("FIFO path blocked the configuration boundary");
        let error = result.expect_err("FIFO must not be accepted as a setting file");
        assert!(error.contains("regular file"), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn database_url_file_accepts_a_projected_secret_symlink() {
        use std::os::unix::fs::symlink;

        let dir = scratch("database-url-projected-secret");
        let target = dir.join("..data-url");
        let path = dir.join("database-url");
        std::fs::write(&target, b"postgres://app:secret@db.example.test/synveda\n")
            .expect("write projected-secret target");
        symlink(&target, &path).expect("link projected-secret path");
        assert_eq!(
            read_database_url_file("DATABASE_URL_FILE", &path).expect("read projected secret"),
            "postgres://app:secret@db.example.test/synveda"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn init_accepts_only_one_explicit_database_authority_plan() {
        const ROLES_JSON: &str = r#"{"migrator":"migrator","gateway":"gateway","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["identity"],"isolated_peer_roles":[]}"#;
        let accepted = validate_init_database_plan(
            init_test_roles(&["identity"]),
            ROLES_JSON.to_owned(),
            "postgres://migrator:secret@db.example.test:5432/synveda".to_owned(),
            "postgres://gateway:secret@db.example.test:5432/synveda".to_owned(),
            "postgres://worker:secret@db.example.test:5432/synveda".to_owned(),
        )
        .expect("one explicit provider-neutral database plan");
        assert_eq!(accepted.roles.migrator(), "migrator");
        assert_eq!(accepted.roles.gateway(), "gateway");
        assert_eq!(accepted.roles.worker(), "worker");

        const SENTINEL: &str = "SYNVEDA_INIT_PLAN_SECRET";
        for (migrator, gateway, worker) in [
            (
                format!("postgres://wrong:{SENTINEL}@db.example.test/synveda"),
                format!("postgres://gateway:{SENTINEL}@db.example.test/synveda"),
                format!("postgres://worker:{SENTINEL}@db.example.test/synveda"),
            ),
            (
                format!("postgres://migrator:{SENTINEL}@db.example.test/synveda"),
                format!("postgres://wrong:{SENTINEL}@db.example.test/synveda"),
                format!("postgres://worker:{SENTINEL}@db.example.test/synveda"),
            ),
            (
                format!("postgres://migrator:{SENTINEL}@db.example.test/synveda"),
                format!("postgres://gateway:{SENTINEL}@db.example.test/synveda"),
                format!("postgres://wrong:{SENTINEL}@db.example.test/synveda"),
            ),
        ] {
            let error = validate_init_database_plan(
                init_test_roles(&["identity"]),
                ROLES_JSON.to_owned(),
                migrator,
                gateway,
                worker,
            )
            .err()
            .expect("every principal mismatch is refused");
            assert!(error.contains("role contract"), "{error}");
            assert!(!error.contains(SENTINEL), "{error}");
        }

        for (gateway, worker) in [
            (
                "postgres://gateway:secret@other.example.test/synveda",
                "postgres://worker:secret@db.example.test/synveda",
            ),
            (
                "postgres://gateway:secret@db.example.test:5440/synveda",
                "postgres://worker:secret@db.example.test/synveda",
            ),
            (
                "postgres://gateway:secret@db.example.test/other",
                "postgres://worker:secret@db.example.test/synveda",
            ),
        ] {
            let error = validate_init_database_plan(
                init_test_roles(&["identity"]),
                ROLES_JSON.to_owned(),
                "postgres://migrator:secret@db.example.test/synveda".to_owned(),
                gateway.to_owned(),
                worker.to_owned(),
            )
            .err()
            .expect("split database endpoints are refused");
            assert!(error.contains("one exact PostgreSQL"), "{error}");
        }

        let socket_error = validate_init_database_plan(
            init_test_roles(&["identity"]),
            ROLES_JSON.to_owned(),
            "postgres:///synveda?host=%2Fvar%2Frun%2Fpostgresql&user=migrator&password=secret"
                .to_owned(),
            "postgres://gateway:secret@localhost/synveda".to_owned(),
            "postgres:///synveda?host=%2Fvar%2Frun%2Fpostgresql&user=worker&password=secret"
                .to_owned(),
        )
        .err()
        .expect("a socket/TCP split is refused");
        assert!(socket_error.contains("socket"), "{socket_error}");

        let forbidden = validate_init_database_plan(
            init_test_roles(&["identity"]),
            ROLES_JSON.to_owned(),
            "postgres://migrator:secret@db.example.test?dbname=identity".to_owned(),
            "postgres://gateway:secret@db.example.test?dbname=identity".to_owned(),
            "postgres://worker:secret@db.example.test?dbname=identity".to_owned(),
        )
        .err()
        .expect("an isolated peer target is refused");
        assert!(forbidden.contains("isolated peer"), "{forbidden}");
    }

    #[test]
    fn init_database_urls_have_no_implicit_or_half_configured_fallback() {
        assert!(
            !resolve_database_url(None, None)
                .expect("legacy direct-command default")
                .explicitly_configured,
            "the legacy default remains identifiable so init can reject it"
        );
        for (direct, file) in [
            (None, None),
            (Some(OsString::from("direct")), Some(OsString::from("file"))),
        ] {
            let error = resolve_required_database_url(
                "SYNVEDA_GATEWAY_DATABASE_URL",
                "SYNVEDA_GATEWAY_DATABASE_URL_FILE",
                direct,
                file,
            )
            .expect_err("init requires exactly one source for each runtime URL");
            assert!(error.contains("SYNVEDA_GATEWAY_DATABASE_URL"), "{error}");
        }
    }

    #[tokio::test]
    async fn init_cutover_gate_refuses_before_any_lifecycle_work() {
        let refusal = init(Plan {
            slug: "must-not-be-read".to_owned(),
            name: "must-not-be-read".to_owned(),
            embedder: "must-not-be-read".to_owned(),
            issuer: Some("not-a-url".to_owned()),
            dry_run: true,
        })
        .await
        .expect_err("CPR-45 init must remain closed");
        assert!(
            refusal.contains("CPR-45 Docker reference cutover"),
            "{refusal}"
        );
        assert!(
            refusal.contains("before profile discovery, Compose, secret-file or database mutation"),
            "{refusal}"
        );
        assert!(refusal.contains("clean-volume Keycloak"), "{refusal}");
    }

    #[test]
    fn credential_fingerprints_change_without_containing_credentials() {
        let first = secret_fingerprint("postgres://role:first-secret@db/synveda");
        let second = secret_fingerprint("postgres://role:second-secret@db/synveda");
        assert_ne!(
            first, second,
            "credential rotation must restart the gateway"
        );
        assert_eq!(first.len(), 64);
        assert!(!first.contains("secret"));
    }

    #[test]
    fn runtime_targets_cannot_split_one_deployment_across_databases() {
        let bootstrap = synveda_store::runtime_role::DatabaseIdentity {
            database: "synveda".to_owned(),
            cluster_system_identifier: "cluster-1".to_owned(),
            database_oid: 16_384,
            postmaster_started_at: chrono::DateTime::UNIX_EPOCH,
        };
        assert!(
            require_one_database_target(&bootstrap, &bootstrap, &bootstrap).is_ok(),
            "three credentials for one target are accepted"
        );

        let another_database = synveda_store::runtime_role::DatabaseIdentity {
            database: "another".to_owned(),
            ..bootstrap.clone()
        };
        assert!(
            require_one_database_target(&bootstrap, &bootstrap, &another_database).is_err(),
            "a worker database split is refused"
        );

        let another_server = synveda_store::runtime_role::DatabaseIdentity {
            cluster_system_identifier: "cluster-2".to_owned(),
            ..bootstrap.clone()
        };
        assert!(
            require_one_database_target(&bootstrap, &another_server, &bootstrap).is_err(),
            "a gateway server split is refused"
        );

        let another_primary_generation = synveda_store::runtime_role::DatabaseIdentity {
            postmaster_started_at: bootstrap.postmaster_started_at + chrono::Duration::seconds(1),
            ..bootstrap.clone()
        };
        assert!(
            require_one_database_target(&bootstrap, &bootstrap, &another_primary_generation)
                .is_err(),
            "a separately started writable fork is refused"
        );
    }

    /// Both deployment logins are subject to forced RLS and can see a tenant
    /// only after the application establishes its transaction-local tenant
    /// context. This is the executable boundary between bootstrap authority
    /// and the one normal gateway runtime (CPR-36, ADR-0095).
    #[tokio::test]
    async fn compose_runtime_logins_are_distinct_and_rls_enforced() {
        let Some(admin_url) = private_test_database_url("SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE")
        else {
            eprintln!(
                "SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE not set; skipping CPR-36 database acceptance"
            );
            return;
        };
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&admin_url)
            .await
            .expect("connect as deployment owner");
        let database_roles = database_roles().expect("read the exact test role contract");
        verify_runtime_role(&admin, database_roles.gateway(), &database_roles)
            .await
            .expect("accept the least-privilege gateway login");
        verify_runtime_role(&admin, database_roles.worker(), &database_roles)
            .await
            .expect("accept the least-privilege worker login");
        assert!(
            verify_runtime_role(&admin, database_roles.migrator(), &database_roles)
                .await
                .is_err(),
            "the bootstrap owner must never pass runtime-role validation",
        );

        let migrator_url = private_test_database_url("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE")
            .expect("test harness migrator URL file");
        let migrator = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&migrator_url)
            .await
            .expect("connect as synveda_migrator");
        let runtime_url = private_test_database_url("SYNVEDA_TEST_GATEWAY_DATABASE_URL_FILE")
            .expect("test harness gateway URL file");
        let runtime = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&runtime_url)
            .await
            .expect("connect as synveda_gateway");

        let refused_id = TenantId::new();
        let refused_suffix = refused_id.to_string();
        let refused_slug = format!(
            "cpr36-refused-{}",
            &refused_suffix[refused_suffix.len() - 12..]
        );
        assert_eq!(
            tenant_admission_footprint(&runtime, refused_id).await,
            [0; 4]
        );
        assert!(
            crate::create_tenant_with_admission_id(
                &admin,
                &database_roles,
                refused_id,
                &refused_slug,
                "CPR-36 refused owner admission",
                TenantStatus::Active,
                crate::TenantAdmission::Standard,
            )
            .await
            .is_err(),
            "the deployment owner must not admit a tenant"
        );
        assert_eq!(
            tenant_admission_footprint(&runtime, refused_id).await,
            [0; 4],
            "refused owner admission wrote tenant, scope, grant or audit state"
        );

        let wrong_role_id = TenantId::new();
        let wrong_role_suffix = wrong_role_id.to_string();
        assert!(
            crate::create_tenant_with_admission_id(
                &runtime,
                &database_roles,
                wrong_role_id,
                &format!(
                    "cpr45-wrong-role-{}",
                    &wrong_role_suffix[wrong_role_suffix.len() - 12..]
                ),
                "CPR-45 refused gateway admission",
                TenantStatus::Active,
                crate::TenantAdmission::Standard,
            )
            .await
            .is_err(),
            "the gateway login must not admit a tenant"
        );
        assert_eq!(
            tenant_admission_footprint(&runtime, wrong_role_id).await,
            [0; 4],
            "wrong-role admission wrote tenant, scope, grant or audit state"
        );

        let wrong_epoch_id = TenantId::new();
        let wrong_epoch_suffix = wrong_epoch_id.to_string();
        let original_epoch = test_schema_epoch(&admin).await;
        let future_epoch = TestSchemaEpoch {
            epoch: synveda_store::epoch::CURRENT_EPOCH + 1,
            updated_at: chrono::Utc::now(),
        };
        set_test_schema_epoch(&admin, &future_epoch).await;
        let wrong_epoch_result = crate::create_tenant_with_admission_id(
            &migrator,
            &database_roles,
            wrong_epoch_id,
            &format!(
                "cpr45-wrong-epoch-{}",
                &wrong_epoch_suffix[wrong_epoch_suffix.len() - 12..]
            ),
            "CPR-45 refused future-epoch admission",
            TenantStatus::Active,
            crate::TenantAdmission::Standard,
        )
        .await;
        // This test runs only through db-test.sh's disposable fixture. Restore
        // the exact marker before inspecting the refusal; on abnormal process
        // termination the harness retains the whole failed fixture as evidence.
        set_test_schema_epoch(&admin, &original_epoch).await;
        assert!(
            wrong_epoch_result.is_err(),
            "a future schema epoch must refuse tenant admission"
        );
        assert_eq!(
            tenant_admission_footprint(&runtime, wrong_epoch_id).await,
            [0; 4],
            "wrong-epoch admission wrote tenant, scope, grant or audit state"
        );

        let suffix_source = TenantId::new().to_string();
        let tenant = crate::create_tenant(
            &migrator,
            &database_roles,
            &format!("cpr36-{}", &suffix_source[suffix_source.len() - 12..]),
            "CPR-36 deployment acceptance",
            TenantStatus::Active,
        )
        .await
        .expect("admit an isolated tenant");
        let tenant_id = tenant.id;

        let worker_url = private_test_database_url("SYNVEDA_TEST_WORKER_DATABASE_URL_FILE")
            .expect("test harness worker URL file");
        let worker = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&worker_url)
            .await
            .expect("connect as synveda_worker");
        synveda_store::runtime_role::verify(&runtime, database_roles.gateway(), &database_roles)
            .await
            .expect("gateway session role sentinel");
        synveda_store::runtime_role::verify(&worker, database_roles.worker(), &database_roles)
            .await
            .expect("worker session role sentinel");
        let migrator_identity = synveda_store::runtime_role::database_identity(&migrator)
            .await
            .expect("migrator database identity");
        let gateway_identity = synveda_store::runtime_role::database_identity(&runtime)
            .await
            .expect("healthy gateway database identity");
        let worker_identity = synveda_store::runtime_role::database_identity(&worker)
            .await
            .expect("healthy worker database identity");
        require_one_database_target(&migrator_identity, &gateway_identity, &worker_identity)
            .expect("all healthy deployment credentials target one database");
        assert!(
            synveda_store::runtime_role::verify(
                &worker,
                database_roles.gateway(),
                &database_roles,
            )
                .await
                .is_err(),
            "worker credentials must not pass as gateway credentials"
        );

        sqlx::query!("drop schema if exists synveda_cli_role_drift cascade")
            .execute(&admin)
            .await
            .expect("remove stale role-drift schema");
        sqlx::query!("create schema synveda_cli_role_drift authorization synveda_gateway")
            .execute(&admin)
            .await
            .expect("give gateway a non-public schema");
        let catalogue_rejected =
            verify_runtime_role(&admin, database_roles.gateway(), &database_roles)
                .await
                .is_err();
        let connection_rejected = verify_runtime_database_url(
            "SYNVEDA_GATEWAY_DATABASE_URL",
            &runtime_url,
            database_roles.gateway(),
            &database_roles,
        )
        .await
        .is_err();
        sqlx::query!("drop schema synveda_cli_role_drift")
            .execute(&admin)
            .await
            .expect("restore non-owning gateway role");
        assert!(
            catalogue_rejected && connection_rejected,
            "non-public schema ownership must fail both deployment sentinels"
        );

        sqlx::query!(
            r#"do $synveda$
               begin
                 if not exists (
                   select 1 from pg_catalog.pg_roles
                    where rolname = 'synveda_cli_role_grantor'
                 ) then
                   create role synveda_cli_role_grantor nologin;
                 end if;
               end
               $synveda$"#
        )
        .execute(&admin)
        .await
        .expect("create alternate CLI membership grantor");
        sqlx::query!(
            r#"alter role synveda_cli_role_grantor
                  with nologin inherit nosuperuser nocreatedb nocreaterole
                       noreplication nobypassrls connection limit -1"#
        )
        .execute(&admin)
        .await
        .expect("converge alternate CLI membership grantor");
        sqlx::query!(
            "grant synveda_app to synveda_cli_role_grantor with admin true, inherit true, set true"
        )
        .execute(&admin)
        .await
        .expect("authorise alternate CLI membership grantor");
        sqlx::query!("revoke synveda_app from synveda_gateway granted by synveda_cli_role_grantor")
            .execute(&admin)
            .await
            .expect("remove stale alternate gateway grant");
        sqlx::query!(
            "grant synveda_app to synveda_gateway with admin true, inherit true, set true granted by synveda_cli_role_grantor"
        )
        .execute(&admin)
        .await
        .expect("add concurrent unsafe gateway grant");
        let duplicate_catalogue_rejected =
            verify_runtime_role(&admin, database_roles.gateway(), &database_roles)
                .await
                .is_err();
        let duplicate_connection_rejected = verify_runtime_database_url(
            "SYNVEDA_GATEWAY_DATABASE_URL",
            &runtime_url,
            database_roles.gateway(),
            &database_roles,
        )
        .await
        .is_err();
        sqlx::query!("revoke synveda_app from synveda_gateway granted by synveda_cli_role_grantor")
            .execute(&admin)
            .await
            .expect("remove concurrent unsafe gateway grant");
        sqlx::query!("revoke synveda_app from synveda_cli_role_grantor")
            .execute(&admin)
            .await
            .expect("remove alternate CLI grantor capability");
        sqlx::query!("drop role synveda_cli_role_grantor")
            .execute(&admin)
            .await
            .expect("drop alternate CLI membership grantor");
        assert!(
            duplicate_catalogue_rejected && duplicate_connection_rejected,
            "a safe grant must not hide an unsafe grant from another grantor"
        );

        let without_context = sqlx::query_scalar!("select count(*) from scopes")
            .fetch_one(&runtime)
            .await
            .expect("query under forced RLS")
            .unwrap_or_default();
        assert_eq!(without_context, 0, "a missing tenant GUC must fail closed");

        let mut tx = synveda_store::rls::begin_tenant_tx(&runtime, tenant_id)
            .await
            .expect("establish the tenant context");
        let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant_id)
            .await
            .expect("create through the RLS-enforced application role");
        tx.commit().await.expect("commit the tenant root");

        let mut visible = synveda_store::rls::begin_tenant_tx(&runtime, tenant_id)
            .await
            .expect("restore the tenant context");
        assert_eq!(
            synveda_store::scopes::get(&mut *visible, tenant_id, root.id)
                .await
                .expect("read the tenant root")
                .map(|scope| scope.id),
            Some(root.id),
        );
        visible
            .rollback()
            .await
            .expect("close the read transaction");

        let mut wrong_tenant = synveda_store::rls::begin_tenant_tx(&runtime, TenantId::new())
            .await
            .expect("establish a different tenant context");
        assert!(
            synveda_store::scopes::get(&mut *wrong_tenant, tenant_id, root.id)
                .await
                .expect("cross-tenant reads fail closed")
                .is_none(),
            "the runtime role crossed a tenant boundary",
        );
        wrong_tenant
            .rollback()
            .await
            .expect("close the cross-tenant transaction");

        let mut worker_visible = synveda_store::rls::begin_tenant_tx(&worker, tenant_id)
            .await
            .expect("establish worker tenant context");
        assert_eq!(
            synveda_store::scopes::get(&mut *worker_visible, tenant_id, root.id)
                .await
                .expect("worker reads through forced RLS")
                .map(|scope| scope.id),
            Some(root.id),
        );
        worker_visible
            .rollback()
            .await
            .expect("close worker tenant transaction");
    }
}
