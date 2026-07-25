//! The `synveda` admin/dev CLI (`synveda init`, `synveda policy apply`,
//! `synveda proposal review`, ...). Talks to the gateway API as a client for
//! everything a running gateway serves; the bootstrap commands below
//! (TEN-1, ADR-0008) go to the database directly because they exist
//! precisely for the moment there is no usable gateway yet — applying
//! migrations, admitting the first tenant, minting a dev token.
//!
//! Since ADPT-1 it is also the credential authority (ADR-0027 decision 4):
//! `synveda login` runs the loopback flow and `synveda auth token` hands a
//! currently-valid bearer to whoever asks — the Claude Code adapter's
//! hooks, a script, a human. One implementation of PKCE, expiry, and
//! refresh, here, rather than a second drifting one per adapter.

// `unsafe` is forbidden in the product code; the credentials tests set
// process environment variables, which is unsafe in edition 2024, and
// they hold a lock while they do it.
#![cfg_attr(not(test), forbid(unsafe_code))]

mod api;
mod channel;
mod credentials;
mod diff;
mod login;
mod proposal;

use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_types::{
    CompositionConfig, IdentityId, IdentityKind, InjectChannels, PackConfig, PromotionConfig,
    ProposalId, ProposalState, RedactionConfig, RedactionMode, Role, ScopeId, ScopeKind, TenantId,
    TenantStatus,
};

#[derive(Parser)]
#[command(name = "synveda", about = "Synveda admin/dev CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Log in to a gateway through your browser (AUTH-1 end to end,
    /// ADR-0027 decision 5) and store the session under a profile. This
    /// is the whole of "zero-config": every other Synveda client on this
    /// machine reads its bearer from what this writes.
    Login {
        /// Gateway base URL. Defaults to $SYNVEDA_GATEWAY, else
        /// http://127.0.0.1:8120.
        #[arg(long)]
        gateway: Option<String>,
        /// Which configured issuer to log in against; optional when the
        /// gateway has exactly one.
        #[arg(long)]
        issuer: Option<String>,
        /// Credential profile to write. Defaults to $SYNVEDA_PROFILE,
        /// else `default`.
        #[arg(long)]
        profile: Option<String>,
        /// Print the login URL instead of opening a browser (headless
        /// machines, SSH sessions).
        #[arg(long)]
        no_browser: bool,
    },
    /// Stored credentials (ADR-0027 decisions 4 and 6).
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Database administration (dev bootstrap).
    #[command(subcommand)]
    Db(DbCommand),
    /// Tenant administration (dev bootstrap).
    #[command(subcommand)]
    Tenant(TenantCommand),
    /// Dev-mode bearer tokens (HS256, ADR-0008). AUTH-1 makes real IdPs the
    /// issuer; this command never applies to a production deployment.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Per-tenant policy packs (AUTHZ-1, ADR-0012). Dev/admin plumbing:
    /// AUTHZ-2 owns the product surface, and VedaFlow eventually governs
    /// packs as reviewed assets.
    #[command(subcommand)]
    Policy(PolicyCommand),
    /// Role bindings (AUTHZ-3, ADR-0015). Dev plumbing and the documented
    /// break-glass: a tenant that revoked its last org-admin recovers
    /// here, at the store level — the product surface is `/v1/roles/*`.
    #[command(subcommand)]
    Role(RoleCommand),
    /// Service identities (AUTH-3, ADR-0018). Dev plumbing and the
    /// break-glass at the store level — the product surface is
    /// `/v1/service-identities`. Credentials live in the IdP: register
    /// the OAuth2 client there, then bind its subject to an anchor here.
    #[command(subcommand)]
    Service(ServiceCommand),
    /// The hash-chained audit log (AUD-1, ADR-0019): verify a tenant's
    /// chain or tail its recent events. Read-only; the operator's and
    /// auditor's chain check ahead of AUD-2's query surface.
    #[command(subcommand)]
    Audit(AuditCommand),
    /// The VedaFlow review flow (FLOW-6, ADR-0035): read, review, and
    /// decide promotion proposals from a terminal.
    ///
    /// These are not dev plumbing, and they open no database connection.
    /// Every verb is a call to /v1/proposals under the bearer `synveda
    /// login` stored, so the PDP decides who may act exactly as it would
    /// for a console, and the gateway chains the event under your own
    /// identity. Run `synveda login` first.
    #[command(subcommand)]
    Proposal(ProposalCommand),
    /// VedaFlow channels (FLOW-7, ADR-0036): what a scope publishes, the
    /// states it has published, rewinding to one of them, and holding
    /// what it serves at a commit.
    ///
    /// Gateway calls under the bearer `synveda login` stored, like
    /// `proposal` and for the same reason: a rewind reaches every agent
    /// under the scope on their next session, and an act that large is
    /// one the PDP must decide and the gateway must chain under your own
    /// identity — never a row a laptop wrote.
    #[command(subcommand)]
    Channel(ChannelCommand),
}

