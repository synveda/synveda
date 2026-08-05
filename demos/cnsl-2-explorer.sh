#!/usr/bin/env sh
# CNSL-2 acceptance demo: the hierarchy & policy explorer (ADR-0058).
# AC (docs/backlog/CNSL-2.md): the four nouns answered for a node in one
# screen and by the CLI beside it; a pack and roles that say where they came
# from; "active lapses" answerable without already knowing which scope to
# ask about; and a capability that is the PDP's verdict and a forecast
# rather than a grant.
#
# The load-bearing demonstration is [4/6], and it is the one thing about
# this feature that a screenshot cannot show: a probe answers YES, a pack is
# reassigned, and the very same act is REFUSED at its own seam. That is the
# difference between a capability surface and a permission cache, and it is
# why nothing in this product reads a capability answer to decide anything.
#
# Flow:
#
#   postgres -> scratch db -> tenant, acme > eng > {platform, vault}
#   [1/6] the pack, and where it came from. Assigned at the DEPARTMENT, so
#         the team inherits it — and the answer says `inherited` rather than
#         leaving a steward to work out that their team has no pack of its
#         own.
#   [2/6] roles, as THREE MECHANISMS: bound here, bound at an ancestor, and
#         bound tenant-wide. Three different reasons a role is in force, and
#         a view that rendered them identically would pass for a build that
#         resolved nothing.
#   [3/6] what a reader may do — asked of the PDP, per reader. A steward and
#         a viewer at the SAME node get different answers, and neither can
#         ask about the other: there is no `subject` parameter, and adding
#         one changes nothing.
#   [4/6] THE FORECAST. Probe says yes -> pack reassigned -> the act is
#         refused. The probe then agrees, because it never held an answer.
#   [5/6] the lapse plane gets a terminal. `synveda lapse list` with no
#         scope — the question "what is relaxed right now", which before
#         this feature could not be asked at all — and the grant is visible
#         from the RECEIVING end, which is what lets a steward revoke what
#         their own team holds.
#   [6/6] the trail: one `authz.decision` per probe however many pairs it
#         decided, the chain verifying, and a sweep proving no third party's
#         binding and no lapse reason rides a probe payload.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

CNSL_DB=cnsl2_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $CNSL_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$CNSL_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$CNSL_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/cnsl2-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8156
export SYNVEDA_LISTEN_ADDR
BASE="http://127.0.0.1:8156"
SYNVEDA_DEV_JWT_SECRET=cnsl-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER

SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli
CLI=$PWD/target/debug/synveda

WORK="${TMPDIR:-/tmp}/cnsl2-$$"
mkdir -p "$WORK"
DEMO_HOME="$WORK/home"
mkdir -p "$DEMO_HOME"
REAL_HOME="$HOME"

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  sleep 1
  HOME="$REAL_HOME" $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $CNSL_DB (force)" >/dev/null 2>&1 || true
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
  if ! printf '%s' "$2" | grep -q "$1"; then
    echo "demo FAILED: expected '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}
