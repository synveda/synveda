//! AUTH-5's acceptance criteria (ADR-0060).
//!
//! The feature text names two: *drift converges ≤ sync interval*, and
//! *deletions handled as leavers*. ADR-0060 decision 4 splits the first into
//! two bounds and this suite asserts them **separately**, because collapsing
//! them is how the widening gets lost:
//!
//! 1. **Joiners, movers and explicit deactivations converge in one complete
//!    pass.** That is the AC's bound as written.
//! 2. **Absence-derived leavers converge in `N + 1`.** That is wider than the
//!    AC asks for, deliberately, and it is the entire content of decision 3.
//!    A test that only measured the first would be measuring the easy half
//!    and reporting it as the whole.
//!
//! Around them, the claims a reader would otherwise take on trust:
//!
//! 3. **An incomplete pass concludes nothing** (decision 3.1) — and the
//!    assertion is a *contrast*, because "nothing happened" is also what a
//!    broken test looks like. The same absence, in a complete pass and an
//!    incomplete one, produces a count and no count.
//! 4. **Deactivation is an act and absence is not** (decision 3): the same
//!    person, gone two ways, seals on pass one or on pass three.
//! 5. **The breaker refuses a bulk departure, and an authorisation releases
//!    it exactly once** (decisions 3.3 and 10).
//! 6. **A tenant the push plane owns is not pulled** (decision 5).
//!
//! The connector is scripted rather than mocked over HTTP: the vendors' wire
//! shapes are `synveda-identity`'s own suite, and what is under test here is
//! what a pass *concludes*, which is a different question and deserves an
//! input this suite can state exactly.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::AppState;
use synveda_gateway::directory_sync::{PassReport, SyncConfig, run_once};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_identity::directory::{
    DirectoryConnector, DirectorySnapshot, DirectoryUserRecord, Enumeration,
};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::{
    directory, directory_sync, group_mappings, hierarchy, identities, rls, tenants,
};
use synveda_types::{ScimCredentialId, ScopeId, ScopeKind, Tenant, TenantId, TenantStatus};

const SECRET: &[u8] = b"auth-5-directory-sync-suite-secret";

// ── The scripted directory ───────────────────────────────────────────────────

/// A directory whose every pass this suite states outright.
///
/// Scripted rather than served over HTTP because the question here is what a
/// pass concludes, not whether Graph's paging is followed — that is
/// `synveda-identity`'s suite, and duplicating it would make this one slower
/// without making it stricter.
struct ScriptedDirectory {
    passes: Mutex<VecDeque<Enumeration>>,
}

impl ScriptedDirectory {
    fn new(passes: Vec<Enumeration>) -> Self {
        Self {
            passes: Mutex::new(passes.into_iter().collect()),
        }
    }
}

#[async_trait]
impl DirectoryConnector for ScriptedDirectory {
    fn name(&self) -> &'static str {
        "scripted"
    }

    async fn enumerate(&self) -> Enumeration {
        self.passes
            .lock()
            .expect("lock")
            .pop_front()
            .expect("the suite scripted fewer passes than it ran")
    }
}

fn person(external_id: &str, user_name: &str, group: &str) -> DirectoryUserRecord {
    DirectoryUserRecord {
        external_id: external_id.to_owned(),
        user_name: user_name.to_owned(),
        active: true,
        display_name: None,
        given_name: None,
        family_name: None,
        work_email: Some(user_name.to_owned()),
        groups: vec![group.to_owned()],
    }
}

fn complete(users: Vec<DirectoryUserRecord>) -> Enumeration {
    Enumeration::Complete(DirectorySnapshot { users })
}

fn partial(users: Vec<DirectoryUserRecord>) -> Enumeration {
    Enumeration::Partial {
        snapshot: DirectorySnapshot { users },
        failure: "429 from the second page".to_owned(),
    }
}

// ── Harness ──────────────────────────────────────────────────────────────────

struct World {
    pool: PgPool,
    tenant: TenantId,
    tenant_row: Tenant,
    state: AppState,
}

