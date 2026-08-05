#!/usr/bin/env sh
# CTX-4 acceptance demo: tiered injection / progressive disclosure
# (ADR-0041).
# AC (docs/backlog/CTX-4.md): token cost of index tier measured; agent can
# navigate index→body in a live session.
#
# Flow: postgres up -> migrate -> tenant, hierarchy, alice -> she works six
# sessions, so six records exist -> a tight budget is applied and `inject`
# carries what fits and DROPS THE REST IN SILENCE, which is the product
# exactly as it behaved before CTX-4 -> the index tier is turned on in the
# pack and the VERY NEXT inject names what it could not carry, each entry
# ending in a recall handle -> the cost of that is printed, which is the
# AC's measurement -> then the navigation, run the way an agent runs it:
# `synveda recall <id>` with DATABASE_URL UNSET, so the body can only have
# come through the gateway under the PDP -> then the half that matters,
# where a handle is shown to be a NAME and not a capability: the policy
# changes, and the same id in the same session stops resolving with nobody
# revoking anything -> and the trail, where `context.injected` says which
# entries were bodies and which were only names, `context.recalled` says
# how many were asked for and how many served but never which were
# refused, and the chain verifies over all of it.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI (this demo
# runs the network-free deterministic extractor and embedder).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1/MEM-6 discipline: the pack
# refresher visits every active tenant per cycle, so on the shared dev
# database a just-admitted tenant waits minutes for its first pass, and
# this demo flips a pack twice.
CTX4_DB=ctx4_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $CTX4_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$CTX4_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$CTX4_DB"
export DATABASE_URL
# `sqlx::query!` expands against DATABASE_URL at compile time, and the
# database named above can still be empty at this point: a crate that needs
# a rebuild here type-checks against a schema that does not exist yet and
# fails with `relation "audit_chain_heads" does not exist` rather than with
# anything about this demo. It is invisible whenever the workspace happens
# to be built already. The checked-in `.sqlx` cache is the answer to
# "compile without a database", and it is what `make ci` and
# scripts/db-test.sh use for the same reason.
SQLX_OFFLINE=true
export SQLX_OFFLINE
SYNVEDA_SEARCH_INDEX_DIR="./data/ctx4-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8141
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=ctx-4-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
SYNVEDA_POLICY_REFRESH_SECS=2
export SYNVEDA_POLICY_REFRESH_SECS

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$CTX4_DB" -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "ctx4-demo-$$" --name "CTX-4 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  sleep 1
  $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $CTX4_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR"
  rm -f "/tmp/ctx4-pack-$$.cedar"
}
trap cleanup EXIT INT TERM

./target/debug/synveda-gateway &
GATEWAY_PID=$!

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8141/healthz >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" \
      "http://127.0.0.1:8141$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8141$path"
  fi
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

# How many entries an array field holds. Fed with `printf '%s'` rather
# than `echo` everywhere below: the composed block's JSON contains `\n`
# escapes, and a POSIX `echo` that interprets them turns valid JSON into
# a parse error.
count() {
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      let v = JSON.parse(d);
      for (const k of process.argv.slice(1)) v = v[k];
      console.log(v.length);
    });
  ' "$@"
}

records_now() {
  psql_t "select count(*) from records where tenant_id = '$tenant_id'"
}

wait_for_records() {
  want=$1
  tries=0
  while :; do
    [ "$(records_now)" = "$want" ] && return 0
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      echo "demo FAILED: expected $want records, stuck at $(records_now)" >&2
      exit 1
    fi
    sleep 0.5
  done
}

observe() {
  body="{\"session_id\":\"$2\",\"events\":[{\"idempotency_key\":\"$2-1\",
    \"kind\":\"tool_result\",\"payload\":{\"text\":\"$3\"},
    \"occurred_at\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}]}"
  accepted=$(api "$1" POST /v1/observe "$body" | field accepted)
  [ "$accepted" = "1" ] || {
    echo "demo FAILED: observe was not accepted ($accepted)" >&2
    exit 1
  }
}

inject() {
  api "$1" POST /v1/inject "{\"session_id\":\"$2\"}"
}

