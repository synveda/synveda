//! Dense Knowledge plan, recall and latency evidence at 1024 dimensions.
//!
//! Two tenants of different corpus sizes share the one partial HNSW index,
//! as every installation's tenants do. For each request shape the product
//! issues, the benchmark records exact ground truth, then each planner and
//! pgvector setting arm on its own fresh connection: recall@10 and
//! recall@limit, rows returned, latency, ordering stability and one explained
//! plan. A separate study runs the unmodified product statement six times on
//! one connection and reads which plan class PostgreSQL chose each time.
//!
//! Run it explicitly with `--ignored` through the exact-role fixture
//! (`demos/ctx-7-dense-plans.sh`). `SYNVEDA_DENSE_PLAN_ITEMS` lists tenant
//! corpus sizes, `SYNVEDA_DENSE_PLAN_QUERIES` the query count,
//! `SYNVEDA_DENSE_PLAN_REPETITIONS` the timed passes, `SYNVEDA_DENSE_PLAN_SEED`
//! the geometry seed, `SYNVEDA_DENSE_PLAN_ARMS` a subset of arm names and
//! `SYNVEDA_DENSE_PLAN_REPORT` a report path.

#[path = "support/context_world.rs"]
mod context_world;
use context_world::*;

#[path = "support/dense_corpus.rs"]
mod dense_corpus;
use dense_corpus::*;

use synveda_types::KnowledgeItemId;

const ARMS: [Arm; 8] = [
    arm(
        "shipped_custom",
        "force_custom_plan",
        "off",
        DEFAULT_EF_SEARCH,
        DEFAULT_MAX_SCAN_TUPLES,
    ),
    arm(
        "shipped_generic",
        "force_generic_plan",
        "off",
        DEFAULT_EF_SEARCH,
        DEFAULT_MAX_SCAN_TUPLES,
    ),
    arm(
        "custom_strict_40",
        "force_custom_plan",
        "strict_order",
        40,
        DEFAULT_MAX_SCAN_TUPLES,
    ),
    arm(
        "custom_strict_100",
        "force_custom_plan",
        "strict_order",
        100,
        DEFAULT_MAX_SCAN_TUPLES,
    ),
    arm(
        "custom_relaxed_100",
        "force_custom_plan",
        "relaxed_order",
        100,
        DEFAULT_MAX_SCAN_TUPLES,
    ),
    arm(
        "custom_strict_200",
        "force_custom_plan",
        "strict_order",
        200,
        DEFAULT_MAX_SCAN_TUPLES,
    ),
    arm(
        "generic_strict_100",
        "force_generic_plan",
        "strict_order",
        100,
        DEFAULT_MAX_SCAN_TUPLES,
    ),
    EXACT,
];

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn corpus_sizes() -> Vec<usize> {
    std::env::var("SYNVEDA_DENSE_PLAN_ITEMS")
        .unwrap_or_else(|_| "10000,50000".to_owned())
        .split(',')
        .map(|size| size.trim().parse().expect("corpus size"))
        .collect()
}

/// `SYNVEDA_DENSE_PLAN_ARMS` narrows the arms by name; unset runs them all.
fn selected_arms() -> Vec<Arm> {
    let Ok(names) = std::env::var("SYNVEDA_DENSE_PLAN_ARMS") else {
        return ARMS.to_vec();
    };
    let names: Vec<&str> = names.split(',').map(str::trim).collect();
    let arms: Vec<Arm> = ARMS
        .into_iter()
        .filter(|arm| names.contains(&arm.name))
        .collect();
    assert_eq!(
        arms.len(),
        names.len(),
        "unknown arm in SYNVEDA_DENSE_PLAN_ARMS"
    );
    arms
}

