//! The FLOW-1 demo runner (ADR-0030): writes governed history through the
//! real object-store API and narrates what the substrate guarantees —
//! content addressing and dedup, a commit that records its author, its pack,
//! and a verifiable signature, then eight concurrent writers racing one ref
//! through compare-and-swap without losing a commit.
//!
//! Usage, with `DATABASE_URL` set:
//!   `cargo run -p synveda-vedaflow --example object_store -- <tenant-uuid>`
//!   `cargo run -p synveda-vedaflow --example object_store -- <tenant-uuid> verify`
//!
//! The second form only re-verifies, so the demo can tamper with a row in
//! between and watch verification name it.
//!
//! Layering note: `synveda-vedaflow` sits beside `synveda-store` and cannot
//! import it (seed §8), so this carries the same three-line tenant-GUC helper
//! `rls::begin_tenant_tx` provides.

use chrono::{TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_types::{AssetKind, IdentityId, Result, ScopeId, TenantId};
use synveda_vedaflow::{
    CommitHash, Ed25519Signer, NewCommit, PolicySnapshot, RefUpdate, Signer, TreeEntry, commit,
    create_ref, force_update_ref, is_ancestor, put_object, put_tree, read_commit, read_ref,
    update_ref, verify, verify_ed25519,
};

/// The channel FLOW-2 will give meaning to. Here it is just a name.
const CHANNEL: &str = "published";
/// Demo signing key. A real deployment supplies one as configuration and
/// never writes it down in a repository (ADR-0030 decision 9).
const DEMO_SEED: [u8; 32] = [0x5e; 32];

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(tenant) = args.next() else {
        eprintln!("usage: object_store <tenant-uuid> [verify]");
        std::process::exit(2);
    };
    let tenant: TenantId = tenant.parse()?;
    let verify_only = args.next().as_deref() == Some("verify");
    let url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set (run `make dev-up`)")?;
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&url)
        .await?;

    if verify_only {
        let mut tx = tenant_tx(&pool, tenant).await?;
        println!("{}", verify(&mut tx, tenant).await?);
        return Ok(());
    }

    let scope = ScopeId::new();
    let author = IdentityId::new();
    let signer = Signer::Ed25519(Box::new(Ed25519Signer::new(DEMO_SEED, "flow-1-demo")?));
    let Signer::Ed25519(ref key) = signer else {
        unreachable!()
    };
    // A fixed instant, so re-running the demo re-derives the same addresses.
    let at = Utc.with_ymd_and_hms(2026, 7, 25, 12, 0, 0).unwrap();
    // What the caller resolved before it got here; vedaflow never asks the
    // PDP itself (ADR-0030 decision 8).
    let pack = PolicySnapshot::new("regulated-strict", 5).with_config(
        serde_json::json!({"budget_tokens": 1500, "channels": "published-and-derived"}),
    );

    println!("scope: {scope}");
    println!("author: {author}");

    // ── Content addressing and dedup ────────────────────────────────────
    let mut tx = tenant_tx(&pool, tenant).await?;
    let body = b"Deploys go through the release train; hotfixes need a second pair of eyes.";
    let first = put_object(&mut tx, tenant, AssetKind::Knowledge, body).await?;
    let again = put_object(&mut tx, tenant, AssetKind::Knowledge, body).await?;
    // Same bytes, different governance: FLOW-3 resolves approvals from asset
    // type, and a skill is executable where a memory is not.
    let as_skill = put_object(&mut tx, tenant, AssetKind::Skill, body).await?;
    println!("\n== content addressing ==");
    println!("  memory  {}  stored", first.hash);
    println!(
        "  memory  {}  deduplicated={}",
        again.hash, again.deduplicated
    );
    println!(
        "  skill   {}  same bytes, different address (kind is in the hash)",
        as_skill.hash
    );

    // ── A commit that records who, which pack, and a signature ──────────
    let tree = put_tree(
        &mut tx,
        tenant,
        &[TreeEntry::object("release-process.md", first.hash)],
    )
    .await?;
    let root = commit(
        &mut tx,
        tenant,
        &NewCommit {
            tree: tree.hash,
            parents: vec![],
            author,
            message: "release process, as agreed in the platform sync".to_string(),
            committed_at: at,
            policy_snapshot: pack.clone(),
        },
        &signer,
    )
    .await?;
    let stored = read_commit(&mut tx, tenant, root.hash)
        .await?
        .expect("the commit we just wrote");
    let signature = stored.signature.clone().expect("signed");
    println!("\n== the commit ==");
    println!("  commit   {}", stored.hash);
    println!("  tree     {}", stored.tree);
    println!("  author   {}", stored.author);
    println!(
        "  pack     {} (regulated-strict@5 — which policy governed this commit)",
        stored.policy_snapshot_hash
    );
    println!(
        "  signed   key={} verifies={}",
        signature.key_id,
        verify_ed25519(stored.hash, &signature.signature, &key.verifying_key())
    );

    create_ref(&mut tx, tenant, scope, CHANNEL, root.hash, author).await?;
    tx.commit().await?;
    println!("  ref      {CHANNEL} -> {}", root.hash);

    // ── Eight concurrent writers, one ref ───────────────────────────────
    println!("\n== 8 concurrent writers x 3 commits, one ref ==");
    let mut handles = Vec::new();
    for id in 0..8usize {
        handles.push(tokio::spawn(race(
            pool.clone(),
            tenant,
            scope,
            author,
            id,
            3,
        )));
    }
    let mut landed = Vec::new();
    let mut races_lost = 0usize;
    for handle in handles {
        let (commits, lost) = handle.await??;
        landed.extend(commits);
        races_lost += lost;
    }

    let mut tx = tenant_tx(&pool, tenant).await?;
    let head = read_ref(&mut tx, tenant, scope, CHANNEL)
        .await?
        .expect("ref exists")
        .commit_hash;
    let mut chain = 1;
    let mut cursor = head;
    while let Some(parent) = read_commit(&mut tx, tenant, cursor)
        .await?
        .expect("commit exists")
        .parents
        .first()
        .copied()
    {
        chain += 1;
        cursor = parent;
    }
    let mut reachable = 0;
    for hash in &landed {
        if is_ancestor(&mut tx, tenant, *hash, head).await? {
            reachable += 1;
        }
    }
    println!("  landed          {} commits", landed.len());
    println!(
        "  reachable       {reachable} of {} from the head",
        landed.len()
    );
    println!("  chain length    {chain} (root + every commit)");
    println!("  races lost      {races_lost} compare-and-swaps retried, none lost");
    println!("  head            {head}");

    // ── The ref only moves forward, unless forced ───────────────────────
    println!("\n== fast-forward, and the deliberate rewind ==");
    let outcome = update_ref(&mut tx, tenant, scope, CHANNEL, head, root.hash, author).await?;
    println!("  update_ref back to the root  -> {outcome:?}");
    let outcome =
        force_update_ref(&mut tx, tenant, scope, CHANNEL, head, root.hash, author).await?;
    println!("  force_update_ref (FLOW-7)    -> {outcome:?}");
    // Put it back where the writers left it, so the demo's final state is
    // the history they built.
    force_update_ref(&mut tx, tenant, scope, CHANNEL, root.hash, head, author).await?;
    println!("  restored                     -> {CHANNEL} -> {head}");

    println!("\n== verification ==");
    println!("  {}", verify(&mut tx, tenant).await?);
    tx.commit().await?;
    Ok(())
}

