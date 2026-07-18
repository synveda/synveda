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
use sqlx::postgres::PgPoolOptions;
use synveda_identity::Hs256Verifier;
use synveda_types::{TenantId, TenantStatus};

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
            let tenant =
                synveda_store::tenants::create(&pool, TenantId::new(), &slug, &name, status)
                    .await
                    .map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&tenant).map_err(|err| err.to_string())?
            );
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

async fn connect() -> Result<sqlx::PgPool, String> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set (dev default is in the Makefile)")?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .map_err(|err| format!("connect to DATABASE_URL: {err}"))
}
