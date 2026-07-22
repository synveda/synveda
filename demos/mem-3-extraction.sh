#!/usr/bin/env sh
# MEM-3 acceptance demo: extraction pipeline (ADR-0022).
# AC (docs/backlog/MEM-3.md): extraction precision measured on a labelled
# fixture set >= target (see EVAL-2); every record carries provenance
# (session, method, model version, confidence).
#
# Flow: migrate -> admit a tenant -> org/team over the API -> alice is
# registered as a service identity at the team (her personal leaf is her
# write target) -> she observes a batch of 3 events (a decision, a tool
# result, a transcript delta) -> the embedded extraction worker (a PGMQ-
# polling task spawned by the gateway, ADR-0022) picks the signals up
# asynchronously and commits 3 derived records, each carrying the AC
# quadruple in its provenance -> a 4th event carrying a seeded AWS key
# QUARANTINES under the strict default pack (no work signal, no exposure)
# -> a security-reviewer (its own role, ADR-0021 decision 6) releases it
# -> the worker commits a 4th record whose content carries the redaction
# placeholder, never the raw key -> the queue drains, extraction metrics
# show on /metrics, and the audit chain (memory.extracted) verifies -> the
# AC test suites run (fixture precision, extractor parsing, queue
# semantics, end-to-end worker path). On Windows, run via Git Bash. Needs
# only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8137
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=mem-3-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# The zero-config, zero-network extractor (ADR-0022 decision 3) — keeps
# this demo self-contained; see the note near the end for live Claude/vLLM.
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
# Tight poll pacing so the worker's async pickup is visible quickly.
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS

# The seeded finding — vendor documentation example, never a real
# credential (the same fixture MEM-2's demo and AC tests use).
SEEDED_KEY="AKIAIOSFODNN7EXAMPLE"

cargo build -p synveda-gateway -p synveda-cli

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "mem3-demo-$$" --name "MEM-3 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
reviewer_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-reviewer)

echo "==> bootstrap bindings: org-admin for the admin, security-reviewer"
echo "    for the reviewer (its first action here: releasing a MEM-3 quarantine)"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-reviewer --role security-reviewer >/dev/null

echo "==> purging leftover observe-queue signals from other runs (shared queue)"
purged=$(psql_t "select pgmq.purge_queue('observe')")
echo "    purged=$purged"

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8137/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8137$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8137$path"
  fi
}

# code <token> <method> <path> [body] — prints the HTTP code only.
code() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  curl -s -o /dev/null -w '%{http_code}' -X "$method" \
    -H "Authorization: Bearer $tok" -H "Content-Type: application/json" \
    ${body:+-d "$body"} "http://127.0.0.1:8137$path"
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

