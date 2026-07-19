//! The `synveda` admin/dev CLI (`synveda init`, `synveda policy apply`,
//! `synveda proposal review`, ...). Talks to the gateway API as a client for
//! everything a running gateway serves; the bootstrap commands below
//! (TEN-1, ADR-0008) go to the database directly because they exist
//! precisely for the moment there is no usable gateway yet — applying
//! migrations, admitting the first tenant, minting a dev token.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_types::{
    IdentityId, IdentityKind, RedactionConfig, RedactionMode, Role, ScopeId, ScopeKind, TenantId,
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

#[tokio::main(flavor = "current_thread")]
async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
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
            file,
        }) => {
            let source = std::fs::read_to_string(&file)
                .map_err(|err| format!("read {}: {err}", file.display()))?;
            // Compile-check before storing: same schema, same validation
            // the gateway's reloader applies (ADR-0012 decision 2).
            synveda_policy::Pdp::new()
                .and_then(|pdp| pdp.compile_check(&name, &source))
                .map_err(|err| err.to_string())?;
            // clap's `requires` makes these all-or-nothing.
            let redaction = redaction_secrets
                .zip(redaction_pii)
                .map(|(secrets, pii)| RedactionConfig { secrets, pii });
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let pack = synveda_store::policy_packs::apply(
                &mut *tx,
                tenant,
                &name,
                &source,
                redaction.as_ref(),
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
                    "redaction": pack.redaction,
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
