#!/usr/bin/env sh
# AUTHZ-5 acceptance demo: ABAC conditions — sensitivity as a policy
# attribute (ADR-0038).
# AC (docs/backlog/AUTHZ-5.md): `restricted` records never injected without
# compliance-granted permission, proven by leak-test suite.
#
# The claim is made from the reader's side, and the tier is *earned* rather
# than seeded: a record becomes `restricted` here the only way anything can
# — a classification proposal whose requirement the invariant approval floor
# priced at the compliance role plus two distinct approvers. Nothing else in
# the product can mint that tier. A model that asks for it gets the one
# below (ADR-0038 decision 8), and this demo shows the refusal.
#
# Then: Priya on payments receives nothing of platform's restricted material
# under any grant that did not *declare* the tier — including a lapse two
# stewards approved — and receives it under one that did, which cost a
# compliance signature nobody wrote a rule for. It is the invariant floor,
# reached from the read side.
#
# **The expiry here is real**, as in the AUTHZ-4 demo: the window is a
# handful of seconds and this script waits it out.
#
# Flow: migrate -> tenant -> acme/eng/{platform,payments} -> principals ->
# the tier is EARNED (one curator refused, compliance completes it) ->
# published through the same floor -> BEFORE (priya reads nothing) -> a
# WORKING-TIER lapse (still nothing: a grant is not a door to a tier it did
# not declare) -> the DECLARED grant (two stewards are no longer enough) ->
# DURING (marked twice: the line restricted, the section lapsed) -> EXPIRY
# (nobody acts) -> THE TRAIL -> THE REFUSALS in the product's own words.
#
# Needs postgres only; the gateway runs in-process here and principals carry
# dev tokens through SYNVEDA_TOKEN. On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

WINDOW_SECS=10

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8151
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=authz-5-demo-secret
export SYNVEDA_DEV_JWT_SECRET
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LAPSE_SWEEP_SECS=2
export SYNVEDA_LAPSE_SWEEP_SECS

BASE="http://127.0.0.1:8151"
CLI=./target/debug/synveda

CEREMONY="the vault break-glass ceremony needs two custodians and the offline shard"
ROTA="deploys go out on tuesdays after the smoke suite passes"

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
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

api() {
  tok=$1; method=$2; path=$3; body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" "$BASE$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" "$BASE$path"
  fi
}

# refused_api <token> <method> <path> <body> — a call that must fail, with
# the refusal printed in the product's own words.
refused_api() {
  tok=$1; method=$2; path=$3; body=${4:-}
  if out=$(curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" ${body:+-d "$body"} "$BASE$path" 2>&1); then
    echo "demo FAILED: $method $path should have been refused, got:" >&2
    echo "$out" >&2
    exit 1
  fi
  curl -sS -X "$method" -H "Authorization: Bearer $tok" \
    -H "Content-Type: application/json" ${body:+-d "$body"} "$BASE$path" 2>/dev/null |
    node -e '
      let d = "";
      process.stdin.on("data", (c) => (d += c));
      process.stdin.on("end", () => {
        try {
          const e = JSON.parse(d);
          console.log("    " + (e.message || e.reason || d));
        } catch { console.log("    " + d); }
      });
    '
}

session() {
  api "$1" POST /v1/inject '{"session_id":"authz-5-demo","task":"vault ceremony custodians"}'
}

# arrival <token> <line> — how that line reached this reader, in one word:
# `lapsed` (present, in a section the block marks as a grant), `reviewed`
# (present, on their own chain), or `absent`. Also prints the tier marker
# the line carries, which is the half AUTHZ-5 adds.
arrival() {
  session "$1" | node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const text = JSON.parse(d).text;
      const needle = process.argv[1];
      if (!text.includes(needle)) return console.log("absent");
      let section = "";
      let line = "";
      for (const l of text.split("\n")) {
        if (l.startsWith("## ")) section = l;
        if (l.includes(needle)) { line = l; break; }
      }
      const how = section.includes("[lapse]") ? "lapsed" : "reviewed";
      const tier = (line.match(/\[(restricted|confidential)\]/) || [])[1];
      console.log(tier ? how + " (" + tier + ")" : how);
    });
  ' "$2"
}

