#!/usr/bin/env sh
# CTX-1 acceptance demo: hybrid retrieval (ADR-0024).
# AC (docs/backlog/CTX-1.md): retrieval quality on the fixture set; NO
# LLM calls on the read path; p99 <80ms at 1M records/tenant (the 1M
# half runs separately — see the end of this script).
#
# Flow: postgres + TEI (BGE-M3) up -> migrate -> tenant, hierarchy,
# alice at the team -> she observes 4 events -> the pipeline extracts,
# embeds through the real TEI, and commits records with 1024-d vectors
# -> the gateway's search indexer converges the per-tenant Tantivy
# sidecar (watched on /metrics; the index directory and its watermark
# are shown on disk) -> the live-TEI quality harness runs the fixture
# set through the full hybrid engine (BM25 + pgvector ANN + RRF under
# the mandatory scope/sensitivity pushdown) and prints per-query
# recall -> the AC suites run (fusion/no-leak/staleness, the quality
# fixture with synthetic geometry, the PDP-derived predicate).
# On Windows, run via Git Bash. Needs postgres and tei; TEI's first
# start downloads ~2.3 GB into the tei-cache volume.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres
docker compose -f deploy/compose/docker-compose.yml up --detach tei

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8139
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=ctx-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
# The real TEI from the dev compose — the product path (tech plan §1.3):
# the pipeline embeds records AND the read path embeds queries with the
# same model. An embedding model, not an LLM — the read path makes no
# LLM calls, structurally (ADR-0024 decision 7).
SYNVEDA_EMBEDDER=tei
export SYNVEDA_EMBEDDER
SYNVEDA_TEI_URL=http://localhost:8110
export SYNVEDA_TEI_URL
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
# A fresh sidecar root per run; fast sweeps so convergence is visible
# in seconds.
SYNVEDA_SEARCH_INDEX_DIR="./data/ctx1-demo-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
SYNVEDA_SEARCH_POLL_MS=300
export SYNVEDA_SEARCH_POLL_MS

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

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "ctx1-demo-$$" --name "CTX-1 Demo Tenant")
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
trap 'kill "$GATEWAY_PID" 2>/dev/null || true; rm -rf "$SYNVEDA_SEARCH_INDEX_DIR"' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8139/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8139$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8139$path"
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
   \"payload\":{\"text\":\"Chose pgvector HNSW with iterative scans for the dense retrieval leg.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e2\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"Tantivy keeps BM25 corpus statistics per tenant in its own directory.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e3\",\"kind\":\"tool_result\",
   \"payload\":{\"output\":\"cargo test: retrieval suites green, fusion beats both legs.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e4\",\"kind\":\"decision\",
   \"payload\":{\"text\":\"Reciprocal rank fusion with k sixty decides the final ordering.\"},\"occurred_at\":\"$now\"}]}"

echo "==> alice observes 4 events; the pipeline extracts, embeds via TEI"
first=$(api "$alice_token" POST /v1/observe "$batch")
[ "$(echo "$first" | field accepted)" = "4" ] || {
  echo "demo FAILED: expected 4 accepted, got: $first" >&2
  exit 1
}
tries=0
while :; do
  have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
  [ "$have" = "4" ] && break
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "demo FAILED: expected 4 records, stuck at $have" >&2
    exit 1
  fi
  sleep 0.5
done
echo "    records=4, all with 1024-d BGE-M3 vectors:"
psql_c "select r.class, e.model, e.dim, left(r.content, 48) as content
        from records r join record_embeddings e on e.record_id = r.id
        where r.tenant_id = '$tenant_id' order by r.content;"

echo "==> the search indexer converges the per-tenant Tantivy sidecar"
echo "    (a fresh index root backfills EVERY tenant in the dev database"
echo "    on its first pass — searches degrade to dense-only until a"
echo "    tenant's turn comes; the demo tenant is the newest, so last)"
tries=0
until [ -f "$SYNVEDA_SEARCH_INDEX_DIR/$tenant_id.state.json" ]; do
  tries=$((tries + 1))
  if [ "$tries" -ge 360 ]; then
    echo "demo FAILED: the sidecar never swept the demo tenant" >&2
    exit 1
  fi
  sleep 0.5
done
echo "    per-tenant index directory (deleting it IS the rebuild procedure):"
ls "$SYNVEDA_SEARCH_INDEX_DIR/$tenant_id" | head -6 | sed 's/^/      /'
echo "    watermark state: $(cat "$SYNVEDA_SEARCH_INDEX_DIR/$tenant_id.state.json")"
echo "    sidecar metrics:"
curl -fsS http://127.0.0.1:8139/metrics |
  grep -E '^synveda_search_index_docs_total' | sed 's/^/      /' || true

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo
echo "==> the live-model quality harness: the fixture set through the full"
echo "    hybrid engine — BM25 + real BGE-M3 ANN + RRF, under the mandatory"
echo "    scope/sensitivity pushdown. Sparse-only plateaus at 0.5 by fixture"
echo "    design; fusion must recover the paraphrase half:"
cargo test -p synveda-gateway --test retrieval_live -- --ignored --nocapture

echo
echo "==> the AC suites (fusion order, no-leak filters, one-sided staleness,"
echo "    degradation modes, indexer watermark/rebuild, synthetic-geometry"
echo "    quality, and the PDP-derived predicate)"
cargo test -p synveda-retrieval --test hybrid
cargo test -p synveda-retrieval --test quality
cargo test -p synveda-retrieval --test permitted_scopes
cargo test -p synveda-retrieval --lib

echo
echo "CTX-1 demo PASSED: observe-ingested records became retrievable through"
echo "the hybrid engine — pgvector ANN (HNSW, iterative scans) fused with a"
echo "per-tenant Tantivy BM25 sidecar by reciprocal rank, every query bounded"
echo "by a PDP-derived scope set and a sensitivity ceiling pushed into both"
echo "legs and re-verified at hydration (a lagging sidecar can only miss,"
echo "never leak); the read path made zero LLM calls — the only model in"
echo "play is the embedding server, and the retrieval crate cannot reach the"
echo "network at all. The 1M-record latency AC runs separately (minutes):"
echo "  cargo test -p synveda-retrieval --test latency -- --ignored --nocapture"
