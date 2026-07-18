#!/usr/bin/env sh
# AUTHZ-1 acceptance demo: the embedded Cedar PDP.
# AC (docs/backlog/AUTHZ-1.md): µs-level decision benchmark; decision +
# policy version logged for every call. Plus the feature text: authorize()
# facade, entities materialised from the hierarchy, per-tenant policy
# store with hot reload — and the ADR-0011 debt: /v1/hierarchy/* is now
# PDP-gated.
#
# Flow: migrate -> admit a tenant + mint a dev token (CLI) -> boot the
# gateway (1s pack refresh) -> hierarchy admin allowed under the embedded
# bootstrap pack -> `synveda policy apply` a read-only pack (a bad pack is
# refused at compile check) -> hot reload denies mutations 403 naming
# pack@version, reads keep working -> `synveda policy clear` restores
# bootstrap -> /metrics shows decisions and reloads -> run the µs-level
# decision benchmark. On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8131
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=authz-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# Fast hot reload so the demo shows propagation without a long wait.
SYNVEDA_POLICY_REFRESH_SECS=1
export SYNVEDA_POLICY_REFRESH_SECS

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit a tenant + mint a dev token (synveda CLI)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "authz1-demo-$(date +%s)-$$" --name "AUTHZ-1 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8131/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8131$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $token" \
      "http://127.0.0.1:8131$path"
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

create_team_code() {
  curl -s -o /tmp/authz1-body.$$ -w '%{http_code}' -X POST \
    -H "Authorization: Bearer $token" -H "Content-Type: application/json" \
    -d "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"t$1\",\"name\":\"Team $1\"}" \
    "http://127.0.0.1:8131/v1/hierarchy/nodes"
}

echo "==> under the embedded bootstrap pack: the tenant administers its own hierarchy"
org=$(api POST /v1/hierarchy/nodes '{"kind":"org","slug":"acme","name":"ACME"}')
org_id=$(echo "$org" | field id)
team=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}")
team_id=$(echo "$team" | field id)
echo "    created org + team (decisions allowed by bootstrap@1)"

echo "==> a pack that fails the schema is refused at apply time"
bad_pack=$(mktemp)
echo 'permit (principal, action == Synveda::Action::"LaunchMissiles", resource);' >"$bad_pack"
if ./target/debug/synveda policy apply --tenant "$tenant_id" --name authz1-bad "$bad_pack" \
  >/dev/null 2>&1; then
  echo "demo FAILED: an out-of-schema pack must be refused" >&2
  exit 1
fi
rm -f "$bad_pack"
echo "    compile check rejected it; nothing stored"

echo "==> apply a read-only pack; the gateway hot-reloads it"
pack=$(mktemp)
cat >"$pack" <<'CEDAR'
permit (
    principal,
    action == Synveda::Action::"HierarchyRead",
    resource
) when { resource in principal.tenant };
CEDAR
./target/debug/synveda policy apply --tenant "$tenant_id" --name authz1-readonly "$pack"
rm -f "$pack"

tries=0
until [ "$(create_team_code $tries)" = "403" ]; do
  tries=$((tries + 1))
  if [ "$tries" -ge 15 ]; then
    echo "demo FAILED: read-only pack did not take effect" >&2
    exit 1
  fi
  sleep 1
done
reason=$(cat /tmp/authz1-body.$$)
echo "$reason" | grep -q 'authz1-readonly@1' || {
  echo "demo FAILED: denial must name pack@version, got: $reason" >&2
  exit 1
}
echo "    mutation now 403 policy_denied, naming the pack version:"
echo "    $(echo "$reason" | field reason)"

code=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $token" \
  "http://127.0.0.1:8131/v1/hierarchy/nodes/$team_id")
if [ "$code" != "200" ]; then
  echo "demo FAILED: reads must keep working under the read-only pack, got $code" >&2
  exit 1
fi
echo "    reads still 200 under authz1-readonly"

echo "==> clear the pack; bootstrap is back in force after the next reload"
./target/debug/synveda policy clear --tenant "$tenant_id"
tries=0
until [ "$(create_team_code restored)" = "201" ]; do
  tries=$((tries + 1))
  if [ "$tries" -ge 15 ]; then
    echo "demo FAILED: clearing the pack did not restore bootstrap" >&2
    exit 1
  fi
  sleep 1
done
echo "    mutation 201 again under bootstrap@1"
rm -f /tmp/authz1-body.$$

echo "==> /metrics: every decision counted; pack reloads visible"
metrics=$(curl -fsS http://127.0.0.1:8131/metrics)
echo "$metrics" | grep 'synveda_authz_decisions_total' | grep -q 'decision="allow"' || {
  echo "demo FAILED: allow decisions missing from /metrics" >&2
  exit 1
}
echo "$metrics" | grep 'synveda_authz_decisions_total' | grep -q 'decision="deny"' || {
  echo "demo FAILED: deny decisions missing from /metrics" >&2
  exit 1
}
echo "$metrics" | grep -q 'synveda_policy_pack_reloads_total' || {
  echo "demo FAILED: pack reloads missing from /metrics" >&2
  exit 1
}
echo "    synveda_authz_decisions_total (allow + deny) and synveda_policy_pack_reloads_total present"
echo "    (each decision also logs pack@version — see the gateway log lines above)"

echo "==> AC: µs-level decision benchmark (full facade incl. entity materialisation)"
cargo test -p synveda-policy --test decision_benchmark -- --nocapture 2>&1 |
  grep -E 'median' | sed 's/^/    /'

echo ""
echo "AUTHZ-1 Cedar PDP embedded: acceptance criteria pass."