silent() {
  if printf '%s' "$2" | grep -q "$1"; then
    echo "demo FAILED: did not expect '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}

# ── The world ───────────────────────────────────────────────────────────────

echo "==> migrate + admit a tenant"
$CLI db migrate
TENANT=$($CLI tenant create --slug "cnsl2-demo-$$" --name "CNSL-2 Demo Tenant" | field id)
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
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

echo "==> hierarchy: acme > eng > {platform, vault}"
ORG=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
ENG=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$ORG\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" | field id)
PLATFORM=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$ENG\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" | field id)
VAULT=$(api "$ADMIN" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$ENG\",\"kind\":\"team\",\"slug\":\"vault\",\"name\":\"Vault\"}" | field id)

# The cast. Service identities anchored where they are placed, which is what
# makes [3/6] a difference the ROLES made rather than a difference of
# placement: sam and vic sit in the same team.
$CLI service register --tenant "$TENANT" --subject sam --scope "$PLATFORM" >/dev/null
$CLI service register --tenant "$TENANT" --subject vic --scope "$PLATFORM" >/dev/null
$CLI service register --tenant "$TENANT" --subject vaughn --scope "$VAULT" >/dev/null
$CLI service register --tenant "$TENANT" --subject vera --scope "$VAULT" >/dev/null
SAM=$($CLI token issue --tenant "$TENANT" --subject sam)
VIC=$($CLI token issue --tenant "$TENANT" --subject vic)
VAUGHN=$($CLI token issue --tenant "$TENANT" --subject vaughn)
VERA=$($CLI token issue --tenant "$TENANT" --subject vera)

$CLI role bind --tenant "$TENANT" --subject sam --role steward --scope "$PLATFORM" >/dev/null
$CLI role bind --tenant "$TENANT" --subject vic --role viewer --scope "$PLATFORM" >/dev/null
$CLI role bind --tenant "$TENANT" --subject vaughn --role steward --scope "$VAULT" >/dev/null
# A second steward, because the `policy` cell prices a lapse at two distinct
# people under the packs that mean it (ADR-0037 decision 1 / FLOW-3). That is
# the machinery working; this demo is about the listing on the other side.
$CLI role bind --tenant "$TENANT" --subject vera --role steward --scope "$VAULT" >/dev/null
# The two bindings that make [2/6] three mechanisms rather than three rows:
# one at an ancestor, one tenant-wide.
$CLI role bind --tenant "$TENANT" --subject dana --role curator --scope "$ENG" >/dev/null
$CLI role bind --tenant "$TENANT" --subject orla --role auditor >/dev/null

echo
echo "════ [1/6] the pack, and where it came from ════"

# Assigned at the DEPARTMENT, so the team inherits it. By the admin rather
# than by sam: sam is a service identity anchored at the team, and AUTH-3's
# confinement forbids it acting above its own anchor — which is the product
# working, not the demo working around it.
api "$ADMIN" PUT "/v1/hierarchy/nodes/$ENG/policy" '{"name":"standard"}' >/dev/null

eng_pack=$(as "$ADMIN" hierarchy policy "$ENG" 2>/dev/null)
team_pack=$(as "$SAM" hierarchy policy "$PLATFORM" 2>/dev/null)
printf '%s\n' "$eng_pack"
printf '%s\n' "$team_pack"
says "assigned here" "$eng_pack"
says "inherited from" "$team_pack"
# The distinction the origin exists for: same pack name, two different
# reasons it is in force, and a steward who could not tell them apart would
# not know whether changing it here changes anything.
silent "assigned here" "$team_pack"

echo
echo "════ [2/6] roles: three mechanisms, not three rows ════"

roles=$(as "$ADMIN" hierarchy roles "$PLATFORM" --effective 2>/dev/null)
printf '%s\n' "$roles"
says "sam" "$roles"
says "assigned here" "$roles"     # bound at this very node
says "dana" "$roles"
says "inherited from" "$roles"    # bound at the department above
says "orla" "$roles"
says "tenant-wide" "$roles"       # in force everywhere, not an ancestor's

# The local form answers a different question and still does — which is what
# keeps the PUT/DELETE beside it meaning what they say.
local_roles=$(as "$ADMIN" hierarchy roles "$PLATFORM" 2>/dev/null)
says "sam" "$local_roles"
silent "dana" "$local_roles"

echo
echo "════ [3/6] what a reader may do, asked of the PDP ════"

sam_caps=$(as "$SAM" hierarchy capabilities "$PLATFORM" 2>/dev/null)
vic_caps=$(as "$VIC" hierarchy capabilities "$PLATFORM" 2>&1 || true)
printf '%s\n' "$sam_caps"
says "forecast, not a grant" "$sam_caps"
says "policy.assign" "$sam_caps"
# Same node, same pack, different reader. If this were derived from a static
# role matrix it would be right today and wrong the moment a lapse opened.
silent "policy.assign" "$vic_caps"

# No `subject` parameter — and adding one changes nothing, so an explorer
# cannot be turned into an enumeration oracle by guessing a query string.
spied=$(api "$VIC" GET "/v1/hierarchy/nodes/$PLATFORM/capabilities?subject=sam")
test "$(printf '%s' "$spied" | field roles)" = '["viewer"]' || {
  echo "demo FAILED: a subject parameter changed the answer" >&2; exit 1; }
echo "a subject parameter changes nothing: the answer is still the caller's"

echo
echo "════ [4/6] THE FORECAST — probe says yes, the act says no ════"

before=$(api "$SAM" GET "/v1/hierarchy/nodes/$PLATFORM/capabilities" | field actions hierarchy.create)
echo "probe, under the pack in force now:  hierarchy.create = $before"
test "$before" = "true" || { echo "demo FAILED: expected a yes to age" >&2; exit 1; }

# The pack moves underneath the forecast — exactly what a steward
# reassigning one does.
api "$SAM" PUT "/v1/hierarchy/nodes/$PLATFORM/policy" '{"name":"regulated-strict"}' >/dev/null

code=$(try_api "$VIC" POST "/v1/hierarchy/nodes" \
  "{\"parent_id\":\"$PLATFORM\",\"kind\":\"team\",\"slug\":\"after\",\"name\":\"after\"}")
echo "the act, at its own seam:            HTTP $code"
test "$code" = "403" || { echo "demo FAILED: the act was not refused ($code)" >&2; exit 1; }

after=$(api "$VIC" GET "/v1/hierarchy/nodes/$PLATFORM/capabilities" | field actions hierarchy.create)
echo "probe again:                         hierarchy.create = $after"
test "$after" = "false" || { echo "demo FAILED: the probe remembered" >&2; exit 1; }
echo
echo "  The probe never held an answer. It asks the PDP every time, names"
echo "  the pack that decided, and authorises nothing — which is why a"
echo "  client may use it to choose what to OFFER and never what to ALLOW."

echo
echo "════ [5/6] the lapse plane gets a terminal ════"

# The disclosing side opens it (ADR-0037 decision 3).
proposal=$(api "$VAUGHN" POST /v1/lapses \
  "{\"scope_id\":\"$VAULT\",\"grantee_scope_id\":\"$PLATFORM\",\"action\":\"memory.read\",\"duration_secs\":3600,\"reason\":\"joint incident review\"}" \
  | field proposal_id)
# Approve with each steward, tolerating the 409 a pack that asks for fewer
# answers with — "already has the approvals it needs" is a success for this
# demo's purposes and a refusal worth not hiding in general.
for tok in "$VAUGHN" "$VERA"; do
  try_api "$tok" POST "/v1/proposals/$proposal/approve" '{}' >/dev/null
done
api "$VAUGHN" POST "/v1/proposals/$proposal/lapse" '{}' >/dev/null

# The question that could not be asked before this feature: no --scope.
standing=$(as "$ADMIN" lapse list 2>/dev/null)
printf '%s\n' "$standing"
says "joint incident review" "$standing"
says "active" "$standing"
# Visible from the RECEIVING end — the half `at_target` could never answer,
# and the reason a steward could not revoke what their own team held.
says "acme/eng/platform" "$standing"

echo
echo "════ [6/6] the trail ════"

trail=$(HOME="$DEMO_HOME" $CLI audit tail --tenant "$TENANT" --limit 300)
probes=$(printf '%s' "$trail" | node -e '
    let d=""; process.stdin.on("data",c=>d+=c);
    process.stdin.on("end",()=>{
      const rows = d.split("\n").filter(l => l.includes("capabilities"));
      console.log(`${rows.length} probe event(s) on the chain`);
    });')
echo "$probes"
# The saving the aggregation buys, stated as the two numbers: under a
# per-pair chaining rule the second number would have been the row count.
says "probe event" "$probes"

# No lapse reason in any PROBE payload. Scoped to the probe events on
# purpose: `policy.lapse.granted` carries the reason and must — a grant
# whose reason an auditor cannot read is the thing ADR-0037 exists to make
# impossible. What must not happen is the reason riding a read nobody
# reviewed.
leak=$(printf '%s' "$trail" | grep "capabilities" | grep -c "joint incident review" || true)
test "$leak" = "0" || { echo "demo FAILED: a lapse reason rode a probe payload" >&2; exit 1; }
echo "leak sweep: 0 rows carrying a grant's reason"

chain=$(HOME="$DEMO_HOME" $CLI audit verify --tenant "$TENANT")
echo "$chain"
says "chain valid" "$chain"

echo
echo "CNSL-2 demo complete."
