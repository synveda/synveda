#!/usr/bin/env sh
# FLOW-3 acceptance demo: proposals & the approval matrix (ADR-0032).
# AC (docs/backlog/FLOW-3.md): full matrix golden tests; a memory
# promotion team->published E2E with 1 curator; restricted asset requires
# compliance + dual approval.
#
# Flow: migrate -> admit a tenant -> org/eng/platform over the API ->
# four principals at the team (dana contributor, cora curator, cleo
# compliance, sam steward) -> seed two team-scope records, one `internal`
# and one `restricted` -> THE FIRST AC: dana opens a proposal, the
# response states what the pack requires (1 curator) and that it is not
# yet met; dana cannot run the effect; cora *reads the content* and
# approves; the proposal becomes approved; cora publishes, and the
# published commit is a MERGE whose second parent is the proposal — the
# lineage tech plan §2.5 promises, in the commit graph -> THE SECOND AC:
# the restricted record. Cora's DIRECT publish is refused by name, and so
# is the direct publish by a principal holding curator AND compliance,
# because two *distinct* approvers is what dual approval means. Through a
# proposal: cora approves (still short), cleo approves (compliance, now
# approved), cleo cannot publish — she holds no ChannelPublish in any
# pack, which is exactly why the deciding approval does not publish —
# and cora runs the effect -> CURATOR FILES: sam commits a CODEOWNERS-
# style file naming himself for `memory/*`; the very next proposal
# requires him too, and the file granted him nothing he did not already
# have -> APPROVALS BIND BYTES: a record edited after its approval
# refuses to publish, naming the record that moved -> the audit trail
# carries the requirement as it was resolved at each act, and the chain
# verifies. On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8139
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=flow-3-demo-secret
export SYNVEDA_DEV_JWT_SECRET
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

BASE="http://127.0.0.1:8139"

cargo build -p synveda-gateway -p synveda-cli

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

# `echo` would interpret the \n escapes inside a JSON body; printf does not.
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

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_id=$(./target/debug/synveda tenant create \
  --slug "flow3-demo-$$" --name "FLOW-3 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/flow3-gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

echo "==> hierarchy: acme > eng > platform, with four principals at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
for who in dana cora cleo sam; do
  ./target/debug/synveda service register --tenant "$tenant_id" \
    --subject "$who" --scope "$team_id" >/dev/null
done
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject dana --role contributor --scope "$team_id" >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject cora --role curator --scope "$team_id" >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject cleo --role compliance --scope "$team_id" >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject sam --role steward --scope "$team_id" >/dev/null
dana_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject dana)
cora_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject cora)
cleo_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject cleo)
sam_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject sam)
echo "    team=$team_id"
echo "    dana=contributor  cora=curator  cleo=compliance  sam=steward"

# MEM-1's observe lands every record at its *owner's personal scope*
# (ADR-0020 decision 3), so the team's own shelf is seeded here directly:
# the feature under review is the review, not the ingestion path. The
# deferred constraint trigger (ADR-0023) is why record and embedding go in
# one transaction.
dana_identity=$(psql_t "select id from identities
                        where tenant_id = '$tenant_id' and subject = 'dana'")
seed_record() {
  psql_t "begin;
          insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                               content, sensitivity, provenance, valid_from)
          values ('$1', '$tenant_id', '$team_id', '$dana_identity', 'derived',
                  'procedure', '$2', '$3', '{\"source\":\"flow-3 demo\"}',
                  now() - interval '1 hour');
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          values ('$1', '$tenant_id', 'hash@1', 4, '[0.25,0.25,0.25,0.25]');
          commit;" >/dev/null
}
runbook_id=$(psql_t "select gen_random_uuid()")
bridge_id=$(psql_t "select gen_random_uuid()")
seed_record "$runbook_id" "Rotate the signing key every 90 days." "internal"
seed_record "$bridge_id" "A sev-1 bridge is opened by the on-call lead." "restricted"
echo "==> two records on the team's shelf:"
psql_c "select left(id::text, 8) as id, sensitivity, content
        from records where tenant_id = '$tenant_id' order by sensitivity;"

echo
echo "==> THE FIRST AC — a memory promotion team->published, with 1 curator"
echo "--> dana (contributor) opens a proposal"
opened=$(api "$dana_token" POST /v1/proposals \
  "{\"scope_id\":\"$team_id\",\"record_ids\":[\"$runbook_id\"],
    \"title\":\"promote the key-rotation runbook\"}")
proposal_id=$(emit "$opened" | field id)
proposal_commit=$(emit "$opened" | field commit)
echo "    proposal $proposal_id at commit $proposal_commit"
echo "    state:       $(emit "$opened" | field state)"
echo "    required:    $(emit "$opened" | field required roles) \
from $(emit "$opened" | field required origins)"
echo "    outstanding: $(emit "$opened" | field outstanding)"
[ "$(emit "$opened" | field state)" = "open" ] || {
  echo "demo FAILED: a fresh proposal must not be approved" >&2; exit 1; }

