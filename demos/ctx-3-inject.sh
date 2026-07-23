#!/usr/bin/env sh
# CTX-3 acceptance demo: the inject API (ADR-0026).
# AC (docs/backlog/CTX-3.md): session-start contract, warm-cache latency
# SLO (asserted by the suite at the end), graceful degradation — partial
# context + warning header rather than failure.
#
# Flow: postgres + TEI up -> migrate -> tenant; org/eng/core hierarchy;
# alice registered at the team -> alice observes 3 events and the
# pipeline extracts + embeds them through REAL TEI (BGE-M3) -> known
# team material is seeded through the store contract (pinned = the
# published stand-in; a relevant and an irrelevant derived record) ->
# alice injects with a task over the full product path (identity ->
# HIER-2 chain -> PDP plan -> query embed via the MEM-4 seam -> hybrid
# -> compose): the block is watermarked, relevance keeps the irrelevant
# derived record out, and ONE `context.injected` event chains -> TEI is
# STOPPED mid-demo: the same inject degrades to sparse-only — 200, the
# X-Synveda-Degraded header, the same relevant material — never a
# failure -> TEI restarts and the next inject is undegraded (recovery)
# -> an unplaced subject receives the empty block (200 — policy is a
# result, not an error) -> the audit tail shows one context.injected
# per inject and the chain verifies -> the AC suites run, including the
# 1k-session latency AC.
# On Windows, run via Git Bash. Needs the dev compose.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres tei

# A scratch database for this run: the long-lived dev database carries
# thousands of leftover test tenants, and the sidecar indexer sweeps
# every active tenant per cycle — minutes of sparse-leg lag before it
# reaches a tenant admitted just now (recorded in STATUS as the CTX-1
# LISTEN/NOTIFY-or-dirty-filter trigger's dev evidence). The demo's
# product path stays whole — background indexer included — on a
# database of its own.
DEMO_DB=ctx3_demo_$$
docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "create database $DEMO_DB" >/dev/null
# The extensions the compose initdb gives the main database
# (deploy/compose/postgres/initdb/01-extensions.sql) — per-database in
# Postgres, so the scratch database needs its own.
docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$DEMO_DB" -c \
  "create extension if not exists vector;
   create extension if not exists age;
   create extension if not exists pgmq" >/dev/null
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DEMO_DB"
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8142
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=ctx-3-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# Deterministic extractor (extraction quality is MEM-3's demo); REAL TEI
# embedder — pipeline vectors and the inject route's query embedding go
# through the same seam and model (ADR-0026 decision 3).
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=tei
export SYNVEDA_EMBEDDER
SYNVEDA_TEI_URL=http://localhost:8110
export SYNVEDA_TEI_URL
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
SYNVEDA_SEARCH_POLL_MS=300
export SYNVEDA_SEARCH_POLL_MS
SYNVEDA_SEARCH_INDEX_DIR="./data/ctx3-demo-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$DEMO_DB" -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "ctx3-demo-$$" --name "CTX-3 Demo Tenant")
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
cleanup() {
  kill "$GATEWAY_PID" 2>/dev/null || true
  wait "$GATEWAY_PID" 2>/dev/null || true
  docker compose -f deploy/compose/docker-compose.yml start tei >/dev/null 2>&1 || true
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -U synveda -d synveda \
    -c "drop database if exists $DEMO_DB with (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR"
}
trap cleanup EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8142/healthz >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

# api <token> <method> <path> [body] — response body to stdout, response
# headers to /tmp/ctx3-headers.txt (the degradation header assertions).
api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -D /tmp/ctx3-headers.txt -X "$method" \
      -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" \
      "http://127.0.0.1:8142$path"
  else
    curl -fsS -D /tmp/ctx3-headers.txt -X "$method" \
      -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8142$path"
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

echo "==> the admin builds org/eng/core; alice is registered at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject alice --scope "$team_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
echo "    org=$org_id eng=$eng_id team=$team_id alice=alice"