async fn measure_shape(
    world: &World,
    shape: &Shape,
    queries: &[Vec<f32>],
    repetitions: usize,
) -> Value {
    let (truth, truth_latency) = ground_truth(world, shape, queries).await;
    let selected = selected_arms();
    let mut arms = Vec::with_capacity(selected.len());
    for arm in &selected {
        let outcome = measure_arm(world, shape, arm, queries, &truth, repetitions).await;
        eprintln!(
            "dense-plans {} {} recall@10={} recall@limit={} returned={} p50={} p99={} plan={:?} hnsw={}",
            shape.name,
            arm.name,
            outcome["recall_at_10"]["mean"],
            outcome["recall_at_limit"]["mean"],
            outcome["returned"]["mean"],
            outcome["latency_ms"]["p50"],
            outcome["latency_ms"]["p99"],
            outcome["plan"]["plan_class"],
            outcome["plan"]["uses_hnsw"],
        );
        arms.push(outcome);
    }
    let switch = switch_study(world, shape, &queries[0]).await;
    json!({
        "shape": shape.name,
        "limit": shape.limit,
        "scope_ids": shape.filters.scope_ids.len(),
        "historical": shape.filters.as_known_at.is_some(),
        "exact_first_pass_latency_ms": quantiles(truth_latency),
        "arms": arms,
        "auto_six_executions": switch,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "operator-run 1024-dimension dense plan, recall and latency benchmark"]
async fn dense_leg_plans_recall_and_latency_at_1024_dimensions() {
    let _guard = serial().await;
    let seed = setting("SYNVEDA_DENSE_PLAN_SEED", 1024);
    let query_count = setting("SYNVEDA_DENSE_PLAN_QUERIES", 40) as usize;
    let repetitions = setting("SYNVEDA_DENSE_PLAN_REPETITIONS", 2) as usize;
    let geometry = Geometry::new(seed);
    let mut tenants = Vec::new();
    for (index, items) in corpus_sizes().into_iter().enumerate() {
        let Some(world) = admitted_world().await else {
            return;
        };
        let started = Instant::now();
        let spec = Spec {
            items,
            projects: 30,
            sourced: (items / 200).max(10),
            seed: seed.wrapping_add(index as u64 + 1),
        };
        let corpus = dense_corpus::seed(&world, &spec, &geometry).await;
        let seconds = started.elapsed().as_secs_f64();
        eprintln!("dense-plans seeded {items} items in {seconds:.1}s");
        tenants.push((world, corpus, seconds));
    }
    analyze(&tenants[0].0).await;
    let environment = environment(&tenants[0].0).await;
    let at = database_now(&tenants[0].0.state.pool).await;
    let mut rng = Rng::new(seed ^ 0xC0FF_EE00);
    let queries: Vec<Vec<f32>> = (0..query_count)
        .map(|_| {
            let topic = rng.below(geometry.topics());
            geometry.query(topic, &mut rng)
        })
        .collect();

    let mut corpora = Vec::new();
    for (world, corpus, seconds) in &tenants {
        let mut measured = Vec::new();
        for shape in shapes(corpus, at) {
            measured.push(measure_shape(world, &shape, &queries, repetitions).await);
        }
        corpora.push(json!({
            "corpus": corpus.stats,
            "seeding_seconds": seconds,
            "shapes": measured,
        }));
    }
    let report = json!({
        "schema": "synveda.dense-plans.v1",
        "dimension": DIMENSION,
        "seed": seed,
        "queries": query_count,
        "repetitions": repetitions,
        "environment": environment,
        "corpora": corpora,
    });
    if let Ok(path) = std::env::var("SYNVEDA_DENSE_PLAN_REPORT") {
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&report).expect("render report"),
        )
        .expect("write dense plan report");
    }
}

/// The reviewed dense statement. Editing the SQL changes this digest and so
/// fails the plan-drift test until its expectations are reviewed again.
const REVIEWED_SQL_BLAKE3: &str =
    "182b1fb84693bb28c19bf151c579fc859a89fce0f167145ceeb5a831ec15c372";

async fn small_corpus(seed: u64) -> Option<(World, DenseCorpus)> {
    let world = admitted_world().await?;
    let spec = Spec {
        items: 400,
        projects: 8,
        sourced: 10,
        seed,
    };
    let corpus = dense_corpus::seed(&world, &spec, &Geometry::new(seed)).await;
    analyze(&world).await;
    Some((world, corpus))
}