#[derive(Subcommand)]
enum ChannelCommand {
    /// What stands at a scope: each channel, where it points, and the
    /// pin holding its readers if there is one.
    Status {
        /// The scope UUID.
        scope: ScopeId,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// The states a channel has held, newest first — the menu a rewind is
    /// chosen from. Everything listed but the head is a legal `--to`, and
    /// nothing outside the listing is (ADR-0036 decisions 1 and 11).
    History {
        /// The scope UUID.
        scope: ScopeId,
        /// Which channel, as its ref name. Defaults to
        /// `memory/published`.
        #[arg(long)]
        channel: Option<String>,
        /// How many states, 1..=200.
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Rewind a channel to a state it has already held. Every session
    /// that starts afterwards composes that state; nothing else has to
    /// happen, and nothing can be installed that was not approved when it
    /// was published.
    Rollback {
        /// The scope UUID.
        scope: ScopeId,
        /// The commit being abandoned — the head as `history` showed it.
        /// Required: a rewind is a decision about which state to leave,
        /// and that decision is stale if someone else moved the ref.
        #[arg(long)]
        from: String,
        /// The state to install: one of the commits `history` lists.
        #[arg(long)]
        to: String,
        /// Why. An auditor reads this, and so does whoever asks next week
        /// why a record stopped being published.
        #[arg(long)]
        message: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Hold what a channel serves at a commit. Publications keep landing
    /// and the channel keeps advancing; what stops moving is what readers
    /// compose (ADR-0036 decision 6).
    Pin {
        /// The scope UUID.
        scope: ScopeId,
        /// The commit to hold readers at — one of the commits `history`
        /// lists, the head included.
        #[arg(long)]
        commit: String,
        /// Why this scope is holding its readers. The pin's only record:
        /// the ref carries who and when and nothing else.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Release the hold. Readers catch up to the channel's head on their
    /// next session.
    Unpin {
        /// The scope UUID.
        scope: ScopeId,
        /// Why the hold is being released.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProposalCommand {
    /// List proposals, newest first.
    ///
    /// Without `--scope` this is a tenant-wide listing, which the packs
    /// grant to tenant-wide review and admin roles only; a curator bound
    /// at one team passes `--scope` instead.
    List {
        /// Restrict to proposals targeting this scope.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// Restrict to one stored state (open/rejected/withdrawn/
        /// published). `approved` is computed, not stored: filter on
        /// `open` and read each row's state.
        #[arg(long)]
        state: Option<ProposalState>,
        /// How many, 1..=500.
        #[arg(long)]
        limit: Option<i64>,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Show one proposal in full: what it needs, who has acted, and what
    /// publishing it would do to the target's channel — with a diff.
    Show {
        /// The proposal UUID.
        id: ProposalId,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Walk the open proposals one at a time, rendering each in full and
    /// asking for a verdict. Oldest first. End of input casts nothing.
    Review {
        /// Review just this one.
        id: Option<ProposalId>,
        /// Restrict the queue to one scope.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// How many to queue, 1..=500.
        #[arg(long)]
        limit: Option<i64>,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Approve one proposal.
    Approve {
        /// The proposal UUID.
        id: ProposalId,
        /// What you want to say about it.
        #[arg(long)]
        comment: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Reject one proposal, with a reason. Terminal — a revision is a new
    /// proposal.
    Reject {
        /// The proposal UUID.
        id: ProposalId,
        /// Why. Mandatory: a rejection an auditor cannot read the reason
        /// for is not a review.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Close your own proposal without a verdict. The proposer's act; a
    /// reviewer rejects with a reason instead.
    Withdraw {
        /// The proposal UUID.
        id: ProposalId,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Run an approved proposal's effect: move the target's published
    /// channel. A separate act from the deciding approval by design
    /// (ADR-0032 decision 9) — it takes `ChannelPublish` and `MemoryRead`
    /// at the target, which the deciding reviewer may not hold.
    Publish {
        /// The proposal UUID.
        id: ProposalId,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Print a currently-valid bearer for the profile, refreshing it
    /// through the gateway first if it has expired. Exits non-zero when
    /// there is nothing to print and says what to run.
    Token {
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
        /// Print the token with its expiry, tenant, and gateway as one
        /// JSON object — the shape the Claude Code adapter reads.
        #[arg(long)]
        json: bool,
    },
    /// Forget a profile's credentials.
    Logout {
        /// Credential profile to forget.
        #[arg(long)]
        profile: Option<String>,
        /// Forget every profile.
        #[arg(long, conflicts_with = "profile")]
        all: bool,
    },
}

#[derive(Subcommand)]
enum AuditCommand {
    /// Walk the tenant's whole chain, recomputing every hash, and report
    /// the first divergence. Exits non-zero on a broken chain.
    Verify {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
    },
    /// Print the tenant's most recent events, newest first, as JSON.
    Tail {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// How many events.
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
}

#[derive(Subcommand)]
enum DbCommand {
    /// Apply all pending migrations to DATABASE_URL.
    Migrate,
}

#[derive(Subcommand)]
enum TenantCommand {
    /// Admit a tenant; prints the created tenant as JSON.
    Create {
        /// Human-stable handle: lowercase, hyphenated, unique.
        #[arg(long)]
        slug: String,
        /// Display name.
        #[arg(long)]
        name: String,
        /// Admit in suspended state (its tokens will not resolve).
        #[arg(long)]
        suspended: bool,
    },
}

#[derive(Subcommand)]
enum PolicyCommand {
    /// Compile-check a Cedar pack and apply it for a tenant (bumps the
    /// version; the gateway hot-reloads it within its refresh interval).
    /// Prints the stored pack row as JSON.
    Apply {
        /// Tenant UUID the pack applies to.
        #[arg(long)]
        tenant: TenantId,
        /// Pack name (slug grammar), e.g. `acme-strict`. Product names
        /// (`regulated-strict`, `standard`, `open-collaboration`) are
        /// reserved (ADR-0014).
        #[arg(long)]
        name: String,
        /// Redaction mode for secret findings on observe ingest
        /// (deny/redact/quarantine — MEM-2, ADR-0021). Both redaction
        /// flags must be given together or neither; unconfigured packs
        /// get the strict default (secrets quarantine, PII redact).
        #[arg(long, requires = "redaction_pii")]
        redaction_secrets: Option<RedactionMode>,
        /// Redaction mode for PII findings on observe ingest.
        #[arg(long, requires = "redaction_secrets")]
        redaction_pii: Option<RedactionMode>,
        /// Estimated-token inject budget at scopes this pack governs
        /// (CTX-2, ADR-0025). Both composition flags must be given
        /// together or neither; unconfigured packs get the product
        /// default (1500, published-and-derived).
        #[arg(long, requires = "composition_channels")]
        composition_budget: Option<u32>,
        /// Inject channel rule (published-and-derived/published-only —
        /// published-only is the bank-mode switch).
        #[arg(long, requires = "composition_budget")]
        composition_channels: Option<InjectChannels>,
        /// Path to a JSON file of auto-promotion rules (FLOW-4,
        /// ADR-0033): `{"rules":[{"name":..., "min_recalls":...,
        /// "min_distinct_members":..., "max_sensitivity":...}]}`. A file
        /// rather than flags because a rule set is a list, not a scalar.
        /// Omitted means the pack carries no rules and nothing
        /// auto-promotes at the scopes it governs — a trigger's fail-safe
        /// is silence.
        #[arg(long)]
        promotion: Option<std::path::PathBuf>,
        /// Path to the Cedar policy source file.
        file: std::path::PathBuf,
    },
    /// Remove one of a tenant's stored packs. Refused while assignments
    /// or the tenant default still reference it; scopes it governed fall
    /// back to their inherited pack (ADR-0014).
    Clear {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// Stored pack name to remove.
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum RoleCommand {
    /// Bind a role to a subject; prints the binding as JSON. Without
    /// --scope the binding is tenant-wide (in force everywhere, the
    /// tenant plane included).
    Bind {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The token subject to bind.
        #[arg(long)]
        subject: String,
        /// The role (viewer/contributor/curator/steward/org-admin/
        /// auditor/security-reviewer/compliance).
        #[arg(long)]
        role: Role,
        /// Hierarchy node UUID to bind at; omit for tenant-wide.
        #[arg(long)]
        scope: Option<ScopeId>,
    },
    /// Remove one binding (exact subject + role + scope).
    Unbind {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The bound subject.
        #[arg(long)]
        subject: String,
        /// The bound role.
        #[arg(long)]
        role: Role,
        /// The bound node UUID; omit for the tenant-wide binding.
        #[arg(long)]
        scope: Option<ScopeId>,
    },
    /// List every binding of the tenant as JSON.
    List {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    /// Register a service identity at an anchor node: creates its
    /// personal leaf under the anchor and the identity row (kind
    /// `service`); prints the identity as JSON. Tokens for its subject
    /// are then confined to the anchor's subtree.
    Register {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The `sub` the IdP puts in the agent's client-credentials
        /// tokens (for Rauthy, the client id).
        #[arg(long)]
        subject: String,
        /// The anchor node UUID.
        #[arg(long)]
        scope: ScopeId,
        /// Display name; defaults to the subject.
        #[arg(long)]
        name: Option<String>,
    },
    /// Revoke a service identity: deletes the row and its personal leaf.
    Remove {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The registered identity UUID (see `service list`).
        #[arg(long)]
        id: IdentityId,
    },
    /// List the tenant's service identities as JSON.
    List {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Issue a token signed with SYNVEDA_DEV_JWT_SECRET; prints the raw
    /// token to stdout.
    Issue {
        /// Tenant UUID for the `tid` claim.
        #[arg(long)]
        tenant: TenantId,
        /// Subject for the `sub` claim.
        #[arg(long, default_value = "dev")]
        subject: String,
        /// Lifetime in seconds for the `exp` claim.
        #[arg(long, default_value_t = 3600)]
        ttl_secs: u64,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("synveda: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The credential profile a command acts on: `--profile`, then
/// `SYNVEDA_PROFILE`, then `default`.
fn profile_name(flag: Option<String>) -> String {
    flag.or_else(|| std::env::var("SYNVEDA_PROFILE").ok())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| credentials::DEFAULT_PROFILE.to_owned())
}

#[tokio::main(flavor = "current_thread")]
async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Login {
            gateway,
            issuer,
            profile,
            no_browser,
        } => {
            login::login(
                login::gateway_url(gateway),
                issuer,
                profile_name(profile),
                !no_browser,
            )
            .await
        }
        Command::Auth(AuthCommand::Token { profile, json }) => {
            login::auth_token(profile_name(profile), json).await
        }
        Command::Auth(AuthCommand::Logout { profile, all }) => {
            let mut stored = credentials::load()?;
            if all {
                let count = stored.profiles.len();
                stored.profiles.clear();
                credentials::save(&stored)?;
                eprintln!("synveda: forgot {count} profile(s)");
            } else {
                let name = profile_name(profile);
                if stored.profiles.remove(&name).is_none() {
                    return Err(format!("no credentials for profile `{name}`"));
                }
                credentials::save(&stored)?;
                eprintln!("synveda: forgot profile `{name}`");
            }
            // The gateway is not told: the IdP owns revocation, and a
            // local forget is exactly that — local (ADR-0027 decision 6).
            eprintln!(
                "         the tokens themselves remain valid at the issuer until they expire"
            );
            Ok(())
        }
        Command::Db(DbCommand::Migrate) => {
            let pool = connect().await?;
            synveda_store::migrate(&pool)
                .await
                .map_err(|err| err.to_string())?;
            eprintln!("migrations applied");
            Ok(())
        }
        Command::Tenant(TenantCommand::Create {
            slug,
            name,
            suspended,
        }) => {
            let status = if suspended {
                TenantStatus::Suspended
            } else {
                TenantStatus::Active
            };
            let pool = connect().await?;
            let tenant_id = TenantId::new();
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant_id)
                .await
                .map_err(|err| err.to_string())?;
            let tenant = synveda_store::tenants::create(&mut *tx, tenant_id, &slug, &name, status)
                .await
                .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant_id,
                AuditAction::TenantCreated,
                format!("tenant {tenant_id}"),
                json!({"slug": tenant.slug, "name": tenant.name, "status": tenant.status}),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&tenant).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Policy(PolicyCommand::Apply {
            tenant,
            name,
            redaction_secrets,
            redaction_pii,
            composition_budget,
            composition_channels,
            promotion,
            file,
        }) => {
            let source = std::fs::read_to_string(&file)
                .map_err(|err| format!("read {}: {err}", file.display()))?;
            // Compile-check before storing: same schema, same validation
            // the gateway's reloader applies (ADR-0012 decision 2).
            synveda_policy::Pdp::new()
                .and_then(|pdp| pdp.compile_check(&name, &source))
                .map_err(|err| err.to_string())?;
            // clap's `requires` makes each config's flags all-or-nothing.
            let redaction = redaction_secrets
                .zip(redaction_pii)
                .map(|(secrets, pii)| RedactionConfig { secrets, pii });
            let composition =
                composition_budget
                    .zip(composition_channels)
                    .map(|(budget_tokens, channels)| CompositionConfig {
                        budget_tokens,
                        channels,
                    });
            // Validated here as well as at install: a rule that asks for
            // zero recalls, or names an asset with no usage signal, is
            // refused when it is written rather than discovered when a
            // sweep silently does nothing (ADR-0033 decision 6).
            let promotion = promotion
                .map(|path| {
                    let raw = std::fs::read_to_string(&path)
                        .map_err(|err| format!("read {}: {err}", path.display()))?;
                    let config: PromotionConfig = serde_json::from_str(&raw)
                        .map_err(|err| format!("parse {}: {err}", path.display()))?;
                    config
                        .validate()
                        .map_err(|err| format!("{}: {err}", path.display()))?;
                    Ok::<_, String>(config)
                })
                .transpose()?;
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let pack = synveda_store::policy_packs::apply(
                &mut *tx,
                tenant,
                &name,
                &source,
                &PackConfig {
                    redaction,
                    composition,
                    promotion,
                    ..Default::default()
                },
            )
            .await
            .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::PolicyPackApplied,
                format!("tenant {tenant}"),
                json!({
                    "pack": pack.name,
                    "version": pack.version,
                    "redaction": pack.config.redaction,
                    "composition": pack.config.composition,
                    "promotion": pack.config.promotion,
                }),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::json!({
                    "tenant_id": pack.tenant_id,
                    "name": pack.name,
                    "version": pack.version,
                    "updated_at": pack.updated_at,
                })
            );
            Ok(())
        }
        Command::Policy(PolicyCommand::Clear { tenant, name }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let removed = synveda_store::policy_packs::clear(&mut tx, tenant, &name)
                .await
                .map_err(|err| err.to_string())?;
            if removed {
                record_break_glass(
                    &mut tx,
                    tenant,
                    AuditAction::PolicyPackCleared,
                    format!("tenant {tenant}"),
                    json!({"pack": name}),
                )
                .await?;
            }
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!(
                "{}",
                if removed {
                    "policy pack cleared; it unloads on the next reload sweep"
                } else {
                    "no stored pack by that name"
                }
            );
            Ok(())
        }
        Command::Role(RoleCommand::Bind {
            tenant,
            subject,
            role,
            scope,
        }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let binding =
                synveda_store::role_bindings::bind(&mut *tx, tenant, &subject, scope, role)
                    .await
                    .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::RoleBound,
                scope.map_or_else(|| format!("tenant {tenant}"), |id| format!("scope {id}")),
                json!({"binding": {"subject": subject, "role": role, "scope_id": scope}}),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&binding).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Role(RoleCommand::Unbind {
            tenant,
            subject,
            role,
            scope,
        }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let removed =
                synveda_store::role_bindings::unbind(&mut *tx, tenant, &subject, scope, role)
                    .await
                    .map_err(|err| err.to_string())?;
            if removed {
                record_break_glass(
                    &mut tx,
                    tenant,
                    AuditAction::RoleUnbound,
                    scope.map_or_else(|| format!("tenant {tenant}"), |id| format!("scope {id}")),
                    json!({"binding": {"subject": subject, "role": role, "scope_id": scope}}),
                )
                .await?;
            }
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!(
                "{}",
                if removed {
                    "role binding removed; it is out of force on the next request"
                } else {
                    "no such binding"
                }
            );
            Ok(())
        }
        Command::Role(RoleCommand::List { tenant }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let bindings = synveda_store::role_bindings::all(&mut *tx, tenant)
                .await
                .map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&bindings).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Service(ServiceCommand::Register {
            tenant,
            subject,
            scope,
            name,
        }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let anchor = synveda_store::hierarchy::node(&mut *tx, scope)
                .await
                .map_err(|err| err.to_string())?
                .filter(|node| node.tenant_id == tenant)
                .ok_or_else(|| format!("no scope {scope} in tenant {tenant}"))?;
            if anchor.slug == synveda_store::identities::QUARANTINE_SLUG && anchor.depth == 1 {
                return Err(
                    "service identities cannot be anchored at the quarantine scope".to_owned(),
                );
            }
            let identity_id = IdentityId::new();
            let display_name = name.as_deref().unwrap_or(&subject);
            let leaf = synveda_store::hierarchy::create(
                &mut tx,
                ScopeId::new(),
                tenant,
                Some(anchor.id),
                ScopeKind::User,
                &personal_slug(None, &subject, identity_id),
                display_name,
            )
            .await
            .map_err(|err| err.to_string())?;
            let identity = synveda_store::identities::create(
                &mut tx,
                identity_id,
                tenant,
                &subject,
                IdentityKind::Service,
                None,
                name.as_deref(),
                leaf.id,
            )
            .await
            .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::ServiceIdentityRegistered,
                format!("scope {}", anchor.id),
                json!({
                    "identity": {"id": identity.id, "subject": identity.subject},
                    "leaf_scope_id": leaf.id,
                    "anchor": {"slug": anchor.slug, "path": anchor.path},
                }),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!(
                "note: a running gateway caches hierarchy out-of-process; restart it or use the API path"
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&identity).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Service(ServiceCommand::Remove { tenant, id }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let identity = synveda_store::identities::by_id(&mut *tx, tenant, id)
                .await
                .map_err(|err| err.to_string())?
                .filter(|identity| identity.kind == IdentityKind::Service)
                .ok_or_else(|| format!("no service identity {id} in tenant {tenant}"))?;
            // The anchor (the leaf's parent) names the event's resource,
            // read before the leaf goes.
            let anchor_id = synveda_store::hierarchy::node(&mut *tx, identity.scope_id)
                .await
                .map_err(|err| err.to_string())?
                .and_then(|leaf| leaf.parent_id);
            // Row first (its FK pins the leaf), then the leaf.
            synveda_store::identities::delete_service(&mut *tx, tenant, id)
                .await
                .map_err(|err| err.to_string())?;
            synveda_store::hierarchy::delete(&mut tx, identity.scope_id)
                .await
                .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::ServiceIdentityRevoked,
                anchor_id.map_or_else(|| format!("tenant {tenant}"), |id| format!("scope {id}")),
                json!({"identity": {"id": identity.id, "subject": identity.subject}}),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!("service identity revoked; its tokens are quarantined from the next request");
            Ok(())
        }
        Command::Service(ServiceCommand::List { tenant }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let identities = synveda_store::identities::services(&mut *tx, tenant)
                .await
                .map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&identities).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Audit(AuditCommand::Verify { tenant }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let verification = synveda_audit::verify(&mut tx, tenant)
                .await
                .map_err(|err| err.to_string())?;
            println!("{verification}");
            match verification {
                synveda_audit::ChainVerification::Valid { .. } => Ok(()),
                synveda_audit::ChainVerification::Broken { .. } => {
                    Err("audit chain verification failed".to_owned())
                }
            }
        }
        Command::Audit(AuditCommand::Tail { tenant, limit }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let events = synveda_audit::tail(&mut tx, tenant, limit)
                .await
                .map_err(|err| err.to_string())?;
            for event in events {
                println!(
                    "{}",
                    json!({
                        "seq": event.seq,
                        "occurred_at": event.occurred_at,
                        "actor": {"kind": event.actor_kind, "subject": event.actor_subject},
                        "action": event.action,
                        "resource": event.resource,
                        "outcome": event.outcome,
                        "payload": event.payload,
                        "trace_id": event.trace_id,
                        "hash": event.hash_hex(),
                    })
                );
            }
            Ok(())
        }
        Command::Proposal(command) => match command {
            ProposalCommand::List {
                scope,
                state,
                limit,
                json,
                profile,
            } => proposal::list(&profile_name(profile), scope, state, limit, json).await,
            ProposalCommand::Show { id, json, profile } => {
                proposal::show(&profile_name(profile), id, json).await
            }
            ProposalCommand::Review {
                id,
                scope,
                limit,
                profile,
            } => proposal::review(&profile_name(profile), id, scope, limit).await,
            ProposalCommand::Approve {
                id,
                comment,
                profile,
            } => proposal::approve(&profile_name(profile), id, comment).await,
            ProposalCommand::Reject {
                id,
                reason,
                profile,
            } => proposal::reject(&profile_name(profile), id, reason).await,
            ProposalCommand::Withdraw { id, profile } => {
                proposal::withdraw(&profile_name(profile), id).await
            }
            ProposalCommand::Publish { id, profile } => {
                proposal::publish(&profile_name(profile), id).await
            }
        },
        Command::Channel(command) => match command {
            ChannelCommand::Status {
                scope,
                json,
                profile,
            } => channel::status(&profile_name(profile), scope, json).await,
            ChannelCommand::History {
                scope,
                channel,
                limit,
                json,
                profile,
            } => channel::history(&profile_name(profile), scope, channel, limit, json).await,
            ChannelCommand::Rollback {
                scope,
                from,
                to,
                message,
                channel,
                json,
                profile,
            } => {
                channel::rollback(
                    &profile_name(profile),
                    scope,
                    from,
                    to,
                    message,
                    channel,
                    json,
                )
                .await
            }
            ChannelCommand::Pin {
                scope,
                commit,
                reason,
                channel,
                json,
                profile,
            } => channel::pin(&profile_name(profile), scope, commit, reason, channel, json).await,
            ChannelCommand::Unpin {
                scope,
                reason,
                channel,
                json,
                profile,
            } => channel::unpin(&profile_name(profile), scope, reason, channel, json).await,
        },
        Command::Token(TokenCommand::Issue {
            tenant,
            subject,
            ttl_secs,
        }) => {
            let secret = std::env::var("SYNVEDA_DEV_JWT_SECRET")
                .ok()
                .filter(|secret| !secret.is_empty())
                .ok_or("SYNVEDA_DEV_JWT_SECRET must be set to issue dev tokens")?;
            let token = Hs256Verifier::new(secret.as_bytes()).issue(
                &subject,
                tenant,
                Duration::from_secs(ttl_secs),
            );
            println!("{token}");
            Ok(())
        }
    }
}

/// The break-glass actor: whoever holds the database credentials, named
/// as well as the OS names them — honest about attribution being weaker
/// than the IdP-authenticated plane (AUD-1, ADR-0019 decision 7).
fn break_glass() -> Actor {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_owned());
    Actor::break_glass(user)
}

/// Chains a break-glass event in the same transaction as the mutation it
/// records: the CLI audits itself like the gateway does (ADR-0019
/// decision 7) — a failed append fails the command.
async fn record_break_glass(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    action: AuditAction,
    resource: String,
    payload: serde_json::Value,
) -> Result<(), String> {
    synveda_audit::append(
        tx,
        tenant,
        &AuditEvent {
            occurred_at: chrono::Utc::now(),
            actor: break_glass(),
            action,
            resource,
            outcome: Outcome::Success,
            payload,
            trace_id: None,
        },
    )
    .await
    .map(|_| ())
    .map_err(|err| err.to_string())
}

async fn connect() -> Result<sqlx::PgPool, String> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set (dev default is in the Makefile)")?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .map_err(|err| format!("connect to DATABASE_URL: {err}"))
}
