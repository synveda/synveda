#!/usr/bin/env sh
# AUD-1 acceptance demo: hash-chained audit log.
# AC (docs/backlog/AUD-1.md): tamper test — mutating any historic row
# breaks chain verification. Plus the feature text: append-only,
# BLAKE3-chained per tenant; every authz decision, policy change, and
# admin action recorded.
#
# Flow: migrate -> admit a tenant (the break-glass chains tenant.created)
# -> bootstrap an admin binding (chained as role.bound, actor
# break_glass) -> the admin builds hierarchy over the API (each mutation
# chains its event with the deciding pack in the payload) -> a read
# chains its allowed decision, a roleless subject's attempt chains the
# denial -> `synveda audit tail` shows the chain, `synveda audit verify`
# walks it -> the append-only trigger refuses direct UPDATE even for the
# superuser -> THE AC: an attacker with database credentials suppresses
# triggers and rewrites a historic row — verification names the broken
# sequence -> the tamper test suite runs the same attack per column.
# On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8134
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=aud-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET

cargo build -p synveda-gateway -p synveda-cli

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

echo "==> migrate + admit a tenant (the break-glass audits itself: tenant.created)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "aud1-demo-$(date +%s)-$$" --name "AUD-1 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
nobody_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-nobody)

echo "==> bootstrap the admin (chained as role.bound, actor kind break_glass)"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8134/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8134$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8134$path"
  fi
}

# code <token> <method> <path> [body] — prints the HTTP code only.
code() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -s -o /dev/null -w '%{http_code}' -X "$method" \
      -H "Authorization: Bearer $tok" -H "Content-Type: application/json" \
      -d "$body" "http://127.0.0.1:8134$path"
  else
    curl -s -o /dev/null -w '%{http_code}' -X "$method" \
      -H "Authorization: Bearer $tok" "http://127.0.0.1:8134$path"
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

echo "==> admin actions chain their events (decision context in the payload)"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" |
  field id)
echo "    org=$org_id team=$team_id (2 x hierarchy.node.created)"

echo "==> a read chains its allowed decision; a roleless subject chains a denial"
api "$admin_token" GET "/v1/hierarchy/nodes/$team_id" >/dev/null
c=$(code "$nobody_token" DELETE "/v1/hierarchy/nodes/$team_id")
[ "$c" = "403" ] || {
  echo "demo FAILED: the roleless subject must be denied, got $c" >&2
  exit 1
}
echo "    GET -> authz.decision/allow; denied DELETE -> authz.decision/deny (403)"

echo "==> the chain, newest first (actors, actions, hashes)"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 10

echo "==> verification walks the whole chain"
./target/debug/synveda audit verify --tenant "$tenant_id"

echo "==> append-only: even the superuser cannot UPDATE past the trigger"
if psql_c "update audit_log set resource = 'rewritten'" 2>/tmp/aud1-guard.$$; then
  echo "demo FAILED: the append-only trigger must reject UPDATE" >&2
  exit 1
fi
grep -q "append-only" /tmp/aud1-guard.$$ || {
  echo "demo FAILED: expected the append-only trigger message" >&2
  cat /tmp/aud1-guard.$$ >&2
  exit 1
}
echo "    UPDATE rejected: audit_log is append-only (AUD-1, ADR-0019)"

echo "==> THE AC: a database-credentialed attacker suppresses triggers and"
echo "    rewrites history — verification breaks at the mutated row"
psql_c "begin;
        set local session_replication_role = replica;
        update audit_log
           set payload = '{\"note\":\"scrubbed\"}'::jsonb
         where tenant_id = '$tenant_id' and seq = 3;
        commit;" >/dev/null
if ./target/debug/synveda audit verify --tenant "$tenant_id"; then
  echo "demo FAILED: verification must break after tampering" >&2
  exit 1
fi
echo "    tampering detected (non-zero exit, broken seq named above)"

echo "==> audit metrics on /metrics"
curl -fsS http://127.0.0.1:8134/metrics | grep -E '^synveda_audit_events_total' | head -5

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo "==> the tamper suite runs the same attack per hashed column"
cargo test -p synveda-audit --test tamper

echo
echo "AUD-1 demo PASSED: every admin action, decision, and denial chained;"
echo "append-only enforced in-schema; tampering with any historic row breaks"
echo "chain verification at the named sequence."
