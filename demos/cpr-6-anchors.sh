#!/usr/bin/env bash
# CPR-6 — authorisation over governed scope anchors (ADR-0073).
#
# The integration suites cover the resolver, the packs and the HTTP surface.
# What they cannot show is the sentence the feature exists for: **a grant now
# decides.** Before this, a workspace `owner` grant was a governed record of
# authority and what actually let somebody administer a workspace was a role
# binding on the hierarchy CPR-7 has since deleted. That is a claim about a
# running gateway, a real database and real tokens, so it is demonstrated
# against all three or it is not demonstrated at all.
#
# Every call below succeeds because of a grant, or is refused. There is no
# other way left to succeed.
#
# What it asserts, in order:
#
#   1. `/v1/me` mints the caller's **own scope** and serves it as their first
#      anchor — before anything else exists.
#   2. One grant at the tenant root, and the founder can create a workspace and
#      a project — with no second grant written anywhere on the way.
#   3. `/v1/me` forecasts what the founder may do **at each anchor**, from real
#      decisions: `workspace.update` true at the workspace they own.
#   4. A **project-only** grant reaches the project and refuses the workspace
#      above it — every verb.
#   5. Revoking that grant is refused on the **very next request**. Nothing ran.
#   6. A **group** grant reaches its members, and emptying the group takes the
#      access with it.
#   7. **Nobody reaches into somebody else's own scope**: the founder holds the
#      tenant root — the widest thing this model can express — and the
#      stranger's own scope is not an anchor of theirs and not readable.
#   8. The audit chain records each act against the thing it decided about —
#      a workspace, a project, a grant — and **verifies**.
#
# Usage: demos/cpr-6-anchors.sh
#   KEEP_DB=1      keep the scratch database on the way out
#
# Cost: one scratch database, one port, no network. Under two minutes.
set -euo pipefail

cd "$(dirname "$0")/.."

# Compiling with the checked-in `.sqlx` cache, so a demo needs no database to
# build — ten-2-rls.sh's reasoning, and the same line.
SQLX_OFFLINE=true
export SQLX_OFFLINE

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DB="synveda_cpr6_demo_$$"
URL="postgres://synveda:synveda-dev@localhost:5432/${DB}"
WORK="$(mktemp -d)"
# Not 8120, 8131 or 8132: a contributor running this may have a deployment or
# another demo on those ports.
PORT=8133
GATEWAY_URL="http://127.0.0.1:${PORT}"
GATEWAY_PID=""
FOUNDER="cpr6-founder"
CONTRACTOR="cpr6-contractor"
STRANGER="cpr6-stranger"

psql_admin() { $COMPOSE exec -T postgres psql -U synveda -d postgres -qtAX -v ON_ERROR_STOP=1 "$@"; }
psql_db() { $COMPOSE exec -T postgres psql -U synveda -d "$DB" -qtAX -v ON_ERROR_STOP=1 "$@"; }

