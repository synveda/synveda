#!/usr/bin/env sh
# FLOW-4 acceptance demo: auto-promotion rules (ADR-0033).
# AC (docs/backlog/FLOW-4.md): rule fires in soak test; proposals carry
# evidence (usage stats) for the reviewer.
#
# Flow: migrate -> admit a tenant -> org/eng/platform over the API -> ada
# (an agent identity with its own personal scope under the team) and cora
# (curator) -> a pack carrying ONE promotion rule becomes the tenant
# default, and no product pack carries any, so nothing auto-promoted
# before this -> seed a procedure on ada's own shelf -> THE SOAK: ada's
# sessions inject repeatedly, which is the only signal FLOW-4 counts —
# nothing writes a usage counter, the recalls are real `context.injected`
# events already under the audit chain's hash -> the gateway's OWN
# background loop (no test harness) crosses the threshold and opens a
# proposal, attributed to `system:promotion` but riding ada's authority
# -> THE EVIDENCE: the proposal states the counts and the audit range
# they came from, and this demo RE-DERIVES that count straight from the
# chain in SQL, which is what makes automated evidence checkable rather
# than merely present -> IDEMPOTENCE: the soak continues, and the same
# bytes are never proposed twice -> cora reviews what a rule raised and
# publishes it, so the record crosses the trust boundary through the
# ordinary FLOW-3 path -> the chain verifies.
#
# On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run, the EVAL-1 discipline and for the same
# recorded reason: both background loops this demo depends on — the pack
# refresher and the promotion engine — visit every active tenant per
# cycle, so on the shared dev database (thousands of leftover test
# tenants) a just-admitted tenant waits minutes for its first pass.
FLOW4_DB=flow4_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $FLOW4_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$FLOW4_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$FLOW4_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/flow4-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
SYNVEDA_LISTEN_ADDR=127.0.0.1:8140
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=flow-4-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# The engine's cadence is minutes in production (a promotion is not a hot
# path); a demo cannot wait five of them.
SYNVEDA_PROMOTION_INTERVAL_SECS=2
export SYNVEDA_PROMOTION_INTERVAL_SECS
SYNVEDA_POLICY_REFRESH_SECS=2
export SYNVEDA_POLICY_REFRESH_SECS
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

BASE="http://127.0.0.1:8140"

cargo build -p synveda-gateway -p synveda-cli

psql_c() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$FLOW4_DB" -c "$1"
}

psql_t() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$FLOW4_DB" -tAc "$1"
}

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  # Give the gateway a moment to release its connections, then discard the
  # scratch database and the sidecar it wrote.
  sleep 1
  $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $FLOW4_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR"
}
trap cleanup EXIT INT TERM

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

# code <token> <method> <path> [body] — prints the HTTP status only.
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

fail() {
  echo "demo FAILED: $1" >&2
  exit 1
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_id=$(./target/debug/synveda tenant create \
  --slug "flow4-demo-$$" --name "FLOW-4 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/flow4-gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    fail "gateway did not become healthy"
  fi
  sleep 1
done

echo "==> hierarchy: acme > eng > platform, with ada and cora"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
for who in ada cora; do
  ./target/debug/synveda service register --tenant "$tenant_id" \
    --subject "$who" --scope "$team_id" >/dev/null
done
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject cora --role curator --scope "$team_id" >/dev/null
ada_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject ada)
cora_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject cora)

