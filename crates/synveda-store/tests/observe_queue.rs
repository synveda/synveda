//! The observe queue's consumer surface (MEM-3, ADR-0022): reading
//! signals under a visibility timeout, the archive-as-lock semantics the
//! worker's exactly-once commit stands on, staged-event loading, and the
//! defensive malformed-message shape.
//!
//! Needs a live Postgres: reads `DATABASE_URL` and skips when unset (CI
//! has no database). Assertions filter by this test's own tenant — the
//! PGMQ queue is shared across suites by design (content-free signals).

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::observe::{self, NewObserveEvent, ObserveMessage, QueuedSignal};
use synveda_store::{identities, rls, tenants};
use synveda_types::scope::ScopeKind;
use synveda_types::{
    IdentityId, IdentityKind, ObserveEventId, ObserveKind, ScopeId, TenantId, TenantStatus,
};

/// Seeding shape the old hierarchy-create calls had, on the governed
/// substrate (CPR-7, ADR-0074): caller-chosen id, the old kinds mapped —
/// `org` is the tenant root, `division`/`department`/`team` are org
/// units, `user` a principal.
async fn mk_scope(
    conn: &mut sqlx::PgConnection,
    id: synveda_types::ScopeId,
    tenant: synveda_types::TenantId,
    parent: Option<synveda_types::ScopeId>,
    kind: synveda_types::scope::ScopeKind,
    slug: &str,
    name: &str,
) -> synveda_types::scope::Scope {
    let mut new = new_scope(tenant, parent, kind, slug, name);
    new.id = id;
    synveda_store::scopes::create(conn, &new)
        .await
        .expect("seed scope")
}

fn new_scope(
    tenant: synveda_types::TenantId,
    parent: Option<synveda_types::ScopeId>,
    kind: synveda_types::scope::ScopeKind,
    slug: &str,
    name: &str,
) -> synveda_store::scopes::NewScope {
    synveda_store::scopes::NewScope {
        id: synveda_types::ScopeId::new(),
        tenant_id: tenant,
        kind,
        parent_scope_id: parent,
        slug: slug.to_owned(),
        display_name: name.to_owned(),
        attributes: serde_json::json!({}),
        principal_id: None,
        created_by: None,
    }
}

/** A principal scope, which must name its subject (ADR-0073 decision 2). */
async fn mk_principal_scope(
    conn: &mut sqlx::PgConnection,
    tenant: synveda_types::TenantId,
    parent: Option<synveda_types::ScopeId>,
    slug: &str,
    name: &str,
    subject: &str,
) -> synveda_types::scope::Scope {
    let mut new = new_scope(
        tenant,
        parent,
        synveda_types::scope::ScopeKind::Principal,
        slug,
        name,
    );
    new.principal_id = Some(subject.to_owned());
    synveda_store::scopes::create(conn, &new)
        .await
        .expect("seed principal scope")
}

/// Serialises the suite: both tests read the one shared queue, and each
/// purges other suites' accumulated leftovers first.
async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