cleanup() {
    [ -n "$GATEWAY_PID" ] && kill "$GATEWAY_PID" 2>/dev/null || true
    if [ "${KEEP_DB:-0}" = "1" ]; then
        echo "keeping ${URL}"
    else
        psql_admin -c "drop database if exists ${DB} with (force)" >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

step() { printf '\n\033[1m== %s\033[0m\n' "$1"; }
ok() { printf '   \033[32mok\033[0m  %s\n' "$1"; }
fail() { printf '   \033[31mFAIL\033[0m %s\n' "$1"; exit 1; }

# One API call as $TOKEN. Prints "<status>\n<body>"; callers split them.
api() {
    local method="$1" path="$2" key="${3:-}" body="${4:-}"
    local args=(-sS -o "$WORK/body" -w '%{http_code}'
                -X "$method" "${GATEWAY_URL}${path}"
                -H "authorization: Bearer ${TOKEN}")
    [ -n "$key" ] && args+=(-H "idempotency-key: ${key}")
    if [ -n "$body" ]; then
        args+=(-H 'content-type: application/json' -d "$body")
    fi
    STATUS="$(curl "${args[@]}")"
    BODY="$(cat "$WORK/body")"
}

# A field out of the last response body, without a jq dependency. The path
# separator is `/` because half the keys worth reading here are action names —
# `workspace.update` — which already contain a dot.
field() { python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
for k in sys.argv[1].split("/"):
    d = d[int(k)] if isinstance(d, list) else d.get(k)
    if d is None:
        break
print("" if d is None else d)' "$1" <<<"$BODY"; }

# One anchor out of a `/v1/me` body, by scope id; then a field inside it.
anchor() { python3 -c 'import json,sys
me = json.loads(sys.stdin.read())
for a in me["anchors"]:
    if a["scope_id"] == sys.argv[1]:
        d = a
        for k in sys.argv[2].split("/"):
            d = d[int(k)] if isinstance(d, list) else d.get(k)
            if d is None:
                break
        print("" if d is None else d)
        break
else:
    print("<absent>")' "$@" <<<"$BODY"; }

anchor_count() { python3 -c 'import json,sys;print(len(json.loads(sys.stdin.read())["anchors"]))' <<<"$BODY"; }

$COMPOSE ps postgres >/dev/null 2>&1 || { echo "run \`make dev-up\` first"; exit 1; }

step "Building"
cargo build -q -p synveda-cli -p synveda-gateway
BIN="./target/debug/synveda"
GATEWAY="./target/debug/synveda-gateway"

psql_admin -c "create database ${DB}" >/dev/null
psql_db -c "create extension if not exists vector; create extension if not exists pgmq;" >/dev/null

export DATABASE_URL="$URL"
export SYNVEDA_LISTEN_ADDR="127.0.0.1:${PORT}"
export SYNVEDA_PUBLIC_URL="$GATEWAY_URL"
export SYNVEDA_SEARCH_INDEX_DIR="$WORK/search-index"
# The dev HS256 mode (ADR-0008): this demo is about the decision point, and a
# real IdP would be a second thing to get right.
export SYNVEDA_DEV_JWT_SECRET="cpr6-demo-secret"
SYNVEDA_KMS_KEY="$("$BIN" kms keygen 2>/dev/null)"
export SYNVEDA_KMS_KEY

"$BIN" db migrate >/dev/null 2>&1 || fail "migrating an empty database"
TENANT_SLUG="cpr6-demo-$$"
"$BIN" tenant create --slug "$TENANT_SLUG" --name 'CPR-6 demo' >"$WORK/tenant.json"
TENANT_ID="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' <"$WORK/tenant.json")"
[ -n "$TENANT_ID" ] || fail "admitting the tenant produced no id"

FOUNDER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$FOUNDER")"
CONTRACTOR_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$CONTRACTOR")"
STRANGER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$STRANGER")"

"$GATEWAY" >"$WORK/gateway.log" 2>&1 &
GATEWAY_PID=$!
for _ in $(seq 1 60); do
    curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 && break
    sleep 0.5
done
curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 ||
    fail "the gateway did not start: $(tail -5 "$WORK/gateway.log")"

TOKEN="$FOUNDER_TOKEN"

step "1. /v1/me mints the caller's own scope and serves it first"
api GET /v1/me
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
[ "$(field onboarding/state)" = "needs_workspace" ] ||
    fail "a fresh caller should be told what is missing: ${BODY}"
OWN_SCOPE="$(field anchors/0/scope_id)"
[ -n "$OWN_SCOPE" ] || fail "no anchors served: ${BODY}"
[ "$(field anchors/0/source)" = "principal_scope" ] ||
    fail "the caller's own scope should sort first: ${BODY}"
[ "$(field anchors/0/kind)" = "principal" ] || fail "wrong shape: ${BODY}"
ROOT_SCOPE="$(field onboarding/tenant_scope_id)"
[ -n "$ROOT_SCOPE" ] || fail "the product should have minted a tenant root: ${BODY}"
ok "${FOUNDER} stands at ${OWN_SCOPE}, under a root nobody was asked to declare"

step "2. One grant, and nothing else written"
# Break-glass at the store level, because nothing mints a tenant's first grant
# yet — the standing gap this feature records rather than solves. Everything
# after this line goes through the API under the PDP.
psql_db -c "insert into scope_grants
              (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
            values (gen_random_uuid(), '${TENANT_ID}', '${ROOT_SCOPE}', 'principal',
                    '${FOUNDER}', 'owner', 'owner')" >/dev/null
GRANTS="$(psql_db -c "select count(*) from scope_grants")"
[ "$GRANTS" = "1" ] || fail "the tenant should hold exactly one grant; found ${GRANTS}"

api POST /v1/workspaces w-1 '{"slug":"payments","display_name":"Payments"}'
[ "$STATUS" = "201" ] || fail "a tenant-root owner should create a workspace: ${STATUS} ${BODY}"
WORKSPACE_ID="$(field id)"
WORKSPACE_SCOPE="$(field scope_id)"
api POST "/v1/workspaces/${WORKSPACE_ID}/projects" p-1 '{"slug":"ledger","display_name":"Ledger"}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
PROJECT_ID="$(field id)"
PROJECT_SCOPE="$(field scope_id)"
GRANTS="$(psql_db -c "select count(*) from scope_grants where scope_id = '${ROOT_SCOPE}'")"
[ "$GRANTS" = "1" ] || fail "the tenant root should still hold one grant; found ${GRANTS}"
ok "a workspace and a project, decided by the one grant at the root"

step "3. /v1/me forecasts each anchor from real decisions"
api GET /v1/me
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
[ "$(anchor "$WORKSPACE_SCOPE" roles/0)" = "owner" ] ||
    fail "the founder should own their workspace: ${BODY}"
[ "$(anchor "$WORKSPACE_SCOPE" actions/workspace.update)" = "True" ] ||
    fail "and be forecast the update: ${BODY}"
[ "$(anchor "$WORKSPACE_SCOPE" actions/membership.grant)" = "True" ] ||
    fail "and the grant: ${BODY}"
[ "$(anchor "$PROJECT_SCOPE" actions/project.update)" = "True" ] ||
    fail "and the project inside it: ${BODY}"
ok "$(anchor_count) anchors, each answered by the PDP rather than by a plan"

step "4. A project-only grant reaches the project and stops"
api POST "/v1/projects/${PROJECT_ID}/members" m-1 \
    "{\"principal_id\":\"${CONTRACTOR}\",\"role\":\"member\"}"
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
GRANT_ID="$(field id)"

TOKEN="$CONTRACTOR_TOKEN"
api GET "/v1/projects/${PROJECT_ID}"
[ "$STATUS" = "200" ] || fail "a project member should read the project: ${STATUS} ${BODY}"
api GET "/v1/workspaces/${WORKSPACE_ID}"
[ "$STATUS" = "403" ] || fail "a project grant must not reach the workspace: ${STATUS} ${BODY}"
api GET "/v1/workspaces/${WORKSPACE_ID}/members"
[ "$STATUS" = "403" ] || fail "nor its membership: ${STATUS} ${BODY}"
api PATCH "/v1/workspaces/${WORKSPACE_ID}" "" '{"expected_revision":1,"display_name":"Mine"}'
[ "$STATUS" = "403" ] || fail "nor its administration: ${STATUS} ${BODY}"
api GET /v1/me
[ "$(anchor "$WORKSPACE_SCOPE" roles/0)" = "<absent>" ] ||
    fail "the workspace above a granted project is not an anchor: ${BODY}"
ok "${CONTRACTOR} holds the project and nothing above it"

step "5. Revocation is in force on the very next request"
TOKEN="$FOUNDER_TOKEN"
api DELETE "/v1/admin/grants/${GRANT_ID}"
[ "$STATUS" = "204" ] || fail "expected 204, got ${STATUS}: ${BODY}"
TOKEN="$CONTRACTOR_TOKEN"
api GET "/v1/projects/${PROJECT_ID}"
[ "$STATUS" = "403" ] || fail "the next request must be refused: ${STATUS} ${BODY}"
ok "nothing ran, nothing was invalidated: the resolution is the check"

step "6. A group grant reaches its members, and stops when they leave"
TOKEN="$FOUNDER_TOKEN"
api POST /v1/admin/groups g-1 \
    "{\"slug\":\"reviewers\",\"display_name\":\"Reviewers\",\"members\":[\"${CONTRACTOR}\"]}"
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
GROUP_ID="$(field id)"
api POST /v1/admin/grants gg-1 \
    "{\"scope_id\":\"${WORKSPACE_SCOPE}\",\"group_id\":\"${GROUP_ID}\",\"role\":\"member\"}"
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
ROWS="$(psql_db -c "select count(*) from scope_grants where scope_id = '${PROJECT_SCOPE}'")"

TOKEN="$CONTRACTOR_TOKEN"
api GET "/v1/projects/${PROJECT_ID}"
[ "$STATUS" = "200" ] ||
    fail "a group's workspace grant should reach the project: ${STATUS} ${BODY}"
ok "the group's grant reaches the project with ${ROWS} row(s) written there"

TOKEN="$FOUNDER_TOKEN"
api PATCH "/v1/admin/groups/${GROUP_ID}" "" '{"expected_revision":1,"members":[]}'
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
TOKEN="$CONTRACTOR_TOKEN"
api GET "/v1/projects/${PROJECT_ID}"
[ "$STATUS" = "403" ] || fail "leaving the group must take the access: ${STATUS} ${BODY}"
ok "and takes it back when they leave it"

step "7. Nobody reaches into somebody else's own scope"
TOKEN="$STRANGER_TOKEN"
api GET /v1/me
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
STRANGER_SCOPE="$(field anchors/0/scope_id)"
[ -n "$STRANGER_SCOPE" ] || fail "the stranger has no own scope: ${BODY}"

TOKEN="$FOUNDER_TOKEN"
api GET /v1/me
[ "$(anchor "$STRANGER_SCOPE" scope_id)" = "<absent>" ] ||
    fail "somebody else's own scope must never be an anchor of mine: ${BODY}"
# The widest thing the model can express, held by the founder, and it does not
# reach in: the base layer refuses it under every pack.
api GET "/v1/admin/grants?scope_id=${STRANGER_SCOPE}"
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
GRANTS_THERE="$(python3 -c 'import json,sys;print(len(json.loads(sys.stdin.read())["grants"]))' <<<"$BODY")"
[ "$GRANTS_THERE" = "0" ] || fail "nobody was granted the stranger's own scope: ${BODY}"
ok "a tenant-root owner holds everything except one person's own notes"

step "8. The chain records what was decided, and verifies"
api GET "/v1/audit/events?limit=200"
# The founder holds `owner` at the root, which every pack prices the audit
# plane at through the read-only admin permit.
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
for needle in "workspace.created" "project.created" "access.granted" "access.revoked"; do
    grep -q "$needle" "$WORK/body" || fail "the chain does not record ${needle}"
done
grep -q "workspace ${WORKSPACE_ID}" "$WORK/body" ||
    grep -q "scope ${WORKSPACE_SCOPE}" "$WORK/body" ||
    fail "the chain does not name what was decided about"
api GET /v1/audit/verify
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
[ "$(field valid)" = "True" ] || fail "the chain does not verify: ${BODY}"
ok "the chain names the thing each decision was about, and verifies"

printf '\n\033[1;32mCPR-6: a grant decides.\033[0m\n'
printf '  own scope   %s\n' "$OWN_SCOPE"
printf '  workspace   %s\n' "$WORKSPACE_SCOPE"
printf '  project     %s\n' "$PROJECT_SCOPE"
printf '  grants      %s\n' "$(psql_db -c 'select count(*) from scope_grants')"