async fn world() -> Option<World> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping the AUTH-5 directory sync suite: DATABASE_URL is not set \
             (run `make dev-up` then `make db-test`)"
        );
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");

    let tenant = TenantId::new();
    let slug = format!("auth5-{}", tenant.as_uuid().simple());
    tenants::create(&pool, tenant, &slug, "AUTH-5 tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");

    let mut tx = pool.begin().await.expect("begin");
    let org = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        None,
        ScopeKind::Org,
        "acme",
        "acme",
    )
    .await
    .expect("org");
    let eng = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Department,
        "eng",
        "eng",
    )
    .await
    .expect("eng");
    let core = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(eng.id),
        ScopeKind::Team,
        "core",
        "core",
    )
    .await
    .expect("core");
    hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
        identities::QUARANTINE_SLUG,
    )
    .await
    .expect("quarantine");
    tx.commit().await.expect("commit hierarchy");

    let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("begin");
    group_mappings::upsert(&mut *tx, tenant, "synveda-eng-core", core.id)
        .await
        .expect("map group");
    tx.commit().await.expect("commit mapping");

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let state = app_state(&url, pdp);
    let tenant_row = tenants::by_id(&pool, tenant)
        .await
        .expect("read tenant")
        .expect("tenant exists");
    Some(World {
        pool,
        tenant,
        tenant_row,
        state,
    })
}

/// One pass, with the suite's default tuning: `N = 2`, and a breaker that
/// tolerates a tenth of a tenant above a floor of five.
async fn pass(w: &World, connector: &ScriptedDirectory) -> PassReport {
    run_once(&w.state, &w.tenant_row, connector, &SyncConfig::default())
        .await
        .expect("a pass")
}

/// Whether this person's personal scope is sealed — the question the AC is
/// really asking, read through the identity that owns it rather than through
/// anything this feature wrote.
///
/// Looked up by email rather than by `userName`, because
/// `directory::user_by_user_name` filters to **live** rows: a sealed person's
/// mirror row is `active: false` and would come back as "no such user",
/// which reads exactly like a test that is not finding what it seeded.
async fn is_sealed(w: &World, email: &str) -> bool {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let identity = identities::by_email(&mut *tx, w.tenant, email)
        .await
        .expect("read identity");
    tx.commit().await.expect("commit");
    identity
        .expect("the suite seeded this person, so an identity exists")
        .sealed()
}

/// Where somebody has been placed.
async fn placed_at(w: &World, email: &str) -> Option<ScopeId> {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let identity = identities::by_email(&mut *tx, w.tenant, email)
        .await
        .expect("read identity");
    tx.commit().await.expect("commit");
    identity.map(|identity| identity.scope_id)
}

// ── 1. Drift bound A: one complete pass ─────────────────────────────────────

/// A joiner is placed by the pass that first lists them.
///
/// This is the AC's bound as written — drift converges within one interval —
/// and it is the half that has to be fast, because a person who started this
/// morning is waiting on it.
#[tokio::test]
async fn a_joiner_converges_in_a_single_complete_pass() {
    let Some(w) = world().await else { return };
    let directory = ScriptedDirectory::new(vec![complete(vec![person(
        "u1",
        "alice@example.test",
        "synveda-eng-core",
    )])]);

    let report = pass(&w, &directory).await;
    assert_eq!(report.seen, 1);
    assert!(report.complete);
    assert_eq!(report.sealed, 0);

    assert!(
        placed_at(&w, "alice@example.test").await.is_some(),
        "one pass places them"
    );
    assert!(
        !is_sealed(&w, "alice@example.test").await,
        "and a joiner is not a leaver"
    );
}

// ── 2. The AC: deletions handled as leavers, at the wider bound ─────────────

/// **The acceptance criterion.** A person the directory stops listing becomes
/// a leaver — and takes `N + 1` passes to do it.
///
/// Asserted as a sequence rather than an endpoint, because the interesting
/// claim is not that they are eventually sealed but that they are *not*
/// sealed before the evidence is in. Pass 1 places them. Pass 2 is the first
/// complete pass that does not list them: a hypothesis, not a finding. Pass 3
/// is the second, and only then does the seal happen.
///
/// This is ADR-0060 decision 4's second bound, and it is wider than the
/// feature text's "≤ sync interval" on purpose: the alternative is sealing
/// somebody because one response was throttled.
#[tokio::test]
async fn a_directory_deletion_becomes_a_leaver_after_two_complete_passes() {
    let Some(w) = world().await else { return };
    let alice = person("u1", "alice@example.test", "synveda-eng-core");
    let bob = person("u2", "bob@example.test", "synveda-eng-core");
    let directory = ScriptedDirectory::new(vec![
        complete(vec![alice.clone(), bob.clone()]),
        // Bob is deleted at the directory: not deactivated, simply gone.
        complete(vec![alice.clone()]),
        complete(vec![alice.clone()]),
    ]);

    let first = pass(&w, &directory).await;
    assert_eq!(first.seen, 2);
    assert_eq!(first.absent, 0, "everybody was listed");

    let second = pass(&w, &directory).await;
    assert_eq!(second.absent, 1, "one person went missing");
    assert_eq!(
        second.sealed, 0,
        "one missed pass is a hypothesis: a throttled page looks exactly \
         like this, and the seal does not lift"
    );
    assert!(!is_sealed(&w, "bob@example.test").await);

    let third = pass(&w, &directory).await;
    assert_eq!(
        third.sealed, 1,
        "two consecutive complete passes is a finding, and the AC's \
         'deletions handled as leavers' is discharged here"
    );
    assert!(
        is_sealed(&w, "bob@example.test").await,
        "and the seal is AUTH-4's own, reached through reconcile"
    );
    assert!(
        !is_sealed(&w, "alice@example.test").await,
        "while everybody the directory kept listing is untouched"
    );
}

