#!/usr/bin/env sh
# AUTHZ-2 acceptance demo: policy packs.
# AC (docs/backlog/AUTHZ-2.md): switching a team's pack changes inject
# composition in the next session; golden tests per pack. Plus the feature
# text: regulated-strict / standard / open-collaboration as versioned
# Cedar bundles applied per node; inheritance with override rules.
#
# Flow: migrate -> admit a tenant + mint a dev token (CLI) -> boot the
# gateway (1s pack refresh) -> list the embedded product packs -> the
# zero-config default is regulated-strict -> assign `standard` at a
# department; its teams inherit it (origin shows the department) -> store
# a restrictive custom pack, assign it at one team: the very next request
# on that team is governed by it (403 naming pack@version) while the
# sibling team keeps `standard` -> remove the assignment (self-rescue:
# decided under the inherited pack, ADR-0014 decision 4) -> /metrics show
# the policy operations -> run the golden tests per pack, including the
# composition-switch AC at the MemoryRead seam. On Windows, run via Git
# Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8132
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=authz-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# Fast hot reload so the stored custom pack compiles in without a wait.
SYNVEDA_POLICY_REFRESH_SECS=1
export SYNVEDA_POLICY_REFRESH_SECS

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit a tenant + mint a dev token (synveda CLI)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "authz2-demo-$(date +%s)-$$" --name "AUTHZ-2 Demo Tenant")
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
until curl -fsS http://127.0.0.1:8132/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8132$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $token" \
      "http://127.0.0.1:8132$path"
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

rename_code() {
  # PATCH-renames node $1; prints the HTTP code, body to a temp file.
  curl -s -o /tmp/authz2-body.$$ -w '%{http_code}' -X PATCH \
    -H "Authorization: Bearer $token" -H "Content-Type: application/json" \
    -d "{\"name\":\"Renamed $2\"}" \
    "http://127.0.0.1:8132/v1/hierarchy/nodes/$1"
}

echo "==> build the hierarchy: acme -> eng -> team-a / team-b"
org_id=$(api POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_a=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"team-a\",\"name\":\"Team A\"}" |
  field id)
team_b=$(api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"team-b\",\"name\":\"Team B\"}" |
  field id)
echo "    org=$org_id eng=$eng_id team-a=$team_a team-b=$team_b"

echo "==> the versioned product packs, embedded in the binary"
api GET /v1/policy/packs | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    for (const p of JSON.parse(d).packs)
      console.log(`    ${p.name}@${p.version} (${p.kind})`);
  });
'

echo "==> zero-config: the tenant default is regulated-strict (seed 2.1)"
effective=$(api GET /v1/policy/default | field effective)
[ "$effective" = "regulated-strict" ] || {
  echo "demo FAILED: expected regulated-strict default, got $effective" >&2
  exit 1
}
echo "    effective default: $effective"

echo "==> assign 'standard' at the department; teams inherit it per node"
api PUT "/v1/hierarchy/nodes/$eng_id/policy" '{"name":"standard"}' >/dev/null
pack=$(api GET "/v1/hierarchy/nodes/$team_a/policy")
name=$(echo "$pack" | field name)
origin=$(echo "$pack" | field origin kind)
origin_scope=$(echo "$pack" | field origin scope_id)
[ "$name" = "standard" ] && [ "$origin" = "assigned" ] && [ "$origin_scope" = "$eng_id" ] || {
  echo "demo FAILED: team-a must inherit standard from eng, got $pack" >&2
  exit 1
}
echo "    team-a runs standard@1, inherited from the department node"

echo "==> store a restrictive custom pack; the gateway hot-reloads it"
pack_file=$(mktemp)
cat >"$pack_file" <<'CEDAR'
permit (
    principal,
    action == Synveda::Action::"HierarchyRead",
    resource
) when { resource in principal.tenant };
CEDAR
./target/debug/synveda policy apply --tenant "$tenant_id" --name demo-frozen "$pack_file"
rm -f "$pack_file"
tries=0
until api GET /v1/policy/packs | grep -q '"demo-frozen"'; do
  tries=$((tries + 1))
  if [ "$tries" -ge 15 ]; then
    echo "demo FAILED: stored pack did not appear in the listing" >&2
    exit 1
  fi
  sleep 1
done
echo "    demo-frozen@1 stored and listed"

echo "==> the AC: switch team-b's pack; the very next request is governed by it"
api PUT "/v1/hierarchy/nodes/$team_b/policy" '{"name":"demo-frozen"}' >/dev/null
code=$(rename_code "$team_b" frozen)
[ "$code" = "403" ] || {
  echo "demo FAILED: team-b mutation must 403 under demo-frozen, got $code" >&2
  exit 1
}
grep -q 'demo-frozen@1' /tmp/authz2-body.$$ || {
  echo "demo FAILED: denial must name pack@version: $(cat /tmp/authz2-body.$$)" >&2
  exit 1
}
echo "    team-b mutation: 403, $(field reason </tmp/authz2-body.$$)"
code=$(rename_code "$team_a" sibling)
[ "$code" = "200" ] || {
  echo "demo FAILED: team-a must keep standard, got $code" >&2
  exit 1
}
echo "    team-a mutation: 200 — the sibling keeps its own pack"

echo "==> override rules: removing the assignment is decided by the *inherited* pack"
code=$(curl -s -o /tmp/authz2-body.$$ -w '%{http_code}' -X DELETE \
  -H "Authorization: Bearer $token" \
  "http://127.0.0.1:8132/v1/hierarchy/nodes/$team_b/policy")
[ "$code" = "204" ] || {
  echo "demo FAILED: unassign must succeed under the inherited pack, got $code" >&2
  exit 1
}
code=$(rename_code "$team_b" thawed)
[ "$code" = "200" ] || {
  echo "demo FAILED: team-b must thaw after unassign, got $code" >&2
  exit 1
}
echo "    unassigned (a frozen pack cannot seal its own node); team-b mutates again"
rm -f /tmp/authz2-body.$$

echo "==> /metrics: policy operations and per-pack decisions counted"
metrics=$(curl -fsS http://127.0.0.1:8132/metrics)
echo "$metrics" | grep 'synveda_policy_operations_total' | grep -q 'op="assign_node_policy"' || {
  echo "demo FAILED: assign ops missing from /metrics" >&2
  exit 1
}
echo "$metrics" | grep 'synveda_authz_decisions_total' | grep -q 'pack="demo-frozen"' || {
  echo "demo FAILED: per-pack decisions missing from /metrics" >&2
  exit 1
}
echo "    synveda_policy_operations_total and per-pack synveda_authz_decisions_total present"

echo "==> AC: golden tests per pack (incl. the composition switch at the MemoryRead seam)"
cargo test -p synveda-policy --test packs 2>&1 | grep -E '^test |test result' | sed 's/^/    /'

echo ""
echo "AUTHZ-2 policy packs: acceptance criteria pass."
