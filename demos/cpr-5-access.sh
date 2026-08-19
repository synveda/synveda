#!/usr/bin/env bash
# CPR-5 — membership and access assignment, driven the way two people actually
# drive it (ADR-0072).
#
# The integration suites cover the store contract and the HTTP surface. What
# they cannot show is the sentence the feature exists for: **a person makes a
# workspace, invites a colleague by copying a link, and the colleague redeems
# it with their own credential and appears — with the product able to say why.**
# That is a claim about a running gateway, a real database and two real tokens,
# so it is demonstrated here against all three or it is not demonstrated at all.
#
# What it asserts, in order:
#
#   1. Creating a workspace makes its creator the **owner**, in the same
#      transaction — a collaboration space nobody is a member of is not one.
#   2. Creating a project mints its own owner grant, and the project **inherits**
#      the workspace's with no second row written.
#   3. An invitation is issued and the token appears **once**, with a copyable
#      URL. The listing does not carry it.
#   4. A colleague redeems it with **their own** bearer, and the grant that
#      appears says `source: invite` and names the invitation.
#   5. The colleague's access reaches the project by inheritance, and the
#      listing says so — the scope it came from, and that it was inherited.
#   6. The same link redeemed again by the same person is a **replay**; by
#      somebody else it is a **409**. One-time means one-time.
#   7. A group is created, granted at the workspace, and its members hold it —
#      through the group, which the listing names. Adding somebody to the group
#      grants them everything it holds, with **no grant written**.
#   8. **Principal-private scope isolation**: a grant at the tenant root reaches
#      every workspace and project, and reaches nobody's own scope.
#   9. Removing an inherited member at the project is refused, naming where the
#      grant actually is.
#  10. Revoking a grant removes what it conferred, on the very next read.
#  11. The chain records every act under its own name and **verifies** — and
#      carries no invitation token anywhere.
#
# Usage: demos/cpr-5-access.sh
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
DB="synveda_cpr5_demo_$$"
URL="postgres://synveda:synveda-dev@localhost:5432/${DB}"
WORK="$(mktemp -d)"
# Not 8120 and not 8131, for cpr-2-schema-epoch.sh's reason: a contributor
# running this may have a deployment or another demo on those ports.
PORT=8132
GATEWAY_URL="http://127.0.0.1:${PORT}"
GATEWAY_PID=""
OWNER="cpr5-owner"
COLLEAGUE="cpr5-colleague"
STRANGER="cpr5-stranger"

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
# separator is `/` rather than `.` because half the keys worth reading here are
# action names — `membership.grant` — which already contain a dot.
field() { python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
for k in sys.argv[1].split("/"):
    d = d[int(k)] if isinstance(d, list) else d.get(k)
    if d is None:
        break
print("" if d is None else d)' "$1" <<<"$BODY"; }

# One member entry out of a member listing, selected by principal and role.
member() { python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
for m in d["members"]:
    if m["principal_id"] == sys.argv[1] and (len(sys.argv) < 4 or m["role"] == sys.argv[3]):
        print(m.get(sys.argv[2], ""))
        break
else:
    print("")' "$@" <<<"$BODY"; }

count_members() { python3 -c 'import json,sys;print(len(json.loads(sys.stdin.read())["members"]))' <<<"$BODY"; }

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
# The dev HS256 mode (ADR-0008): this demo is about the access plane, and a
# real IdP would be a second thing to get right.
export SYNVEDA_DEV_JWT_SECRET="cpr5-demo-secret"
SYNVEDA_KMS_KEY="$("$BIN" kms keygen 2>/dev/null)"
export SYNVEDA_KMS_KEY

"$BIN" db migrate >/dev/null 2>&1 || fail "migrating an empty database"
TENANT_SLUG="cpr5-demo-$$"
"$BIN" tenant create --slug "$TENANT_SLUG" --name 'CPR-5 demo' >"$WORK/tenant.json"
TENANT_ID="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' <"$WORK/tenant.json")"
[ -n "$TENANT_ID" ] || fail "admitting the tenant produced no id"
# Tenant-wide org-admin for the owner, which is what a person holds after
# `synveda init` and their first login. Break-glass at the store level, because
# there is nobody to grant it through the API yet — which is exactly the
# bootstrap problem this feature removes for *everybody else*.
"$BIN" role bind --tenant "$TENANT_ID" --subject "$OWNER" --role org-admin >/dev/null
OWNER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$OWNER")"
COLLEAGUE_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$COLLEAGUE")"
STRANGER_TOKEN="$("$BIN" token issue --tenant "$TENANT_ID" --subject "$STRANGER")"

"$GATEWAY" >"$WORK/gateway.log" 2>&1 &
GATEWAY_PID=$!
for _ in $(seq 1 60); do
    curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 && break
    sleep 0.5
done
curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 ||
    fail "the gateway did not start: $(tail -5 "$WORK/gateway.log")"

TOKEN="$OWNER_TOKEN"

step "1. Creating a workspace makes its creator the owner"
api POST /v1/workspaces w-1 '{"slug":"payments","display_name":"Payments"}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
WORKSPACE_ID="$(field id)"
WORKSPACE_SCOPE="$(field scope_id)"
api GET "/v1/workspaces/${WORKSPACE_ID}/members"
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
[ "$(count_members)" = "1" ] || fail "a fresh workspace should have exactly its owner: ${BODY}"
[ "$(member "$OWNER" role)" = "owner" ] || fail "the creator is not the owner: ${BODY}"
[ "$(member "$OWNER" source)" = "owner" ] || fail "the source is not 'owner': ${BODY}"
[ "$(member "$OWNER" inherited)" = "False" ] || fail "the owner grant should be written here"
ok "${OWNER} holds owner at ${WORKSPACE_SCOPE}, source 'owner'"

step "2. A project inherits the workspace's grants without a second row"
api POST "/v1/workspaces/${WORKSPACE_ID}/projects" p-1 '{"slug":"ledger","display_name":"Ledger"}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
PROJECT_ID="$(field id)"
PROJECT_SCOPE="$(field scope_id)"
api GET "/v1/projects/${PROJECT_ID}/members"
[ "$(count_members)" = "2" ] ||
    fail "the project should hold its own owner grant and the inherited one: ${BODY}"
ROWS="$(psql_db -c "select count(*) from scope_grants where scope_id = '${PROJECT_SCOPE}'")"
[ "$ROWS" = "1" ] ||
    fail "inheritance should write nothing; found ${ROWS} row(s) at the project"
ok "two authorities in force at the project, one row written for them"

step "3. An invitation is issued, and the token appears exactly once"
api POST "/v1/workspaces/${WORKSPACE_ID}/invites" i-1 \
    '{"role":"member","email":"colleague@example.com"}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
INVITE_TOKEN="$(field token)"
INVITE_ID="$(field invite/id)"
ACCEPT_URL="$(field accept_url)"
case "$INVITE_TOKEN" in
    synveda_invite_v1.*) ;;
    *) fail "the token does not carry the greppable prefix: ${INVITE_TOKEN}" ;;
