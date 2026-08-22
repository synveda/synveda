#!/usr/bin/env bash
# CPR-10 — the session ledger and runtime API (ADR-0076).
#
# The integration suites cover the routes, the PDP and the audit events. What
# they cannot show is the sentence this feature exists for: **what an agent
# does is now a governed record**, opened, appended to, composed for and
# closed through the public API, with a timeline projected over it and a chain
# that verifies.
#
# That is a claim about a running gateway, a real database and real tokens, so
# it is demonstrated against all three or it is not demonstrated at all.
#
# There is **no `/v1/observe` and no `/v1/inject` anywhere in this script**.
# That is the point: the old correlation string is untouched by this feature
# and nothing here reads or writes it (Prompt 11 deletes it).
#
# What it asserts, in order:
#
#   1. An agent opens a run, and the run's **governed scope is derived** —
#      the project's, which no request named.
#   2. A client that names its own tenant or acting principal is **refused**,
#      not quietly obeyed.
#   3. A batch of events appends once; a redelivery appends only what is new
#      and answers `duplicate` for the rest, at the original positions.
#   4. A context run composes through the real retrieval engine and persists.
#   5. The **timeline is a projection**: events and context runs merged, in
#      order, from two tables and no third.
#   6. The two-phase close: `ending` still accepts buffered events, and a
#      closed run accepts none and never reopens.
#   7. A member granted at one project sees that project's run and not the
#      other's — the listing deciding per row.
#   8. The audit chain records every act and **verifies**.
#
# Usage: demos/cpr-10-sessions.sh
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
DB="synveda_cpr10_demo_$$"
URL="postgres://synveda:synveda-dev@localhost:5432/${DB}"
WORK="$(mktemp -d)"
# Not 8120, 8131–8134: a contributor running this may have a deployment or
# another demo on those ports.
PORT=8135
GATEWAY_URL="http://127.0.0.1:${PORT}"
GATEWAY_PID=""
FOUNDER="cpr10-founder"
MEMBER="cpr10-member"
OUTSIDER="cpr10-outsider"

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

