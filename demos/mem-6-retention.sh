#!/usr/bin/env sh
# MEM-6 acceptance demo: decay, TTL & staleness (ADR-0040).
# AC (docs/backlog/MEM-6.md): a retention policy change re-evaluates
# existing records; audit trail of expiries.
#
# Flow: postgres up -> migrate -> tenant, hierarchy, alice -> she observes
# a session summary ninety days ago and another yesterday, and `inject`
# returns both -> an operator applies a RETENTION SCHEDULE (episodes kept
# 30 days) and NOBODY TOUCHES A RECORD -> the very next inject stops
# carrying the old one, because nothing was ever stamped on it and the pack
# is read in the query that asks -> the gateway's own sweep then expires it
# out of the live corpus and chains `memory.expired` under actor_kind=system
# -> the record is gone from `records`, and its version is still in
# `records_versions`, which is what keeps "what did the agent know in April"
# answerable -> a PINNED record of the same age survives both layers (seed
# §4.2) -> then the second horizon: DESTRUCTION takes the history the expiry
# left, the as-of question that had an answer stops having one, and
# `memory.disposed` says so -> and the observe staging plane, which has held
# every payload since MEM-1, is disposed of on its own horizon -> chain
# verifies over all of it.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI (this demo
# runs the network-free deterministic extractor and embedder).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1 discipline, for the same
# recorded reason and one this feature feels twice over: both background
# loops this demo depends on (the pack refresher and the retention sweep)
# visit every active tenant per cycle, so on the shared dev database, with
# its thousands of leftover test tenants, a just-admitted tenant waits
# minutes for its first pass.
MEM6_DB=mem6_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $MEM6_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$MEM6_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$MEM6_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/mem6-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8140
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=mem-6-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
# The sweep and the pack refresher on a demo cadence. In production both
# are slack — the read path has already stopped serving expired material,
# so a slow sweep costs storage rather than exposure (ADR-0040 decision 2).
SYNVEDA_RETENTION_INTERVAL_SECS=3
export SYNVEDA_RETENTION_INTERVAL_SECS
SYNVEDA_POLICY_REFRESH_SECS=2
export SYNVEDA_POLICY_REFRESH_SECS

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$MEM6_DB" -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "mem6-demo-$$" --name "MEM-6 Demo Tenant")
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
  # Give the gateway a moment to release its connections, then discard the
  # scratch database and the sidecar it wrote.
  sleep 1
  $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $MEM6_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR"
  rm -f "/tmp/mem6-retention-$$.json" "/tmp/mem6-pack-$$.cedar"
}
trap cleanup EXIT INT TERM

./target/debug/synveda-gateway &
GATEWAY_PID=$!

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8140/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8140$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8140$path"
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

# observe <token> <session> <occurred_at> <text> — `tool_result` is the
# kind the deterministic extractor routes to `episode`, the class a real
# retention schedule shortens first.
observe() {
  body="{\"session_id\":\"$2\",\"events\":[{\"idempotency_key\":\"$2-1\",
    \"kind\":\"tool_result\",\"payload\":{\"text\":\"$4\"},\"occurred_at\":\"$3\"}]}"
  accepted=$(api "$1" POST /v1/observe "$body" | field accepted)
  [ "$accepted" = "1" ] || {
    echo "demo FAILED: observe was not accepted ($accepted)" >&2
    exit 1
  }
}

