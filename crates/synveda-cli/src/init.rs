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
/// How to get an installed release, in the one place every message that
/// suggests it reads from. A raw GitHub URL rather than a vanity domain:
/// three of these messages pointed at `synveda.dev`, which does not
/// resolve, and an error that names a dead URL is worse than one that names
/// none (OPS-8).
const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh";
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

    if plan.dry_run {
        return dry_run(&plan, &profile, &issuer, bundled);
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
    // Beside the compose file, which is where compose's `env_file` looks
    // and where its own variable substitution reads a `.env` from.
    let env_file = compose_file.with_file_name(".env");
    write_env_file(&env_file, &plan, tenant_id, &issuer)?;
    println!("    configuration: {}", env_file.display());
    if containerised(&plan) {
        // `--build` only where there is something to build from. A release
        // bundle's compose file has no build context by construction, and
        // asking compose to build one is an error rather than a no-op.
        let mut up = vec!["up", "--detach", "--wait"];
        if profile.may_build() {
            up.push("--build");
        }
        up.push("gateway");
        compose(&compose_file, &up)?;
    } else {
        start_host_gateway(&profile, &plan, tenant_id, &issuer)?;
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
fn dry_run(plan: &Plan, profile: &Profile, issuer: &str, bundled: bool) -> Result<(), String> {
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
    profile: &Profile,
    plan: &Plan,
    tenant_id: TenantId,
    issuer: &str,
) -> Result<(), String> {
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

    let binary = gateway_binary(profile)?;
    let log = std::fs::File::create(&logfile)
        .map_err(|err| format!("create {}: {err}", logfile.display()))?;
    let errors = log
        .try_clone()
        .map_err(|err| format!("open {}: {err}", logfile.display()))?;

    let mut command = Command::new(&binary);
    command
        .current_dir(profile.working_dir())
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

/// `DATABASE_URL`, or the single-node profile's own Postgres. Shared with
/// `main::connect` so that an installed CLI and the installer it came with
/// cannot disagree about which database this deployment is.
pub fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| "postgres://synveda:synveda-dev@localhost:5432/synveda".to_owned())
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
