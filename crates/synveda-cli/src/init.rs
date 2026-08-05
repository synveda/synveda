//! `synveda init` — a laptop to working governed memory (OPS-1, ADR-0055).
//!
//! The whole design of this file is what it *does not* do. It applies
//! migrations and admits a tenant, both of which are pre-existing audited
//! break-glass paths with no governed surface and no operator to run them
//! yet (ADR-0055 decision 1). It writes no scope, no identity, no role
//! binding and no record. Those arrive the way a customer's do:
//!
//!   * the **org root**, the operator's **identity** and their tenant-wide
//!     **org-admin** binding on the first `synveda login`, inside AUTH-2's
//!     provisioning transaction, chained under the operator's own subject
//!     (`ensure_root` + ADR-0015 decision 6, reused rather than copied);
//!   * **departments and teams** through `synveda hierarchy create`, a PDP
//!     decision at the parent scope per node;
//!   * **memory** through observe → extract → embed.
//!
//! That is not fastidiousness. An installer runs once, as root-equivalent,
//! before anybody is watching, and whatever it writes becomes the tenant's
//! history — so it is the single worst place in this product to keep a
//! shortcut past the PDP (seed §2.2, CLAUDE.md). It is also why the
//! bootstrap this replaces needed a *second gateway* running with
//! `SYNVEDA_DEV_JWT_SECRET` just to create three nodes, and why nothing
//! here does: the gateway starts once, in OIDC mode, and the dev secret
//! never appears on the install path (ADR-0010 decision 3).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use synveda_types::{TenantId, TenantStatus};

/// The bundled dev IdP, as `deploy/compose/docker-compose.yml` publishes
/// it. Dev-only credentials; the API key is the one in
/// `deploy/compose/rauthy/config.toml`, which exists precisely so a
/// bootstrap has something to authenticate with before a human does.
const RAUTHY_URL: &str = "http://localhost:8100";
const RAUTHY_ISSUER: &str = "http://localhost:8100/auth/v1/";
const RAUTHY_API_KEY: &str =
    "API-Key synveda-dev$6xxmjZD7Wqe9zWN1fWzOW1jA4uxAkFQ9rYlVFpxBzVgJ0xEj2KWSLiaRTZzKV1oz";
const GATEWAY_URL: &str = "http://127.0.0.1:8120";
/// The group AUTHZ-3 binds tenant-wide `org-admin` from at login.
const ADMIN_GROUP: &str = "synveda-admins";

/// What one `init` was asked for.
pub struct Plan {
    pub slug: String,
    pub name: String,
    pub embedder: String,
    pub issuer: Option<String>,
    pub demo: bool,
    pub dry_run: bool,
}

/// The demo organisation (ADR-0055 decision 8). Two departments, three
/// teams; the people are IdP users in convention-shaped groups, so AUTH-2
/// places each of them by the same mapping rule a customer's directory
/// drives (ADR-0013 decision 3) — nothing here assigns a scope directly.
const DEMO_DEPARTMENTS: &[(&str, &str)] = &[("eng", "Engineering"), ("sales", "Sales")];
const DEMO_TEAMS: &[(&str, &str, &str)] = &[
    ("eng", "platform", "Platform"),
    ("eng", "payments", "Payments"),
    ("sales", "emea", "EMEA"),
];
/// `(local-part, given, family, group)` — the group is convention-shaped
/// (`synveda-<department>-<team>`), which is what makes placement a
/// mapping rather than an assignment.
///
/// The address is completed with the tenant slug by [`demo_email`], for the
/// same reason [`operator_email`] does it: two deployments on one laptop
/// must not share a login. These were hard-coded at `@demo.localhost` while
/// the operator beside them was already slug-derived and carried a test
/// saying why — the rule was written down and then not applied here.
///
/// It is not only tidiness. `init` sets a password for everyone it creates,
/// and Rauthy refuses one it has seen in its last three; two deployments
/// writing the same address means the second cannot set its password and
/// [`ensure_user`] falls back to leaving whatever is there. A shared login
/// whose password nobody can restore is the failure that took
/// `demos/adpt-1-claude-code.sh` and `demos/auth-2-jit-provisioning.sh`
/// down together, and this constant is the same shape one layer up — with a
/// *different* `DEMO_PASSWORD` from the one those demos use, so the two
/// writers could never have agreed.
const DEMO_PEOPLE: &[(&str, &str, &str, &str)] = &[
    ("alice", "Alice", "Chen", "synveda-eng-platform"),
    ("bob", "Bob", "Okafor", "synveda-eng-platform"),
    ("carol", "Carol", "Diaz", "synveda-eng-payments"),
    ("dan", "Dan", "Novak", "synveda-sales-emea"),
];
const DEMO_PASSWORD: &str = "Synveda-Demo-Passw0rd!";

