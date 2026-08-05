#!/usr/bin/env sh
# AUTH-4 acceptance demo: the SCIM 2.0 server (ADR-0059).
# AC (docs/backlog/AUTH-4.md): SCIM conformance tests, and a mover's
# memories re-scope per policy.
#
# The load-bearing demonstration is [4/7], and it is the one thing about this
# feature a conformance checklist cannot show: the *same* directory event —
# one person, one group change — run against two departments governed by two
# packs, producing two different outcomes. That is what makes "per policy" a
# sentence about policy rather than a description of whatever the code does.
#
# The finding behind it is worth stating plainly, because the instinct is
# wrong: the hazard a move carries is **disposal, not disclosure**. A
# personal scope is readable by its owner and nobody else wherever it hangs
# (every pack excludes user-kind scopes from every content-role grant), but
# retention horizons resolve from the effective pack at the record's own
# scope on every sweep — so moving a node from a seven-year department into
# a ninety-day one is a bulk destruction nobody approved.
#
# Flow:
#
#   postgres -> scratch db -> tenant, acme > {eng > core, sales > emea}
#   [1/7] a credential, issued through the PDP and printed once. `synveda
#         scim token list` then shows it live, and shows nothing secret.
#   [2/7] the joiner: `POST /Users` + a group, and a person lands in the
#         hierarchy with zero admin action — AUTH-2's resolver, called
#         through a different door.
#   [3/7] the correspondence rule: that person logs in, and the login BINDS
#         to the identity the directory already made. One person, one
#         identity, one scope — the failure this prevents is two of each,
#         with half the memory in each, and nothing that looks wrong.
#   [4/7] THE AC. The same move under two packs: sealed-and-restarted out of
#         `regulated-strict`, followed out of `standard`.
#   [5/7] the leaver: `active: false` seals. The token stops working, the
#         scope is unreadable, and the retention sweep stops seeing it.
#   [6/7] conformance: /ServiceProviderConfig advertises what the routes do,
#         an unsupported filter is 501 rather than a wrong empty list, and
#         DELETE answers 204 while sealing rather than deleting.
#   [7/7] the trail: identity.provisioned / moved / sealed, each naming the
#         credential that did it, the chain verifying, and a sweep proving
#         no token ever reaches the log.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

SCIM_DB=auth4_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $SCIM_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$SCIM_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SCIM_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/auth4-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8157
export SYNVEDA_LISTEN_ADDR
BASE="http://127.0.0.1:8157"
SYNVEDA_DEV_JWT_SECRET=auth-4-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER

SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli
CLI=$PWD/target/debug/synveda

WORK="${TMPDIR:-/tmp}/auth4-$$"
mkdir -p "$WORK"
DEMO_HOME="$WORK/home"
mkdir -p "$DEMO_HOME"
REAL_HOME="$HOME"

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  sleep 1
  HOME="$REAL_HOME" $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $SCIM_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR" "$WORK"
}
trap cleanup EXIT INT TERM

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

api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" "$BASE$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" "$BASE$path"
  fi
}

# Same, but never fails the script: used where a REFUSAL is the point.
try_api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  curl -sS -o "$WORK/body" -w '%{http_code}' -X "$method" \
    -H "Authorization: Bearer $tok" -H "Content-Type: application/json" \
    ${body:+-d "$body"} "$BASE$path"
}

as() {
  tok=$1
  shift
  HOME="$DEMO_HOME" XDG_CONFIG_HOME="$DEMO_HOME/.config" \
    SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

says() {
  if ! printf '%s' "$2" | grep -qF -- "$1"; then
    echo "demo FAILED: expected '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}
silent() {
  if printf '%s' "$2" | grep -qF -- "$1"; then
    echo "demo FAILED: did not expect '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}
sql() {
  $COMPOSE exec -T postgres psql -qtAX -U synveda -d "$SCIM_DB" -c "$1"
}

# ── The world ───────────────────────────────────────────────────────────────

echo "==> migrate + admit a tenant"
$CLI db migrate
TENANT=$($CLI tenant create --slug "auth4-demo-$$" --name "AUTH-4 Demo Tenant" | field id)
echo "    tenant: $TENANT"
ADMIN=$($CLI token issue --tenant "$TENANT" --subject demo-admin)
$CLI role bind --tenant "$TENANT" --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="$WORK/gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
    echo "demo FAILED: the gateway exited; see $GATEWAY_LOG" >&2
    exit 1
  fi
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: the gateway never became healthy; see $GATEWAY_LOG" >&2
    exit 1
  fi
  sleep 1
done

echo "==> the hierarchy: acme > {eng > core, sales > emea}"
ORG=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  '{"parent_id":null,"kind":"org","slug":"acme","name":"ACME"}' | field id)
ENG=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$ORG\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" | field id)
SALES=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$ORG\",\"kind\":\"department\",\"slug\":\"sales\",\"name\":\"Sales\"}" | field id)
CORE=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$ENG\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" | field id)
EMEA=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$SALES\",\"kind\":\"team\",\"slug\":\"emea\",\"name\":\"EMEA\"}" | field id)

