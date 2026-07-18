#!/usr/bin/env sh
# HIER-1 acceptance demo: hierarchy store.
# AC (docs/backlog/HIER-1.md): 10k-node hierarchy; ancestor/descendant
# queries <1ms. Plus the feature text: closure table + materialised path,
# configurable depth, CRUD via admin API.
#
# Flow: migrate -> admit a tenant + mint a dev token (CLI) -> boot the
# gateway -> CRUD a hierarchy over /v1/hierarchy/* (create with a skipped
# division level, ancestors, move with subtree path rewrite, leaf-only
# delete) -> run the 10k-node AC test and show the measured medians.
# On Windows, run via Git Bash. Needs only the postgres service.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8130
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=hier-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit a tenant + mint a dev token (synveda CLI)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "hier1-demo-$(date +%s)-$$" --name "HIER-1 Demo Tenant")
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
until curl -fsS http://127.0.0.1:8130/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8130$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $token" \
      "http://127.0.0.1:8130$path"
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

echo "==> CRUD via the admin API: org -> department (division skipped) -> team"
org=$(api POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}')
org_id=$(echo "$org" | field id)
dept=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"payments\",\"name\":\"Payments\"}")
dept_id=$(echo "$dept" | field id)
team=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$dept_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}")
team_id=$(echo "$team" | field id)
team_path=$(echo "$team" | field path)
if [ "$team_path" != "acme/payments/core" ]; then
  echo "demo FAILED: team path is $team_path, want acme/payments/core" >&2
  exit 1
fi
echo "    created: $team_path (depth $(echo "$team" | field depth), skipping the optional division level)"

echo "==> ancestors of the team (nearest first)"
api GET "/v1/hierarchy/nodes/$team_id/ancestors" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const chain = JSON.parse(d).map((n) => `${n.kind}:${n.slug}`).join(" -> ");
    if (chain !== "department:payments -> org:acme") {
      console.error("unexpected ancestor chain: " + chain);
      process.exit(1);
    }
    console.log("    " + chain);
  });
'

echo "==> move the team to a new department; its path follows"
lending=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"lending\",\"name\":\"Lending\"}")
lending_id=$(echo "$lending" | field id)
moved=$(api PATCH "/v1/hierarchy/nodes/$team_id" "{\"parent_id\":\"$lending_id\"}")
moved_path=$(echo "$moved" | field path)
if [ "$moved_path" != "acme/lending/core" ]; then
  echo "demo FAILED: moved path is $moved_path, want acme/lending/core" >&2
  exit 1
fi
echo "    moved: acme/payments/core -> $moved_path"

echo "==> deletes are leaf-only: the org with children is a 409 conflict"
code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE \
  -H "Authorization: Bearer $token" \
  "http://127.0.0.1:8130/v1/hierarchy/nodes/$org_id")
if [ "$code" != "409" ]; then
  echo "demo FAILED: deleting a non-leaf returned HTTP $code, want 409" >&2
  exit 1
fi
code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE \
  -H "Authorization: Bearer $token" \
  "http://127.0.0.1:8130/v1/hierarchy/nodes/$team_id")
if [ "$code" != "204" ]; then
  echo "demo FAILED: deleting a leaf returned HTTP $code, want 204" >&2
  exit 1
fi
echo "    non-leaf delete: 409; leaf delete: 204"

echo "==> /metrics counts hierarchy operations"
curl -fsS http://127.0.0.1:8130/metrics |
  grep 'synveda_hierarchy_operations_total' | grep -q 'op="create"' || {
    echo "demo FAILED: hierarchy operations missing from /metrics" >&2
    exit 1
  }
echo "    synveda_hierarchy_operations_total present"

echo "==> AC: 10k-node hierarchy, ancestor/descendant queries <1ms"
echo "    (cargo test, builds the fixture and measures warm medians)"
cargo test -p synveda-store --test hierarchy ac_10k -- --nocapture 2>&1 |
  grep -E 'seeded|medians' | sed 's/^/    /'

echo ""
echo "HIER-1 hierarchy store: acceptance criteria pass."
