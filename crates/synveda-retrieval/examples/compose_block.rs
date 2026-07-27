//! Composes and prints one context block for an identity — the CTX-2
//! demo runner (ADR-0025), and a working reference for the CTX-3 inject
//! seam: identity → scope chain (HIER-2 cache) → decision inputs →
//! PDP-derived composition plan → compose. The PDP is never bypassed
//! (seed §2.2): every scope in the plan is a per-request `MemoryRead`
//! allow, and channel rules/budget come from the effective packs.
//!
//! Usage: `cargo run -p synveda-retrieval --example compose_block -- \
//!   <tenant-uuid> <subject> [rfc3339-instant]` with `DATABASE_URL`
//! set. The optional instant is the valid-time input; passing the same
//! one re-composes byte-identically (the determinism AC).

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use synveda_policy::{Pdp, Principal};
use synveda_retrieval::{ComposeRequest, MemoryReadInputs, compose, composition_plan};
use synveda_store::{
    ScopeChainCache, identities, policy_assignments, policy_packs, rls, role_bindings,
};
use synveda_types::{Error, ScopeId, TenantId};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(tenant), Some(subject)) = (args.next(), args.next()) else {
        eprintln!("usage: compose_block <tenant-uuid> <subject> [rfc3339-instant]");
        std::process::exit(2);
    };
    let tenant: TenantId = tenant.parse()?;
    let at = match args.next() {
        Some(instant) => DateTime::parse_from_rfc3339(&instant)?.with_timezone(&Utc),
        None => Utc::now(),
    };
    let url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set (run `make dev-up`)")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;

    // The PDP with the tenant's stored packs installed — the gateway
    // refresher's job, done once here so bank-mode packs govern the demo.
    let pdp = Pdp::new()?;
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await?;
    for pack in policy_packs::stored(&mut *tx, tenant).await? {
        pdp.install_source(
            tenant,
            &pack.name,
            pack.version,
            &pack.source,
            pack.config.clone(),
        )?;
    }

    // Identity → placement chain (the HIER-2 cache, ADR-0016's
    // direction) → the decision inputs the gateway gathers per request.
    let identity = identities::by_subject(&mut *tx, tenant, &subject)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("identity {subject:?}"),
        })?;
    let chain = ScopeChainCache::new()
        .resolve(&mut *tx, tenant, identity.scope_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: "placement chain".to_owned(),
        })?;
    let chain_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    let assignments = policy_assignments::for_scopes(&mut *tx, tenant, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(&mut *tx, tenant).await?;
    let bindings =
        role_bindings::for_subject_on_scopes(&mut *tx, tenant, &subject, &chain_ids).await?;

    let principal = Principal {
        tenant_id: tenant,
        subject: subject.clone(),
        quarantined: identity.quarantined,
        scope_id: Some(identity.scope_id),
        token_scope: None,
    };
    let plan = composition_plan(
        &pdp,
        &MemoryReadInputs {
            principal: &principal,
            chain: &chain,
            assignments: &assignments,
            default_pack: default_pack.as_deref(),
            role_bindings: &bindings,
            lapses: &[],
            lapsed: &[],
            candidates: &[],
        },
    )?;

    let request = ComposeRequest::new(plan.scopes, plan.budget_tokens, at);
    let block = compose(&mut tx, tenant, &request).await?;
    drop(tx);

    println!("{}", block.text);
    eprintln!(
        "-- composed {} entr{} · {} of {} estimated tokens · \
         {} conflict loser(s) dropped · {} skipped over budget",
        block.entries.len(),
        if block.entries.len() == 1 { "y" } else { "ies" },
        block.tokens,
        block.budget_tokens,
        block.dropped_conflicts,
        block.skipped_over_budget,
    );
    eprintln!("-- block blake3={}", block.block_hash);
    Ok(())
}