echo "==> migrate + admit a tenant"
$CLI db migrate
tenant_id=$($CLI tenant create \
  --slug "authz5-demo-$$" --name "AUTHZ-5 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$($CLI token issue --tenant "$tenant_id" --subject demo-admin)
$CLI role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/authz5-gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
    echo "demo FAILED: the gateway exited; see $GATEWAY_LOG" >&2
    echo "  (is another demo's gateway already on $SYNVEDA_LISTEN_ADDR?)" >&2
    exit 1
  fi
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

echo "==> hierarchy: acme > eng > {platform, payments}"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
platform_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
payments_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}" |
  field id)

seed_user() {
  uid=$(psql_t "select gen_random_uuid()")
  leaf=$(psql_t "select gen_random_uuid()")
  psql_t "begin;
          insert into hierarchy_nodes (id, tenant_id, parent_id, kind, slug, name, depth, path)
          select '$leaf'::uuid, '$tenant_id'::uuid, '$2'::uuid, 'user', 'u-$1', '$1',
                 n.depth + 1, n.path || '/u-$1'
          from hierarchy_nodes n where n.id = '$2';
          insert into hierarchy_closure (tenant_id, ancestor_id, descendant_id, distance)
          select '$tenant_id'::uuid, c.ancestor_id, '$leaf'::uuid, c.distance + 1
          from hierarchy_closure c where c.descendant_id = '$2'
          union all select '$tenant_id'::uuid, '$leaf'::uuid, '$leaf'::uuid, 0;
          insert into identities (id, tenant_id, subject, scope_id, kind)
          values ('$uid', '$tenant_id', '$1', '$leaf', 'user');
          commit;" >/dev/null
}

echo "==> principals"
# Cleo holds compliance *and* a content role: approving a restricted
# publication means reading it, and the effect asks its own read. The
# separation that matters is between compliance and the stewards who open
# grants — not between compliance and the material it signs for.
seed_user cara "$platform_id"
seed_user cleo "$platform_id"
seed_user sam "$platform_id"
seed_user nadia "$eng_id"
seed_user omar "$eng_id"
seed_user priya "$payments_id"
$CLI role bind --tenant "$tenant_id" --subject cara --role curator --scope "$platform_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject cleo --role compliance --scope "$platform_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject cleo --role curator --scope "$platform_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject nadia --role steward --scope "$eng_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject omar --role steward --scope "$eng_id" >/dev/null
cara_token=$($CLI token issue --tenant "$tenant_id" --subject cara)
cleo_token=$($CLI token issue --tenant "$tenant_id" --subject cleo)
nadia_token=$($CLI token issue --tenant "$tenant_id" --subject nadia)
omar_token=$($CLI token issue --tenant "$tenant_id" --subject omar)
priya_token=$($CLI token issue --tenant "$tenant_id" --subject priya)
echo "    cara=curator@platform  cleo=compliance+curator@platform  nadia,omar=steward@eng"
echo "    the reader: priya@payments"

sam_identity=$(psql_t "select id from identities
                       where tenant_id = '$tenant_id' and subject = 'sam'")
seed_record() {
  rid=$(psql_t "select gen_random_uuid()")
  psql_t "begin;
          insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                               content, sensitivity, provenance, valid_from)
          values ('$rid', '$tenant_id', '$platform_id', '$sam_identity', 'derived',
                  'procedure', \$content\$$1\$content\$, '$2',
                  '{\"source\":\"authz-5 demo\"}', now() - interval '1 hour');
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          values ('$rid', '$tenant_id', 'hash@1', 16,
                  '[0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25,0.25]');
          commit;" >/dev/null
  echo "$rid"
}

echo "==> platform's shelf: two lines, both ordinary to begin with"
ceremony_id=$(seed_record "$CEREMONY" internal)
rota_id=$(seed_record "$ROTA" internal)
echo "    both at 'internal' — the tier a pipeline may assign, and the"
echo "    highest one a model may ask for (ADR-0038 decision 8)"