async fn admitted_tenant() -> Option<(PgPool, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping MEM-3 observe-queue test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    // Other suites stage signals they never consume; purge so reads see
    // only this test's messages (they'd otherwise sit behind the backlog).
    sqlx::query_scalar!(r#"select pgmq.purge_queue('observe') as "purged!""#)
        .fetch_one(&pool)
        .await
        .expect("purge observe queue");
    let id = TenantId::new();
    let slug = format!("mem3q-{}", id.as_uuid().simple());
    tenants::create(
        &pool,
        id,
        &slug,
        "MEM-3 queue test tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    Some((pool, id))
}

/// Seeds an org → user leaf → identity, and stages one event of `kind`
/// with a work signal. Returns the identity's scope, id, and the staged
/// event id.
async fn stage_one(
    pool: &PgPool,
    tenant: TenantId,
    text: &str,
    kind: ObserveKind,
) -> (ScopeId, IdentityId, ObserveEventId) {
    let mut tx = pool.begin().await.expect("begin");
    let org = mk_scope(
        &mut tx,
        ScopeId::new(),
        tenant,
        None,
        ScopeKind::Tenant,
        "acme",
        "ACME",
    )
    .await;
    let owner = IdentityId::new();
    let leaf = mk_principal_scope(
        &mut tx,
        tenant,
        Some(org.id),
        &format!("u-{}", owner.as_uuid().simple()),
        "queue-owner",
        &format!("queue-owner-{owner}"),
    )
    .await;
    identities::create(
        &mut tx,
        owner,
        tenant,
        Some("queue-owner"),
        IdentityKind::User,
        None,
        None,
        leaf.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit seed");

    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let admitted = observe::buffer_batch(
        &mut tx,
        tenant,
        leaf.id,
        owner,
        "queue-session",
        &[NewObserveEvent {
            idempotency_key: format!("q-{}", ObserveEventId::new().as_uuid().simple()),
            kind,
            payload: serde_json::json!({"text": text}),
            occurred_at: chrono::Utc::now(),
            redactions: None,
            quarantine: false,
        }],
    )
    .await
    .expect("buffer event");
    tx.commit().await.expect("commit staging");
    (leaf.id, owner, admitted[0].id)
}

/// Reads up to `qty` messages and returns this tenant's signals.
async fn read_own(pool: &PgPool, tenant: TenantId, vt_secs: i32, qty: i32) -> Vec<QueuedSignal> {
    let mut conn = pool.acquire().await.expect("acquire");
    observe::read_signals(&mut conn, vt_secs, qty)
        .await
        .expect("read signals")
        .into_iter()
        .filter_map(|message| match message {
            ObserveMessage::Signal(signal) if signal.tenant_id == tenant => Some(signal),
            _ => None,
        })
        .collect()
}

/// The consumer contract end to end: a staged event's signal reads back
/// with its ids, disappears behind the visibility timeout, redelivers
/// with a climbing read count after expiry, and the archive consumes it
/// exactly once — the second archive returns `false`, the loser's signal
/// in the worker's race (ADR-0022 decision 2).
#[tokio::test]
async fn signals_read_redeliver_and_archive_exactly_once() {
    let _serial = serial().await;
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (scope, owner, event_id) = stage_one(
        &pool,
        tenant,
        "queue roundtrip",
        ObserveKind::TranscriptDelta,
    )
    .await;

    // First read: the signal, invisible to a second reader while vt runs.
    let signals = read_own(&pool, tenant, 1, 100).await;
    assert_eq!(signals.len(), 1);
    let signal = signals[0];
    assert_eq!(signal.event_id, event_id);
    assert_eq!(signal.read_ct, 1);
    assert!(read_own(&pool, tenant, 1, 100).await.is_empty());

    // After expiry it redelivers, read count climbing — the worker's
    // dead-letter threshold is built on exactly this.
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let redelivered = read_own(&pool, tenant, 30, 100).await;
    assert_eq!(redelivered.len(), 1);
    assert_eq!(redelivered[0].msg_id, signal.msg_id);
    assert_eq!(redelivered[0].read_ct, 2);

    // Archive inside a tenant transaction (the worker's shape): first
    // wins, second sees nothing to consume.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    assert!(
        observe::archive_signal(&mut tx, signal.msg_id)
            .await
            .expect("archive")
    );
    assert!(
        !observe::archive_signal(&mut tx, signal.msg_id)
            .await
            .expect("re-archive")
    );

    // The staging row loads with the placement and provenance the
    // pipeline needs; a random id loads nothing.
    let event = observe::load_event(&mut tx, tenant, event_id)
        .await
        .expect("load event")
        .expect("row exists");
    assert_eq!(event.id, event_id);
    assert_eq!(event.scope_id, scope);
    assert_eq!(event.owner_id, owner);
    assert_eq!(event.session_id, "queue-session");
    assert_eq!(event.kind, ObserveKind::TranscriptDelta);
    assert_eq!(event.payload["text"], "queue roundtrip");
    assert!(event.redactions.is_none());
    assert!(
        observe::load_event(&mut tx, tenant, ObserveEventId::new())
            .await
            .expect("load missing")
            .is_none()
    );
    tx.commit().await.expect("commit");
}

/// A message body outside the signal shape (only a database-credentialed
/// writer can mint one) surfaces as `Malformed` with its archive handle —
/// the worker consumes it defensively instead of wedging the queue.
#[tokio::test]
async fn malformed_messages_surface_with_their_archive_handle() {
    let _serial = serial().await;
    let Some((pool, _tenant)) = admitted_tenant().await else {
        return;
    };
    let sent: i64 = sqlx::query_scalar!(
        r#"select pgmq.send('observe', '{"not": "a signal"}'::jsonb) as "msg_id!""#
    )
    .fetch_one(&pool)
    .await
    .expect("send garbage");

    let mut conn = pool.acquire().await.expect("acquire");
    let malformed = observe::read_signals(&mut conn, 1, 100)
        .await
        .expect("read signals")
        .into_iter()
        .find_map(|message| match message {
            ObserveMessage::Malformed { msg_id } if msg_id == sent => Some(msg_id),
            _ => None,
        })
        .expect("the garbage message surfaces as malformed");
    assert!(
        observe::archive_signal(&mut conn, malformed)
            .await
            .expect("archive garbage")
    );
}

/// The stored vocabulary and [`ObserveKind`] are two spellings of one list,
/// and nothing but this test makes them agree. The staging column is
/// CHECK-constrained (migration 0012, widened to four values by 0035 for
/// ADR-0057 decision 8's `assertion`), so a variant added in Rust without a
/// migration inserts fine in every unit test and fails at the database on
/// the first real write — a failure that surfaces in production traffic
/// rather than in CI.
///
/// Reads the constraint back from the catalogue rather than restating it:
/// a test that hard-codes the four strings passes when the migration was
/// never applied.
#[tokio::test]
async fn the_check_constraint_admits_exactly_the_observe_kinds_rust_knows() {
    // Reads nothing from the queue, but `admitted_tenant` purges it — so
    // this holds the same lock as everything else here, or it purges the
    // backlog out from under a test that is mid-read.
    let _serial = serial().await;
    let Some((pool, _tenant)) = admitted_tenant().await else {
        return;
    };
    let definition: String = sqlx::query_scalar!(
        r#"select pg_get_constraintdef(oid) as "def!"
           from pg_constraint
           where conname = 'observe_events_kind_check'"#
    )
    .fetch_one(&pool)
    .await
    .expect("the kind CHECK constraint exists");

    for kind in ObserveKind::ALL {
        assert!(
            definition.contains(&format!("'{}'", kind.as_str())),
            "ObserveKind::{kind:?} ({}) is missing from {definition} — \
             a migration widening the CHECK is missing",
            kind.as_str()
        );
    }
    // The other direction: a value the database accepts that Rust cannot
    // name is just as broken, and is how a removed variant leaves rows
    // nothing can parse. Counting quoted literals is enough — the
    // constraint is a single `= ANY (ARRAY[...])` over string literals.
    let admitted = definition.matches("::text").count();
    assert_eq!(
        admitted,
        ObserveKind::ALL.len(),
        "the CHECK admits {admitted} values but ObserveKind has {} — {definition}",
        ObserveKind::ALL.len()
    );
}

/// The point of the migration, end to end: an `assertion` reaches the
/// staging buffer and reads back as one. Before 0035 this insert fails the
/// CHECK — which is exactly what a model-driven `remember` call would have
/// hit on its first use (ADR-0057 decision 8).
#[tokio::test]
async fn an_assertion_buffers_and_reads_back_with_its_kind() {
    let _guard = serial().await;
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (_scope, _owner, event_id) = stage_one(
        &pool,
        tenant,
        "the deploy target is eu-west-1",
        ObserveKind::Assertion,
    )
    .await;

    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let staged = observe::load_event(&mut tx, tenant, event_id)
        .await
        .expect("load staged event")
        .expect("the assertion is staged");
    assert_eq!(
        staged.kind,
        ObserveKind::Assertion,
        "the kind must survive the round trip — if it degrades to \
         transcript_delta the model-asserted distinction is lost at the \
         first hop (ADR-0057 decision 8)"
    );
}