# The pack knobs this whole demo turns on (ADR-0025 decision 3's machinery,
# extended by ADR-0041 decision 11): the budget, the index tier, and how
# wide an index line is.
#
# `--composition-index-chars 64` rather than the 320 default because the
# deterministic extractor caps a record at 300 characters, and the tier
# only ever demotes when naming is genuinely cheaper than showing
# (ADR-0041 decision 2) — at the default width, against a corpus whose
# every record is 300 characters, naming would cost about what showing
# costs and nothing would ever be named. That is the rule doing its job
# rather than a misconfigured demo, and it is why the width is a pack
# field: it wants to sit well below the median body. A corpus of context
# packs or skills (PRMT-2, SKIL-1), whose bodies run to thousands of
# tokens, needs no such adjustment.
apply_pack() {
  name=$1
  version=$2
  tier=$3
  # A permissive source: every grant comes from this permit, and the
  # compiled base layer's forbids still ride along — a pack, not a hole
  # (seed §2.2). This demo is about what a block carries, not about who
  # may read; AUTHZ-3/FLOW-3 demo the decision layer. It has to cover the
  # admin actions too, because it becomes the tenant default and the
  # admin has to be able to replace it in step 3.
  cat >"/tmp/ctx4-pack-$$.cedar" <<'CEDAR'
permit (principal, action, resource) when { resource in principal.tenant };
CEDAR
  ./target/debug/synveda policy apply \
    --tenant "$tenant_id" --name "$name" \
    --composition-budget 300 \
    --composition-channels published-and-derived \
    --composition-index-tier "$tier" \
    --composition-index-chars 64 \
    "/tmp/ctx4-pack-$$.cedar" >/dev/null
  api "$admin_token" PUT /v1/policy/default "{\"name\":\"$name\"}" >/dev/null
  # The refresher's cadence, so the next request is governed by what was
  # just applied (ADR-0014's promise, on this demo's clock).
  sleep 3
  echo "    pack $name applied: budget 300 tokens, index tier $tier, index width 64 chars"
}

echo "==> the admin builds the hierarchy; alice is registered at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject alice --scope "$team_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
echo "    org=$org_id team=$team_id alice=alice"

echo
echo "==> [1/5] alice works six sessions; six records exist."
i=1
# Each text runs well past the deterministic extractor's 300-character
# cap, so every record lands at the same length — the shape a runbook, a
# context pack or a skill has, and the shape the index tier exists for.
for text in \
  "Settlement mismatch drill: freeze the reconciliation job from the operator console before comparing the ledger tail against the acquirer statement for the affected window, then page the payments on-call and the finance controller together, because a mismatch that survives one reconciliation cycle becomes a regulatory reporting item within twenty-four hours and the window stops being recoverable from the application side." \
  "Rollout runbook: drain the nodes one availability zone at a time, wait for the readiness gate to report green on every remaining replica, upgrade the control plane before any data plane component, and let the fleet roll forward only after the canary has held for a full metrics window, because a rollout that outruns its own telemetry cannot be rolled back on evidence." \
  "Vacuum maintenance procedure: analyze the bloated relations weekly, record the resulting table sizes so the growth curve stays visible to the whole team, and schedule the aggressive pass for the maintenance window rather than the quiet hours, because an autovacuum that competes with the settlement batch will lose and then look like a storage problem instead of a scheduling one." \
  "Incident comms procedure: page the on-call and the controller together rather than in sequence, open the shared channel before the first hypothesis, and post the timeline as it is discovered rather than once it is complete, because an incident that is reconstructed afterwards is reconstructed from memory and memory is exactly what the postmortem is trying to check." \
  "Sandbox credential rotation: rotate the payments sandbox key, redeploy the two consumers in either order, then confirm the old key is actually refused before closing the ticket, because a rotation nobody verified is indistinguishable from a rotation that silently failed and both look identical in the deployment log." \
  "Release checklist: freeze the branch, run the migration dry run against a freshly restored snapshot rather than a synthetic fixture, obtain the change review sign-off from somebody who did not write the change, and hold the deploy window open long enough to roll back inside it rather than after it."; do
  observe "$alice_token" "ctx4-s$i" "$text"
  i=$((i + 1))
done
wait_for_records 6
echo "    six records at $team_id"

echo
echo "==> [2/5] the product as it behaved BEFORE CTX-4: a tight budget,"
echo "    and what does not fit is dropped in silence."
apply_pack "ctx4-off-$$" 1 off
before=$(inject "$alice_token" ctx4-before)
before_named=$(printf "%s" "$before" | count record_ids)
before_tokens=$(printf "%s" "$before" | field tokens)
printf "%s" "$before" | field text | sed 's/^/    | /'
echo "    -> $before_named of 6 records reached the block, $before_tokens tokens."
echo "       The other $((6 - before_named)) are not mentioned. An agent cannot ask for"
echo "       what it was never told exists — that is the defect CTX-4 fixes."

