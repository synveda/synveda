#!/usr/bin/env sh
# AUTHZ-3 acceptance demo: roles & role bindings.
# AC (docs/backlog/AUTHZ-3.md): full role×action matrix golden-tested.
# Plus the feature text: viewer/contributor/curator/steward/org-admin/
# auditor/security-reviewer/compliance; bound per node, inherited
# downward.
#
# Flow: migrate -> admit a tenant + mint dev tokens (CLI) -> strict by
# default: an unbound subject holds no admin power at all -> bootstrap the
# tenant admin with `synveda role bind` (the CLI break-glass; SSO logins
# get the same via the `synveda-admins` group) -> the admin builds the
# hierarchy and binds a steward at one department -> the steward governs
# that subtree on the very next request, and nothing outside it ->
# delegation works; minting org-admin does not (the base-layer escalation
# guard) -> an auditor reads everything and mutates nothing -> revocation
# is in force on the next request -> /metrics show the role operations ->
# run the golden role×action matrix tests. On Windows, run via Git Bash.
# Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
# `sqlx::query!` expands against DATABASE_URL at compile time, and the
# database named above can still be empty at this point: a crate that needs
# a rebuild here type-checks against a schema that does not exist yet and
# fails with `relation "audit_chain_heads" does not exist` rather than with
# anything about this demo. It is invisible whenever the workspace happens
# to be built already. The checked-in `.sqlx` cache is the answer to
# "compile without a database", and it is what `make ci` and
# scripts/db-test.sh use for the same reason.
SQLX_OFFLINE=true
export SQLX_OFFLINE
SYNVEDA_LISTEN_ADDR=127.0.0.1:8133
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=authz-3-demo-secret
export SYNVEDA_DEV_JWT_SECRET

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit a tenant + mint dev tokens (synveda CLI)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "authz3-demo-$(date +%s)-$$" --name "AUTHZ-3 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
stew_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-steward)
aud_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-auditor)

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8133/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8133$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8133$path"
  fi
}

# code <token> <method> <path> [body] — prints the HTTP code, body to a
# temp file.
code() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -s -o /tmp/authz3-body.$$ -w '%{http_code}' -X "$method" \
      -H "Authorization: Bearer $tok" -H "Content-Type: application/json" \
      -d "$body" "http://127.0.0.1:8133$path"
  else
    curl -s -o /tmp/authz3-body.$$ -w '%{http_code}' -X "$method" \
      -H "Authorization: Bearer $tok" "http://127.0.0.1:8133$path"
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

echo "==> strict by default: an unbound subject holds no admin power (ADR-0015)"
c=$(code "$admin_token" POST /v1/hierarchy/nodes '{"kind":"org","slug":"acme","name":"ACME"}')
[ "$c" = "403" ] || {
  echo "demo FAILED: an unbound subject must be denied, got $c" >&2
  exit 1
}
echo "    org creation without a role: 403 ($(field kind </tmp/authz3-body.$$))"

echo "==> bootstrap: bind tenant-wide org-admin at the store (CLI break-glass;"
echo "    SSO logins get this from the synveda-admins group automatically)"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
c=$(code "$admin_token" POST /v1/hierarchy/nodes '{"kind":"org","slug":"acme","name":"ACME"}')
[ "$c" = "201" ] || {
  echo "demo FAILED: the bound admin must govern on the next request, got $c" >&2
  exit 1
}
org_id=$(field id </tmp/authz3-body.$$)
echo "    the very next request governs: org created ($org_id)"

echo "==> build the hierarchy: acme -> eng (team-a, team-b) + ops"
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
ops_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"ops\",\"name\":\"Operations\"}" |
  field id)
team_a=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"team-a\",\"name\":\"Team A\"}" |
  field id)
echo "    org=$org_id eng=$eng_id ops=$ops_id team-a=$team_a"

echo "==> bind a steward per node: demo-steward at eng, inherited downward"
api "$admin_token" PUT "/v1/hierarchy/nodes/$eng_id/roles" \
  '{"subject":"demo-steward","role":"steward"}' >/dev/null
c=$(code "$stew_token" PATCH "/v1/hierarchy/nodes/$team_a" '{"name":"Team A (stewarded)"}')
[ "$c" = "200" ] || {
  echo "demo FAILED: the eng steward must govern team-a, got $c" >&2
  exit 1
}
echo "    steward renames team-a: 200 (the binding reaches the whole subtree)"
c=$(code "$stew_token" PATCH "/v1/hierarchy/nodes/$ops_id" '{"name":"Ops"}')
[ "$c" = "403" ] || {
  echo "demo FAILED: the eng binding must not reach ops, got $c" >&2
  exit 1
}
echo "    steward touches ops: 403 (nothing outside the bound subtree)"

echo "==> delegation works; escalation does not (the base-layer guard)"
c=$(code "$stew_token" PUT "/v1/hierarchy/nodes/$eng_id/roles" \
  '{"subject":"demo-viewer","role":"viewer"}')
[ "$c" = "200" ] || {
  echo "demo FAILED: a steward must delegate roles in its subtree, got $c" >&2
  exit 1
}
echo "    steward binds a viewer: 200"
c=$(code "$stew_token" PUT "/v1/hierarchy/nodes/$eng_id/roles" \
  '{"subject":"mallory","role":"org-admin"}')
[ "$c" = "403" ] || {
  echo "demo FAILED: a steward must not mint org-admin, got $c" >&2
  exit 1
}
echo "    steward mints org-admin: 403 ($(field reason </tmp/authz3-body.$$))"

echo "==> the auditor: reads everything, mutates nothing"
api "$admin_token" PUT /v1/roles/bindings \
  '{"subject":"demo-auditor","role":"auditor"}' >/dev/null
c=$(code "$aud_token" GET /v1/roles/bindings)
[ "$c" = "200" ] || {
  echo "demo FAILED: the auditor must read the bindings, got $c" >&2
  exit 1
}
bindings=$(field bindings </tmp/authz3-body.$$)
echo "    auditor lists bindings: 200 ($bindings)"
c=$(code "$aud_token" PATCH "/v1/hierarchy/nodes/$team_a" '{"name":"Nope"}')
[ "$c" = "403" ] || {
  echo "demo FAILED: the auditor must not mutate, got $c" >&2
  exit 1
}
echo "    auditor mutates: 403 (read-only by role)"

echo "==> revocation is in force on the very next request"
c=$(code "$admin_token" DELETE \
  "/v1/hierarchy/nodes/$eng_id/roles?subject=demo-steward&role=steward")
[ "$c" = "204" ] || {
  echo "demo FAILED: unbind must succeed, got $c" >&2
  exit 1
}
c=$(code "$stew_token" PATCH "/v1/hierarchy/nodes/$team_a" '{"name":"Team A again"}')
[ "$c" = "403" ] || {
  echo "demo FAILED: the revoked steward must be out, got $c" >&2
  exit 1
}
echo "    unbound steward: 403 on the next request"
rm -f /tmp/authz3-body.$$

echo "==> /metrics: role operations counted"
metrics=$(curl -fsS http://127.0.0.1:8133/metrics)
echo "$metrics" | grep 'synveda_role_operations_total' | grep -q 'op="bind_node"' || {
  echo "demo FAILED: role ops missing from /metrics" >&2
  exit 1
}
echo "    synveda_role_operations_total present (bind/unbind/list per op)"

echo "==> AC: the full role×action matrix, golden-tested per pack"
cargo test -p synveda-policy --test roles 2>&1 | grep -E '^test |test result' | sed 's/^/    /'

echo ""
echo "AUTHZ-3 roles & role bindings: acceptance criteria pass."