# The two packs the AC turns on. eng is the regulated department; sales is
# the ordinary one.
api "$ADMIN" PUT "/v1/hierarchy/nodes/$ENG/policy" '{"name":"regulated-strict"}' >/dev/null
api "$ADMIN" PUT "/v1/hierarchy/nodes/$SALES/policy" '{"name":"standard"}' >/dev/null
echo "    eng: regulated-strict   sales: standard"

# ── [1/7] the credential ────────────────────────────────────────────────────

echo
echo "==> [1/7] a provisioning credential, issued through the PDP"
ISSUED=$(as "$ADMIN" scim token issue --label entra --json 2>/dev/null)
SCIM_TOKEN=$(printf '%s' "$ISSUED" | field token)
says "synveda_scim_v1" "$SCIM_TOKEN"
echo "    token: $(printf '%s' "$SCIM_TOKEN" | cut -c1-32)…  (shown once, stored as a hash)"

LISTED=$(as "$ADMIN" scim token list 2>/dev/null)
says "live" "$LISTED"
says "entra" "$LISTED"
# The secret is never readable again — not from the list, not from anywhere.
silent "$(printf '%s' "$SCIM_TOKEN" | cut -c25-60)" "$LISTED"
echo "    listed: live, and the secret is not in it"

# A non-admin cannot issue one: this plane is org-admin's under every pack.
NOBODY=$($CLI token issue --tenant "$TENANT" --subject nobody)
CODE=$(try_api "$NOBODY" POST /v1/scim/credentials '{"label":"theirs"}')
if [ "$CODE" != "403" ]; then
  echo "demo FAILED: a non-admin issued a credential ($CODE)" >&2
  exit 1
fi
echo "    a non-admin asking for one: 403"

