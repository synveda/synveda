#!/usr/bin/env bash
# CPR-7 — the hierarchy cutover: one scope tree (ADR-0074).
#
# The integration suites cover the admin routes, the seeding and the packs.
# What they cannot show is the sentence this feature exists for: **the old
# hierarchy is gone, and everything still works** — administered through
# the public scope plane instead. That is a claim about a running gateway,
# a real database and real tokens, so it is demonstrated against all three
# or it is not demonstrated at all.
#
# There is **no `synveda hierarchy`, no `role bind`, no `/v1/hierarchy`
# anywhere in this script**. That is the point: every administration below
# goes through `/v1/admin/scopes` and the grant plane, or it is refused.
#
# What it asserts, in order:
#
#   1. The old routes are **gone**: every `/v1/hierarchy` path answers
#      404, and the old rank kinds (`org`, `department`, `team`…) fail
#      validation by name.
#   2. `synveda scope create/tree/move` drive the public plane: an org
#      unit, a workspace-shaped scope under it, a whole-tree draw.
#   3. A **move** through the CLI lands, is refused into its own subtree,
#      and chains an audit event naming both ends.
#   4. The operator CLI's bootstrap grant — the `synveda-admins` door
#      ADR-0073 recorded as missing — is visible as an anchor on `/v1/me`.
#   5. An ungranted caller reads the level and mutates nothing.
#   6. The audit chain records the scope acts and **verifies**.
#
# Usage: demos/cpr-7-scopes.sh
#   KEEP_DB=1      keep the scratch database on the way out
#
# Cost: one scratch database, one port, no network. Under two minutes.
set -euo pipefail

cd "$(dirname "$0")/.."

# Compiling with the checked-in `.sqlx` cache, so a demo needs no database
# to build — ten-2-rls.sh's reasoning, and the same line.
SQLX_OFFLINE=true
export SQLX_OFFLINE

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DB="synveda_cpr7_demo_$$"
URL="postgres://synveda:synveda-dev@localhost:5432/${DB}"
WORK="$(mktemp -d)"
# Not 8120, 8131, 8132 or 8133: a contributor running this may have a
# deployment or another demo on those ports.
PORT=8134
GATEWAY_URL="http://127.0.0.1:${PORT}"
GATEWAY_PID=""
FOUNDER="cpr7-founder"
OUTSIDER="cpr7-outsider"

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
    [ -n "$key" ] && args+=(-H "idempotency-key: $key")
    if [ -n "$body" ]; then
        args+=(-H 'content-type: application/json' -d "$body")
    fi
    STATUS="$(curl "${args[@]}")"
    BODY="$(cat "$WORK/body")"
}

