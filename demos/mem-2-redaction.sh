#!/usr/bin/env sh
# MEM-2 acceptance demo: redaction & secret scanning (ADR-0021).
# AC (docs/backlog/MEM-2.md): seeded secrets never reach storage in any
# mode; quarantine review queue works. Plus the feature text: PII
# patterns + gitleaks-derived secret rules; modes deny/redact/quarantine
# per policy pack.
#
# Flow: migrate -> admit a tenant -> org/team over the API -> an agent is
# registered at the team -> under regulated-strict (zero-config default)
# a batch carrying a seeded AWS key, an email, and a card is observed:
# the secret event QUARANTINES (staged redacted, NO work signal), the PII
# redacts and flows -> the raw literals are swept for across staging,
# quarantine, audit, and both queue tables (zero hits — THE AC) -> a
# security-reviewer works the queue: list shows redacted content only,
# release sends the standard signal, a second verdict conflicts, the
# agent itself is denied the queue -> a custom pack applied with
# --redaction-secrets deny flips the mode: the same secret is now refused
# per event, nothing persists -> audit tail + verify -> the AC test suite
# runs. On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8136
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=mem-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# Tight reload pacing so the deny-pack switch is visible immediately.
SYNVEDA_POLICY_REFRESH_SECS=1
export SYNVEDA_POLICY_REFRESH_SECS

# The seeded findings — vendor documentation examples, never real
# credentials (the same fixtures the AC test uses).
SEEDED_KEY="AKIAIOSFODNN7EXAMPLE"
SEEDED_EMAIL="leaky.human@example.com"
SEEDED_CARD="4111 1111 1111 1111"

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "mem2-demo-$(date +%s)-$$" --name "MEM-2 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
reviewer_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-reviewer)

echo "==> bootstrap bindings: org-admin for the admin, security-reviewer"
echo "    for the reviewer (its first live action, ADR-0021 decision 6)"
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-reviewer --role security-reviewer >/dev/null

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8136/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8136$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8136$path"
  fi
}

code() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  curl -s -o /dev/null -w '%{http_code}' -X "$method" \
    -H "Authorization: Bearer $tok" -H "Content-Type: application/json" \
    ${body:+-d "$body"} "http://127.0.0.1:8136$path"
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

# Sweeps every storage surface for a literal; prints the hit count.
sweep() {
  literal=$1
  compact=$(echo "$literal" | tr -d ' ')
  psql_t "select
      (select count(*) from observe_events
        where tenant_id = '$tenant_id'
          and (payload::text like '%${literal}%'
               or payload::text like '%${compact}%'
               or coalesce(redactions::text,'') like '%${literal}%'))
    + (select count(*) from observe_quarantine
        where tenant_id = '$tenant_id' and findings::text like '%${literal}%')
    + (select count(*) from audit_log
        where tenant_id = '$tenant_id' and payload::text like '%${literal}%')
    + (select count(*) from pgmq.q_observe
        where message->>'tenant_id' = '$tenant_id'
          and message::text like '%${literal}%')
    + (select count(*) from pgmq.a_observe
        where message->>'tenant_id' = '$tenant_id'
          and message::text like '%${literal}%')"
}

assert_clean() {
  for literal in "$SEEDED_KEY" "$SEEDED_EMAIL" "$SEEDED_CARD"; do
    hits=$(sweep "$literal")
    [ "$hits" = "0" ] || {
      echo "demo FAILED: seeded literal '$literal' found in storage ($hits hits)" >&2
      exit 1
    }
  done
}

echo "==> the admin builds the hierarchy; an agent is registered at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject demo-agent --scope "$team_id" >/dev/null
agent_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-agent)
echo "    org=$org_id team=$team_id agent=demo-agent"

now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
secret_batch="{\"session_id\":\"demo-leak-1\",\"events\":[
  {\"idempotency_key\":\"m1\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"here are my creds: $SEEDED_KEY please remember\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"m2\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"invoice $SEEDED_CARD for $SEEDED_EMAIL\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"m3\",\"kind\":\"decision\",
   \"payload\":{\"decision\":\"retries use exponential backoff\"},\"occurred_at\":\"$now\"}]}"

echo "==> QUARANTINE mode (regulated-strict, the zero-config default):"
echo "    a seeded AWS key, a card + email, and a clean event"
first=$(api "$agent_token" POST /v1/observe "$secret_batch")
[ "$(echo "$first" | field quarantined)" = "1" ] &&
  [ "$(echo "$first" | field accepted)" = "2" ] || {
  echo "demo FAILED: expected 1 quarantined + 2 accepted, got: $first" >&2
  exit 1
}
q_event=$(echo "$first" | field events | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d)[0].event_id));
')
echo "    quarantined=1 (the secret) accepted=2 (PII redacted + clean)"

echo "==> the quarantined event staged REDACTED and sent no work signal"
signals=$(psql_t "select count(*) from pgmq.q_observe where message->>'tenant_id' = '$tenant_id'")
[ "$signals" = "2" ] || {
  echo "demo FAILED: expected 2 signals (quarantine must not signal), got $signals" >&2
  exit 1
}
echo "    signals=2; the staged secret event reads:"
psql_t "select payload->>'text' from observe_events
        where tenant_id = '$tenant_id' and idempotency_key = 'm1'" | sed 's/^/      /'
