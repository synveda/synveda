#!/usr/bin/env sh
# MEM-4 acceptance demo: transactional embed-or-fail (ADR-0023).
# AC (docs/backlog/MEM-4.md): chaos test kills TEI mid-batch; zero lost
# or embedding-less records.
#
# Flow: postgres + TEI (BGE-M3) up -> migrate -> tenant, hierarchy, alice
# -> she observes 3 events -> the worker extracts, embeds through the
# real TEI, and commits each record ATOMICALLY with its vector (one
# statement; migration 0015's deferred trigger refuses anything less) ->
# CHAOS: TEI is stopped -> 2 more events -> extraction succeeds but
# embedding cannot, so the signals redeliver and NO record commits —
# never a vector-less record, never a silent drop (the documented Mem0
# failure mode) -> TEI restarts -> the stragglers drain, every record
# model-tagged and embedded -> the audit chain (memory.extracted, now
# carrying the embedder identity) verifies -> the AC suites run (the
# mock-TEI chaos test, the embedder HTTP contract, the store suites).
# On Windows, run via Git Bash. Needs postgres and tei; TEI's first
# start downloads ~2.3 GB into the tei-cache volume.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres
docker compose -f deploy/compose/docker-compose.yml up --detach tei

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
# `sqlx::query!` expands against DATABASE_URL at compile time, and the
# database named above can still be empty at this point: a crate that needs
# a rebuild here type-checks against a schema that does not exist yet and
# fails with `relation "audit_chain_heads" does not exist` rather than with
# anything about this demo. It is invisible whenever the workspace happens
# to be built already. The checked-in `.sqlx` cache is the answer to
# "compile without a database", and it is what `make ci` and
# scripts/db-test.sh use for the same reason.
SQLX_OFFLINE=true
export SQLX_OFFLINE
SYNVEDA_LISTEN_ADDR=127.0.0.1:8138
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=mem-4-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
# The real TEI from the dev compose — the product path (tech plan §1.3).
SYNVEDA_EMBEDDER=tei
export SYNVEDA_EMBEDDER
SYNVEDA_TEI_URL=http://localhost:8110
export SYNVEDA_TEI_URL
# Tight pacing so the chaos is visible in seconds: fast polls, a short
# visibility timeout (failed embeds redeliver quickly), and a dead-letter
# threshold far above what the outage can consume — in production the
# threshold bounds poison messages, and an outage that outlives it is
# re-driven by a break-glass signal re-send (ADR-0022 decision 6).
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
SYNVEDA_EXTRACTION_VT_SECS=3
export SYNVEDA_EXTRACTION_VT_SECS
SYNVEDA_EXTRACTION_MAX_READS=1000
export SYNVEDA_EXTRACTION_MAX_READS

cargo build -p synveda-gateway -p synveda-cli

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

echo "==> waiting for TEI on $SYNVEDA_TEI_URL (first start downloads BGE-M3, ~2.3 GB)"
tries=0
until curl -fsS "$SYNVEDA_TEI_URL/info" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 120 ]; then
    echo "demo FAILED: TEI did not become healthy (model download stalled?)" >&2
    exit 1
  fi
  sleep 5
done
echo "    tei model: $(curl -fsS "$SYNVEDA_TEI_URL/info" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).model_id));
')"

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "mem4-demo-$$" --name "MEM-4 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

echo "==> purging leftover observe-queue signals from other runs (shared queue)"
purged=$(psql_t "select pgmq.purge_queue('observe')")
echo "    purged=$purged"

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8138/healthz >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

# api <token> <method> <path> [body]
api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" \
      "http://127.0.0.1:8138$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8138$path"
  fi
}

field() {
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      let v = JSON.parse(d);
      for (const k of process.argv.slice(1)) v = v[k];
      console.log(typeof v === "string" ? v : JSON.stringify(v));
    });
  ' "$@"
}

# wait_for_records <n> — polls for the worker's async commits.
wait_for_records() {
  want=$1
  tries=0
  while :; do
    have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
    [ "$have" = "$want" ] && return 0
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      echo "demo FAILED: expected $want records, stuck at $have after $tries tries" >&2
      exit 1
    fi
    sleep 0.5
  done
}