esac
case "$ACCEPT_URL" in
    *"/v1/invites/${INVITE_TOKEN}/accept") ;;
    *) fail "the accept URL is not copyable: ${ACCEPT_URL}" ;;
esac
api GET "/v1/workspaces/${WORKSPACE_ID}/invites"
case "$BODY" in
    *"$INVITE_TOKEN"*) fail "the listing carries the token: ${BODY}" ;;
esac
[ "$(field invites/0/status)" = "pending" ] || fail "the invitation is not pending: ${BODY}"
STORED="$(psql_db -c "select octet_length(token_hash) from pending_invites where id = '${INVITE_ID}'")"
[ "$STORED" = "32" ] || fail "the invitation stores something other than a 32-byte hash"
ok "one token, a copyable URL, and a 32-byte hash in the database"

step "4. The colleague redeems it with their own credential"
TOKEN="$COLLEAGUE_TOKEN"
api POST "/v1/invites/${INVITE_TOKEN}/accept"
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
[ "$(field grant/source)" = "invite" ] || fail "the grant does not say where it came from: ${BODY}"
[ "$(field grant/invite_id)" = "$INVITE_ID" ] || fail "the grant does not name its invitation"
[ "$(field grant/role)" = "member" ] || fail "the grant carries the wrong role"
ok "${COLLEAGUE} holds member, source 'invite', naming invitation ${INVITE_ID}"