pub async fn init(plan: Plan) -> Result<(), String> {
    let started = Instant::now();
    let repo = repo_root()?;
    let compose_file = repo.join("deploy/compose/docker-compose.yml");
    if !compose_file.exists() {
        return Err(format!(
            "no compose file at {} — run `synveda init` from a Synveda checkout, \
             or set SYNVEDA_COMPOSE_FILE",
            compose_file.display()
        ));
    }

    let bundled = plan.issuer.is_none();
    let issuer = plan
        .issuer
        .clone()
        .unwrap_or_else(|| RAUTHY_ISSUER.to_owned());

    if plan.dry_run {
        return dry_run(&plan, &compose_file, &issuer, bundled);
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
    println!("    {}", services.join(", "));
    compose(
        &compose_file,
        &[&["up", "--detach", "--wait"], &services[..]].concat(),
    )?;

    // ── 2. schema ───────────────────────────────────────────────────────
    step(2, "applying migrations");
    let database_url = database_url();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(30))
        .connect(&database_url)
        .await
        .map_err(|err| format!("connect to {database_url}: {err}"))?;
    synveda_store::migrate(&pool)
        .await
        .map_err(|err| format!("apply migrations: {err}"))?;

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
            let tenant = crate::create_tenant(&pool, &plan.slug, &plan.name, TenantStatus::Active)
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

    // ── 5. the gateway ──────────────────────────────────────────────────
    step(5, "starting the gateway");
    let env_file = repo.join("deploy/compose/.env");
    write_env_file(&env_file, &plan, tenant_id, &issuer)?;
    println!("    configuration: {}", env_file.display());
    if containerised(&plan) {
        compose(
            &compose_file,
            &["up", "--detach", "--wait", "--build", "gateway"],
        )?;
    } else {
        start_host_gateway(&repo, &plan, tenant_id, &issuer)?;
    }
    wait_for_health(&format!("{GATEWAY_URL}/healthz"), Duration::from_secs(90)).await?;
    println!("    {GATEWAY_URL} healthy");

    // ── 6. the demo organisation, if asked for ──────────────────────────
    if plan.demo {
        if !bundled {
            return Err(
                "--demo builds its people in the bundled IdP; it cannot create \
                        users in your directory (ADR-0055 decision 4)"
                    .to_owned(),
            );
        }
        step(6, "the ACME demo organisation");
        println!("    people and groups are in the IdP; scopes are created after you log in");
        seed_demo_people(&plan.slug).await?;
    }

    let elapsed = started.elapsed();
    println!();
    println!("synveda: initialised in {}s", elapsed.as_secs());
    println!();
    println!("Next, and this is where the organisation starts to exist:");
    println!();
    println!("    synveda login --gateway {GATEWAY_URL}");
    println!();
    println!(
        "That first login provisions the org root `{}` from the tenant you just admitted,",
        plan.slug
    );
    println!("places you under it, and binds you tenant-wide org-admin — all of it audited");
    println!("under your own subject rather than an installer's. Then:");
    println!();
    println!("    synveda hierarchy list");
    if plan.demo {
        println!();
        println!("    # then build ACME's scopes, as yourself:");
        println!("    synveda init --demo --dry-run   # prints the exact commands");
        println!();
        println!("    # demo logins (bundled IdP): password {DEMO_PASSWORD}");
        for (local, _, _, group) in DEMO_PEOPLE {
            let email = demo_email(local, &plan.slug);
            println!("    #   {email:<26} {group}");
        }
    }
    if plan.embedder == "deterministic" {
        println!();
        println!("Embedder: deterministic (hash). Retrieval works and is exact for the");
        println!("lexical leg; semantic similarity is not meaningful. `--embedder tei`");
        println!("serves BGE-M3 — but choose before writing records: `record_embeddings`");
        println!("stores the model, and nothing in the product re-embeds a corpus.");
    }
    Ok(())
}