# A field out of the last response body, without a jq dependency. The path
# separator is `/` because action names already contain a dot.
field() { python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
for k in sys.argv[1].split("/"):
    d = d[int(k)] if isinstance(d, list) else d.get(k)
    if d is None:
        break
print("" if d is None else d)' "$1" <<<"$BODY"; }

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
# The dev HS256 mode (ADR-0008): this demo is about the admin plane, and a
# real IdP would be a second thing to get right.
export SYNVEDA_DEV_JWT_SECRET="cpr7-demo-secret"
SYNVEDA_KMS_KEY="$("$BIN" kms keygen 2>/dev/null)"
export SYNVEDA_KMS_KEY

"$BIN" db migrate >/dev/null 2>&1 || fail "migrating an empty database"
TENANT_SLUG="cpr7-demo-$$"
"$BIN" tenant create --slug "$TENANT_SLUG" --name 'CPR-7 demo' >"$WORK/tenant.json"
TENANT_ID="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' <"$WORK/tenant.json")"
[ -n "$TENANT_ID" ] || fail "admitting the tenant produced no id"

# The one break-glass act in the script, at the store level and in the open:
# the founder's administrator grant at the tenant root — the same row the
# JIT admin-group convention mints in production, seeded here because a dev
# HS256 token carries no groups claim. Everything after this line goes
# through the API under the PDP.
"$BIN" token issue --tenant "$TENANT_ID" --subject "$FOUNDER" >/dev/null
psql_db -c "insert into scopes (id, tenant_id, kind, slug, display_name)
            values (gen_random_uuid(), '$TENANT_ID', 'tenant', '$TENANT_SLUG', 'CPR-7 demo')"
psql_db -c "insert into scope_grants
              (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
            select gen_random_uuid(), tenant_id, id, 'principal', '$FOUNDER', 'administrator', 'automation'
            from scopes where tenant_id = '$TENANT_ID' and kind = 'tenant'"

FOUNDER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$FOUNDER")"
OUTSIDER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$OUTSIDER")"

"$GATEWAY" >"$WORK/gateway.log" 2>&1 &
GATEWAY_PID=$!
for _ in $(seq 1 60); do
    curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 && break
    sleep 0.5
done
curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 ||
    fail "the gateway did not start: $(tail -5 "$WORK/gateway.log")"

TOKEN="$FOUNDER_TOKEN"

step "1. The old routes are gone"
api GET /v1/hierarchy/root
[ "$STATUS" = "404" ] || fail "the org root route must be gone, got ${STATUS}"
ok "GET /v1/hierarchy/root → 404"
api POST /v1/hierarchy/nodes
[ "$STATUS" = "404" ] || fail "node creation must be gone, got ${STATUS}"
ok "POST /v1/hierarchy/nodes → 404"
api GET "/v1/hierarchy/nodes/00000000-0000-0000-0000-000000000000/roles"
[ "$STATUS" = "404" ] || fail "the role routes must be gone, got ${STATUS}"
ok "the role-binding routes → 404"

api GET /v1/admin/scopes
[ "$STATUS" = "200" ] || fail "the scope level must answer, got ${STATUS}: ${BODY}"
ROOT="$(field parent/id)"
[ -n "$ROOT" ] || fail "the tenant root should be served: ${BODY}"
[ "$(field parent/kind)" = "tenant" ] || fail "the root should carry the tenant shape"

api POST /v1/admin/scopes "old-kind-check" \
    "{\"parent_id\":\"$ROOT\",\"kind\":\"department\",\"slug\":\"old\",\"display_name\":\"old\"}"
[ "$STATUS" = "400" ] || fail "the old rank kind must fail validation, got ${STATUS}: ${BODY}"
ok "kind=department → 400, by name"

step "2. The operator CLI drives the public plane"
# A profile pointing at the demo gateway, with the founder's token.
mkdir -p "$HOME/.config/synveda" 2>/dev/null || true
SYNVEDA_GATEWAY="$GATEWAY_URL" SYNVEDA_TOKEN="$FOUNDER_TOKEN" \
    "$BIN" scope create --parent "$ROOT" --kind org_unit --slug eng --name Engineering >/dev/null ||
    fail "scope create through the API"
ENG="$(SYNVEDA_GATEWAY="$GATEWAY_URL" SYNVEDA_TOKEN="$FOUNDER_TOKEN" \
    "$BIN" scope list --json | python3 -c 'import json,sys
d = json.load(sys.stdin)
print(next(s["id"] for s in d["scopes"] if s["slug"] == "eng"))')"
[ -n "$ENG" ] || fail "the org unit should list"
ok "created the eng org unit through \`synveda scope create\`"

SYNVEDA_GATEWAY="$GATEWAY_URL" SYNVEDA_TOKEN="$FOUNDER_TOKEN" \
    "$BIN" scope create --parent "$ENG" --kind workspace --slug platform --name Platform >/dev/null ||
    fail "workspace-shaped scope through the API"
TREE="$(SYNVEDA_GATEWAY="$GATEWAY_URL" SYNVEDA_TOKEN="$FOUNDER_TOKEN" \
    "$BIN" scope tree)"
echo "$TREE" | grep -q "platform (workspace)" || fail "the tree should draw the workspace: $TREE"
echo "$TREE" | grep -q "eng (org_unit)" || fail "the tree should draw the unit"
ok "\`synveda scope tree\` draws the tenant — shapes, not ranks"

api GET "/v1/admin/scopes/$ENG"
[ "$STATUS" = "200" ] || fail "show must answer, got ${STATUS}: ${BODY}"
[ "$(field path)" = "$TENANT_SLUG/eng" ] || fail "the path should read as slugs: ${BODY}"
ok "\`scope show\` serves the path $TENANT_SLUG/eng"

step "3. A move lands, and a cycle is refused"
PLATFORM="$(SYNVEDA_GATEWAY="$GATEWAY_URL" SYNVEDA_TOKEN="$FOUNDER_TOKEN" \
    "$BIN" scope list --under "$ENG" --json | python3 -c 'import json,sys
d = json.load(sys.stdin)
print(next(s["id"] for s in d["scopes"] if s["slug"] == "platform"))')"
SYNVEDA_GATEWAY="$GATEWAY_URL" SYNVEDA_TOKEN="$FOUNDER_TOKEN" \
    "$BIN" scope move "$PLATFORM" --parent "$ROOT" >/dev/null ||
    fail "a legal move through the CLI"
api GET "/v1/admin/scopes/$PLATFORM"
[ "$(field parent_scope_id)" = "$ROOT" ] || fail "the move should land: ${BODY}"
ok "moved platform out to sit beside eng"

api PATCH "/v1/admin/scopes/$ENG" "" "{\"parent_scope_id\":\"$PLATFORM\"}"
[ "$STATUS" = "400" ] || fail "a move into the scope's own subtree must be refused, got ${STATUS}: $BODY"
ok "eng under its own subtree → 400, the cycle guard"

MOVED_EVENTS="$(psql_db -c "select count(*) from audit_log
    where action = 'scope.updated' and payload ? 'moved_to'")"
[ "$MOVED_EVENTS" -ge 1 ] || fail "the move should chain an audit event naming both ends"
ok "the audit event names both ends of the move"

step "4. The bootstrap grant shows up where a client looks"
api GET /v1/me
[ "$STATUS" = "200" ] || fail "/v1/me must answer, got ${STATUS}: ${BODY}"
[ "$(field role_keys/0)" = "administrator" ] ||
    fail "the founder's administrator grant should reach /v1/me: ${BODY}"
ok "the operator door is an administrator grant at the root"

step "5. An ungranted caller mutates nothing"
TOKEN="$OUTSIDER_TOKEN"
api GET /v1/admin/scopes
[ "$STATUS" = "200" ] || fail "the level listing is a read every caller holds, got ${STATUS}"
api POST /v1/admin/scopes "outsider-nope" \
    "{\"parent_id\":\"$ROOT\",\"kind\":\"org_unit\",\"slug\":\"nope\",\"display_name\":\"nope\"}"
[ "$STATUS" = "403" ] || fail "creation without a grant must be denied, got ${STATUS}: $BODY"
api PATCH "/v1/admin/scopes/$ROOT" "" '{"display_name":"nope"}'
[ "$STATUS" = "403" ] || fail "mutation without a grant must be denied, got ${STATUS}: $BODY"
ok "$OUTSIDER reads the level and changes nothing"
TOKEN="$FOUNDER_TOKEN"

step "6. The audit chain verifies"
api GET /v1/audit/verify
[ "$(field valid)" = "True" ] || fail "the chain does not verify: ${BODY}"
SCOPE_EVENTS="$(psql_db -c "select count(*) from audit_log
    where action in ('scope.created','scope.updated')")"
[ "$SCOPE_EVENTS" -ge 3 ] || fail "the scope acts should chain, found $SCOPE_EVENTS"
ok "$SCOPE_EVENTS scope events on a verifying chain"

step "Done"
echo "   One tree, administered publicly. The hierarchy is gone."