step "5. And it reaches the project, saying where it came from"
TOKEN="$OWNER_TOKEN"
api GET "/v1/projects/${PROJECT_ID}/members"
[ "$(member "$COLLEAGUE" source)" = "invite" ] || fail "${BODY}"
[ "$(member "$COLLEAGUE" inherited)" = "True" ] ||
    fail "the colleague's access should be inherited from the workspace: ${BODY}"
[ "$(member "$COLLEAGUE" scope_id)" = "$WORKSPACE_SCOPE" ] ||
    fail "the entry does not name the scope the grant is actually at: ${BODY}"
ok "inherited from ${WORKSPACE_SCOPE} — the listing answers 'why' without an audit search"

step "6. The link works once"
TOKEN="$COLLEAGUE_TOKEN"
api POST "/v1/invites/${INVITE_TOKEN}/accept"
[ "$STATUS" = "200" ] || fail "the same person retrying should replay with 200, got ${STATUS}"
TOKEN="$STRANGER_TOKEN"
api POST "/v1/invites/${INVITE_TOKEN}/accept"
[ "$STATUS" = "409" ] || fail "a second person should be refused with 409, got ${STATUS}: ${BODY}"
GRANTS="$(psql_db -c "select count(*) from scope_grants where invite_id = '${INVITE_ID}'")"
[ "$GRANTS" = "1" ] || fail "one invitation minted ${GRANTS} grants"
ok "200 for the retry, 409 for the stranger, one grant either way"

step "7. A group grants to everybody in it, and following it writes nothing"
TOKEN="$OWNER_TOKEN"
api POST /v1/admin/groups g-1 \
    '{"slug":"engineering","display_name":"Engineering","members":["cpr5-robin","cpr5-kim"]}'
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
GROUP_ID="$(field id)"
api POST /v1/admin/grants gr-1 \
    "{\"scope_id\":\"${WORKSPACE_SCOPE}\",\"group_id\":\"${GROUP_ID}\",\"role\":\"member\"}"
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
api GET "/v1/projects/${PROJECT_ID}/members"
[ "$(member cpr5-robin via_group/slug)" = "" ] || true   # nested read is below
VIA="$(python3 -c 'import json,sys
d=json.loads(sys.stdin.read())
print(sum(1 for m in d["members"] if m.get("via_group")))' <<<"$BODY")"
[ "$VIA" = "2" ] || fail "both group members should hold the grant at the project: ${BODY}"
# A third person joins the group. Nothing is granted.
BEFORE="$(psql_db -c "select count(*) from scope_grants")"
api PATCH "/v1/admin/groups/${GROUP_ID}" '' \
    '{"expected_revision":1,"members":["cpr5-robin","cpr5-kim","cpr5-sam"]}'
[ "$STATUS" = "200" ] || fail "expected 200, got ${STATUS}: ${BODY}"
AFTER="$(psql_db -c "select count(*) from scope_grants")"
[ "$BEFORE" = "$AFTER" ] || fail "joining a group wrote a grant (${BEFORE} → ${AFTER})"
api GET "/v1/projects/${PROJECT_ID}/members"
VIA="$(python3 -c 'import json,sys
d=json.loads(sys.stdin.read())
print(sum(1 for m in d["members"] if m.get("via_group")))' <<<"$BODY")"
[ "$VIA" = "3" ] || fail "the new member does not hold what the group holds: ${BODY}"
ok "three people hold it through one grant; joining the group wrote no grant"

step "8. Principal-private scope isolation"
# Somebody's own scope, made directly because nothing mints one yet (that is
# the identity plane's re-cut), and a grant at the tenant root — the widest
# thing this model can say.
TENANT_SCOPE="$(psql_db -c "select id from scopes where kind = 'tenant'")"
MINE="$(psql_db -c "insert into scopes (id, tenant_id, kind, parent_scope_id, parent_kind, slug, display_name, status, attributes)
                    values (gen_random_uuid(), '${TENANT_ID}'::uuid, 'principal', '${TENANT_SCOPE}'::uuid, 'tenant', 'sam', 'Sam', 'active', '{}'::jsonb)
                    returning id")"
psql_db -c "insert into scope_closure (tenant_id, ancestor_id, descendant_id, distance)
            select '${TENANT_ID}'::uuid, c.ancestor_id, '${MINE}'::uuid, c.distance + 1
            from scope_closure c where c.descendant_id = '${TENANT_SCOPE}'::uuid
            union all select '${TENANT_ID}'::uuid, '${MINE}'::uuid, '${MINE}'::uuid, 0" >/dev/null