/// The demo organisation's shape, as the commands that build it. Printed
/// rather than executed for the reason in ADR-0055 decision 1: these are
/// governed creates that need the operator's own bearer, and the operator
/// has not logged in yet when `init` runs.
fn dry_run(plan: &Plan, compose_file: &Path, issuer: &str, bundled: bool) -> Result<(), String> {
    println!("synveda init --dry-run");
    println!();
    println!("  compose file   {}", compose_file.display());
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
        "  would start    postgres, jaeger{}{}",
        if bundled { ", rauthy" } else { "" },
        if plan.embedder == "tei" { ", tei" } else { "" },
    );
    println!("  would write    migrations, one tenant row (audited break-glass)");
    println!("  would NOT      write any scope, identity, role binding or record");
    if plan.demo {
        println!();
        println!("  after `synveda login`, ACME is built by these governed creates:");
        println!();
        println!("    root=$(synveda hierarchy root)");
        for (slug, name) in DEMO_DEPARTMENTS {
            println!(
                "    synveda hierarchy create --parent $root --kind department \\\n\
                 \x20     --slug {slug} --name '{name}'"
            );
        }
        for (department, slug, name) in DEMO_TEAMS {
            println!(
                "    synveda hierarchy create --parent ${department} --kind team \\\n\
                 \x20     --slug {slug} --name '{name}'"
            );
        }
    }
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
    println!("    Group claims drive placement: a `{ADMIN_GROUP}` member becomes");
    println!("    tenant-wide org-admin, and `synveda-<department>-<team>` places by");
    println!("    convention. This command configures an issuer; it does not");
    println!("    synchronise a directory.");
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
    println!("    operator: {operator}  (password {DEMO_PASSWORD})");
    Ok(())
}