# wait_for_records <n> — polls up to 30 * 0.5s for the worker (async,
# ADR-0022) to commit the expected number of records for this tenant.
wait_for_records() {
  want=$1
  tries=0
  while :; do
    have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
    [ "$have" = "$want" ] && return 0
    tries=$((tries + 1))
    if [ "$tries" -ge 30 ]; then
      echo "demo FAILED: expected $want records, stuck at $have after $tries tries" >&2
      exit 1
    fi
    sleep 0.5
  done
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
  {\"idempotency_key\":\"d1\",\"kind\":\"decision\",
   \"payload\":{\"text\":\"Chose PGMQ over Kafka for the observe buffer.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"d2\",\"kind\":\"tool_result\",
   \"payload\":{\"output\":\"cargo test: 12 passed, 0 failed.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"d3\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"We always use small single-purpose commits.\"},\"occurred_at\":\"$now\"}]}"

echo "==> alice observes a batch of 3 events (enqueue-only ack, 202)"
first=$(api "$alice_token" POST /v1/observe "$batch")
[ "$(echo "$first" | field accepted)" = "3" ] || {
  echo "demo FAILED: expected 3 accepted, got: $first" >&2
  exit 1
}
echo "    accepted=3 (staged under RLS; a content-free signal queued per event)"

echo "==> the embedded extraction worker picks the signals up asynchronously"
echo "    (SYNVEDA_EXTRACTION_POLL_MS=300) and commits derived records"
wait_for_records 3
echo "    records=3"

echo "==> the committed records — class, sensitivity, and content"
psql_c "select class, sensitivity, content from records
        where tenant_id = '$tenant_id' order by valid_from;"

echo "==> one record's provenance — the AC quadruple (session, method,"
echo "    model_version, confidence), plus traceability into the staging"
echo "    event and its finding summary"
psql_t "select jsonb_pretty(provenance) from records
        where tenant_id = '$tenant_id' order by valid_from limit 1" |
  sed 's/^/      /'
session=$(psql_t "select provenance->>'session_id' from records
                  where tenant_id = '$tenant_id' order by valid_from limit 1")
method=$(psql_t "select provenance->>'method' from records
                 where tenant_id = '$tenant_id' order by valid_from limit 1")
model_version=$(psql_t "select provenance->>'model_version' from records
                        where tenant_id = '$tenant_id' order by valid_from limit 1")
confidence=$(psql_t "select provenance->>'confidence' from records
                     where tenant_id = '$tenant_id' order by valid_from limit 1")
echo "    session=$session method=$method model_version=$model_version confidence=$confidence"

echo "==> a 4th event carries a seeded AWS key: the strict default pack"
echo "    (regulated-strict, zero-config) quarantines it — redacted"
echo "    staging, NO work signal, nothing extracted until reviewed"
secret_batch="{\"session_id\":\"demo-session\",\"events\":[
  {\"idempotency_key\":\"d4\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"here are my creds: $SEEDED_KEY please remember\"},\"occurred_at\":\"$now\"}]}"
fourth=$(api "$alice_token" POST /v1/observe "$secret_batch")
status4=$(echo "$fourth" | field events | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d)[0].status));
')
[ "$status4" = "quarantined" ] || {
  echo "demo FAILED: expected the secret event to quarantine, got: $fourth" >&2
  exit 1
}
q_event=$(echo "$fourth" | field events | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d)[0].event_id));
')
echo "    status=$status4 event=$q_event"

echo "==> the security-reviewer releases it (mem-2's release flow): the"
echo "    standard work signal goes out, and only now can it be extracted"
released=$(api "$reviewer_token" POST "/v1/quarantine/$q_event/release" \
  '{"reason":"vendor docs example key; safe to extract"}')
[ "$(echo "$released" | field state)" = "released" ] || {
  echo "demo FAILED: expected released, got: $released" >&2
  exit 1
}
echo "    released"

echo "==> the worker picks up the release signal and commits the 4th record"
wait_for_records 4
# Select by provenance: every record names its staging event (ADR-0022),
# so the released event's record is addressable without ordering games.
redacted_content=$(psql_t "select content from records
                           where tenant_id = '$tenant_id'
                             and provenance->>'event_id' = '$q_event'")
case "$redacted_content" in
*"[REDACTED:aws-access-key-id]"*) : ;;
*)
  echo "demo FAILED: expected the redaction placeholder, got: $redacted_content" >&2
  exit 1
  ;;
esac
case "$redacted_content" in
*"$SEEDED_KEY"*)
  echo "demo FAILED: the raw key leaked into a record!" >&2
  exit 1
  ;;
*) : ;;
esac
echo "    records=4, content: $redacted_content"

echo "==> the observe queue is drained"
remaining=$(psql_t "select count(*) from pgmq.q_observe")
[ "$remaining" = "0" ] || {
  echo "demo FAILED: expected the queue empty, got $remaining remaining" >&2
  exit 1
}
echo "    pgmq.q_observe=0"

echo "==> extraction metrics on /metrics"
curl -fsS http://127.0.0.1:8137/metrics | grep -E '^synveda_extraction_' | head -12

echo "==> the audit trail: memory.extracted (aggregated, ADR-0022 decision"
echo "    5) chained alongside the batch and release events; the chain verifies"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 12
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> to run the same pipeline against live Claude or vLLM: set"
echo "    SYNVEDA_EXTRACTOR=claude plus ANTHROPIC_API_KEY on the gateway"
echo "    (or SYNVEDA_EXTRACTOR=vllm plus SYNVEDA_VLLM_BASE_URL and"
echo "    SYNVEDA_EXTRACTOR_MODEL) — the extractor seam is env-selected,"
echo "    the rest of this flow is unchanged. To measure fixture"
echo "    precision directly against a live model instead:"
echo "      SYNVEDA_EXTRACTOR=claude ANTHROPIC_API_KEY=... \\"
echo "        cargo test -p synveda-ingest --test extraction_precision -- --ignored --nocapture"
echo "      SYNVEDA_EXTRACTOR=vllm SYNVEDA_VLLM_BASE_URL=http://... SYNVEDA_EXTRACTOR_MODEL=... \\"
echo "        cargo test -p synveda-ingest --test extraction_precision -- --ignored --nocapture"

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo
echo "==> the AC test suites (fixture precision, extractor parsing, queue"
echo "    semantics, and the end-to-end worker path)"
cargo test -p synveda-ingest --test extraction_precision -- --nocapture
cargo test -p synveda-ingest --test extractor_http
cargo test -p synveda-store --test observe_queue
cargo test -p synveda-gateway --test extraction

echo
echo "MEM-3 demo PASSED: observe events flowed through the embedded"
echo "extraction worker into bitemporal records with full provenance —"
echo "every record carried the AC quadruple (session, method,"
echo "model_version, confidence); commits held exactly-once under the"
echo "archive lock; a quarantined secret reached no record until a"
echo "security-reviewer released it, and even then only its"
echo "[REDACTED:*] placeholder landed, never the raw key; the observe"
echo "queue drained; and the fixture precision, extractor, queue, and"
echo "end-to-end AC suites passed."