ada_identity=$(psql_t "select id from identities
                       where tenant_id = '$tenant_id' and subject = 'ada'")
ada_scope=$(psql_t "select scope_id from identities
                    where tenant_id = '$tenant_id' and subject = 'ada'")
echo "    ada's own shelf: $ada_scope   (every record the write path"
echo "    produces lands on a personal leaf like this one — ADR-0018"
echo "    places an agent identity 'like a user', so this is as true for"
echo "    a fleet agent as for a person)"

echo
echo "==> the rule: one, on ada's pack, and no product pack carries any"
cat >"${TMPDIR:-/tmp}/flow4-rules.json" <<'JSON'
{
  "rules": [
    {
      "name": "well-used-procedures",
      "classes": ["procedure"],
      "max_sensitivity": "internal",
      "min_recalls": 3,
      "min_distinct_members": 1,
      "recency_hours": 24
    }
  ]
}
JSON
cat >"${TMPDIR:-/tmp}/flow4-pack.cedar" <<'CEDAR'
// Promotion rules are configuration, not authority: this pack permits
// exactly what the strict default does for a placed principal, and
// differs only in the rules it carries (ADR-0033 decision 6). Everything
// a rule raises still faces the whole approval matrix.
permit (principal, action == Synveda::Action::"MemoryRead", resource)
    when { principal in resource };
permit (principal, action == Synveda::Action::"MemoryWrite", resource)
    when { principal has home && resource == principal.home };
permit (principal, action == Synveda::Action::"ProposalOpen", resource)
    when { principal in resource };
permit (principal, action == Synveda::Action::"ProposalRead", resource)
    when { principal in resource };
permit (principal, action == Synveda::Action::"ProposalRead", resource)
    when { resource in principal.tenant
           && context.roles.containsAny(["curator", "steward", "org-admin"]) };
permit (principal, action == Synveda::Action::"ProposalReview", resource)
    when { resource in principal.tenant
           && context.roles.containsAny(["curator", "steward", "org-admin"]) };
permit (principal, action == Synveda::Action::"ChannelPublish", resource)
    when { resource in principal.tenant
           && context.roles.containsAny(["curator", "steward", "org-admin"]) };
// So an admin can see which pack is in force — this demo polls for it
// rather than sleeping, because a pack that has not been picked up yet
// falls back to the strict default and would measure the wrong rules.
permit (principal, action == Synveda::Action::"PolicyRead", resource)
    when { context.roles.containsAny(["steward", "org-admin"]) };
CEDAR
./target/debug/synveda policy apply --tenant "$tenant_id" --name promoting \
  --promotion "${TMPDIR:-/tmp}/flow4-rules.json" \
  "${TMPDIR:-/tmp}/flow4-pack.cedar" >/dev/null
api "$admin_token" PUT /v1/policy/default '{"name":"promoting"}' >/dev/null
echo "    rule: well-used-procedures — procedure, internal or below,"
echo "          3 recalls by 1 distinct member, recalled within 24h"

# The refresher installs stored packs on its own interval, so wait for the
# pack to actually be in force rather than assuming a fixed sleep — a
# stored pack that has not been picked up yet falls back to the strict
# default, and the demo would then be measuring the wrong pack.
echo "--> waiting for the refresher to put the pack in force"
tries=0
until curl -s -H "Authorization: Bearer $admin_token" \
  "$BASE/v1/hierarchy/nodes/$team_id/policy" | grep -q '"name":"promoting"'; do
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    curl -s -H "Authorization: Bearer $admin_token" \
      "$BASE/v1/hierarchy/nodes/$team_id/policy" >&2
    fail "the promoting pack never came into force"
  fi
  sleep 1
done
echo "    in force: promoting" 

# The rule counts usage; the material has to exist first. MEM-1's observe
# lands every record at its owner's personal scope, which is exactly where
# this one goes — seeded directly because the feature under review is the
# promotion, not the ingestion path (ADR-0023's deferred constraint is why
# record and embedding go in one transaction).
record_id=$(psql_t "select gen_random_uuid()")
psql_t "begin;
        insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                             content, sensitivity, provenance, valid_from)
        values ('$record_id', '$tenant_id', '$ada_scope', '$ada_identity',
                'derived', 'procedure',
                'Restart the ingest worker only after the queue reports zero visible messages.',
                'internal', '{\"source\":\"flow-4 demo\"}', now() - interval '1 hour');
        insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
        values ('$record_id', '$tenant_id', 'hash@1', 4, '[0.25,0.25,0.25,0.25]');
        commit;" >/dev/null
echo "    seeded one procedure on ada's shelf: ${record_id%%-*}"

echo
echo "==> THE SOAK: ada's sessions, injecting the same procedure"
echo "    (nothing writes a usage counter — each of these is a real"
echo "    inject, and its \`context.injected\` event IS the signal)"
for round in 1 2; do
  api "$ada_token" POST /v1/inject "{\"session_id\":\"flow-4-soak-$round\"}" >/dev/null
done
sleep 6
open_now=$(api "$cora_token" GET "/v1/proposals?scope_id=$ada_scope" | field proposals length)
[ "$open_now" = "0" ] ||
  fail "two recalls is under the rule's threshold of three; got $open_now proposals"
echo "    after 2 recalls: 0 proposals — the threshold is 3"

api "$ada_token" POST /v1/inject '{"session_id":"flow-4-soak-3"}' >/dev/null
echo "    after 3 recalls: waiting for the gateway's own promotion loop..."
tries=0
until [ "$(api "$cora_token" GET "/v1/proposals?scope_id=$ada_scope" | field proposals length)" = "1" ]; do
  tries=$((tries + 1))
  [ "$tries" -ge 60 ] && fail "the rule did not fire within 60s"
  sleep 1
done
echo "    the rule fired — nobody asked it to"

echo
echo "==> THE EVIDENCE the proposal carries"
proposal=$(api "$cora_token" GET "/v1/proposals?scope_id=$ada_scope" | field proposals 0)
proposal_id=$(emit "$proposal" | field id)
emit "$proposal" | field title | sed 's/^/    title:    /'
emit "$proposal" | field promotion rule | sed 's/^/    rule:     /'
emit "$proposal" | field promotion actions | sed 's/^/    counted:  /'
recalls=$(emit "$proposal" | field promotion members 0 recalls)
members=$(emit "$proposal" | field promotion members 0 distinct_members)
from_seq=$(emit "$proposal" | field promotion from_seq)
to_seq=$(emit "$proposal" | field promotion to_seq)
echo "    claims:   $recalls recalls by $members member, over chain [$from_seq, $to_seq]"
echo "    proposer: $(emit "$proposal" | field proposer_subject) — the material's owner,"
echo "              whose ProposalOpen decision the rule rode"

echo
echo "--> and the claim is checkable: re-deriving it from the chain itself"
counted=$(psql_t "select count(*) from audit_log, lateral jsonb_array_elements(payload->'entries') e
                  where tenant_id = '$tenant_id'
                    and action = 'context.injected'
                    and seq between $from_seq and $to_seq
                    and e->>'record_id' = '$record_id'")
echo "    the audit chain, counted independently: $counted"
[ "$counted" = "$recalls" ] ||
  fail "the evidence claims $recalls recalls; the chain says $counted"
echo "    they agree — the reviewer never has to trust the counter"

echo
echo "==> who acted, and whose authority it was: two different facts"
psql_c "select actor_kind, actor_subject, payload->'proposer'->>'subject' as rode_authority_of,
               payload->'approvals'->'required'->>'origins' as requirement_origins
        from audit_log
        where tenant_id = '$tenant_id' and action = 'vedaflow.proposal.opened';"

echo "==> IDEMPOTENCE: the soak continues, and nothing is proposed twice"
for round in 4 5 6; do
  api "$ada_token" POST /v1/inject "{\"session_id\":\"flow-4-soak-$round\"}" >/dev/null
done
sleep 8
still=$(api "$cora_token" GET "/v1/proposals?scope_id=$ada_scope" | field proposals length)
[ "$still" = "1" ] ||
  fail "the same bytes were proposed again: $still proposals stand open"
echo "    3 more recalls, 2 more sweeps, still 1 proposal —"
echo "    the content address is the idempotency key"

echo
echo "==> the review: what the matrix asks for here"
emit "$proposal" | field state | sed 's/^/    state:       /'
emit "$proposal" | field outstanding | sed 's/^/    outstanding: /'
echo "    (this scope's pack asks for no approvals on internal memory, so"
echo "     the rule's proposal is already satisfied — the matrix decides"
echo "     that, not the rule, and a vote satisfying nothing is refused:)"
why "$cora_token" POST "/v1/proposals/$proposal_id/approve" \
  '{"comment":"looks fine to me"}' | sed 's/^/    cora approves: /'

echo
echo "--> and cora, a curator, still cannot publish it"
cora_code=$(code "$cora_token" POST "/v1/proposals/$proposal_id/publish" '{}')
[ "$cora_code" = "403" ] ||
  fail "a curator must not publish material they cannot read (got $cora_code)"
why "$cora_token" POST "/v1/proposals/$proposal_id/publish" '{}' | sed 's/^/    /'
echo "    and under the product's own packs the same call fails for a"
echo "    second reason: publishing takes MemoryRead on every record it"
echo "    admits, and nobody reads another member's personal scope."
echo "    So for personal material the reviewer is its owner — which is"
echo "    exactly what a 1-distinct-member rule promotes, and why that"
echo "    threshold is a product case rather than a weakened one"
echo "    (ADR-0033 decision 7)."

echo
echo "==> ada holds curator on her own shelf, and publishes her own"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject ada --role curator --scope "$ada_scope" >/dev/null
ada_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject ada)
publish=$(api "$ada_token" POST "/v1/proposals/$proposal_id/publish" '{}')
emit "$publish" | field commit | sed 's/^/    published commit: /'
emit "$publish" | field proposal_commit |
  sed 's/^/    its second parent: /'
echo "    (the publication is a merge whose second parent is the"
echo "     proposal a rule opened — lineage in the commit graph)" 

published=$(api "$ada_token" POST /v1/inject '{"session_id":"flow-4-after"}')
case "$(emit "$published" | field text)" in
  *unreviewed*) fail "the record should no longer be watermarked unreviewed" ;;
  *) echo "    ada's very next inject composes it as published, not unreviewed" ;;
esac

echo
echo "==> the audit chain verifies"
./target/debug/synveda audit verify --tenant "$tenant_id" | sed 's/^/    /'

echo
echo "FLOW-4 demo OK — a rule fired on real use without anyone asking it"
echo "to, the proposal it opened carried usage stats that were checked"
echo "against the hash chain rather than trusted, the same bytes were"
echo "never raised twice however long the soak ran, and the promotion"
echo "crossed the trust boundary through the ordinary FLOW-3 path, under"
echo "the authority of the identity whose material it was."