/// One writer: `rounds` commits onto the channel, each compare-and-swapped
/// against the head it was parented on, retrying whenever it loses.
async fn race(
    pool: PgPool,
    tenant: TenantId,
    scope: ScopeId,
    author: IdentityId,
    id: usize,
    rounds: usize,
) -> Result<(Vec<CommitHash>, usize)> {
    let at = Utc.with_ymd_and_hms(2026, 7, 25, 12, 0, 0).unwrap();
    let pack = PolicySnapshot::new("regulated-strict", 5);
    let mut landed = Vec::new();
    let mut lost = 0;
    for round in 0..rounds {
        loop {
            let mut tx = tenant_tx(&pool, tenant).await?;
            let head = read_ref(&mut tx, tenant, scope, CHANNEL)
                .await?
                .expect("ref exists")
                .commit_hash;
            let body = format!("writer {id}, round {round}, attempt {}", lost + 1);
            let object = put_object(&mut tx, tenant, AssetKind::Knowledge, body.as_bytes()).await?;
            let tree = put_tree(
                &mut tx,
                tenant,
                &[TreeEntry::object("note.md", object.hash)],
            )
            .await?;
            let next = commit(
                &mut tx,
                tenant,
                &NewCommit {
                    tree: tree.hash,
                    parents: vec![head],
                    author,
                    message: body,
                    committed_at: at,
                    policy_snapshot: pack.clone(),
                },
                &Signer::Unsigned,
            )
            .await?;
            if update_ref(&mut tx, tenant, scope, CHANNEL, head, next.hash, author).await?
                == RefUpdate::Updated
            {
                tx.commit().await.map_err(storage)?;
                landed.push(next.hash);
                break;
            }
            // A lost race rolls back the whole attempt — object, tree, and
            // commit — so no unreachable history is left behind.
            tx.rollback().await.map_err(storage)?;
            lost += 1;
        }
    }
    Ok((landed, lost))
}

/// A transaction with the tenant GUC set — the same transaction-local shape
/// as `synveda_store::rls::begin_tenant_tx`.
async fn tenant_tx(pool: &PgPool, tenant: TenantId) -> Result<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await.map_err(storage)?;
    sqlx::query("select set_config('synveda.tenant_id', $1, true)")
        .bind(tenant.as_uuid().to_string())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    Ok(tx)
}

fn storage(err: sqlx::Error) -> synveda_types::Error {
    synveda_types::Error::Storage {
        message: err.to_string(),
    }
}
