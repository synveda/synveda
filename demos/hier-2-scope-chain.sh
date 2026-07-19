#!/usr/bin/env sh
# HIER-2 acceptance demo: scope chain resolver.
# AC (docs/backlog/HIER-2.md): cache invalidation test; p99 <0.5ms warm.
# Plus the feature text: identity -> ordered scope chain (user->...->org),
# cached with invalidation on hierarchy change (ADR-0016).
#
# Flow: migrate -> admit a tenant, bind the demo admin, mint a dev token
# (CLI) -> boot the gateway -> build org + two divisions + a team, assign
# `standard` at one division -> governed requests resolve the team's chain
# through the cache (/metrics shows hits) -> move the team to the other
# division -> the very next request leaves the old division's pack behind
# (/metrics shows the invalidation) -> run the AC tests and show the
# measured warm p99. On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8134
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=hier-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit a tenant + bind the admin + mint a dev token (synveda CLI)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "hier2-demo-$(date +%s)-$$" --name "HIER-2 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)

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

api() {
  method=$1
  path=$2
  body=${3:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $token" \
      -H "Content-Type: application/json" -d "$body" \
      "http://127.0.0.1:8134$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $token" \
      "http://127.0.0.1:8134$path"
  fi
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
  curl -fsS http://127.0.0.1:8134/metrics | grep "$1" || true
}

echo "==> build the hierarchy: acme -> emea + apac; payments team under emea"
org_id=$(api POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
emea_id=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"division\",\"slug\":\"emea\",\"name\":\"EMEA\"}" | field id)
apac_id=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"division\",\"slug\":\"apac\",\"name\":\"APAC\"}" | field id)
team_id=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$emea_id\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}" | field id)

echo "==> assign 'standard' at EMEA; the team inherits it through its scope chain"
api PUT "/v1/hierarchy/nodes/$emea_id/policy" '{"name":"standard"}' >/dev/null
shown=$(api GET "/v1/hierarchy/nodes/$team_id/policy")
name=$(echo "$shown" | field name)
if [ "$name" != "standard" ]; then
  echo "demo FAILED: team pack is $name, want standard" >&2
  exit 1
fi
echo "    team runs: $name (inherited from EMEA)"

echo "==> ask again: the chain is warm — the resolver answers from memory"
api GET "/v1/hierarchy/nodes/$team_id/policy" >/dev/null
hits=$(metric 'synveda_scope_chain_resolutions_total{outcome="hit"}')
if [ -z "$hits" ]; then
  echo "demo FAILED: no scope-chain cache hits in /metrics" >&2
  exit 1
fi
echo "    $hits"

echo "==> move the team to APAC: the handler invalidates post-commit, and the"
echo "    very next request leaves EMEA's pack behind"
api PATCH "/v1/hierarchy/nodes/$team_id" "{\"parent_id\":\"$apac_id\"}" >/dev/null
shown=$(api GET "/v1/hierarchy/nodes/$team_id/policy")
name=$(echo "$shown" | field name)
origin=$(echo "$shown" | field origin)
if [ "$name" != "regulated-strict" ]; then
  echo "demo FAILED: moved team still runs $name, want regulated-strict" >&2
  exit 1
fi
echo "    team now runs: $name (origin: $origin)"
invalidations=$(metric 'synveda_scope_chain_invalidations_total')
echo "    $invalidations"

echo "==> AC: cache invalidation test + warm p99 <0.5ms (cargo test)"
# Stop the gateway first: on Windows the running exe would block cargo
# from relinking it for the integration tests (a silent lock error).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true
run_ac_test() {
  pkg=$1
  name=$2
  shift 2
  if ! out=$(cargo test -p "$pkg" --test "$name" -- "$@" 2>&1); then
    echo "$out" | tail -20
    echo "demo FAILED: $pkg --test $name" >&2
    exit 1
  fi
  echo "$out" | grep -E '^test |warm resolve|test result' | sed 's/^/    /'
}
run_ac_test synveda-store scope_chain --nocapture
run_ac_test synveda-gateway scope_chain_routes

echo ""
echo "HIER-2 scope chain resolver: acceptance criteria pass."