# A field out of the last response body, without a jq dependency.
field() { python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
for k in sys.argv[1].split("/"):
    d = d[int(k)] if isinstance(d, list) else d.get(k)
    if d is None:
        break
print("" if d is None else d)' "$1" <<<"$BODY"; }

count() { python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
for k in sys.argv[1].split("/"):
    d = d[int(k)] if isinstance(d, list) else d.get(k)
print(len(d))' "$1" <<<"$BODY"; }

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
export SYNVEDA_DEV_JWT_SECRET="cpr10-demo-secret"
SYNVEDA_KMS_KEY="$("$BIN" kms keygen 2>/dev/null)"
export SYNVEDA_KMS_KEY

"$BIN" db migrate >/dev/null 2>&1 || fail "migrating an empty database"
TENANT_SLUG="cpr10-demo-$$"
"$BIN" tenant create --slug "$TENANT_SLUG" --name 'CPR-10 demo' >"$WORK/tenant.json"
TENANT_ID="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' <"$WORK/tenant.json")"
[ -n "$TENANT_ID" ] || fail "admitting the tenant produced no id"

# The one break-glass act in the script, at the store level and in the open:
# the founder's administrator grant at the tenant root — the row the
# `synveda-admins` IdP-group convention mints in production, seeded here
# because a dev HS256 token carries no groups claim. Everything after this
# line goes through the API under the PDP.
#
# Three statements rather than two, and the third is the one worth reading:
# closure maintenance is store code inside the caller's transaction and there
# is no trigger behind it (ADR-0011), so a scope inserted by raw SQL has **no
# self-row in `scope_closure`** — and the anchor resolver joins that table to
# find a grant. Without it the founder's grant exists and reaches nothing, and
# the first `POST /v1/workspaces` is a 403 quoting the pack. That is the
# honest cost of break-glass seeding, and it is spelled out here because the
# next script to copy this block will hit it too.
psql_db -c "insert into scopes (id, tenant_id, kind, slug, display_name)
            values (gen_random_uuid(), '$TENANT_ID', 'tenant', '$TENANT_SLUG', 'CPR-10 demo')"
psql_db -c "insert into scope_closure (tenant_id, ancestor_id, descendant_id, distance)
            select tenant_id, id, id, 0
            from scopes where tenant_id = '$TENANT_ID' and kind = 'tenant'"
psql_db -c "insert into scope_grants
              (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
            select gen_random_uuid(), tenant_id, id, 'principal', '$FOUNDER', 'administrator', 'automation'
            from scopes where tenant_id = '$TENANT_ID' and kind = 'tenant'"

FOUNDER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$FOUNDER")"
MEMBER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$MEMBER")"
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

step "0. Somewhere to work"
api POST /v1/workspaces ws-1 '{"slug":"payments","display_name":"Payments"}'
[ "$STATUS" = "201" ] || fail "creating the workspace: ${STATUS} ${BODY}"
WORKSPACE="$(field id)"
api POST "/v1/workspaces/${WORKSPACE}/projects" pr-1 '{"slug":"ledger","display_name":"Ledger"}'
[ "$STATUS" = "201" ] || fail "creating the project: ${STATUS} ${BODY}"
PROJECT="$(field id)"
PROJECT_SCOPE="$(field scope_id)"
ok "a workspace and a project, through the public routes"

api POST "/v1/workspaces/${WORKSPACE}/projects" pr-2 '{"slug":"reporting","display_name":"Reporting"}'
OTHER_PROJECT="$(field id)"

step "1. An agent opens a run, and its scope is derived"
api POST /v1/sessions run-1 "$(cat <<JSON
{"workspace_id":"${WORKSPACE}","project_id":"${PROJECT}",
 "client_name":"claude-code","client_version":"2.1.0",
 "external_session_id":"harness-abc","model_name":"claude-opus-5",
 "branch":"main","task_summary":"Fix ledger rounding",
 "metadata":{"cwd":"/work/payments"}}
JSON
)"
[ "$STATUS" = "201" ] || fail "opening the run: ${STATUS} ${BODY}"
SESSION="$(field id)"
[ "$(field status)" = "active" ] || fail "a new run is active: ${BODY}"
[ "$(field principal_id)" = "$FOUNDER" ] || fail "the token decides who opened it: ${BODY}"
[ "$(field scope_id)" = "$PROJECT_SCOPE" ] ||
    fail "the run should be anchored at the project's scope, got $(field scope_id)"
ok "the run's governed scope is the project's — derived, not sent"

api POST /v1/sessions run-1 "{\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\",\"client_name\":\"claude-code\",\"client_version\":\"2.1.0\",\"external_session_id\":\"harness-abc\",\"model_name\":\"claude-opus-5\",\"branch\":\"main\",\"task_summary\":\"Fix ledger rounding\",\"metadata\":{\"cwd\":\"/work/payments\"}}"
[ "$STATUS" = "200" ] || fail "a retry with the same key must replay 200, got ${STATUS}"
[ "$(field id)" = "$SESSION" ] || fail "and the same run"
ok "the same Idempotency-Key replays the run rather than opening a second"

step "2. A client may not name its own tenant or acting principal"
api POST /v1/sessions id-1 \
    "{\"workspace_id\":\"${WORKSPACE}\",\"client_name\":\"claude-code\",\"principal_id\":\"somebody-else\"}"
[ "$STATUS" = "400" ] || fail "principal_id must be refused, got ${STATUS}: ${BODY}"
api POST /v1/sessions id-2 \
    "{\"workspace_id\":\"${WORKSPACE}\",\"client_name\":\"claude-code\",\"tenant_id\":\"${TENANT_ID}\"}"
[ "$STATUS" = "400" ] || fail "tenant_id must be refused, got ${STATUS}: ${BODY}"
api POST /v1/sessions id-3 \
    "{\"workspace_id\":\"${WORKSPACE}\",\"client_name\":\"claude-code\",\"scope_id\":\"${PROJECT_SCOPE}\"}"
[ "$STATUS" = "400" ] || fail "scope_id must be refused, got ${STATUS}: ${BODY}"
ok "tenant, principal and scope are the server's — a body naming one is a 400"

step "3. Events append once, and a redelivery appends only what is new"
api POST "/v1/sessions/${SESSION}/events" '' "$(cat <<'JSON'
{"events":[
 {"event_type":"message.user","client_event_id":"e1","occurred_at":"2020-01-01T10:00:00Z","payload":{"text":"fix the rounding"}},
 {"event_type":"tool.invoked","client_event_id":"e2","occurred_at":"2020-01-01T10:00:05Z","payload":{"tool":"grep"}},
 {"event_type":"file.changed","client_event_id":"e3","occurred_at":"2020-01-01T10:00:30Z","payload":{"path":"ledger.rs"}}
]}
JSON
)"
[ "$STATUS" = "200" ] || fail "appending: ${STATUS} ${BODY}"
[ "$(field appended)" = "3" ] || fail "three events should append: ${BODY}"
[ "$(field events/0/event/sequence)" = "1" ] || fail "positions start at one: ${BODY}"
ok "three events appended at positions 1–3"

api POST "/v1/sessions/${SESSION}/events" '' "$(cat <<'JSON'
{"events":[
 {"event_type":"file.changed","client_event_id":"e3","occurred_at":"2020-01-01T10:00:30Z","payload":{"path":"ledger.rs"}},
 {"event_type":"command.executed","client_event_id":"e4","occurred_at":"2020-01-01T10:01:00Z","payload":{"command":"cargo test"}}
]}
JSON
)"
[ "$STATUS" = "200" ] || fail "redelivering: ${STATUS} ${BODY}"
[ "$(field appended)" = "1" ] || fail "one new event: ${BODY}"
[ "$(field duplicates)" = "1" ] || fail "one duplicate: ${BODY}"
[ "$(field events/0/outcome)" = "duplicate" ] || fail "the overlap is reported: ${BODY}"
[ "$(field events/0/event/sequence)" = "3" ] ||
    fail "a duplicate keeps its original position, got $(field events/0/event/sequence)"
