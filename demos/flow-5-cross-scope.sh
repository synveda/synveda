#!/usr/bin/env sh
# FLOW-5 acceptance demo: cross-scope promotion (ADR-0034).
# AC (docs/backlog/FLOW-5.md): E2E of knowledge climbing two levels with
# distinct approver sets; denial at any level audited with reason.
#
# Flow: migrate -> admit a tenant -> acme > {eng > platform|payments,
# sales > field} -> approvers bound one per level (tara curator@platform,
# dana curator@eng + evan steward@eng, olive curator@org + owen
# steward@org) and two readers who never approve anything (pia@payments,
# sam@field) -> seed one runbook on the platform team's shelf -> THE
# BASELINE: pia and sam both inject, and neither block contains it ->
# HOP ONE, platform -> eng: ravi opens the climb (his own MemoryRead at
# the source is the disclosure, and it is on the audit event); TARA IS
# REFUSED — she curates the team the material came from and holds nothing
# at the department, because bindings inherit downward and never up;
# dana and evan approve (a department publication takes a curator AND a
# steward, two distinct people); evan cannot run the effect — steward
# reads no content in any pack — so dana does. PIA'S VERY NEXT INJECT
# CARRIES THE RUNBOOK, unmarked, sectioned under the department, and she
# still cannot read the team it lives at. Sam's does not -> HOP TWO,
# eng -> org, on material the department holds only by publishing it (the
# record never moved): dana opens; DANA IS REFUSED at the org for the
# same reason tara was at the department; olive REJECTS WITH A REASON,
# which is the denial the AC asks to see audited -> a new proposal (a
# revision is a new proposal), olive and owen approve, olive publishes,
# and SAM — in a different department entirely — receives it -> the trail:
# both climbs and the denial read off the hash chain with source, target,
# and reason, and the chain verifies -> and a sideways promotion is
# refused by name, because a climb goes up.
#
# Dev service identities are confined to their anchor subtree (ADR-0018
# decision 4), so every actor is registered at the org root and their
# authority comes from the role binding alone — which is the thing under
# test. The two readers are registered where they actually sit, because
# placement is what composition reads.
#
# On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8141
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=flow-5-demo-secret
export SYNVEDA_DEV_JWT_SECRET
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

BASE="http://127.0.0.1:8141"

cargo build -p synveda-gateway -p synveda-cli

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

emit() {
  printf '%s' "$1"
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

# api <token> <method> <path> [body]
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

# code <token> <method> <path> [body] — prints the HTTP code only.
code() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  curl -s -o /dev/null -w '%{http_code}' -X "$method" \
    -H "Authorization: Bearer $tok" -H "Content-Type: application/json" \
    ${body:+-d "$body"} "$BASE$path"
}

# why <token> <method> <path> [body] — prints the refusal's own words.
why() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  curl -s -X "$method" -H "Authorization: Bearer $tok" \
    -H "Content-Type: application/json" ${body:+-d "$body"} "$BASE$path" |
    node -e '
      let d = "";
      process.stdin.on("data", (c) => (d += c));
      process.stdin.on("end", () => {
        const b = JSON.parse(d);
        console.log(b.reason || b.message || d);
      });
    '
}

# reads <token> <session> — does this principal's own context block carry
# the runbook? Prints "yes" or "no".
reads() {
  api "$1" POST /v1/inject "{\"session_id\":\"$2\"}" |
    node -e '
      let d = "";
      process.stdin.on("data", (c) => (d += c));
      process.stdin.on("end", () => {
        const text = JSON.parse(d).text || "";
        console.log(text.includes("Rotate the signing key") ? "yes" : "no");
      });
    '
}