// ── 3. An incomplete pass concludes nothing ─────────────────────────────────

/// The same absence, twice, in a complete pass and an incomplete one.
///
/// A contrast rather than an assertion about one run, because "nobody was
/// sealed" is also what a test that silently did nothing would report. The
/// incomplete pass must still record **presence** — the people it listed were
/// listed — while contributing nothing at all to the absence count.
///
/// This is the one place ADR-0060 decision 3.1 is enforced rather than
/// described, and it is enforced by an omission: an incomplete pass returns
/// before `complete_pass`, so `passes_completed` never moves.
#[tokio::test]
async fn an_incomplete_pass_records_presence_and_concludes_nothing() {
    let Some(w) = world().await else { return };
    let alice = person("u1", "alice@example.test", "synveda-eng-core");
    let bob = person("u2", "bob@example.test", "synveda-eng-core");
    let carol = person("u3", "carol@example.test", "synveda-eng-core");
    let directory = ScriptedDirectory::new(vec![
        complete(vec![alice.clone(), bob.clone()]),
        // A pass that failed on its second page, listing Alice and a person
        // it had never listed before, and never reaching Bob.
        partial(vec![alice.clone(), carol.clone()]),
        partial(vec![alice.clone(), carol.clone()]),
        partial(vec![alice.clone(), carol.clone()]),
    ]);

    pass(&w, &directory).await;

    let mut passes_completed_before = 0;
    {
        let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
            .await
            .expect("begin");
        if let Some(state) = directory_sync::state(&mut *tx, w.tenant)
            .await
            .expect("state")
        {
            passes_completed_before = state.passes_completed;
        }
        tx.commit().await.expect("commit");
    }

    // Three incomplete passes in a row — more than `N` — and Bob is absent
    // from every one of them.
    for _ in 0..3 {
        let report = pass(&w, &directory).await;
        assert!(!report.complete);
        assert_eq!(report.absent, 0, "an incomplete pass counts no absence");
        assert_eq!(report.sealed, 0);
    }

    assert!(
        !is_sealed(&w, "bob@example.test").await,
        "three incomplete passes are not two complete ones, however many \
         of them miss the same person"
    );
    assert!(
        placed_at(&w, "carol@example.test").await.is_some(),
        "and presence survives: somebody first seen in a failed pass is \
         still placed, because being listed is not conditional on \
         everybody being listed"
    );

    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let state = directory_sync::state(&mut *tx, w.tenant)
        .await
        .expect("state")
        .expect("a state row");
    tx.commit().await.expect("commit");
    assert_eq!(
        state.passes_completed, passes_completed_before,
        "the completeness proof did not move, which is the mechanism \
         rather than the symptom"
    );
}

// ── 4. Deactivation is an act; absence is not ───────────────────────────────

/// The asymmetry, as a contrast against test 2.
///
/// The same person leaving the same company, told two ways. `active: false`
/// is something the directory *did*, and it seals on the first complete pass
/// that sees it. Vanishing from the listing is something we *inferred*, and
/// it takes `N`. That difference is the whole of decision 3.
#[tokio::test]
async fn an_explicit_deactivation_seals_on_the_first_complete_pass() {
    let Some(w) = world().await else { return };
    let alice = person("u1", "alice@example.test", "synveda-eng-core");
    let mut bob = person("u2", "bob@example.test", "synveda-eng-core");
    let directory = ScriptedDirectory::new(vec![complete(vec![alice.clone(), bob.clone()]), {
        bob.active = false;
        complete(vec![alice.clone(), bob.clone()])
    }]);

    pass(&w, &directory).await;
    assert!(!is_sealed(&w, "bob@example.test").await);

    let second = pass(&w, &directory).await;
    assert_eq!(
        second.absent, 0,
        "he was listed, so no absence is counted for him at all"
    );
    assert!(
        is_sealed(&w, "bob@example.test").await,
        "an act seals immediately; only an inference waits"
    );
}