ok "a redelivered batch appends the new event and reports the duplicate at position 3"

step "4. Context is composed for the run, and persisted"
api POST "/v1/sessions/${SESSION}/context-runs" ctx-1 '{"query":"how do we round money"}'
[ "$STATUS" = "201" ] || fail "composing: ${STATUS} ${BODY}"
RUN="$(field id)"
[ -n "$(field block_hash)" ] || fail "a context run carries a block hash: ${BODY}"
[ "$(field scope_id)" = "$PROJECT_SCOPE" ] || fail "composed at the run's own scope: ${BODY}"
ok "a context run composed and persisted (block $(field block_hash | cut -c1-12)…)"

step "5. The timeline is a projection over two tables"
api GET "/v1/sessions/${SESSION}/timeline"
[ "$STATUS" = "200" ] || fail "the timeline: ${STATUS} ${BODY}"
[ "$(count entries)" = "5" ] || fail "four events and one context run, got $(count entries): ${BODY}"
# The events keep the server's own `sequence` order and the run is placed among
# them by instant — a **merge**, not a sort. The events above are dated 2020 on
# purpose: `occurred_at` is the client's clock and `created_at` is this
# deployment's, so a fixture dated in the future would put every event after
# every run and hide whichever ordering the projection actually has.
SEQUENCES="$(python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
print(",".join(str(e["sequence"]) for e in d["entries"] if e["kind"] == "event"))' <<<"$BODY")"
[ "$SEQUENCES" = "1,2,3,4" ] || fail "events must keep their sequence order, got ${SEQUENCES}"
[ "$(field entries/4/kind)" = "context_run" ] ||
    fail "the run's own clock is now, so it lands after the 2020 events: ${BODY}"
[ "$(field event_counts/message.user)" = "1" ] || fail "the run's shape is reported: ${BODY}"
# The projection is not a table: there is no third place the entries live.
TABLES="$(psql_db -c "select count(*) from information_schema.tables
                      where table_schema='public' and table_name like '%timeline%'")"
[ "$TABLES" = "0" ] || fail "a timeline table exists; the projection was materialised"
ok "5 entries merged from session_events and session_context_runs — and no timeline table"

step "6. The two-phase close"
api POST "/v1/sessions/${SESSION}/end" '' '{"status":"ending"}'
[ "$STATUS" = "200" ] || fail "beginning the close: ${STATUS} ${BODY}"
[ "$(field status)" = "ending" ] || fail "the run should be ending: ${BODY}"
api POST "/v1/sessions/${SESSION}/events" '' \
    '{"events":[{"event_type":"session.ended","client_event_id":"e5","occurred_at":"2020-01-01T10:02:00Z","payload":{}}]}'