block() {
  api "$1" POST /v1/inject "{\"session_id\":\"$2\"}" | field text
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

long_ago=$(date -u -d '90 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-90d +%Y-%m-%dT%H:%M:%SZ)
yesterday=$(date -u -d '1 day ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-1d +%Y-%m-%dT%H:%M:%SZ)

OLD="Session summary: we walked the staging cluster runbook end to end."
RECENT="Session summary: we rotated the payments sandbox credentials."

echo
echo "==> [1/6] alice's sessions: one ninety days ago, one yesterday."
observe "$alice_token" mem6-old "$long_ago" "$OLD"
observe "$alice_token" mem6-recent "$yesterday" "$RECENT"
wait_for_records 2
before=$(block "$alice_token" cold-1)
echo "$before" | grep -q "runbook" || {
  echo "demo FAILED: the old summary never reached the block" >&2
  exit 1
}
echo "$before" | grep -q "credentials" || {
  echo "demo FAILED: the recent summary never reached the block" >&2
  exit 1
}
echo "    a cold session start receives both — this is the product default,"
echo "    where the machinery is on and no record horizon is set."

echo
echo "==> [2/6] AC part one: an operator applies a RETENTION SCHEDULE."
echo "    Nobody touches a record. No sweep runs. Nothing restarts."
cat >/tmp/mem6-retention-$$.json <<'JSON'
{
  "mode": "enforce",
  "ttl": { "episode": 30 },
  "destroy_after_days": 0,
  "staging_days": 7,
  "staleness_half_life_days": 90
}
JSON
cat >/tmp/mem6-pack-$$.cedar <<'CEDAR'
permit (principal, action == Synveda::Action::"MemoryRead", resource)
when { principal in resource };
permit (principal, action == Synveda::Action::"MemoryWrite", resource)
when { principal has home && resource == principal.home };
CEDAR
./target/debug/synveda policy apply --tenant "$tenant_id" --name mem6-schedule \
  --retention "/tmp/mem6-retention-$$.json" "/tmp/mem6-pack-$$.cedar" >/dev/null
psql_t "select 'the stored schedule: ' || retention::text
        from policy_packs where tenant_id = '$tenant_id' and name = 'mem6-schedule'"
api "$admin_token" PUT /v1/policy/default '{"name":"mem6-schedule"}' >/dev/null
echo "    waiting for the gateway's pack refresher..."
tries=0
while block "$alice_token" "probe-$tries" | grep -q "runbook"; do
  tries=$((tries + 1))
  if [ "$tries" -ge 20 ]; then
    echo "demo FAILED: the schedule never reached the read path" >&2
    exit 1
  fi
  sleep 1
done
after=$(block "$alice_token" cold-2)
if echo "$after" | grep -q "runbook"; then
  echo "demo FAILED: material past the horizon is still being injected!" >&2
  exit 1
fi
echo "$after" | grep -q "credentials" || {
  echo "demo FAILED: material inside the horizon stopped composing" >&2
  exit 1
}
echo "    the very next inject carries the recent summary and NOT the old one."
echo "    Nothing was stamped on that record when it was written: the pack is"
echo "    read in the query that asks, so this holds whether or not the sweep"
echo "    has run yet — enforcement is the read path's, disposal is the"
echo "    sweep's (ADR-0040 decision 2). Records still in the store: $(records_now)."

echo
echo "==> [3/6] AC part two: the gateway's OWN sweep expires it, and says so."
tries=0
until [ "$(records_now)" = "1" ]; do
  tries=$((tries + 1))
  if [ "$tries" -ge 90 ]; then
    echo "demo FAILED: the sweep never expired the due record" >&2
    exit 1
  fi
  sleep 1
done
echo "    records: $(records_now)"
psql_t "select 'horizon: ' || (payload -> 'horizons' -> 0 ->> 'class') || ' @ ' ||
        (payload -> 'horizons' -> 0 ->> 'ttl_days') || ' days   age: ' ||
        (payload -> 'records' -> 0 ->> 'age_days') || ' days   actor: ' || actor_kind
        from audit_log where tenant_id = '$tenant_id' and action = 'memory.expired'"
psql_t "select 'content in the payload: ' ||
        case when payload::text like '%runbook%' then 'YES (bug!)' else 'none' end
        from audit_log where tenant_id = '$tenant_id' and action = 'memory.expired'"
versions=$(psql_t "select count(*) from records_versions
                   where tenant_id = '$tenant_id' and content like '%runbook%'")
[ "$versions" -ge 1 ] || {
  echo "demo FAILED: the expired record's history is gone already" >&2
  exit 1
}
echo "    the expired record left the live corpus; its version is still in"
echo "    records_versions ($versions) — 'what did the agent know in April'"
echo "    still has an answer, which is what the FIRST horizon means."

echo
echo "==> [4/6] pinned material of the same age is exempt (seed §4.2)."
# Pinned records have no authoring surface yet (PRMT-2 brings one), so this
# one is written directly — the embedding rides with it, because MEM-4 makes
# an embedding-less record impossible to commit.
pinned_id=$(psql_t "
  with r as (
    insert into records (id, tenant_id, scope_id, owner_id, kind, class, content,
                         sensitivity, provenance, valid_from, tx_from)
    select gen_random_uuid(), '$tenant_id', i.scope_id, i.id, 'pinned', 'episode',
           'Canonical: the incident review of the 2025 outage.', 'internal',
           '{\"source\":\"demo\"}'::jsonb, now() - interval '400 days', now()
    from identities i where i.tenant_id = '$tenant_id' and i.subject = 'alice'
    returning id, tenant_id
  ), e as (
    insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
    select id, tenant_id, 'hash@1', 16,
           array_fill(0.1::real, array[16])::vector from r
  )
  select id from r")
sleep 10
still_there=$(psql_t "select count(*) from records where id = '$pinned_id'")
[ "$still_there" = "1" ] || {
  echo "demo FAILED: the sweep expired PINNED material" >&2
  exit 1
}
block "$alice_token" cold-3 | grep -q "incident review" || {
  echo "demo FAILED: pinned material did not compose" >&2
  exit 1
}
echo "    400 days old, under a 30-day horizon, and it both composes and"
echo "    survives the sweep. There is no pack field that could change that."

echo
echo "==> [5/6] the SECOND horizon: destruction, and what it costs."
# A destruction horizon is measured in days from the instant a version
# closed, and a demo cannot wait a day: the archived row is aged through the
# superuser connection, triggers suspended, exactly as the AC test does.
psql_t "alter table records_history disable trigger records_history_append_only" >/dev/null
psql_t "update records_history
        set tx_from = tx_from - interval '20 days', tx_to = tx_to - interval '10 days'
        where tenant_id = '$tenant_id'" >/dev/null
psql_t "alter table records_history enable trigger records_history_append_only" >/dev/null
cat >/tmp/mem6-retention-$$.json <<'JSON'
{
  "mode": "enforce",
  "ttl": { "episode": 30 },
  "destroy_after_days": 7,
  "staging_days": 7,
  "staleness_half_life_days": 90
}
JSON
./target/debug/synveda policy apply --tenant "$tenant_id" --name mem6-schedule \
  --retention "/tmp/mem6-retention-$$.json" "/tmp/mem6-pack-$$.cedar" >/dev/null
tries=0
until [ "$(psql_t "select count(*) from records_versions
                   where tenant_id = '$tenant_id' and content like '%runbook%'")" = "0" ]; do
  tries=$((tries + 1))
  if [ "$tries" -ge 90 ]; then
    echo "demo FAILED: the destruction horizon never took the history" >&2
    exit 1
  fi
  sleep 1
done
echo "    the content is gone from every version the database holds."
psql_t "select 'plane: ' || (payload ->> 'plane') || '   versions: ' ||
        (payload ->> 'versions') || '   horizon: ' ||
        (payload ->> 'destroy_after_days') || ' days   actor: ' || actor_kind
        from audit_log where tenant_id = '$tenant_id'
          and action = 'memory.disposed'
          and payload ->> 'plane' = 'records_history'"
echo "    the as-of question that had an answer a moment ago no longer does."
echo "    That is the difference between the two horizons, and it is the"
echo "    half of 'retention enforced' the product did not have before."

echo
echo "==> [6/6] the observe staging plane, disposed of on its own horizon."
staged_before=$(psql_t "select count(*) from observe_events where tenant_id = '$tenant_id'")
psql_t "update observe_events set received_at = received_at - interval '40 days'
        where tenant_id = '$tenant_id'" >/dev/null
tries=0
until [ "$(psql_t "select count(*) from observe_events where tenant_id = '$tenant_id'")" = "0" ]; do
  tries=$((tries + 1))
  if [ "$tries" -ge 90 ]; then
    echo "demo FAILED: the staging plane was never disposed of" >&2
    exit 1
  fi
  sleep 1
done
echo "    staged payloads: $staged_before -> 0 (MEM-1 has kept every one until now)"
psql_t "select 'plane: ' || (payload ->> 'plane') || '   events: ' ||
        (payload ->> 'events') || '   pending reviews aged out: ' ||
        (payload ->> 'quarantine_pending')
        from audit_log where tenant_id = '$tenant_id'
          and action = 'memory.disposed'
          and payload ->> 'plane' = 'observe_staging'"
echo "    the records extracted from those payloads are untouched: $(records_now)"

echo
echo "==> the trail, in order, and the chain over all of it"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 6
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> the AC suites"
cargo test -p synveda-gateway --test retention
cargo test -p synveda-retrieval --test compose
cargo test -p synveda-store --test rls
cargo test -p synveda-types --lib retention

echo
echo "MEM-6 demo OK: a schedule applied to a running system governed the very"
echo "next inject with nobody acting, the sweep expired what it caught and"
echo "chained it under actor_kind=system, pinned material was exempt, the"
echo "second horizon destroyed what the first had only retired, the staging"
echo "plane was disposed of, and the chain verified over all of it."