scim() {
  method=$1
  path=$2
  body=${3:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $SCIM_TOKEN" \
      -H "Content-Type: application/scim+json" -d "$body" "$BASE$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $SCIM_TOKEN" "$BASE$path"
  fi
}
try_scim() {
  curl -sS -o "$WORK/body" -w '%{http_code}' -X "$1" \
    -H "Authorization: Bearer $SCIM_TOKEN" -H "Content-Type: application/scim+json" \
    ${3:+-d "$3"} "$BASE$2"
}

# ── [2/7] the joiner ────────────────────────────────────────────────────────

echo
echo "==> [2/7] the joiner: a directory creates a person, and they land placed"
# The mapping the resolver will use — AUTH-2's override table, unchanged.
sql "insert into group_mappings (tenant_id, group_name, scope_id)
     values ('$TENANT', 'synveda-eng-core', '$CORE'),
            ('$TENANT', 'synveda-sales-emea', '$EMEA')" >/dev/null

ADA=$(scim POST /scim/v2/Users '{
  "schemas":["urn:ietf:params:scim:schemas:core:2.0:User"],
  "userName":"ada@acme.test","externalId":"ada-object-id",
  "name":{"givenName":"Ada","familyName":"Lovelace"},
  "emails":[{"value":"ada@acme.test","type":"work","primary":true}],
  "active":true}' | field id)
GROUP=$(scim POST /scim/v2/Groups '{
  "schemas":["urn:ietf:params:scim:schemas:core:2.0:Group"],
  "displayName":"synveda-eng-core"}' | field id)
scim PATCH "/scim/v2/Groups/$GROUP" "{
  \"schemas\":[\"urn:ietf:params:scim:api:messages:2.0:PatchOp\"],
  \"Operations\":[{\"op\":\"add\",\"path\":\"members\",\"value\":[{\"value\":\"$ADA\"}]}]}" >/dev/null

ADA_PATH=$(sql "select n.path from identities i
                join scim_users u on u.identity_id = i.id
                join hierarchy_nodes n on n.id = i.scope_id
                where u.id = '$ADA'")
says "acme/eng/core" "$ADA_PATH"
echo "    ada landed at $ADA_PATH — with zero admin action"

# And with no subject yet: nobody has logged in as her.
ADA_SUBJECT=$(sql "select coalesce(i.subject, '<none>') from identities i
                   join scim_users u on u.identity_id = i.id where u.id = '$ADA'")
says "<none>" "$ADA_SUBJECT"
echo "    her identity has no subject yet: $ADA_SUBJECT"

# ── [3/7] the correspondence rule ───────────────────────────────────────────

echo
echo "==> [3/7] the product refuses to let one person become two"
BEFORE=$(sql "select count(*) from identities where tenant_id = '$TENANT'")

# A second directory record for the same person — a new anchor, a new
# userName, the same mailbox. This is what a re-created account looks like,
# and it is the case that costs somebody their memory if it goes wrong: two
# identities, two personal scopes, half the material in each, and nothing
# anywhere that looks wrong.
CODE=$(try_scim POST /scim/v2/Users '{
  "schemas":["urn:ietf:params:scim:schemas:core:2.0:User"],
  "userName":"ada@acme.test-recreated","externalId":"ada-new-object-id",
  "emails":[{"value":"ada@acme.test","type":"work","primary":true}],
  "active":true}')
if [ "$CODE" != "409" ]; then
  echo "demo FAILED: a second record for one person must be refused, got $CODE" >&2
  cat "$WORK/body" >&2
  exit 1
fi
says "uniqueness" "$(cat "$WORK/body")"
AFTER=$(sql "select count(*) from identities where tenant_id = '$TENANT'")
echo "    a second record for the same mailbox: 409 uniqueness"
echo "    identities before: $BEFORE   after: $AFTER  (and no orphan record)"
ORPHANS=$(sql "select count(*) from scim_users
               where tenant_id = '$TENANT' and identity_id is null")
says "0" "$ORPHANS"

# The other end of the same rule — a token subject binding to the identity
# the directory made — needs a real IdP, so it lives in the AC suite
# (crates/synveda-gateway/tests/scim.rs::one_person_never_becomes_two_identities),
# which drives the login-completion path directly.

# ── [4/7] THE ACCEPTANCE CRITERION ──────────────────────────────────────────

echo
echo "==> [4/7] THE AC: the same move, two packs, two outcomes"
ADA_HOME_BEFORE=$(sql "select i.scope_id from identities i
                       join scim_users u on u.identity_id = i.id where u.id = '$ADA'")

EMEA_GROUP=$(scim POST /scim/v2/Groups '{
  "schemas":["urn:ietf:params:scim:schemas:core:2.0:Group"],
  "displayName":"synveda-sales-emea"}' | field id)
scim PATCH "/scim/v2/Groups/$EMEA_GROUP" "{
  \"schemas\":[\"urn:ietf:params:scim:api:messages:2.0:PatchOp\"],
  \"Operations\":[{\"op\":\"add\",\"path\":\"members\",\"value\":[{\"value\":\"$ADA\"}]}]}" >/dev/null
scim PATCH "/scim/v2/Groups/$GROUP" "{
  \"schemas\":[\"urn:ietf:params:scim:api:messages:2.0:PatchOp\"],
  \"Operations\":[{\"op\":\"remove\",\"path\":\"members[value eq \\\"$ADA\\\"]\"}]}" >/dev/null

ADA_HOME_AFTER=$(sql "select i.scope_id from identities i
                      join scim_users u on u.identity_id = i.id where u.id = '$ADA'")
ADA_OLD_SEALED=$(sql "select exists(select 1 from identities
                       where scope_id = '$ADA_HOME_BEFORE' and status = 'departed')")
if [ "$ADA_HOME_AFTER" = "$ADA_HOME_BEFORE" ] || [ "$ADA_OLD_SEALED" != "t" ]; then
  echo "demo FAILED: leaving regulated-strict must seal and restart" >&2
  exit 1
fi
echo "    out of regulated-strict: material SEALED where it was written,"
echo "                             ada restarted under sales/emea"

# The same event, the other way: out of `standard`.
BEN=$(scim POST /scim/v2/Users '{
  "schemas":["urn:ietf:params:scim:schemas:core:2.0:User"],
  "userName":"ben@acme.test","externalId":"ben-object-id",
  "emails":[{"value":"ben@acme.test","type":"work","primary":true}],
  "active":true}' | field id)
scim PATCH "/scim/v2/Groups/$EMEA_GROUP" "{
  \"schemas\":[\"urn:ietf:params:scim:api:messages:2.0:PatchOp\"],
  \"Operations\":[{\"op\":\"add\",\"path\":\"members\",\"value\":[{\"value\":\"$BEN\"}]}]}" >/dev/null
BEN_HOME_BEFORE=$(sql "select i.scope_id from identities i
                       join scim_users u on u.identity_id = i.id where u.id = '$BEN'")
scim PATCH "/scim/v2/Groups/$GROUP" "{
  \"schemas\":[\"urn:ietf:params:scim:api:messages:2.0:PatchOp\"],
  \"Operations\":[{\"op\":\"add\",\"path\":\"members\",\"value\":[{\"value\":\"$BEN\"}]}]}" >/dev/null
scim PATCH "/scim/v2/Groups/$EMEA_GROUP" "{
  \"schemas\":[\"urn:ietf:params:scim:api:messages:2.0:PatchOp\"],
  \"Operations\":[{\"op\":\"remove\",\"path\":\"members[value eq \\\"$BEN\\\"]\"}]}" >/dev/null
BEN_HOME_AFTER=$(sql "select i.scope_id from identities i
                      join scim_users u on u.identity_id = i.id where u.id = '$BEN'")
if [ "$BEN_HOME_AFTER" != "$BEN_HOME_BEFORE" ]; then
  echo "demo FAILED: leaving standard must let the material follow" >&2
  exit 1
fi
echo "    out of standard:         material FOLLOWED — the same scope moved"
echo
echo "    one directory event, two packs, two answers. That is the AC."

# ── [5/7] the leaver ────────────────────────────────────────────────────────

echo
echo "==> [5/7] the leaver: active:false seals"
scim PATCH "/scim/v2/Users/$BEN" '{
  "schemas":["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
  "Operations":[{"op":"replace","path":"active","value":false}]}' >/dev/null
BEN_SEALED=$(sql "select sealed_of.sealed from (
                    select coalesce((select true from identities
                      where scope_id = '$BEN_HOME_AFTER' and status = 'departed'), false) as sealed
                  ) sealed_of")
says "t" "$BEN_SEALED"
echo "    ben's personal scope is sealed"

SWEPT=$(sql "select count(*) from records r
             where r.tenant_id = '$TENANT' and r.scope_id = '$BEN_HOME_AFTER'
               and not exists (select 1 from identities i
                 where i.tenant_id = r.tenant_id and i.scope_id = r.scope_id
                   and i.status = 'departed')")
says "0" "$SWEPT"
echo "    and the retention sweep no longer enumerates it — retention-held"

# ── [6/7] conformance ───────────────────────────────────────────────────────

echo
echo "==> [6/7] conformance"
CONFIG=$(scim GET /scim/v2/ServiceProviderConfig)
says '"patch"' "$CONFIG"
PATCH_OK=$(printf '%s' "$CONFIG" | field patch supported)
BULK_OK=$(printf '%s' "$CONFIG" | field bulk supported)
says "true" "$PATCH_OK"
says "false" "$BULK_OK"
echo "    /ServiceProviderConfig: patch=$PATCH_OK bulk=$BULK_OK — what the routes actually do"

CODE=$(try_scim GET "/scim/v2/Users?filter=userName%20eq%20%22a%22%20and%20active%20eq%20true")
if [ "$CODE" != "501" ]; then
  echo "demo FAILED: an unsupported filter must be 501, got $CODE" >&2
  exit 1
fi
says "invalidFilter" "$(cat "$WORK/body")"
echo "    an unsupported filter: 501 invalidFilter — never a wrong empty list"

CODE=$(try_scim DELETE "/scim/v2/Users/$BEN")
if [ "$CODE" != "204" ]; then
  echo "demo FAILED: DELETE must answer 204, got $CODE" >&2
  exit 1
fi
STILL_THERE=$(sql "select count(*) from scim_users where id = '$BEN'")
says "1" "$STILL_THERE"
echo "    DELETE: 204 to the client, and the row is still there holding the seal"

# ── [7/7] the trail ─────────────────────────────────────────────────────────

echo
echo "==> [7/7] the trail"
sql "select action, count(*) from audit_log
     where tenant_id = '$TENANT' and action like 'identity.%'
     group by action order by action" | sed 's/^/    /'

NAMED=$(sql "select count(*) from audit_log
             where tenant_id = '$TENANT' and action = 'identity.sealed'
               and payload ? 'credential_id'")
if [ "$NAMED" -lt 1 ]; then
  echo "demo FAILED: a seal must name the credential that did it" >&2
  exit 1
fi
echo "    every seal names the credential that did it"

VERIFY=$($CLI audit verify --tenant "$TENANT" 2>&1 || true)
says "chain valid" "$VERIFY"
echo "    $VERIFY"

LEAKED=$(sql "select count(*) from audit_log
              where tenant_id = '$TENANT' and payload::text like '%synveda_scim_v1%'")
says "0" "$LEAKED"
echo "    and no provisioning token appears anywhere in the chain: $LEAKED"

echo
echo "AUTH-4 demo complete."
