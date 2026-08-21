#!/usr/bin/env bash
# CPR-4 — workspaces, projects and canonical repository identity, driven the
# way a person and an agent actually drive them (ADR-0071).
#
# The integration suites cover the store contract and the HTTP surface. What
# they cannot show is the sentence the feature exists for: **a person with a
# fresh deployment goes from nothing to a project with a repository, and is
# never asked to declare an organisation on the way.** That is a claim about a
# running gateway, a real database and a real token, so it is demonstrated here
# against all three or it is not demonstrated at all.
#
# What it asserts, in order:
#
#   1. A fresh tenant has **no scope tree at all** — nobody has been asked for
#      one — and `GET /v1/me` says the next step is a workspace.
#   2. `POST /v1/workspaces` creates the workspace **and its governed scope**,
#      minting the tenant root on the way past.
#   3. Retrying that exact request with the same key returns the **same**
#      workspace with 200 and creates nothing.
#   4. The same key with a *different* body is a 409.
#   5. A creation with no `Idempotency-Key` is refused, and the refusal names
#      the header.
#   6. `/v1/me` now says the next step is a project.
#   7. `POST .../projects` creates the project and its scope **under the
#      workspace's scope** — the two models agree, and the scope path reads as
#      the product nouns do.
#   8. A repository attaches by **canonical identity**: an `ssh` remote with a
#      credential in it comes back as a clean `https` URI, and the same
#      repository written another way is already attached.
#   9. A **filesystem path is refused by name**, with a message saying what to
#      send instead.
#  10. An update with a stale `expected_revision` is a 409 and writes nothing.
#  11. `/v1/me` says `ready`.
#  12. The audit chain records every act under its own action name — and
#      **verifies**.
#  13. The chain carries no credential, and no act was recorded under the
#      deleted hierarchy/role vocabulary.
#
# Usage: demos/cpr-4-workspaces.sh
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
DB="synveda_cpr4_demo_$$"
URL="postgres://synveda:synveda-dev@localhost:5432/${DB}"
WORK="$(mktemp -d)"
# Not 8120, for cpr-2-schema-epoch.sh's reason: a contributor running this may
# have a deployment on the default port.
PORT=8131
GATEWAY_URL="http://127.0.0.1:${PORT}"
GATEWAY_PID=""
SUBJECT="cpr4-demo"

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

# One API call. Prints "<status>\n<body>"; callers split them.
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
# separator is `/` rather than `.` because half the keys worth reading here are
# action names — `workspace.create` — which already contain a dot.
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
# The dev HS256 mode (ADR-0008): this demo is about the workspace plane, and a
# real IdP would be a second thing to get right.
export SYNVEDA_DEV_JWT_SECRET="cpr4-demo-secret"
SYNVEDA_KMS_KEY="$("$BIN" kms keygen 2>/dev/null)"
export SYNVEDA_KMS_KEY

"$BIN" db migrate >/dev/null 2>&1 || fail "migrating an empty database"
TENANT_SLUG="cpr4-demo-$$"
"$BIN" tenant create --slug "$TENANT_SLUG" --name 'CPR-4 demo' >"$WORK/tenant.json"
TENANT_ID="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' <"$WORK/tenant.json")"
[ -n "$TENANT_ID" ] || fail "admitting the tenant produced no id"
TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$SUBJECT")"

"$GATEWAY" >"$WORK/gateway.log" 2>&1 &
GATEWAY_PID=$!
for _ in $(seq 1 60); do
    curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 && break
    sleep 0.5
done
curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 ||
    fail "the gateway did not start: $(tail -5 "$WORK/gateway.log")"

step "1. A fresh tenant has no scope tree, and /v1/me says what is missing"
SCOPES="$(psql_db -c "select count(*) from scopes")"
[ "$SCOPES" = "0" ] || fail "a fresh tenant already has ${SCOPES} scope(s)"
api GET /v1/me
[ "$STATUS" = "200" ] || fail "/v1/me answered ${STATUS}: ${BODY}"
[ "$(field onboarding/state)" = "needs_workspace" ] ||
    fail "expected needs_workspace, got '$(field onboarding/state)'"