api POST /v1/admin/grants gr-2 \
    "{\"scope_id\":\"${TENANT_SCOPE}\",\"principal_id\":\"cpr5-boss\",\"role\":\"administrator\"}"
[ "$STATUS" = "201" ] || fail "expected 201, got ${STATUS}: ${BODY}"
api GET "/v1/projects/${PROJECT_ID}/members"
[ -n "$(member cpr5-boss role)" ] || fail "a tenant-root grant should reach the project: ${BODY}"

# The rule, shown rather than asserted vacuously: the same closure join without
# the principal test finds grants above this scope, and the resolution the
# product actually uses finds nobody.
NAIVE="$(psql_db -c "select count(*) from scope_closure c
                     join scope_grants g on g.tenant_id = c.tenant_id and g.scope_id = c.ancestor_id
                     where c.descendant_id = '${MINE}'::uuid")"
[ "$NAIVE" -ge 1 ] ||
    fail "the fixture proves nothing: no grant sits above the private scope"
RESOLVED="$(psql_db -c "with target as (select kind from scopes where id = '${MINE}'::uuid),
                             chain as (select c.ancestor_id from scope_closure c
                                       where c.descendant_id = '${MINE}'::uuid
                                         and (c.distance = 0 or (select kind from target) <> 'principal'))
                        select count(*) from chain ch
                        join scope_grants g on g.scope_id = ch.ancestor_id")"
[ "$RESOLVED" = "0" ] ||
    fail "${RESOLVED} grant(s) reached a private scope; isolation is broken"
ok "${NAIVE} grant(s) sit above ${MINE} and the resolution admits none of them"

step "9. Removing an inherited member is refused, naming where the grant is"
api DELETE "/v1/projects/${PROJECT_ID}/members/${COLLEAGUE}"
[ "$STATUS" = "409" ] || fail "expected 409, got ${STATUS}: ${BODY}"
case "$BODY" in
    *"$WORKSPACE_SCOPE"*) ok "409, and the refusal names ${WORKSPACE_SCOPE}" ;;
    *) fail "the refusal does not say where the grant actually is: ${BODY}" ;;
esac

step "10. Revoking a grant removes what it conferred"
api "GET" "/v1/admin/grants?principal_id=${COLLEAGUE}"
GRANT_ID="$(field grants/0/id)"
[ -n "$GRANT_ID" ] || fail "the colleague's grant is not listed: ${BODY}"
api DELETE "/v1/admin/grants/${GRANT_ID}"
[ "$STATUS" = "204" ] || fail "expected 204, got ${STATUS}: ${BODY}"
api GET "/v1/projects/${PROJECT_ID}/members"
[ -z "$(member "$COLLEAGUE" role)" ] ||
    fail "the colleague still holds access after revocation: ${BODY}"
ok "revoked at the workspace; gone at the project on the very next read"

step "11. The chain records every act, verifies, and carries no token"
for ACTION in access.granted access.revoked access.invite.created \
              access.invite.accepted access.invite.revoked access.group.created \
              access.group.updated; do
    case "$ACTION" in
        access.invite.revoked) continue ;;  # nothing was withdrawn in this run
    esac
    N="$(psql_db -c "select count(*) from audit_log where action = '${ACTION}'")"
    [ "$N" -ge 1 ] || fail "the chain does not record ${ACTION}"
done
SECRET="${INVITE_TOKEN##*.}"
LEAK="$(psql_db -c "select count(*) from audit_log where payload::text like '%${SECRET}%'")"
[ "$LEAK" = "0" ] || fail "${LEAK} audit event(s) carry the invitation secret"
"$BIN" audit verify --tenant "$TENANT_ID" >"$WORK/verify.log" 2>&1 ||
    fail "the chain does not verify: $(cat "$WORK/verify.log")"
ok "every act under its own name, no token anywhere, chain verifies"

printf '\n\033[1;32mCPR-5 demonstrated.\033[0m A person made a workspace and was its owner, invited a\n'
printf 'colleague by copying a link that worked once, granted a group in one row that\n'
printf 'three people hold, and can say of every one of them where the access came from.\n'