echo
echo "==> THE TIER IS EARNED — a classification is a proposal, and the"
echo "    invariant floor prices the top one in compliance approvals"
classification=$(api "$cara_token" POST /v1/proposals \
  "{\"scope_id\":\"$platform_id\",\"record_ids\":[\"$ceremony_id\"],
    \"title\":\"classify the break-glass ceremony as restricted\",
    \"effect\":\"classify\",\"sensitivity\":\"restricted\"}")
classification_id=$(echo "$classification" | field id)
echo "    proposal $classification_id needs: $(echo "$classification" | field outstanding)"

api "$cara_token" POST "/v1/proposals/$classification_id/approve" '{}' >/dev/null
echo "--> cara approves, and runs the effect anyway. One curator is not a"
echo "    compliance decision:"
refused_api "$cara_token" POST "/v1/proposals/$classification_id/classify" '{}'

api "$cleo_token" POST "/v1/proposals/$classification_id/approve" '{}' >/dev/null
classified=$(api "$cara_token" POST "/v1/proposals/$classification_id/classify" '{}')
echo "--> compliance signs, and the effect runs:"
echo "    $(echo "$classified" | field records 0 record_id) : \
$(echo "$classified" | field records 0 was) -> $(echo "$classified" | field sensitivity)"
echo "    and the tier it left is still readable in history, which is what"
echo "    makes 'what did this carry in March' an answerable question:"
psql_t "select 'records: ' || sensitivity from records where id = '$ceremony_id'
        union all
        select 'history: ' || sensitivity from records_history where id = '$ceremony_id'" |
  sed 's/^/    /'

echo
echo "==> PUBLISHED through the same floor — a lapse discloses what the"
echo "    target stands behind, so this is what gives a grant anything to"
echo "    disclose at all"
publication=$(api "$cara_token" POST /v1/proposals \
  "{\"scope_id\":\"$platform_id\",\"record_ids\":[\"$ceremony_id\",\"$rota_id\"],
    \"title\":\"publish the platform runbooks\"}")
publication_id=$(echo "$publication" | field id)
echo "    proposal $publication_id needs: $(echo "$publication" | field outstanding)"
api "$cara_token" POST "/v1/proposals/$publication_id/approve" '{}' >/dev/null
api "$cleo_token" POST "/v1/proposals/$publication_id/approve" '{}' >/dev/null
api "$cara_token" POST "/v1/proposals/$publication_id/publish" '{}' >/dev/null
echo "    published"

echo
echo "==> BEFORE — priya asks for exactly the thing"
echo "    ceremony: $(arrival "$priya_token" "$CEREMONY")"

echo
echo "==> A WORKING-TIER LAPSE — two stewards, a real grant, and it changes"
echo "    nothing about the top tier: a grant is not a door to a tier it"
echo "    did not declare"
weak=$(api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"duration_secs\":$WINDOW_SECS,
    \"reason\":\"joint incident review\"}")
weak_id=$(echo "$weak" | field proposal_id)
api "$nadia_token" POST "/v1/proposals/$weak_id/approve" '{}' >/dev/null
api "$omar_token" POST "/v1/proposals/$weak_id/approve" '{}' >/dev/null
api "$nadia_token" POST "/v1/proposals/$weak_id/lapse" '{}' >/dev/null
echo "    granted. ceremony: $(arrival "$priya_token" "$CEREMONY")"
echo "    rota:      $(arrival "$priya_token" "$ROTA")   <- the working tier does travel"

echo
echo "==> THE DECLARED GRANT — the same two stewards are no longer enough"
strong=$(api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"max_sensitivity\":\"restricted\",
    \"duration_secs\":$WINDOW_SECS,
    \"reason\":\"the vault incident: payments needs the ceremony\"}")
strong_id=$(echo "$strong" | field proposal_id)
echo "    proposal $strong_id needs: $(echo "$strong" | field outstanding)"
api "$nadia_token" POST "/v1/proposals/$strong_id/approve" '{}' >/dev/null
api "$omar_token" POST "/v1/proposals/$strong_id/approve" '{}' >/dev/null
echo "--> both stewards have approved, and the grant is still refused:"
refused_api "$nadia_token" POST "/v1/proposals/$strong_id/lapse" '{}'
echo "    that refusal IS the acceptance criterion's"
echo "    'compliance-granted permission' — nobody wrote a rule for it"