echo "--> dana tries to run the effect herself"
dana_code=$(code "$dana_token" POST "/v1/proposals/$proposal_id/publish")
[ "$dana_code" = "403" ] || { echo "demo FAILED: dana got $dana_code" >&2; exit 1; }
echo "    403 — a contributor holds no ChannelPublish, proposal or not"

echo "--> cora (curator) reads the proposal: this is what a review is"
detail=$(api "$cora_token" GET "/v1/proposals/$proposal_id")
echo "    member:    $(emit "$detail" | field members 0 content)"
echo "    unchanged: $(emit "$detail" | field members 0 unchanged) \
(still the bytes that were proposed)"

echo "--> cora approves"
approved=$(api "$cora_token" POST "/v1/proposals/$proposal_id/approve" \
  '{"comment":"matches the runbook we agreed"}')
echo "    state:       $(emit "$approved" | field state)"
echo "    counted as:  $(emit "$approved" | field counted_roles)"
echo "    outstanding: $(emit "$approved" | field outstanding)"
[ "$(emit "$approved" | field state)" = "approved" ] || {
  echo "demo FAILED: one curator must satisfy this cell" >&2; exit 1; }

echo "--> cora runs the effect"
published=$(api "$cora_token" POST "/v1/proposals/$proposal_id/publish")
channel_commit=$(emit "$published" | field commit)
echo "    memory/published -> $channel_commit  (members=$(emit "$published" | field members))"
echo "--> the publication descends from the proposal it is the effect of"
parents() {
  psql_c "select p.ordinal, left(encode(p.parent_hash, 'hex'), 16) as parent,
                 left(c.message, 44) as message
          from vedaflow_commit_parents p
          join vedaflow_commits c
            on c.tenant_id = p.tenant_id and c.hash = p.parent_hash
          where p.tenant_id = '$tenant_id'
            and p.commit_hash = decode('$1', 'hex')
          order by p.ordinal;"
}
parents "$channel_commit"
merged=$(psql_t "select count(*) from vedaflow_commit_parents
                 where tenant_id = '$tenant_id'
                   and commit_hash = decode('$channel_commit', 'hex')
                   and parent_hash = decode('$proposal_commit', 'hex')")
[ "$merged" = "1" ] || {
  echo "demo FAILED: the publication must descend from the proposal" >&2; exit 1; }
echo "    the channel had no head yet, so the proposal is its only parent —"
echo "    the second publication below is where the merge has two"


echo
echo "==> THE SECOND AC — restricted requires compliance + dual approval"
publish_body="{\"record_ids\":[\"$bridge_id\"],\"message\":\"straight to published\"}"
echo "--> cora (curator) tries the DIRECT route on the restricted record"
direct_code=$(code "$cora_token" POST "/v1/channels/$team_id/publish" "$publish_body")
[ "$direct_code" = "403" ] || { echo "demo FAILED: got $direct_code" >&2; exit 1; }
echo "    403: $(why "$cora_token" POST "/v1/channels/$team_id/publish" "$publish_body")"
echo "    the SAME matrix governs the direct route — it is the degenerate"
echo "    case where one approval is enough, and here one is not"

echo "--> even a principal holding curator AND compliance is refused alone"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject cleo --role curator --scope "$team_id" >/dev/null
both_code=$(code "$cleo_token" POST "/v1/channels/$team_id/publish" "$publish_body")
[ "$both_code" = "403" ] || { echo "demo FAILED: got $both_code" >&2; exit 1; }
echo "    403 — both role lines met, one identity: dual approval is two people"
./target/debug/synveda role unbind --tenant "$tenant_id" \
  --subject cleo --role curator --scope "$team_id" >/dev/null

echo "--> dana opens a proposal for it instead"
opened=$(api "$dana_token" POST /v1/proposals \
  "{\"scope_id\":\"$team_id\",\"record_ids\":[\"$bridge_id\"],
    \"title\":\"publish the sev-1 bridge procedure\"}")
restricted_id=$(emit "$opened" | field id)
echo "    required:    $(emit "$opened" | field required roles)"
echo "    distinct:    $(emit "$opened" | field required distinct_approvers)"
echo "    origins:     $(emit "$opened" | field required origins)"
echo "    (the floor contributed: no pack can author compliance away)"

echo "--> cora approves: still short"
first=$(api "$cora_token" POST "/v1/proposals/$restricted_id/approve")
echo "    state:       $(emit "$first" | field state)"
echo "    outstanding: $(emit "$first" | field outstanding)"
early_code=$(code "$cora_token" POST "/v1/proposals/$restricted_id/publish")
[ "$early_code" = "409" ] || { echo "demo FAILED: got $early_code" >&2; exit 1; }
echo "    409 on publish: $(why "$cora_token" POST "/v1/proposals/$restricted_id/publish")"

echo "--> cleo (compliance) casts the deciding vote"
second=$(api "$cleo_token" POST "/v1/proposals/$restricted_id/approve" \
  '{"comment":"classification reviewed"}')
echo "    state:      $(emit "$second" | field state)"
echo "    counted as: $(emit "$second" | field counted_roles)"

echo "--> cleo tries to run the effect"
cleo_code=$(code "$cleo_token" POST "/v1/proposals/$restricted_id/publish")
[ "$cleo_code" = "403" ] || { echo "demo FAILED: got $cleo_code" >&2; exit 1; }
echo "    403 — compliance holds no ChannelPublish in any pack. Had the"
echo "    deciding approval published, THIS call would have run under system"
echo "    authority: the bypass ADR-0032 decision 9 refuses"

echo "--> cora runs the effect"
second_published=$(api "$cora_token" POST "/v1/proposals/$restricted_id/publish")
second_commit=$(emit "$second_published" | field commit)
echo "    published set is now $(emit "$second_published" | field members) members"
echo "--> and THIS one is a real merge: the channel head, then the proposal"
parents "$second_commit"
two=$(psql_t "select count(*) from vedaflow_commit_parents
              where tenant_id = '$tenant_id'
                and commit_hash = decode('$second_commit', 'hex')")
[ "$two" = "2" ] || {
  echo "demo FAILED: a publication with a head must merge, got $two parents" >&2
  exit 1
}
mainline=$(psql_t "select encode(parent_hash, 'hex') from vedaflow_commit_parents
                   where tenant_id = '$tenant_id'
                     and commit_hash = decode('$second_commit', 'hex')
                     and ordinal = 0")
[ "$mainline" = "$channel_commit" ] || {
  echo "demo FAILED: first parent must be the channel head" >&2; exit 1; }
echo "    first parent is the channel's own line; the second is the review."
echo "    'every published sentence traces to an author through an approval'"
echo "    (tech plan §2.5) is now a fact about the graph, not a join"


echo
echo "==> CURATOR FILES — requirements added, authority granted to nobody"
curators='# platform runbooks are sams
memory/* @sam
'
sam_written=$(api "$sam_token" PUT "/v1/hierarchy/nodes/$team_id/curators" \
  "$(node -e 'console.log(JSON.stringify({source: process.argv[1],
     message: "platform runbooks need sam"}))' "$curators")")
echo "    committed as object $(emit "$sam_written" | field object_hash)"
dana_code=$(code "$dana_token" PUT "/v1/hierarchy/nodes/$team_id/curators" \
  '{"source":"* @dana","message":"seize the review"}')
[ "$dana_code" = "403" ] || { echo "demo FAILED: got $dana_code" >&2; exit 1; }
echo "    403 for a contributor: editing who must approve is PolicyAssign"

third_id=$(psql_t "select gen_random_uuid()")
seed_record "$third_id" "Deploys go out on tuesdays." "internal"
opened=$(api "$dana_token" POST /v1/proposals \
  "{\"scope_id\":\"$team_id\",\"record_ids\":[\"$third_id\"],
    \"title\":\"promote the deploy window\"}")
named_id=$(emit "$opened" | field id)
echo "    the very next proposal requires: $(emit "$opened" | field required subjects)"
echo "    origins: $(emit "$opened" | field required origins)"
after_cora=$(api "$cora_token" POST "/v1/proposals/$named_id/approve")
echo "    cora approves -> state=$(emit "$after_cora" | field state), \
outstanding: $(emit "$after_cora" | field outstanding)"
after_sam=$(api "$sam_token" POST "/v1/proposals/$named_id/approve")
echo "    sam approves  -> state=$(emit "$after_sam" | field state)"
[ "$(emit "$after_sam" | field state)" = "approved" ] || {
  echo "demo FAILED: the named approver must complete it" >&2; exit 1; }

echo
echo "==> APPROVALS BIND BYTES — an edit after the review refuses"
psql_c "update records set content = 'Deploy whenever you like.'
        where id = '$third_id';" >/dev/null
echo "    the approved record was rewritten under its own id"
drift_code=$(code "$cora_token" POST "/v1/proposals/$named_id/publish")
[ "$drift_code" = "409" ] || { echo "demo FAILED: got $drift_code" >&2; exit 1; }
echo "    409: $(why "$cora_token" POST "/v1/proposals/$named_id/publish")"
echo "    approve -> edit -> publish is the attack; the address is the guard"
echo "    and the review surface says so before anyone tries:"
api "$cora_token" GET "/v1/proposals/$named_id" | field members 0 unchanged |
  sed 's/^/    members[0].unchanged = /'

echo
echo "==> THE TRAIL — every act, with the requirement as it was resolved"
psql_c "select action,
               payload->'approvals'->'required'->>'distinct_approvers' as needs,
               payload->'approvals'->>'outstanding' as outstanding,
               payload->'approvals'->'required'->'origins' as origins
        from audit_log
        where tenant_id = '$tenant_id'
          and action like 'vedaflow.%'
        order by seq;"
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> THE AC SUITES"
echo "--> the full matrix golden (3 packs x 5 assets x 4 sensitivities x 5 scopes)"
cargo test -p synveda-policy --test approvals -- --nocapture
echo
echo "--> the promotion and dual-approval walks, on the product surfaces"
cargo test -p synveda-gateway --test proposals -- --test-threads=1
echo
echo "--> and the RLS suite, with the two new tables in the adversarial set"
cargo test -p synveda-store --test rls

echo
echo "FLOW-3 demo complete."