// ── 5. The breaker, and its release ─────────────────────────────────────────

/// A bulk departure is refused, and an authorisation releases it once.
///
/// The arc decision 10 exists for: the pass proposes more than the breaker
/// tolerates and seals nobody; an operator signs a bounded, reasoned
/// authorisation; the next pass spends it and seals exactly that set; and a
/// later bulk departure trips again, because the authorisation was one-shot
/// and did not become a standing permission.
#[tokio::test]
async fn the_breaker_refuses_a_bulk_departure_until_somebody_authorises_it() {
    let Some(w) = world().await else { return };
    let everyone: Vec<DirectoryUserRecord> = (0..20)
        .map(|index| {
            person(
                &format!("u{index}"),
                &format!("person-{index}@example.test"),
                "synveda-eng-core",
            )
        })
        .collect();
    // Twelve of twenty leave at once — over the floor and over the share.
    let survivors: Vec<DirectoryUserRecord> = everyone[..8].to_vec();
    let directory = ScriptedDirectory::new(vec![
        complete(everyone.clone()),
        complete(survivors.clone()),
        complete(survivors.clone()),
        complete(survivors.clone()),
    ]);

    pass(&w, &directory).await;
    pass(&w, &directory).await;

    // The pass at which they reach the threshold: refused, and sized.
    let refused = pass(&w, &directory).await;
    assert_eq!(refused.sealed, 0, "the breaker seals none of them");
    assert_eq!(
        refused.refused,
        Some(12),
        "and says how many it declined, which is what an operator needs \
         to judge it"
    );
    assert!(!is_sealed(&w, "person-19@example.test").await);

    // An operator signs for exactly that many.
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let granted = directory_sync::authorise_seals(
        &mut *tx,
        w.tenant,
        12,
        3600.0,
        "alice@example.test",
        "Q3 restructure, ticket OPS-1123",
    )
    .await
    .expect("authorise");
    tx.commit().await.expect("commit");
    assert!(granted);

    let released = pass(&w, &directory).await;
    assert!(released.authorised, "the authorisation was spent");
    assert_eq!(released.sealed, 12, "and exactly the proposed set sealed");
    assert!(is_sealed(&w, "person-19@example.test").await);
    assert!(
        !is_sealed(&w, "person-0@example.test").await,
        "while everybody still listed is untouched"
    );

    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let state = directory_sync::state(&mut *tx, w.tenant)
        .await
        .expect("state")
        .expect("a state row");
    tx.commit().await.expect("commit");
    assert!(
        state.authorisation.is_none(),
        "one-shot: it did not become a standing permission for the next \
         directory failure"
    );
}

// ── 6. The push plane owns its tenants ──────────────────────────────────────

/// A tenant with a live SCIM credential is not pulled at all.
///
/// Two authorities for one fact is bad enough; two where one of them infers
/// departure from absence is a working directory deprovisioning people it
/// never deprovisioned (decision 5). The pull yields, and it yields
/// *entirely* — it does not enumerate, so it cannot even record presence.
#[tokio::test]
async fn a_tenant_the_push_plane_owns_is_not_pulled() {
    let Some(w) = world().await else { return };
    let minted = synveda_identity::scim::mint(w.tenant).expect("mint");
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    directory::issue_credential(
        &mut *tx,
        ScimCredentialId::new(),
        w.tenant,
        &minted.hash,
        "push plane",
        Utc::now() + chrono::Duration::days(1),
        "test-operator",
    )
    .await
    .expect("issue credential");
    tx.commit().await.expect("commit");

    // Scripted with a pass it must never take: `enumerate` panics if called.
    let directory = ScriptedDirectory::new(Vec::new());
    let report = pass(&w, &directory).await;

    assert!(report.yielded, "the pull yields to the push plane");
    assert_eq!(report.seen, 0);
    assert!(
        placed_at(&w, "alice@example.test").await.is_none(),
        "and nothing was read, so nothing was written"
    );
}

// ── Harness plumbing ────────────────────────────────────────────────────────

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: std::sync::OnceLock<PrometheusHandle> = std::sync::OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install metrics"))
        .clone()
}

fn index_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("synveda-auth5-{}", ScopeId::new().as_uuid()));
    std::fs::create_dir_all(&root).expect("create index root");
    root
}

fn app_state(url: &str, pdp: Arc<Pdp>) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp,
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
    }
}