async fn seed_demo_people(slug: &str) -> Result<(), String> {
    let http = idp_client()?;
    for (_, _, team_group) in DEMO_TEAMS
        .iter()
        .map(|(department, team, _)| (department, team, format!("synveda-{department}-{team}")))
    {
        ensure_group(&http, &team_group).await?;
    }
    for (local, given, family, group) in DEMO_PEOPLE {
        let email = demo_email(local, slug);
        ensure_user(&http, &email, given, family, group).await?;
        println!("    {email}  {group}");
    }
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
/// nothing else has set a different one in between. If something has, the
/// constant is in the history without being current, no `init` can put it
/// back, and the login is unrecoverable — which is precisely what happened
/// to `alice@demo.localhost` when this command and two demo scripts wrote
/// the same address with two different passwords. Slug-derived addresses
/// ([`demo_email`], [`operator_email`]) are what make the assumption true,
/// so they are the reason this stays simple rather than growing a
/// delete-and-recreate path — which would be wrong here anyway: the
/// operator goes through this function, and minting a new `sub` for them
/// would orphan the org-admin binding their deployment depends on.
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
    with_password["password"] = json!(DEMO_PASSWORD);
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

/// A demo person's login for this deployment, on [`operator_email`]'s rule
/// and for [`DEMO_PEOPLE`]'s reasons.
fn demo_email(local: &str, slug: &str) -> String {
    format!("{local}@{slug}.localhost")
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
    repo: &Path,
    plan: &Plan,
    tenant_id: TenantId,
    issuer: &str,
) -> Result<(), String> {
    let state = repo.join("data");
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
    let desired = format!("{issuers}\n{}\n{GATEWAY_URL}\n", plan.embedder);
    match (
        running_gateway(&pidfile),
        std::fs::read_to_string(&configfile).ok(),
    ) {
        (Some(pid), Some(current)) if current == desired => {
            println!("    already running with this configuration (pid {pid})");
            return Ok(());
        }
        (Some(pid), _) => {
            println!("    configuration changed — restarting (pid {pid})");
            let _ = Command::new("kill").arg(pid.to_string()).status();
            // Give it the moment it needs to release port 8120; a fresh
            // process that cannot bind is a worse failure than a slow one.
            std::thread::sleep(Duration::from_millis(500));
        }
        (None, _) => {}
    }

    let binary = gateway_binary(repo)?;
    let log = std::fs::File::create(&logfile)
        .map_err(|err| format!("create {}: {err}", logfile.display()))?;
    let errors = log
        .try_clone()
        .map_err(|err| format!("open {}: {err}", logfile.display()))?;

    let mut command = Command::new(&binary);
    command
        .current_dir(repo)
        .env("DATABASE_URL", database_url())
        .env("SYNVEDA_OIDC_ISSUERS", &issuers)
        .env("SYNVEDA_PUBLIC_URL", GATEWAY_URL)
        .env("SYNVEDA_LISTEN_ADDR", "127.0.0.1:8120")
        .env("SYNVEDA_EMBEDDER", &plan.embedder)
        // ADR-0010 decision 3: one auth mode, never two. The dev secret is
        // removed rather than merely not set, so an operator who exported
        // it in this shell does not get a gateway that trusts it.
        .env_remove("SYNVEDA_DEV_JWT_SECRET")
        .stdout(log)
        .stderr(errors);
    if plan.embedder == "tei" {
        command.env("SYNVEDA_TEI_URL", "http://localhost:8110");
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
    Ok(())
}

/// The pid in `pidfile`, if that process is alive.
fn running_gateway(pidfile: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pidfile).ok()?.trim().parse().ok()?;
    // `kill -0`: does this pid exist and may we signal it.
    let alive = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .ok()?
        .success();
    alive.then_some(pid)
}

/// Finds a built gateway. Like the image build, producing one is the
/// untimed part of the acceptance criterion — a release ships this binary.
fn gateway_binary(repo: &Path) -> Result<PathBuf, String> {
    for candidate in [
        repo.join("target/release/synveda-gateway"),
        repo.join("target/debug/synveda-gateway"),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("no synveda-gateway binary — build one with \
         `cargo build -p synveda-gateway` and re-run `synveda init`"
        .to_owned())
}

/// `docker compose -f <file> <args...>`, inheriting stdout/stderr so a
/// slow image pull is visible rather than silent.
fn compose(compose_file: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .args(args)
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
) -> Result<(), String> {
    // One line, no quotes: compose's env_file takes the rest of the line
    // verbatim, and a quoted JSON value arrives at the gateway with the
    // quotes still on it.
    let issuers = issuers_json(tenant_id, issuer);
    let tei = if plan.embedder == "tei" {
        "SYNVEDA_TEI_URL=http://tei:80\n"
    } else {
        ""
    };
    let body = format!(
        "# Written by `synveda init` (OPS-1, ADR-0055). Regenerate by re-running it.\n\
         # Dev-shaped credentials: this is the single-node profile, not a production one.\n\
         SYNVEDA_TENANT_ID={tenant_id}\n\
         SYNVEDA_OIDC_ISSUERS={issuers}\n\
         SYNVEDA_PUBLIC_URL={GATEWAY_URL}\n\
         SYNVEDA_LISTEN_ADDR=0.0.0.0:8120\n\
         SYNVEDA_EMBEDDER={}\n\
         {tei}",
        plan.embedder,
    );
    std::fs::write(path, body).map_err(|err| format!("write {}: {err}", path.display()))
}

/// The gateway's `SYNVEDA_OIDC_ISSUERS`, in one place so the host process
/// and the container cannot be configured differently.
fn issuers_json(tenant_id: TenantId, issuer: &str) -> String {
    json!([{
        "issuer": issuer,
        "client_id": "synveda",
        "tenant": {"static": {"tenant_id": tenant_id}},
        "login_scopes": ["openid", "profile", "email", "groups"],
    }])
    .to_string()
}

/// Polls a health endpoint until it answers or the budget runs out.
async fn wait_for_health(url: &str, budget: Duration) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("build the HTTP client: {err}"))?;
    let deadline = Instant::now() + budget;
    let mut last = String::new();
    while Instant::now() < deadline {
        match http.get(url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
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

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| "postgres://synveda:synveda-dev@localhost:5432/synveda".to_owned())
}

/// Where the compose file lives. `SYNVEDA_COMPOSE_FILE`'s directory wins;
/// otherwise the current directory is assumed to be a checkout. A released
/// binary would carry its own profile — see ADR-0055 decision 6's trigger.
fn repo_root() -> Result<PathBuf, String> {
    if let Some(file) = std::env::var("SYNVEDA_COMPOSE_FILE")
        .ok()
        .filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(file);
        return path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| "SYNVEDA_COMPOSE_FILE must be a path to a compose file".to_owned());
    }
    std::env::current_dir().map_err(|err| format!("resolve the working directory: {err}"))
}