[ "$STATUS" = "200" ] || fail "a buffered event must still land while ending: ${STATUS} ${BODY}"
ok "\`ending\` still accepts the events the adapter had already buffered"

api POST "/v1/sessions/${SESSION}/end" '' '{"status":"ended","task_summary":"Ledger rounding fixed"}'
[ "$STATUS" = "200" ] || fail "closing: ${STATUS} ${BODY}"
[ "$(field status)" = "ended" ] || fail "the run should be ended: ${BODY}"
[ -n "$(field ended_at)" ] || fail "a closed run carries an end time: ${BODY}"

api POST "/v1/sessions/${SESSION}/events" '' \
    '{"events":[{"event_type":"message.user","client_event_id":"e6","occurred_at":"2020-01-01T10:03:00Z","payload":{}}]}'
[ "$STATUS" = "409" ] || fail "a closed run must refuse events, got ${STATUS}"
api POST "/v1/sessions/${SESSION}/end" '' '{"status":"active"}'
[ "$STATUS" = "400" ] || fail "a closed run must not reopen, got ${STATUS}"
ok "a closed run accepts no events and never reopens"

step "7. A grant at one project reaches that project's runs and no others"
# A second run, in the other project, so the member has something to not see.
api POST /v1/sessions run-2 \
    "{\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${OTHER_PROJECT}\",\"client_name\":\"cursor\"}"
[ "$STATUS" = "201" ] || fail "opening the second run: ${STATUS} ${BODY}"
OTHER_SESSION="$(field id)"

api POST /v1/admin/grants grant-1 \
    "{\"scope_id\":\"${PROJECT_SCOPE}\",\"principal_id\":\"${MEMBER}\",\"role\":\"member\"}"
[ "$STATUS" = "201" ] || fail "granting the member: ${STATUS} ${BODY}"

TOKEN="$MEMBER_TOKEN"
api GET /v1/sessions
[ "$STATUS" = "200" ] || fail "the member's listing: ${STATUS} ${BODY}"
[ "$(count sessions)" = "1" ] || fail "exactly one run, got $(count sessions): ${BODY}"
[ "$(field sessions/0/id)" = "$SESSION" ] || fail "and it is the one in their project: ${BODY}"
api GET "/v1/sessions/${OTHER_SESSION}"
[ "$STATUS" = "403" ] || fail "the other project's run must be refused, got ${STATUS}"
ok "the member sees one run of two, and the listing agrees with the per-object route"

TOKEN="$OUTSIDER_TOKEN"
api GET /v1/sessions
[ "$STATUS" = "403" ] || fail "a caller who holds nothing must be refused, got ${STATUS}"
api POST /v1/sessions outsider-1 \
    "{\"workspace_id\":\"${WORKSPACE}\",\"client_name\":\"claude-code\"}"
[ "$STATUS" = "403" ] || fail "and must not be able to open a run, got ${STATUS}"
ok "a caller who holds nothing reads nothing and opens nothing"

step "8. The chain records every act, and verifies"
TOKEN="$FOUNDER_TOKEN"
CHAIN="$(psql_db -c "select action from audit_log where tenant_id = '$TENANT_ID' order by seq")"
for action in session.opened session.events.appended session.context.composed session.ended; do
    grep -q "^${action}$" <<<"$CHAIN" || fail "the chain should carry ${action}: ${CHAIN}"
done
ok "session.opened, session.events.appended, session.context.composed, session.ended"

# The metadata this run carried never reached the chain — an agent's
# environment is where credentials live, and the chain records that there was
# metadata and how much, never what.
psql_db -c "select count(*) from audit_log
            where tenant_id = '$TENANT_ID' and payload::text like '%/work/payments%'" |
    grep -q '^0$' || fail "a session's metadata reached the audit chain"
ok "and the run's metadata did not: the chain carries its size, never its contents"

api GET /v1/audit/verify
[ "$STATUS" = "200" ] || fail "verifying the chain: ${STATUS} ${BODY}"
[ "$(field valid)" = "True" ] || [ "$(field valid)" = "true" ] ||
    fail "the chain must verify: ${BODY}"
ok "chain valid ($(field events) events)"

printf '\n\033[1mCPR-10 demonstrated.\033[0m An agent run is a governed record: opened,\n'
printf 'appended to, composed for and closed through the public API, decided per\n'
printf 'row, projected as a timeline over two tables and no third, and chained.\n'