ROOT_SCOPE="$(field onboarding/tenant_scope_id)"
[ -n "$ROOT_SCOPE" ] || fail "/v1/me should have minted the tenant root: ${BODY}"
# The first grant a tenant holds. Break-glass at the store level, because
# nothing mints one for a dev-token tenant with no IdP admin group to read
# (CPR-7, ADR-0074 decision 4 — a login through `synveda-admins` is the
# operator door; this is the admission-level gap it leaves standing).
psql_db -c "insert into scope_grants
              (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
            values (gen_random_uuid(), '${TENANT_ID}', '${ROOT_SCOPE}', 'principal',
                    '${SUBJECT}', 'owner', 'owner')" >/dev/null
api GET /v1/me
[ "$(field capabilities/actions/workspace.create)" = "True" ] ||
    fail "the tenant root's owner cannot create a workspace: ${BODY}"
ok "no workspace yet, and the server says the next step is one"

step "2. POST /v1/workspaces creates the workspace and its governed scope"
api POST /v1/workspaces k-1 '{"slug":"payments","display_name":"Payments"}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
WORKSPACE_ID="$(field id)"
WORKSPACE_SCOPE="$(field scope_id)"
[ "$(field revision)" = "1" ] || fail "a fresh workspace is at revision 1"
[ -n "$WORKSPACE_SCOPE" ] || fail "the workspace owns no scope"
KIND="$(psql_db -c "select kind from scopes where id = '${WORKSPACE_SCOPE}'")"
[ "$KIND" = "workspace" ] || fail "the owned scope is a '${KIND}', not a workspace"
ROOTS="$(psql_db -c "select count(*) from scopes where kind = 'tenant'")"
[ "$ROOTS" = "1" ] || fail "expected exactly one tenant root, found ${ROOTS}"
ok "workspace ${WORKSPACE_ID}, scope ${WORKSPACE_SCOPE}, under the one tenant root"

step "3. The same request with the same key returns the same workspace"
api POST /v1/workspaces k-1 '{"slug":"payments","display_name":"Payments"}'
[ "$STATUS" = "200" ] || fail "a replay should be 200, got ${STATUS}: ${BODY}"
[ "$(field id)" = "$WORKSPACE_ID" ] || fail "the replay returned a different workspace"
COUNT="$(psql_db -c "select count(*) from workspaces")"
[ "$COUNT" = "1" ] || fail "the retry created a second workspace (${COUNT} rows)"
ok "200 with the original workspace; nothing was created"

step "4. The same key with a different body is a conflict"
api POST /v1/workspaces k-1 '{"slug":"ledger","display_name":"Ledger"}'
[ "$STATUS" = "409" ] || fail "expected 409, got ${STATUS}: ${BODY}"
ok "409 — a key identifies one request"

step "5. A creation with no Idempotency-Key is refused"
api POST /v1/workspaces '' '{"slug":"ledger","display_name":"Ledger"}'
[ "$STATUS" = "400" ] || fail "expected 400, got ${STATUS}: ${BODY}"
case "$BODY" in
    *Idempotency-Key*) ok "400, and the refusal names the header" ;;
    *) fail "the refusal does not name the header: ${BODY}" ;;
esac

step "6. /v1/me now says the next step is a project"
api GET /v1/me
[ "$(field onboarding/state)" = "needs_project" ] ||
    fail "expected needs_project, got '$(field onboarding/state)'"
[ -n "$(field onboarding/tenant_scope_id)" ] || fail "the tenant root is not reported"
ok "needs_project, with the tenant root now reported"

step "7. A project's scope sits under its workspace's"
api POST "/v1/workspaces/${WORKSPACE_ID}/projects" p-1 \
    '{"slug":"ledger","display_name":"Ledger"}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
PROJECT_ID="$(field id)"
PROJECT_SCOPE="$(field scope_id)"
PARENT="$(psql_db -c "select parent_scope_id from scopes where id = '${PROJECT_SCOPE}'")"
[ "$PARENT" = "$WORKSPACE_SCOPE" ] ||
    fail "the project's scope is under '${PARENT}', not the workspace's scope"
PATH_SQL="select string_agg(s.slug, '/' order by c.distance desc)
          from scope_closure c join scopes s on s.id = c.ancestor_id
          where c.descendant_id = '${PROJECT_SCOPE}'"