api "$cleo_token" POST "/v1/proposals/$strong_id/approve" '{}' >/dev/null
api "$nadia_token" POST "/v1/proposals/$strong_id/lapse" '{}' >/dev/null

echo
echo "==> DURING — the same request priya made before"
echo "    ceremony: $(arrival "$priya_token" "$CEREMONY")"
echo "    marked twice: the section says how it got here, the line says"
echo "    what it carries — a guest harness cannot know otherwise"
session "$priya_token" | field text | sed -n '/lapse/,$p' | head -6 | sed 's/^/    | /'

echo
echo "==> EXPIRY — nobody revokes anything, nothing restarts"
sleep $((WINDOW_SECS + 1))
echo "    ceremony: $(arrival "$priya_token" "$CEREMONY")"

echo
echo "==> THE TRAIL — one chain, in order, with no record text anywhere"
sweep_deadline=$(($(date +%s) + 20))
until [ "$(psql_t "select count(*) from audit_log
                   where tenant_id = '$tenant_id' and action = 'policy.lapse.expired'")" -ge 1 ]; do
  [ "$(date +%s)" -lt "$sweep_deadline" ] || { echo "demo FAILED: no expiry event" >&2; exit 1; }
  sleep 1
done
psql_t "select seq || '  ' || rpad(action, 28) || '  ' || coalesce(actor_kind, '-')
        from audit_log
        where tenant_id = '$tenant_id'
          and action in ('memory.classified', 'vedaflow.proposal.approved',
                         'policy.lapse.granted', 'policy.lapse.expired')
        order by seq" | sed 's/^/    /'
echo "    the classification event carries both tiers:"
psql_t "select payload->'records'->0->>'was' || ' -> ' || (payload->>'sensitivity')
        from audit_log
        where tenant_id = '$tenant_id' and action = 'memory.classified'" | sed 's/^/    /'
echo "    the grant carries the ceiling its approvers signed for:"
psql_t "select payload->'lapse'->>'max_sensitivity'
        from audit_log
        where tenant_id = '$tenant_id' and action = 'policy.lapse.granted'
        order by seq desc limit 1" | sed 's/^/    /'
$CLI audit verify --tenant "$tenant_id" | sed 's/^/    /'

echo
echo "==> THE REFUSALS, in the product's own words"
echo "--> a classify proposal that does not say which tier it would install:"
refused_api "$cara_token" POST /v1/proposals \
  "{\"scope_id\":\"$platform_id\",\"record_ids\":[\"$rota_id\"],
    \"title\":\"classify something\",\"effect\":\"classify\"}"
echo "--> a publication that names a tier it would not move:"
refused_api "$cara_token" POST /v1/proposals \
  "{\"scope_id\":\"$platform_id\",\"record_ids\":[\"$rota_id\"],
    \"title\":\"publish something\",\"sensitivity\":\"restricted\"}"
echo "--> running a classification through the publish route — a route per"
echo "    effect, because a reviewer who approved a tier change did not"
echo "    approve a channel move:"
open_classification=$(api "$cara_token" POST /v1/proposals \
  "{\"scope_id\":\"$platform_id\",\"record_ids\":[\"$rota_id\"],
    \"title\":\"classify the rota as confidential\",
    \"effect\":\"classify\",\"sensitivity\":\"confidential\"}" | field id)
api "$cara_token" POST "/v1/proposals/$open_classification/approve" '{}' >/dev/null
refused_api "$cara_token" POST "/v1/proposals/$open_classification/publish" '{}'
echo "--> and the reader asking for the access she wants:"
refused_api "$priya_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"max_sensitivity\":\"restricted\",
    \"duration_secs\":60,\"reason\":\"I would like to read this\"}"

echo
echo "AUTHZ-5 demo complete: the top tier was earned through a compliance"
echo "decision, never reached a reader without one, and closed by itself."
