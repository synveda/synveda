#!/usr/bin/env sh
# MEM-1 acceptance demo: observe API + PGMQ buffer (ADR-0020).
# AC (docs/backlog/MEM-1.md): load test 1k events/s on dev hardware;
# duplicate delivery does not duplicate memories. Plus the feature text:
# batched transcript/event ingestion, ack <20ms, idempotency keys.
#
# Flow: migrate -> admit a tenant -> admin builds org/team over the API
# -> a service identity is registered at the team (its personal leaf is
# where its observations land) -> the agent posts a batch (202, one
# chained memory.observed event) -> redelivers the SAME batch (202, all
# duplicates, nothing new staged or enqueued — THE idempotency AC) -> a
# mixed batch admits only the new key -> the staging table and the
# content-free queue signals are shown -> subjects with no placement are
# denied (roles never grant writes; placement does) -> the app role
# cannot rewrite what was observed -> audit tail + verify -> the test
# suite runs, with the load AC (1k events/s, ack budget) in --release.
# On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8135
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=mem-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET

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
  --slug "mem1-demo-$(date +%s)-$$" --name "MEM-1 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
nobody_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-nobody)

echo "==> bootstrap the admin binding (break-glass, chained as role.bound)"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8135/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8135$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8135$path"
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
    ${body:+-d "$body"} "http://127.0.0.1:8135$path"
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

echo "==> the admin builds the hierarchy over the API"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" |
  field id)
echo "    org=$org_id team=$team_id"

echo "==> register an agent at the team (its personal leaf is its write target)"
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject demo-agent --scope "$team_id" >/dev/null
agent_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-agent)

now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
batch="{\"session_id\":\"demo-session-1\",\"events\":[
  {\"idempotency_key\":\"d1\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"user asked about the retry policy\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"d2\",\"kind\":\"tool_result\",
   \"payload\":{\"tool\":\"grep\",\"summary\":\"3 call sites\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"d3\",\"kind\":\"decision\",
   \"payload\":{\"decision\":\"retries use exponential backoff\"},\"occurred_at\":\"$now\"}]}"

echo "==> the agent observes a batch (enqueue-only ack, 202)"
first=$(api "$agent_token" POST /v1/observe "$batch")
accepted=$(echo "$first" | field accepted)
[ "$accepted" = "3" ] || {
  echo "demo FAILED: expected 3 accepted, got: $first" >&2
  exit 1
}
echo "    accepted=3 duplicates=0"

echo "==> THE AC: the same delivery again — acked, all duplicates, nothing new"
retry=$(api "$agent_token" POST /v1/observe "$batch")
r_accepted=$(echo "$retry" | field accepted)
r_duplicates=$(echo "$retry" | field duplicates)
[ "$r_accepted" = "0" ] && [ "$r_duplicates" = "3" ] || {
  echo "demo FAILED: redelivery must duplicate nothing, got: $retry" >&2
  exit 1
}
first_id=$(echo "$first" | field events | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d)[0].event_id));
')
retry_id=$(echo "$retry" | field events | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d)[0].event_id));
')
[ "$first_id" = "$retry_id" ] || {
  echo "demo FAILED: a retry must ack with the winning delivery's ids" >&2
  exit 1
}
echo "    accepted=0 duplicates=3, ids identical to the first delivery"

echo "==> a mixed batch admits only the genuinely new key"
mixed="{\"session_id\":\"demo-session-2\",\"events\":[
  {\"idempotency_key\":\"d3\",\"kind\":\"decision\",
   \"payload\":{\"decision\":\"replayed\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"d4\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"a fresh delta\"},\"occurred_at\":\"$now\"}]}"
m=$(api "$agent_token" POST /v1/observe "$mixed")
[ "$(echo "$m" | field accepted)" = "1" ] || {
  echo "demo FAILED: expected exactly 1 admitted, got: $m" >&2
  exit 1
}
echo "    accepted=1 duplicates=1"

echo "==> the buffer: 4 staged rows, 4 content-free queue signals"
staged=$(psql_t "select count(*) from observe_events where tenant_id = '$tenant_id'")
signals=$(psql_t "select count(*) from pgmq.q_observe where message->>'tenant_id' = '$tenant_id'")
[ "$staged" = "4" ] && [ "$signals" = "4" ] || {
  echo "demo FAILED: expected 4 staged + 4 signals, got $staged/$signals" >&2
  exit 1
}
echo "    a queue signal carries ids only (content stays under RLS):"
psql_t "select message from pgmq.q_observe where message->>'tenant_id' = '$tenant_id' limit 1" |
  sed 's/^/      /'

echo "==> no placement, no write: the org-admin and a nobody are both denied"
c=$(code "$admin_token" POST /v1/observe "$batch")
[ "$c" = "403" ] || {
  echo "demo FAILED: an unplaced org-admin must not observe, got $c" >&2
  exit 1
}
c=$(code "$nobody_token" POST /v1/observe "$batch")
[ "$c" = "403" ] || {
  echo "demo FAILED: an unplaced subject must not observe, got $c" >&2
  exit 1
}
echo "    both 403: roles never grant writes; placement does (ADR-0020)"

echo "==> the app role cannot rewrite what was observed (SELECT+INSERT only)"
if psql_c "begin;
           set local role synveda_app;
           update observe_events set payload = '{}'::jsonb;
           commit;" 2>/tmp/mem1-guard.$$; then
  echo "demo FAILED: the app role must not hold UPDATE on observe_events" >&2
  exit 1
fi
grep -q "permission denied" /tmp/mem1-guard.$$ || {
  echo "demo FAILED: expected a permission denial" >&2
  cat /tmp/mem1-guard.$$ >&2
  exit 1
}
echo "    UPDATE rejected: staging rows are provenance (ADR-0020 decision 1)"

echo "==> each batch chained one memory.observed event; the chain verifies"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 5
./target/debug/synveda audit verify --tenant "$tenant_id"

echo "==> observe metrics on /metrics"
curl -fsS http://127.0.0.1:8135/metrics | grep -E '^synveda_observe' | head -5

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo "==> the load AC runs in --release (1k events/s sustained, ack p99"
echo "    inside the 20ms budget plus the measured dev-database link tax)"
cargo test --release -p synveda-gateway --test observe -- --nocapture

echo
echo "MEM-1 demo PASSED: batched ingestion acked enqueue-only; duplicate"
echo "delivery admitted nothing twice (buffer-level idempotency); content"
echo "staged under RLS with content-free PGMQ work signals; unplaced"
echo "subjects fail closed; every batch audit-chained; 1k events/s held."