fail() {
  echo "demo FAILED: $1" >&2
  exit 1
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_id=$(./target/debug/synveda tenant create \
  --slug "flow5-demo-$$" --name "FLOW-5 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/flow5-gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    fail "gateway did not become healthy"
  fi
  sleep 1
done

echo "==> hierarchy: two departments, so 'the org' means more than 'eng'"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
node_under() {
  api "$admin_token" POST /v1/hierarchy/nodes \
    "{\"parent_id\":\"$1\",\"kind\":\"$2\",\"slug\":\"$3\",\"name\":\"$4\"}" | field id
}
eng_id=$(node_under "$org_id" department eng Engineering)
platform_id=$(node_under "$eng_id" team platform Platform)
payments_id=$(node_under "$eng_id" team payments Payments)
sales_id=$(node_under "$org_id" department sales Sales)
field_id=$(node_under "$sales_id" team field Field)
cat <<EOF
    acme
    ├── eng
    │   ├── platform   <- the runbook lives here, and never moves
    │   └── payments   <- pia reads; cannot read platform, ever
    └── sales
        └── field      <- sam reads; cannot read eng, ever
EOF

echo "==> approvers, one level each; readers where they actually sit"
for who in ravi tara dana evan olive owen; do
  ./target/debug/synveda service register --tenant "$tenant_id" \
    --subject "$who" --scope "$org_id" >/dev/null
done
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject pia --scope "$payments_id" >/dev/null
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject sam --scope "$field_id" >/dev/null

bind_role() {
  ./target/debug/synveda role bind --tenant "$tenant_id" \
    --subject "$1" --role "$2" --scope "$3" >/dev/null
}
bind_role ravi contributor "$eng_id"
bind_role tara curator "$platform_id"
bind_role dana curator "$eng_id"
bind_role evan steward "$eng_id"
bind_role olive curator "$org_id"
bind_role owen steward "$org_id"

for who in ravi tara dana evan olive owen pia sam; do
  eval "${who}_token=\$(./target/debug/synveda token issue --tenant \"$tenant_id\" --subject $who)"
done
echo "    platform: tara=curator          eng: dana=curator evan=steward"
echo "    org:      olive=curator owen=steward"
echo "    readers:  pia@payments  sam@field  (no roles anywhere)"

# The team's shelf, seeded directly: observe lands records at their
# *owner's* personal scope (ADR-0020 decision 3), and the feature under
# review is the climb, not the ingestion path.
ravi_identity=$(psql_t "select id from identities
                        where tenant_id = '$tenant_id' and subject = 'ravi'")
runbook_id=$(psql_t "select gen_random_uuid()")
psql_t "begin;
        insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                             content, sensitivity, provenance, valid_from)
        values ('$runbook_id', '$tenant_id', '$platform_id', '$ravi_identity',
                'derived', 'procedure',
                'Rotate the signing key every 90 days, on the first tuesday.',
                'internal', '{\"source\":\"flow-5 demo\"}', now() - interval '1 hour');
        insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
        values ('$runbook_id', '$tenant_id', 'hash@1', 4, '[0.25,0.25,0.25,0.25]');
        commit;" >/dev/null
echo "==> one runbook on the platform team's shelf:"
psql_c "select left(id::text, 8) as id, sensitivity, content
        from records where tenant_id = '$tenant_id';"

echo
echo "==> THE BASELINE — tribal knowledge, stuck"
[ "$(reads "$pia_token" flow-5-before)" = "no" ] || fail "pia already reads it"
[ "$(reads "$sam_token" flow-5-before)" = "no" ] || fail "sam already reads it"
echo "    pia (payments): no    sam (field): no"
echo "    two teams away from a runbook neither can find."

echo
echo "==> HOP ONE — platform -> eng, under Engineering's approvers"
opened=$(api "$ravi_token" POST /v1/proposals \
  "{\"scope_id\":\"$eng_id\",\"source_scope_id\":\"$platform_id\",
    \"record_ids\":[\"$runbook_id\"],
    \"title\":\"promote the key-rotation runbook to Engineering\"}")
hop1=$(emit "$opened" | field id)
echo "    proposal $hop1"
echo "    source:      $(emit "$opened" | field source_scope_id)"
echo "    target:      $(emit "$opened" | field target_scope_id)"
echo "    required:    $(emit "$opened" | field required roles)"
echo "    outstanding: $(emit "$opened" | field outstanding)"

echo "--> tara curates the team the material came FROM"
tara_code=$(code "$tara_token" POST "/v1/proposals/$hop1/approve")
[ "$tara_code" = "403" ] || fail "the team's curator approved at the department ($tara_code)"
echo "    403 — bindings inherit downward and never up, so the level below"
echo "          cannot approve the level above's publication"

echo "--> dana (curator@eng) approves"
after_dana=$(api "$dana_token" POST "/v1/proposals/$hop1/approve" '{}')
echo "    state: $(emit "$after_dana" | field state)  \
outstanding: $(emit "$after_dana" | field outstanding)"
[ "$(emit "$after_dana" | field state)" = "open" ] ||
  fail "a department publication is not one curator's to make"

echo "--> evan (steward@eng) approves — two distinct people, as the matrix says"
after_evan=$(api "$evan_token" POST "/v1/proposals/$hop1/approve" '{}')
[ "$(emit "$after_evan" | field state)" = "approved" ] || fail "the matrix is still unmet"
echo "    state: approved"

echo "--> evan tries to run the effect"
evan_code=$(code "$evan_token" POST "/v1/proposals/$hop1/publish")
[ "$evan_code" = "403" ] || fail "a steward published content it cannot read ($evan_code)"
echo "    403 — steward is an authority role that reads no content in any"
echo "          pack, and nobody publishes what they cannot read"

echo "--> dana publishes"
published=$(api "$dana_token" POST "/v1/proposals/$hop1/publish")
echo "    eng/memory/published now at $(emit "$published" | field commit)"
echo "    second parent (the review): $(emit "$published" | field proposal_commit)"

echo
echo "==> WHAT CHANGED, from the reader's side"
[ "$(reads "$pia_token" flow-5-mid)" = "yes" ] || fail "the climb changed nothing for pia"
[ "$(reads "$sam_token" flow-5-mid)" = "no" ] ||
  fail "a department publication reached another department"
echo "    pia (payments): YES   sam (field): no"
api "$pia_token" POST /v1/inject '{"session_id":"flow-5-show"}' | field text |
  sed 's/\\n/\n/g' | sed 's/^/    | /'
echo "    the record never moved: it still lives at platform, which pia"
echo "    cannot read. What admits it is Engineering's publication."
psql_c "select left(r.id::text, 8) as record, n.path as still_lives_at
        from records r join hierarchy_nodes n on n.id = r.scope_id
        where r.tenant_id = '$tenant_id';"

echo
echo "==> HOP TWO — eng -> org, on material eng holds only by publishing it"
opened2=$(api "$dana_token" POST /v1/proposals \
  "{\"scope_id\":\"$org_id\",\"source_scope_id\":\"$eng_id\",
    \"record_ids\":[\"$runbook_id\"],
    \"title\":\"promote the key-rotation runbook to ACME\"}")
hop2=$(emit "$opened2" | field id)
echo "    proposal $hop2 (source eng, and the record still lives at platform)"

echo "--> dana now curates the level BELOW the target"
dana_code=$(code "$dana_token" POST "/v1/proposals/$hop2/approve")
[ "$dana_code" = "403" ] || fail "the department's curator approved at the org ($dana_code)"
echo "    403 — the same rule that stopped tara, one level up"

echo "--> olive (curator@org) rejects, with a reason"
rejected=$(api "$olive_token" POST "/v1/proposals/$hop2/reject" \
  '{"reason":"org-wide runbooks must name the owning team"}')
[ "$(emit "$rejected" | field state)" = "rejected" ] || fail "the rejection did not close it"
echo "    state: rejected"
[ "$(reads "$sam_token" flow-5-rejected)" = "no" ] || fail "a rejected climb changed something"
echo "    sam still reads nothing — a denial is a denial"

echo "--> a revision is a new proposal (ADR-0032 decision 12)"
opened3=$(api "$dana_token" POST /v1/proposals \
  "{\"scope_id\":\"$org_id\",\"source_scope_id\":\"$eng_id\",
    \"record_ids\":[\"$runbook_id\"],
    \"title\":\"promote the key-rotation runbook to ACME (owner: platform)\"}")
hop2b=$(emit "$opened3" | field id)
api "$olive_token" POST "/v1/proposals/$hop2b/approve" '{}' >/dev/null
final=$(api "$owen_token" POST "/v1/proposals/$hop2b/approve" '{}')
[ "$(emit "$final" | field state)" = "approved" ] || fail "the org matrix is unmet"
echo "    olive + owen: approved — the org's own curator and its own steward"
api "$olive_token" POST "/v1/proposals/$hop2b/publish" >/dev/null
[ "$(reads "$sam_token" flow-5-after)" = "yes" ] || fail "the second climb changed nothing"
echo "    sam (field, a different department entirely): YES"

echo
echo "==> A CLIMB GOES UP — sideways is refused by name"
sideways=$(why "$ravi_token" POST /v1/proposals \
  "{\"scope_id\":\"$payments_id\",\"source_scope_id\":\"$platform_id\",
    \"record_ids\":[\"$runbook_id\"],\"title\":\"climb sideways\"}")
echo "    $sideways"
case "$sideways" in
  *"not an ancestor"*) : ;;
  *) fail "a sideways promotion was not refused by the direction rule" ;;