SCOPE_PATH="$(psql_db -c "$PATH_SQL")"
[ "$SCOPE_PATH" = "${TENANT_SLUG}/payments/ledger" ] ||
    fail "the scope path is '${SCOPE_PATH}', not '${TENANT_SLUG}/payments/ledger'"
ok "project ${PROJECT_ID} at ${SCOPE_PATH}"

step "8. A repository attaches by canonical identity"
api POST "/v1/projects/${PROJECT_ID}/repositories" r-1 \
    '{"remote_uri":"ssh://x-token:ghp_supersecret@github.com:22/Acme/payments.git","default_branch":"main"}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
CANONICAL="$(field canonical_uri)"
[ "$CANONICAL" = "https://github.com/Acme/payments" ] ||
    fail "the canonical URI is '${CANONICAL}'"
case "$BODY" in
    *ghp_*) fail "the response carries the credential: ${BODY}" ;;
esac
[ "$(field provider)" = "github" ] || fail "provider is '$(field provider)'"
api POST "/v1/projects/${PROJECT_ID}/repositories" r-2 \
    '{"remote_uri":"git@github.com:acme/payments.git"}'
[ "$STATUS" = "409" ] ||
    fail "the same repository written another way should already be attached, got ${STATUS}"
ok "${CANONICAL} — transport, credential and port collapsed; the second form is already attached"

step "9. A filesystem path is refused by name"
api POST "/v1/projects/${PROJECT_ID}/repositories" r-3 \
    '{"remote_uri":"/Users/sam/src/payments"}'
[ "$STATUS" = "400" ] || fail "expected 400, got ${STATUS}: ${BODY}"
case "$BODY" in
    *local_fingerprint*) ok "400, and the refusal says what to send instead" ;;
    *) fail "the refusal does not say what to send instead: ${BODY}" ;;
esac

step "10. A stale revision is refused and writes nothing"
api PATCH "/v1/workspaces/${WORKSPACE_ID}" '' \
    '{"expected_revision":1,"display_name":"Payments platform"}'
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
[ "$(field revision)" = "2" ] || fail "the revision did not move"
api PATCH "/v1/workspaces/${WORKSPACE_ID}" '' \
    '{"expected_revision":1,"display_name":"Payments (old)"}'
[ "$STATUS" = "409" ] || fail "a stale precondition should be 409, got ${STATUS}: ${BODY}"
api GET "/v1/workspaces/${WORKSPACE_ID}"
[ "$(field display_name)" = "Payments platform" ] ||
    fail "the refused update was applied anyway"
[ "$(field revision)" = "2" ] || fail "a refused update bumped the revision"
ok "409, and the losing writer changed nothing"

step "11. /v1/me says ready"
api GET /v1/me
[ "$(field onboarding/state)" = "ready" ] ||
    fail "expected ready, got '$(field onboarding/state)'"
ok "ready — a workspace, a project, and somewhere to work"

step "12. The chain records every act, and verifies"
for ACTION in workspace.created workspace.updated project.created \
              project.repository.attached; do
    N="$(psql_db -c "select count(*) from audit_log where action = '${ACTION}'")"
    [ "$N" -ge 1 ] || fail "the chain does not record ${ACTION}"
done
"$BIN" audit verify --tenant "$TENANT_ID" >"$WORK/verify.log" 2>&1 ||
    fail "the chain does not verify: $(cat "$WORK/verify.log")"
ok "workspace.created, workspace.updated, project.created, project.repository.attached — chain verifies"

step "13. Nothing leaked, and every act is under its own name"
LEAK="$(psql_db -c "select count(*) from audit_log where payload::text like '%ghp_%'")"
[ "$LEAK" = "0" ] || fail "${LEAK} audit event(s) carry a credential"
WRONG="$(psql_db -c "select count(*) from audit_log
                     where action like 'hierarchy.%' or action like 'role.%'")"
[ "$WRONG" = "0" ] ||
    fail "${WRONG} act(s) were recorded under the deleted hierarchy vocabulary"
ok "no credential in the chain; no act under a deleted action name"

printf '\n\033[1;32mCPR-4 demonstrated.\033[0m A person went from nothing to a project with a\n'
printf 'repository, was never asked to declare an organisation, and every act is in the\n'
printf 'chain under its own name.\n'