# The AC's core invariant, asserted repeatedly: a record without an
# embedding row must never exist, in any phase.
assert_none_embedding_less() {
  orphans=$(psql_t "select count(*) from records r
                    left join record_embeddings e on e.record_id = r.id
                    where r.tenant_id = '$tenant_id' and e.record_id is null")
  [ "$orphans" = "0" ] || {
    echo "demo FAILED: $orphans embedding-less record(s) — the Mem0 failure mode!" >&2
    exit 1
  }
}

echo "==> the admin builds the hierarchy; alice is registered at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject alice --scope "$team_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
echo "    org=$org_id team=$team_id alice=alice"

now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
batch="{\"session_id\":\"demo-session\",\"events\":[
  {\"idempotency_key\":\"e1\",\"kind\":\"decision\",
   \"payload\":{\"text\":\"Chose embed-or-fail over async embedding.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e2\",\"kind\":\"tool_result\",
   \"payload\":{\"output\":\"cargo test: 14 passed, 0 failed.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e3\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"We keep vectors transactional with their records.\"},\"occurred_at\":\"$now\"}]}"

echo "==> alice observes 3 events; the worker extracts, embeds via TEI,"
echo "    and commits record + vector in ONE statement per record"
first=$(api "$alice_token" POST /v1/observe "$batch")
[ "$(echo "$first" | field accepted)" = "3" ] || {
  echo "demo FAILED: expected 3 accepted, got: $first" >&2
  exit 1
}
wait_for_records 3
assert_none_embedding_less
echo "    records=3, embedding-less=0"

echo "==> each record's vector, model-tagged (BGE-M3 dense is 1024-d)"
psql_c "select r.class, e.model, e.dim, left(r.content, 44) as content
        from records r join record_embeddings e on e.record_id = r.id
        where r.tenant_id = '$tenant_id' order by r.class;"

echo "==> CHAOS: stopping TEI mid-pipeline"
docker compose -f deploy/compose/docker-compose.yml stop tei >/dev/null

chaos_batch="{\"session_id\":\"demo-session\",\"events\":[
  {\"idempotency_key\":\"e4\",\"kind\":\"decision\",
   \"payload\":{\"text\":\"Decided during the TEI outage.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e5\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"Also observed while embeddings are down.\"},\"occurred_at\":\"$now\"}]}"
echo "==> alice observes 2 more events into the outage (ack is unaffected:"
echo "    observe never blocks the session — the queue is the buffer)"
chaos=$(api "$alice_token" POST /v1/observe "$chaos_batch")
[ "$(echo "$chaos" | field accepted)" = "2" ] || {
  echo "demo FAILED: expected 2 accepted, got: $chaos" >&2
  exit 1
}

echo "==> the worker retries; NOTHING commits while TEI is down"
sleep 5
have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
[ "$have" = "3" ] || {
  echo "demo FAILED: a record committed without TEI ($have)!" >&2
  exit 1
}
assert_none_embedding_less
inflight=$(psql_t "select count(*) from pgmq.q_observe
                   where message->>'tenant_id' = '$tenant_id'")
echo "    records still 3, embedding-less=0, signals redelivering: $inflight"
echo "==> embedder failures on /metrics"
curl -fsS http://127.0.0.1:8138/metrics |
  grep -E '^synveda_embedder_requests_total' || true

echo "==> TEI returns; the stragglers drain"
docker compose -f deploy/compose/docker-compose.yml start tei >/dev/null
tries=0
until curl -fsS "$SYNVEDA_TEI_URL/info" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "demo FAILED: TEI did not come back" >&2
    exit 1
  fi
  sleep 2
done
wait_for_records 5
assert_none_embedding_less
remaining=$(psql_t "select count(*) from pgmq.q_observe
                    where message->>'tenant_id' = '$tenant_id'")
[ "$remaining" = "0" ] || {
  echo "demo FAILED: expected the tenant's signals drained, got $remaining" >&2
  exit 1
}
echo "    records=5, embedding-less=0, queue drained: ZERO LOST, ZERO VECTOR-LESS"

echo "==> the audit trail: memory.extracted now names the embedder; the"
echo "    chain verifies"
psql_c "select outcome, payload->>'embedder' as embedder,
               payload->>'embedding_model' as model,
               jsonb_array_length(payload->'events') as events
        from audit_log
        where tenant_id = '$tenant_id' and action = 'memory.extracted'
        order by seq;"
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> the deterministic embedder (SYNVEDA_EMBEDDER=deterministic,"
echo "    the zero-config default) runs this same flow with no TEI at"
echo "    all — hash vectors, honest placeholders, never a retrieval"
echo "    substrate (ADR-0023 decision 6)."

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo
echo "==> the AC test suites (the mock-TEI chaos test, the embedder HTTP"
echo "    contract, and the store suites the embedding column touched)"
cargo test -p synveda-gateway --test embedding
cargo test -p synveda-ingest --test embedder_http
cargo test -p synveda-ingest --lib
cargo test -p synveda-store --test bitemporal
cargo test -p synveda-store --test rls

echo
echo "MEM-4 demo PASSED: records committed atomically with their BGE-M3"
echo "vectors through the real TEI; killing TEI mid-pipeline stopped"
echo "commits entirely — extraction retried, the ack path kept accepting,"
echo "and not one record existed without its embedding at any point; on"
echo "recovery every event landed model-tagged and the queue drained —"
echo "zero lost, zero embedding-less (the documented Mem0 failure mode is"
echo "unrepresentable: the store API writes record + vector in one"
echo "statement and migration 0015's deferred trigger refuses anything"
echo "less, even from raw SQL); the audit chain records the embedder"
echo "identity per commit group and verifies."