now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
batch="{\"session_id\":\"demo-session\",\"events\":[
  {\"idempotency_key\":\"e1\",\"kind\":\"decision\",
   \"payload\":{\"text\":\"Chose blue-green rollouts for the core services.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e2\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"Alice prefers small focused pull requests.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e3\",\"kind\":\"tool_result\",
   \"payload\":{\"output\":\"cargo test: inject suites green.\"},\"occurred_at\":\"$now\"}]}"

echo "==> alice observes 3 events; the pipeline extracts + embeds them"
echo "    through REAL TEI at her home scope (placement decides)"
first=$(api "$alice_token" POST /v1/observe "$batch")
[ "$(echo "$first" | field accepted)" = "3" ] || {
  echo "demo FAILED: expected 3 accepted, got: $first" >&2
  exit 1
}
tries=0
while :; do
  have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
  [ "$have" = "3" ] && break
  tries=$((tries + 1))
  if [ "$tries" -ge 120 ]; then
    echo "demo FAILED: expected 3 records, stuck at $have" >&2
    exit 1
  fi
  sleep 0.5
done

echo "==> seeding known team material through the store contract"
echo "    (pinned = the published stand-in; one task-relevant and one"
echo "    task-irrelevant derived record — the relevance fixture)"
alice_identity=$(psql_t "select id from identities
                         where tenant_id = '$tenant_id' and subject = 'alice'")
vec="[0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1]"
# seed <scope-id> <kind> <class> <content> — the hash@1 vector keeps the
# embed-or-fail constraint honest; the sparse leg indexes content
# regardless of vector model, which is what this demo's ranking uses.
seed() {
  psql_t "with new_record as (
            insert into records
              (id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, tx_from)
            values (gen_random_uuid(), '$tenant_id', '$1', '$alice_identity',
                    '$2', '$3', '$4', 'internal',
                    '{\"source\": \"ctx-3 demo seed\"}', now(), now())
            returning id, tenant_id)
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          select id, tenant_id, 'hash@1', 16, '$vec'::vector from new_record" >/dev/null
}
seed "$team_id" pinned procedure "Deploys go through make deploy; never push directly."
seed "$team_id" derived fact "The release train leaves on fridays."
seed "$team_id" derived fact "Postgres vacuum maintenance runs weekly."

task="when does the release train leave"
# Every inject chains one event; the audit assertion at the end compares
# against this count (retries included — one event per inject, exactly).
inject_count=0

echo
echo "==> inject #1: the full product path (plan -> TEI query embed ->"
echo "    hybrid -> compose -> one chained audit event); retried until"
echo "    the sidecar sweep has covered this tenant (the CTX-1 lag"
echo "    contract: a cold sidecar can miss, never fail — and a dev"
echo "    database full of leftover test tenants makes a full sweep"
echo "    cycle slow; the dense leg is live from the first call)"
tries=0
while :; do
  block=$(api "$alice_token" POST /v1/inject \
    "{\"task\":\"$task\",\"session_id\":\"demo-session\"}")
  inject_count=$((inject_count + 1))
  if echo "$block" | field text | grep -q "release train leaves"; then
    break
  fi
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "demo FAILED: the sidecar never surfaced the seeded record: $block" >&2
    exit 1
  fi
  sleep 1
done
echo "$block" | field text
[ "$(echo "$block" | field degraded)" = "[]" ] || {
  echo "demo FAILED: the full path must not be degraded: $block" >&2
  exit 1
}
if grep -qi "x-synveda-degraded" /tmp/ctx3-headers.txt; then
  echo "demo FAILED: no degradation header expected on the full path" >&2
  exit 1
fi
echo "$block" | field text | grep -q "make deploy" || {
  echo "demo FAILED: pinned material must compose regardless of the task" >&2
  exit 1
}
if echo "$block" | field text | grep -q "vacuum maintenance"; then
  echo "demo FAILED: the irrelevant derived record must stay out (relevance)" >&2
  exit 1
fi
hash1=$(echo "$block" | field block_hash)
echo "$block" | field text | grep -q "$hash1" || {
  echo "demo FAILED: the block hash must ride the watermark line" >&2
  exit 1
}
echo "    relevance held (release-train in, vacuum out), watermark blake3=$hash1"

echo
echo "==> TEI goes DOWN mid-demo; the same inject DEGRADES, never fails"
docker compose -f deploy/compose/docker-compose.yml stop tei >/dev/null
degraded_block=$(api "$alice_token" POST /v1/inject \
  "{\"task\":\"$task\",\"session_id\":\"demo-session-degraded\"}")
inject_count=$((inject_count + 1))
[ "$(echo "$degraded_block" | field degraded)" = '["embedder"]' ] || {
  echo "demo FAILED: expected the embedder degradation: $degraded_block" >&2
  exit 1
}
grep -qi "^x-synveda-degraded: embedder" /tmp/ctx3-headers.txt || {
  echo "demo FAILED: the X-Synveda-Degraded warning header must be set" >&2
  exit 1
}
echo "$degraded_block" | field text | grep -q "release train leaves" || {
  echo "demo FAILED: the sparse leg must still rank the relevant record" >&2
  exit 1
}
echo "    degraded to sparse-only: 200, X-Synveda-Degraded: embedder,"
echo "    the relevant material still composed — partial context, no failure"

echo
echo "==> TEI returns; the very next inject is whole again"
docker compose -f deploy/compose/docker-compose.yml up --detach --wait tei >/dev/null
# Warm TEI's model before timing the gateway's 100ms embed deadline.
tries=0
until curl -fsS -X POST -H 'Content-Type: application/json' \
  -d '{"inputs":"warmup"}' "$SYNVEDA_TEI_URL/embed" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "demo FAILED: TEI did not come back" >&2
    exit 1
  fi
  sleep 1
done
recovered=$(api "$alice_token" POST /v1/inject \
  "{\"task\":\"$task\",\"session_id\":\"demo-session-recovered\"}")
inject_count=$((inject_count + 1))
[ "$(echo "$recovered" | field degraded)" = "[]" ] || {
  echo "demo FAILED: recovery inject still degraded: $recovered" >&2
  exit 1
}
echo "    recovered: the dense leg is back without a restart"

echo
echo "==> an unplaced subject receives the EMPTY block (200 — policy is"
echo "    a result, not an error; the surface is not a placement oracle)"
ghost_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject ghost)
empty=$(api "$ghost_token" POST /v1/inject '{"task":"anything"}')
inject_count=$((inject_count + 1))
[ "$(echo "$empty" | field record_ids)" = "[]" ] || {
  echo "demo FAILED: an unplaced subject must compose nothing: $empty" >&2
  exit 1
}
echo "    empty block, tokens=$(echo "$empty" | field tokens)"

