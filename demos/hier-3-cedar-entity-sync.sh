#!/usr/bin/env sh
# HIER-3 acceptance demo: Cedar entity sync.
# AC (docs/backlog/HIER-3.md): move a team between departments -> authz
# decisions reflect it in the same transaction boundary. Plus the feature
# text: hierarchy changes stream into the Cedar entity store
# transactionally (ADR-0017).
#
# Flow: migrate -> admit a tenant, bind the demo admin, mint dev tokens
# (CLI) -> boot the gateway -> build org + two departments + a team,
# bind a steward at dept-x -> the steward governs the team (rename OK,
# fragments warm in /metrics) -> the steward moves the team to dept-y ->
# the SAME steward request that just succeeded is 403 on the very next
# call (its authority left with the team; /metrics shows the fragment
# rebuild and the flush) -> run the AC tests. On Windows, run via Git
# Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8135
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=hier-3-demo-secret
export SYNVEDA_DEV_JWT_SECRET

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit a tenant + bind the admin + mint dev tokens (synveda CLI)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "hier3-demo-$(date +%s)-$$" --name "HIER-3 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
steward_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-steward)

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

api() {
  token=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $token" \
      -H "Content-Type: application/json" -d "$body" \
      "http://127.0.0.1:8135$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $token" \
      "http://127.0.0.1:8135$path"
  fi
}

# Like api() but prints only the status code and never fails the script
# — for requests whose denial is the point.
api_status() {
  token=$1
  method=$2
  path=$3
  body=${4:-}
  curl -s -o /dev/null -w "%{http_code}" -X "$method" \
    -H "Authorization: Bearer $token" \
    -H "Content-Type: application/json" ${body:+-d "$body"} \
    "http://127.0.0.1:8135$path"
}

field() {
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const v = JSON.parse(d)[process.argv[1]];
      console.log(typeof v === "string" ? v : JSON.stringify(v));
    });
  ' "$1"
}

metric() {
  # Anchored so HELP/TYPE lines never match.
  curl -fsS http://127.0.0.1:8135/metrics | grep "^$1" || true
}

echo "==> build the hierarchy: acme -> dept-x + dept-y; payments team under dept-x"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
dept_x_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"dept-x\",\"name\":\"Dept X\"}" | field id)
dept_y_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"dept-y\",\"name\":\"Dept Y\"}" | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$dept_x_id\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}" | field id)

echo "==> bind demo-steward as steward at dept-x (CLI bootstrap path)"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-steward --role steward --scope "$dept_x_id" >/dev/null

echo "==> the steward governs the team while it lives in dept-x"
api "$steward_token" PATCH "/v1/hierarchy/nodes/$team_id" \
  '{"name":"Payments Platform"}' >/dev/null
echo "    rename: 200 (steward of dept-x)"
# Two reads after the (flushing) rename: the first rebuilds the team's
# fragment, the second serves it warm — the hit shows in /metrics.
api "$steward_token" GET "/v1/hierarchy/nodes/$team_id" >/dev/null
api "$steward_token" GET "/v1/hierarchy/nodes/$team_id" >/dev/null
warm=$(metric 'synveda_cedar_entity_fragments_total{outcome="hit"}')
if [ -z "$warm" ]; then
  echo "demo FAILED: no entity fragment hits in /metrics" >&2
  exit 1
fi
echo "    $warm"

echo "==> the steward moves the team to dept-y — and loses it in the same"
echo "    transaction boundary: the identical next request is denied"
api "$steward_token" PATCH "/v1/hierarchy/nodes/$team_id" \
  "{\"parent_id\":\"$dept_y_id\"}" >/dev/null
status=$(api_status "$steward_token" PATCH "/v1/hierarchy/nodes/$team_id" \
  '{"name":"Payments Core"}')
if [ "$status" != "403" ]; then
  echo "demo FAILED: post-move steward rename returned $status, want 403" >&2
  exit 1
fi
echo "    rename after move: 403 (authority left with the team)"
rebuilds=$(metric 'synveda_cedar_entity_fragments_total{outcome="rebuild"}')
flushes=$(metric 'synveda_cedar_entity_flushes_total')
echo "    $rebuilds"
echo "    $flushes"

echo "==> the admin still governs everywhere: move it back"
api "$admin_token" PATCH "/v1/hierarchy/nodes/$team_id" \
  "{\"parent_id\":\"$dept_x_id\"}" >/dev/null
status=$(api_status "$steward_token" PATCH "/v1/hierarchy/nodes/$team_id" \
  '{"name":"Payments"}')
if [ "$status" != "200" ]; then
  echo "demo FAILED: steward rename after move-back returned $status, want 200" >&2
  exit 1
fi
echo "    steward rename after move-back: 200"

echo "==> AC: decision-flip tests through the facade and the HTTP surface"
# Stop the gateway first: on Windows the running exe would block cargo
# from relinking it for the integration tests (a silent lock error).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true
run_ac_test() {
  if ! out=$(cargo test -p "$1" --test "$2" 2>&1); then
    echo "$out" | tail -20
    echo "demo FAILED: $1 --test $2" >&2
    exit 1
  fi
  echo "$out" | grep -E '^test |test result' | sed 's/^/    /'
}
run_ac_test synveda-policy entity_sync
run_ac_test synveda-gateway cedar_entity_sync

echo ""
echo "HIER-3 Cedar entity sync: acceptance criteria pass."