echo "    the staged PII event reads:"
psql_t "select payload->>'text' from observe_events
        where tenant_id = '$tenant_id' and idempotency_key = 'm2'" | sed 's/^/      /'

echo "==> THE AC SWEEP: the seeded literals appear NOWHERE in storage"
assert_clean
echo "    0 hits for the key, the email, and the card across staging,"
echo "    quarantine, audit, and both queue tables"

echo "==> the review queue: the agent itself is denied; the reviewer sees"
echo "    redacted content only"
c=$(code "$agent_token" GET /v1/quarantine)
[ "$c" = "403" ] || {
  echo "demo FAILED: the owner must not review its own quarantine, got $c" >&2
  exit 1
}
queue=$(api "$reviewer_token" GET /v1/quarantine)
echo "$queue" | field pending | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const p = JSON.parse(d);
    if (p.length !== 1) { console.error("expected 1 pending, got", p.length); process.exit(1); }
    const text = p[0].payload.text;
    if (!text.includes("[REDACTED:aws-access-key-id]")) {
      console.error("reviewer must see the placeholder, got:", text); process.exit(1);
    }
    console.log("      pending:", p[0].event_id);
    console.log("      payload:", text);
    console.log("      findings:", JSON.stringify(p[0].findings));
  });
'

echo "==> release: the standard work signal goes out; review is one-shot"
released=$(api "$reviewer_token" POST "/v1/quarantine/$q_event/release" \
  '{"reason":"vendor docs example key; safe to extract"}')
[ "$(echo "$released" | field state)" = "released" ] || {
  echo "demo FAILED: expected released, got: $released" >&2
  exit 1
}
signals=$(psql_t "select count(*) from pgmq.q_observe where message->>'tenant_id' = '$tenant_id'")
[ "$signals" = "3" ] || {
  echo "demo FAILED: release must send exactly one signal, got $signals" >&2
  exit 1
}
c=$(code "$reviewer_token" POST "/v1/quarantine/$q_event/reject" '{}')
[ "$c" = "409" ] || {
  echo "demo FAILED: a second verdict must conflict, got $c" >&2
  exit 1
}
echo "    released; signals=3; second verdict=409"

echo "==> DENY mode: a custom pack with --redaction-secrets deny"
pack_file=$(mktemp)
cat > "$pack_file" <<'CEDAR'
// The member floor plus the write floor — enough for the demo agent.
permit (principal, action == Synveda::Action::"MemoryRead", resource)
when { principal in resource };
permit (principal, action == Synveda::Action::"MemoryWrite", resource)
when { principal has home && resource == principal.home };
CEDAR
./target/debug/synveda policy apply --tenant "$tenant_id" --name acme-deny \
  --redaction-secrets deny --redaction-pii redact "$pack_file" >/dev/null
rm -f "$pack_file"
api "$admin_token" PUT "/v1/hierarchy/nodes/$org_id/policy" \
  '{"name":"acme-deny"}' >/dev/null
sleep 3 # one refresh interval: the stored pack hot-loads

deny_batch="{\"session_id\":\"demo-leak-2\",\"events\":[
  {\"idempotency_key\":\"m4\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"again: $SEEDED_KEY\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"m5\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"a clean sibling\"},\"occurred_at\":\"$now\"}]}"
denied=$(api "$agent_token" POST /v1/observe "$deny_batch")
[ "$(echo "$denied" | field denied)" = "1" ] &&
  [ "$(echo "$denied" | field accepted)" = "1" ] || {
  echo "demo FAILED: expected 1 denied + 1 accepted, got: $denied" >&2
  exit 1
}
staged_m4=$(psql_t "select count(*) from observe_events
                    where tenant_id = '$tenant_id' and idempotency_key = 'm4'")
[ "$staged_m4" = "0" ] || {
  echo "demo FAILED: a denied event must persist nothing" >&2
  exit 1
}
echo "    denied=1 (nothing persisted), the clean sibling admitted"

echo "==> the sweep still finds nothing, after all three modes"
assert_clean

echo "==> the audit trail: batch events with counts + rule ids, the"
echo "    release chained, and the chain verifies"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 6
./target/debug/synveda audit verify --tenant "$tenant_id"

echo "==> redaction metrics on /metrics"
curl -fsS http://127.0.0.1:8136/metrics |
  grep -E '^synveda_(redaction_findings|observe_events|quarantine)' | head -8

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo "==> the AC test suite (all three modes + the review contract)"
cargo test -p synveda-gateway --test observe_redaction -- --nocapture
cargo test -p synveda-ingest

echo
echo "MEM-2 demo PASSED: scanning runs before persistence — the seeded"
echo "key, email, and card reached no table, queue, or audit row in any"
echo "mode; regulated-strict quarantined the secret (redacted, signal-"
echo "less) and the review queue released it with one standard signal;"
echo "a custom pack's deny mode refused the event outright."