echo
echo "==> [3/5] AC part one: the index tier is turned on in the pack."
echo "    Nothing is restarted. No record is touched."
apply_pack "ctx4-on-$$" 1 demote
after=$(inject "$alice_token" ctx4-after)
after_named=$(printf "%s" "$after" | count record_ids)
after_tokens=$(printf "%s" "$after" | field tokens)
index_entries=$(printf "%s" "$after" | field index_entries)
index_tokens=$(printf "%s" "$after" | field index_tokens)
printf "%s" "$after" | field text | sed 's/^/    | /'
echo
echo "    THE MEASUREMENT (the acceptance criterion's first half):"
echo "      records named   : $before_named -> $after_named"
echo "      block tokens    : $before_tokens -> $after_tokens  (budget 300)"
echo "      index tier cost : $index_tokens tokens across $index_entries entries"
[ "$after_named" -gt "$before_named" ] || {
  echo "demo FAILED: the index tier named nothing new" >&2
  exit 1
}
[ "$after_tokens" -le 300 ] || {
  echo "demo FAILED: the block exceeded its budget" >&2
  exit 1
}

echo
echo "==> [4/5] AC part two: index -> body, the way an agent does it."
handle=$(printf "%s" "$after" | field text |
  sed -n 's/.*(recall \([0-9a-f-]*\)).*/\1/p' | head -1)
[ -n "$handle" ] || {
  echo "demo FAILED: no recall handle in the block" >&2
  exit 1
}
echo "    the agent lifts a handle out of the block it was given: $handle"
echo
# DATABASE_URL is unset for the recall: the body cannot have come from a
# psql the demo ran. It came through /v1/recall, under alice's own bearer,
# decided by the PDP and chained under her identity (the FLOW-6 discipline).
SYNVEDA_TOKEN="$alice_token" SYNVEDA_GATEWAY="http://127.0.0.1:8141" \
  env -u DATABASE_URL ./target/debug/synveda recall "$handle" | sed 's/^/    | /'

echo
echo "==> [5/5] and a handle is a NAME, not a capability."
echo "    The policy changes. Nobody revokes the handle — there is nothing"
echo "    to revoke, because the handle never carried authority."
cat >"/tmp/ctx4-pack-$$.cedar" <<'CEDAR'
permit (principal, action, resource) when { false };
CEDAR
./target/debug/synveda policy apply \
  --tenant "$tenant_id" --name "ctx4-locked-$$" \
  "/tmp/ctx4-pack-$$.cedar" >/dev/null
api "$admin_token" PUT /v1/policy/default '{"name":"ctx4-locked-'"$$"'"}' >/dev/null
sleep 3
locked=$(api "$alice_token" POST /v1/recall "{\"ids\":[\"$handle\"]}")
served=$(printf "%s" "$locked" | count entries)
requested=$(printf "%s" "$locked" | field requested)
echo "    the same id, the same session, the very next call:"
echo "      requested $requested, served $served"
[ "$served" = "0" ] || {
  echo "demo FAILED: the handle still resolved after the decision changed" >&2
  exit 1
}
echo "    -> 200, not an error: a policy outcome on a read is a result."
echo "       And the response never says WHICH id was refused, because a"
echo "       recall that distinguished them would answer 'that one exists'."

echo
echo "==> the trail"
echo "    what the block carried, per entry — body or only a name:"
psql_t "select jsonb_pretty(jsonb_agg(e -> 'tier'))
        from audit_log, jsonb_array_elements(payload -> 'entries') e
        where tenant_id = '$tenant_id' and action = 'context.injected'
          and payload ->> 'session_id' = 'ctx4-after'" | sed 's/^/    /'
echo "    what recall was asked for and what it served:"
psql_t "select 'requested ' || (payload ->> 'requested') ||
        ', served ' || (payload ->> 'served')
        from audit_log where tenant_id = '$tenant_id'
          and action = 'context.recalled' order by seq" | sed 's/^/    /'
echo "    and no record content anywhere in either payload:"
psql_t "select case when count(*) = 0 then 'confirmed: none'
               else 'LEAK: ' || count(*)::text end
        from audit_log where tenant_id = '$tenant_id'
          and action in ('context.injected', 'context.recalled')
          and payload::text like '%reconciliation%'" | sed 's/^/    /'
echo
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 5
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> the AC suites"
cargo test -p synveda-gateway --test tiered
cargo test -p synveda-retrieval --test compose

echo
echo "CTX-4 demo OK: a tight budget dropped material in silence, the index"
echo "tier applied to the running system named it on the very next inject at"
echo "a measured cost, an agent navigated from a handle to the body through"
echo "the gateway with no database of its own, and when the decision behind"
echo "that handle changed the same id stopped resolving — with nothing"
echo "revoked, because a handle is a name."