echo
echo "==> the audit tail: one context.injected per inject (watermark +"
echo "    aggregated decisions + degradations, task as hash only), and"
echo "    the chain verifies"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 6
injected=$(psql_t "select count(*) from audit_log
                   where tenant_id = '$tenant_id' and action = 'context.injected'")
[ "$injected" = "$inject_count" ] || {
  echo "demo FAILED: $inject_count injects but $injected context.injected events" >&2
  exit 1
}
leaked=$(psql_t "select count(*) from audit_log
                 where tenant_id = '$tenant_id' and action = 'context.injected'
                   and payload::text ilike '%release train%'")
[ "$leaked" = "0" ] || {
  echo "demo FAILED: task text leaked into an audit payload" >&2
  exit 1
}
./target/debug/synveda audit verify --tenant "$tenant_id"

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo
echo "==> the AC suites: the contract + degradation matrix, then the"
echo "    latency AC (1,000 sessions at 50/s, median asserted under the"
echo "    150ms budget, tails + stage split + the chain-lock saturation"
echo "    ceiling reported — the ADR-0019 option 2 trigger's evidence)"
cargo test -p synveda-gateway --test inject
cargo test -p synveda-gateway --test inject_latency -- --ignored --nocapture

echo
echo "CTX-3 demo PASSED: /v1/inject serves token-budgeted, watermarked,"
echo "relevance-ranked context over the full governed path (identity ->"
echo "HIER-2 chain -> one PDP MemoryRead per scope -> MEM-4 query embed ->"
echo "CTX-1 hybrid -> CTX-2 compose), chains exactly one context.injected"
echo "event per inject (decisions aggregated, task as hash only), and"
echo "degrades instead of failing: TEI down means sparse-only + the"
echo "X-Synveda-Degraded header, a broken sidecar means unranked compose"
echo "(suite), an unplaced or quarantined caller means the empty block —"
echo "and the latency AC holds its median under 150ms at 1k sessions."