fn query_vector(seed: u64) -> Vec<f32> {
    let geometry = Geometry::new(seed);
    let mut rng = Rng::new(seed ^ 0x51DE);
    geometry.query(rng.below(geometry.topics()), &mut rng)
}

async fn session_setting(conn: &mut sqlx::PgConnection, name: &str) -> Option<String> {
    sqlx::query_scalar("select current_setting($1, true)")
        .bind(name)
        .fetch_one(&mut *conn)
        .await
        .expect("read setting")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_execution_plans_with_its_arguments_and_returns_exact_ordering() {
    let _guard = serial().await;
    let Some((world, corpus)) = small_corpus(2_001).await else {
        return;
    };
    let vector = query_vector(2_001);
    for shape in shapes(&corpus, database_now(&world.state.pool).await) {
        let (truth, _) = ground_truth(&world, &shape, std::slice::from_ref(&vector)).await;
        let pool = one_connection(&world).await;
        // A session that would otherwise use the generic plan from the first
        // execution: only the statement's own setting keeps it custom.
        sqlx::query("set plan_cache_mode = force_generic_plan")
            .execute(&pool)
            .await
            .expect("session default");
        let mut previous = (0, 0);
        for execution in 1..=6 {
            let (ids, _) = run_dense(&pool, world.tenant_id, &shape, &vector, None).await;
            assert_eq!(
                ids, truth[0],
                "{} execution {execution} ordering",
                shape.name
            );
            let mut conn = pool.acquire().await.expect("the same connection");
            let counts = product_plan_counts(&mut conn).await;
            assert_eq!(
                (counts.0 - previous.0, counts.1 - previous.1),
                (0, 1),
                "{} execution {execution} must be a custom plan",
                shape.name
            );
            previous = counts;
        }
        pool.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_plan_setting_never_outlives_the_dense_statement() {
    let _guard = serial().await;
    let Some((world, corpus)) = small_corpus(2_002).await else {
        return;
    };
    let other = admitted_world().await.expect("second tenant");
    let shape = shapes(&corpus, database_now(&world.state.pool).await).remove(0);
    let vector = query_vector(2_002);
    let pool = one_connection(&world).await;
    let mut tx = rls::begin_tenant_tx(&pool, world.tenant_id)
        .await
        .expect("begin");
    sqlx::query("select set_config('plan_cache_mode', 'force_generic_plan', true)")
        .execute(&mut *tx)
        .await
        .expect("caller's own setting");
    dense_read(&mut tx, &world, &shape, &vector).await;
    assert_eq!(
        session_setting(&mut tx, "plan_cache_mode").await.as_deref(),
        Some("force_generic_plan")
    );
    tx.commit().await.expect("commit");

    let mut tx = rls::begin_tenant_tx(&pool, world.tenant_id)
        .await
        .expect("begin");
    dense_read(&mut tx, &world, &shape, &vector).await;
    assert_eq!(
        session_setting(&mut tx, "plan_cache_mode").await.as_deref(),
        Some("auto")
    );
    tx.commit().await.expect("commit");

    let mut tx = rls::begin_tenant_tx(&pool, other.tenant_id)
        .await
        .expect("next tenant");
    assert_eq!(
        session_setting(&mut tx, "synveda.tenant_id").await,
        Some(other.tenant_id.as_uuid().to_string())
    );
    assert_planner_defaults(&mut tx).await;
    tx.rollback().await.expect("end");
    pool.close().await;
}

async fn dense_read(conn: &mut sqlx::PgConnection, world: &World, shape: &Shape, vector: &[f32]) {
    synveda_store::knowledge_search::semantic_candidates(
        conn,
        world.tenant_id,
        &shape.filters,
        MODEL,
        vector,
        10,
    )
    .await
    .expect("dense read");
}

/// The next transaction sees the server's planner defaults, untouched by
/// the dense statement before it on the same connection.
async fn assert_planner_defaults(conn: &mut sqlx::PgConnection) {
    assert_eq!(
        session_setting(conn, "plan_cache_mode").await.as_deref(),
        Some("auto")
    );
    let ef_search = session_setting(conn, "hnsw.ef_search").await;
    assert!(
        matches!(ef_search.as_deref(), None | Some("40")),
        "{ef_search:?}"
    );
    let iterative = session_setting(conn, "hnsw.iterative_scan").await;
    assert!(
        matches!(iterative.as_deref(), None | Some("off")),
        "{iterative:?}"
    );
    assert_eq!(
        session_setting(conn, "enable_indexscan").await.as_deref(),
        Some("on")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reused_connections_keep_tenant_scope_source_and_time_semantics() {
    let _guard = serial().await;
    let Some((first, first_corpus)) = small_corpus(2_003).await else {
        return;
    };
    let Some((second, second_corpus)) = small_corpus(2_004).await else {
        return;
    };
    let at = database_now(&first.state.pool).await;
    let vector = query_vector(2_003);
    let pool = one_connection(&first).await;
    for round in 0..3 {
        for (world, corpus) in [(&first, &first_corpus), (&second, &second_corpus)] {
            let mut empty = shapes(corpus, at).remove(0);
            empty.name = "empty_allowlist";
            empty.filters.scope_ids.clear();
            for shape in shapes(corpus, at).into_iter().chain([empty]) {
                let (ids, _) = run_dense(&pool, world.tenant_id, &shape, &vector, None).await;
                let (truth, _) = ground_truth(world, &shape, std::slice::from_ref(&vector)).await;
                assert_eq!(ids, truth[0], "round {round} {}", shape.name);
                assert!(!ids.is_empty(), "{} returned nothing", shape.name);
                assert_admissible(corpus, &shape, &ids);
            }
        }
    }
    pool.close().await;
}

/// Checks returned items against what seeding recorded, not against the
/// statement under test.
fn assert_admissible(corpus: &DenseCorpus, shape: &Shape, ids: &[KnowledgeItemId]) {
    let ledger = &corpus.ledger;
    let historical = shape.filters.as_known_at.is_some();
    for id in ids {
        let scope = ledger
            .scope_of
            .get(id)
            .unwrap_or_else(|| panic!("{} returned an item from another tenant", shape.name));
        assert!(
            !ledger.withdrawn.contains(id),
            "{} returned a withdrawn-source item",
            shape.name
        );
        if historical {
            assert!(
                !ledger.late.contains(id),
                "{} returned an item created after its cut-off",
                shape.name
            );
        } else {
            assert!(
                !ledger.inactive.contains(id),
                "{} returned an inactive item",
                shape.name
            );
        }
        let allowed = shape.filters.scope_ids.is_empty() || shape.filters.scope_ids.contains(scope);
        assert!(
            allowed,
            "{} returned an item outside its scopes",
            shape.name
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_dense_plan_shape_matches_its_reviewed_expectation() {
    let _guard = serial().await;
    assert_eq!(
        blake3::hash(SEMANTIC_SQL.as_bytes()).to_hex().as_str(),
        REVIEWED_SQL_BLAKE3,
        "the dense statement changed: re-run the dense plan benchmark and review this expectation"
    );
    let Some((world, corpus)) = small_corpus(2_005).await else {
        return;
    };
    let vector = query_vector(2_005);
    let environment = environment(&world).await;
    for shape in shapes(&corpus, database_now(&world.state.pool).await) {
        let pool = one_connection(&world).await;
        let mut tx = rls::begin_tenant_tx(&pool, world.tenant_id)
            .await
            .expect("begin");
        sqlx::query("select set_config('plan_cache_mode', 'force_custom_plan', true)")
            .execute(&mut *tx)
            .await
            .expect("product policy");
        prepare_probe(&mut tx, "dense_drift_probe").await;
        let mut plans = Vec::new();
        for _ in 1..=6 {
            plans.push(
                explain_execution(
                    &mut tx,
                    "dense_drift_probe",
                    world.tenant_id,
                    &shape,
                    &vector,
                )
                .await,
            );
        }
        tx.rollback().await.expect("end");
        pool.close().await;
        for (execution, plan) in plans.iter().enumerate() {
            assert_eq!(
                plan.plan_class,
                "custom",
                "{} execution {}",
                shape.name,
                execution + 1
            );
            assert!(
                !plan.uses_hnsw,
                "{} now walks the HNSW index ({:?}) on {environment}; an HNSW scan returns at most \
                 hnsw.ef_search candidates before filtering, so review recall and the plan policy",
                shape.name, plan.indexes,
            );
            assert_eq!(plan.root, "Limit", "{}: {:?}", shape.name, plan.nodes);
        }
    }
}

/// A loopback TEI-compatible embedder for the seeded geometry: a text whose
/// first two words name a topic embeds near that topic's centroid.
async fn topic_embedder(seed: u64) -> (String, tokio::task::JoinHandle<()>) {
    use axum::routing::post;
    let geometry = Arc::new(Geometry::new(seed));
    let topics: Arc<HashMap<(&'static str, &'static str), usize>> = Arc::new(
        (0..geometry.topics())
            .map(|topic| (topic_words(topic), topic))
            .collect(),
    );
    let embed = move |axum::Json(body): axum::Json<Value>| {
        let (geometry, topics) = (Arc::clone(&geometry), Arc::clone(&topics));
        async move {
            let inputs = body["inputs"].as_array().cloned().unwrap_or_default();
            let vectors: Vec<Vec<f32>> = inputs
                .iter()
                .map(|input| {
                    let text = input.as_str().unwrap_or_default();
                    let mut words = text.split_whitespace();
                    let key = (
                        words.next().unwrap_or_default(),
                        words.next().unwrap_or_default(),
                    );
                    let topic = topics
                        .iter()
                        .find(|((first, second), _)| *first == key.0 && *second == key.1)
                        .map_or(0, |(_, topic)| *topic);
                    let mut rng = Rng::new(u64::from_le_bytes(
                        blake3::hash(text.as_bytes()).as_bytes()[..8]
                            .try_into()
                            .unwrap_or_default(),
                    ));
                    geometry.query(topic, &mut rng)
                })
                .collect();
            axum::Json(vectors)
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind topic embedder");
    let address = listener.local_addr().expect("topic embedder address");
    let app = Router::new().route("/embed", post(embed));
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}"), task)
}

/// Opens Alice's Session in the corpus's second project, whose exact scope
/// chain is the benchmark's planner shape.
async fn corpus_session(world: &World, corpus: &DenseCorpus, app: &Router, token: &str) -> String {
    let (status, session) = call(
        app,
        "POST",
        "/v1/sessions",
        token,
        Some("dense-context-session"),
        Some(json!({
            "workspace_id": world.workspace_id,
            "project_id": corpus.scopes.projects[1].0.to_string(),
            "client_name": "dense-context",
            "external_session_id": "dense-context",
            "task_summary": "ContextRun latency at corpus scale"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    session["id"].as_str().expect("session id").to_owned()
}

/// Cumulative `(sum seconds, count)` per planner stage from the in-process
/// recorder; stages are exported as summaries, so means are what a delta
/// can give.
fn planner_stages() -> HashMap<String, (f64, f64)> {
    let mut stages = HashMap::new();
    for line in metrics_handle().render().lines() {
        let Some(rest) = line.strip_prefix("synveda_context_planner_stage_duration_seconds_")
        else {
            continue;
        };
        let Some((kind, rest)) = rest.split_once('{') else {
            continue;
        };
        let Some(stage) = rest.split('"').nth(1) else {
            continue;
        };
        let Some(value) = rest
            .rsplit(' ')
            .next()
            .and_then(|value| value.parse::<f64>().ok())
        else {
            continue;
        };
        let entry = stages.entry(stage.to_owned()).or_insert((0.0, 0.0));
        match kind {
            "sum" => entry.0 += value,
            "count" => entry.1 += value,
            _ => {}
        }
    }
    stages
}

fn stage_means(before: &HashMap<String, (f64, f64)>, after: &HashMap<String, (f64, f64)>) -> Value {
    let mut means = serde_json::Map::new();
    for (stage, (sum, count)) in after {
        let (old_sum, old_count) = before.get(stage).copied().unwrap_or((0.0, 0.0));
        if count > &old_count {
            means.insert(
                stage.clone(),
                json!(((sum - old_sum) / (count - old_count) * 1_000.0 * 10.0).round() / 10.0),
            );
        }
    }
    Value::Object(means)
}

async fn timed_runs(
    app: &Router,
    token: &str,
    session: &str,
    runs: usize,
    seed: u64,
) -> (Vec<f64>, Vec<f64>, Value) {
    let topics = Geometry::new(seed).topics();
    let mut rng = Rng::new(seed);
    let (mut composed, mut searched) = (Vec::new(), Vec::new());
    let mut modes = serde_json::Map::new();
    for run in 0..runs + 5 {
        let (first, second) = topic_words(rng.below(topics));
        let query = format!("{first} {second} guidance");
        let started = Instant::now();
        let (status, body) = call(
            app,
            "POST",
            &format!("/v1/sessions/{session}/context-runs"),
            token,
            Some(&format!("dense-context-{seed}-{run}")),
            Some(json!({"query": query, "budget_tokens": 1500})),
        )
        .await;
        let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let started = Instant::now();
        let (status, found) = call(
            app,
            "GET",
            &format!("/v1/knowledge?query={first}+{second}&limit=20"),
            token,
            None,
            None,
        )
        .await;
        let search = started.elapsed().as_secs_f64() * 1_000.0;
        assert_eq!(status, StatusCode::OK, "{found}");
        if run >= 5 {
            composed.push(elapsed);
            searched.push(search);
            let mode = found["retrieval_mode"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned();
            let count = modes.get(&mode).and_then(Value::as_u64).unwrap_or(0) + 1;
            modes.insert(mode, json!(count));
        }
    }
    (composed, searched, Value::Object(modes))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "operator-run ContextRun and Knowledge search latency at corpus scale"]
async fn context_runs_and_search_at_corpus_scale() {
    let _guard = serial().await;
    let seed = setting("SYNVEDA_DENSE_PLAN_SEED", 1024);
    let runs = setting("SYNVEDA_DENSE_PLAN_CONTEXT_RUNS", 60) as usize;
    let (embedder_url, embedder) = topic_embedder(seed).await;
    let mut measured = Vec::new();
    for (index, items) in corpus_sizes().into_iter().enumerate() {
        let Some(mut world) = admitted_world().await else {
            return;
        };
        let spec = Spec {
            items,
            projects: 30,
            sourced: (items / 200).max(10),
            seed: seed.wrapping_add(index as u64 + 1),
        };
        let corpus = dense_corpus::seed(&world, &spec, &Geometry::new(seed)).await;
        analyze(&world).await;
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .expect("begin");
        configuration_support::allow_tei(&mut tx, world.tenant_id, corpus.scopes.root).await;
        tx.commit().await.expect("admit the embedding provider");
        world.state.embedder = Arc::new(synveda_ingest::embedding::AnyEmbedder::Tei(
            synveda_ingest::embedding::TeiEmbedder::new(MODEL.to_owned(), embedder_url.clone())
                .expect("stub embedder"),
        ));
        let app = router(world.state.clone());
        let token =
            Hs256Verifier::new(SECRET).issue(ALICE, world.tenant_id, Duration::from_secs(3_600));
        let session = corpus_session(&world, &corpus, &app, &token).await;
        let before = planner_stages();
        let (composed, searched, modes) =
            timed_runs(&app, &token, &session, runs, seed ^ index as u64).await;
        let stages = stage_means(&before, &planner_stages());
        eprintln!(
            "dense-plans context items={items} context_run={} knowledge_search={} modes={modes} stage_mean_ms={stages}",
            quantiles(composed.clone()),
            quantiles(searched.clone())
        );
        measured.push(json!({
            "corpus": corpus.stats,
            "context_run_ms": quantiles(composed),
            "knowledge_search_ms": quantiles(searched),
            "knowledge_search_modes": modes,
            "planner_stage_mean_ms": stages,
        }));
    }
    embedder.abort();
    if let Ok(path) = std::env::var("SYNVEDA_DENSE_PLAN_CONTEXT_REPORT") {
        let report = json!({"schema": "synveda.dense-context.v1", "seed": seed, "runs": runs, "corpora": measured});
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&report).expect("render report"),
        )
        .expect("write context report");
    }
}