fn step(number: u8, what: &str) {
    println!("==> {number}. {what}");
}

#[cfg(test)]
mod tests {
    use super::*;

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
                demo: false,
                dry_run: false,
            },
            tenant,
            RAUTHY_ISSUER,
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
                demo: false,
                dry_run: false,
            },
            TenantId::new(),
            RAUTHY_ISSUER,
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
    fn every_demo_team_belongs_to_a_declared_department() {
        // The demo people's groups are convention-shaped, so a team whose
        // department does not exist would place its person in quarantine
        // and the demo would silently show an empty block.
        for (department, team, _) in DEMO_TEAMS {
            assert!(
                DEMO_DEPARTMENTS.iter().any(|(slug, _)| slug == department),
                "team {team} names department {department}, which is not declared"
            );
        }
        for (local, _, _, group) in DEMO_PEOPLE {
            let expected: Vec<String> = DEMO_TEAMS
                .iter()
                .map(|(department, team, _)| format!("synveda-{department}-{team}"))
                .collect();
            assert!(
                expected.iter().any(|candidate| candidate == group),
                "{local} is in {group}, which no declared team maps from"
            );
        }
    }

    #[test]
    fn two_deployments_on_one_laptop_do_not_share_an_operator() {
        assert_ne!(operator_email("acme"), operator_email("globex"));
    }

    /// The same rule, for the people the operator was already following.
    ///
    /// This assertion existed for `operator_email` alone while `DEMO_PEOPLE`
    /// sat hard-coded at `@demo.localhost` two hundred lines above it — the
    /// rule was written down, tested, and then not applied to the constant
    /// beside it. `init` sets a password for everyone it creates and Rauthy
    /// refuses one it has seen recently, so a shared address is a login the
    /// second deployment cannot fix.
    #[test]
    fn two_deployments_on_one_laptop_do_not_share_a_demo_person_either() {
        for (local, _, _, _) in DEMO_PEOPLE {
            assert_ne!(
                demo_email(local, "acme"),
                demo_email(local, "globex"),
                "{local} is the same login in both deployments"
            );
        }
        // And no demo person collides with the operator, whose address is
        // built by the same rule.
        let taken: Vec<String> = DEMO_PEOPLE
            .iter()
            .map(|(local, _, _, _)| demo_email(local, "acme"))
            .collect();
        assert!(!taken.contains(&operator_email("acme")), "{taken:?}");
    }
}