esac

echo
echo "==> THE TRAIL — two climbs and one denial, read off the hash chain"
psql_c "select a.action,
               src.path as from_scope,
               tgt.path as to_scope,
               coalesce(a.payload->'climb'->>'levels', '') as levels,
               coalesce(a.payload->>'reason', '') as reason
        from audit_log a
        left join hierarchy_nodes src on src.id = (a.payload->>'source_scope_id')::uuid
        left join hierarchy_nodes tgt on tgt.id = (a.payload->>'target_scope_id')::uuid
        where a.tenant_id = '$tenant_id'
          and a.action in ('vedaflow.proposal.opened', 'vedaflow.proposal.rejected',
                           'vedaflow.channel.published')
          and a.payload ? 'source_scope_id'
        order by a.seq;"
echo "    the denial carries its reason, and every act names both levels."
echo "    the proposer's read at the source is on the opened event too:"
psql_c "select src.path as read_at,
               a.payload->'climb'->'source_read'->>'pack' as decided_under,
               a.payload->'climb'->'source_read'->>'action' as decision
        from audit_log a
        left join hierarchy_nodes src on src.id = (a.payload->>'source_scope_id')::uuid
        where a.tenant_id = '$tenant_id'
          and a.action = 'vedaflow.proposal.opened'
          and a.payload->'climb' is not null
        order by a.seq;"

verified=$(./target/debug/synveda audit verify --tenant "$tenant_id")
echo "    $verified"
case "$verified" in
  *intact* | *valid* | *Valid*) : ;;
  *) fail "the audit chain did not verify: $verified" ;;
esac

echo
echo "FLOW-5 demo OK — a team's runbook reached the whole org in two"
echo "reviewed hops, each under that level's own approvers, with the"
echo "denial in between recorded with its reason. The record never moved."
