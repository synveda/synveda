#!/usr/bin/env sh
# AUD-2 acceptance demo: the audit query surface (ADR-0045).
# AC (docs/backlog/AUD-2.md): both questions — "who could see X on date D"
# and "what did agent A know at time T" — answerable via one API call
# each (uses bitemporal + refs).
#
# The whole demo turns on one idea: the surface answers from the chain as
# it was **recorded**, never from a replay of the state that produced it.
# So nothing here seeds an audit row. Alice and bob work; the chain
# records what they were served; and dana — an auditor who holds
# `AuditRead` and no `MemoryRead` — asks about it afterwards.
#
# The auditor's half runs with **DATABASE_URL unset**. Every answer in
# sections [1] through [6] can therefore only have come through the
# gateway, under the PDP, on dana's own bearer — which is what makes the
# refusals in [4] mean something.
#
# Flow: postgres up -> tenant, hierarchy, five identities -> alice and bob
# work, carol works on payments -> [1] Q1: who could see alice's record,
# one call -> [2] the answer is TWO lists it refuses to merge -> [3] Q2:
# what did alice know, one call, and the same call at an earlier instant
# -> [4] the refusals in the product's own words -> [5] no content reaches
# an audit answer, swept -> [6] reading the trail is itself on the trail,
# and the chain verifies over all of it.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI and no IdP
# (network-free deterministic extractor and embedder throughout).
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8146
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=aud-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
RUST_LOG=${RUST_LOG:-error}
export RUST_LOG

BASE=http://127.0.0.1:8146

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "aud2-demo-$$" --name "AUD-2 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
# The bootstrap, and the only direct write in this demo: `synveda role
# bind` is how a tenant gets its first org-admin (AUTHZ-3), and it chains
# as break-glass rather than as a governed act.
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

echo "==> purging leftover observe-queue signals from other runs (shared queue)"
psql_t "select pgmq.purge_queue('observe')" >/dev/null

./target/debug/synveda-gateway &
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

# The HTTP status only — for the refusals, where the point is the code.
code() {
  tok=$1
  method=$2
  path=$3
  curl -s -o /dev/null -w '%{http_code}' -X "$method" \
    -H "Authorization: Bearer $tok" "$BASE$path"
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

# Renders a JSON array as an aligned table of the named keys.
table() {
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const keys = process.argv.slice(1);
      const rows = JSON.parse(d);
      if (!rows.length) { console.log("      (none)"); return; }
      const w = keys.map((k) =>
        Math.max(k.length, ...rows.map((r) => String(r[k] ?? "-").length)));
      const line = (cells) =>
        "      " + cells.map((c, i) => String(c).padEnd(w[i])).join("  ");
      console.log(line(keys));
      console.log(line(w.map((n) => "-".repeat(n))));
      for (const r of rows) console.log(line(keys.map((k) => r[k] ?? "-")));
    });
  ' "$@"
}

wait_for_records() {
  want=$1
  tries=0
  while :; do
    have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
    [ "$have" = "$want" ] && return 0
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      echo "demo FAILED: expected $want records, stuck at $have after $tries tries" >&2
      exit 1
    fi
    sleep 0.5
  done
}

# observe <token> <session> <idem> <text>
observe() {
  now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  body="{\"session_id\":\"$2\",\"events\":[{\"idempotency_key\":\"$3\",
    \"kind\":\"decision\",\"payload\":{\"text\":\"$4\"},\"occurred_at\":\"$now\"}]}"
  accepted=$(api "$1" POST /v1/observe "$body" | field accepted)
  [ "$accepted" = "1" ] || {
    echo "demo FAILED: observe was not accepted ($accepted)" >&2
    exit 1
  }
}

echo "==> the admin builds the hierarchy"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
platform_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
payments_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}" |
  field id)
echo "    org=$org_id platform=$platform_id payments=$payments_id"

echo "==> three workers: alice and bob on platform, carol on payments."
for who in alice bob; do
  ./target/debug/synveda service register --tenant "$tenant_id" \
    --subject "$who" --scope "$platform_id" >/dev/null
done
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject carol --scope "$payments_id" >/dev/null

# dana and erin get NO placement and NO identity row — only a role
# binding, which is subject-keyed and may precede a first login (ADR-0015
# decision 2). That is not a shortcut, it is the shape of the role: an
# auditor is a member of nothing, so the membership floor grants them no
# `MemoryRead` anywhere, and every byte they see below comes from
# `AuditRead` alone. It is also why the workers above cannot be the
# auditors: alice and bob are *service* identities, and AUTH-3 confines a
# service token to its anchor subtree — the tenant plane is never inside
# one, so no agent can read the trail however it is bound (ADR-0018
# decision 4, which AUD-2 inherits without naming it).

alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
bob_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject bob)
carol_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject carol)
dana_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject dana)
erin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject erin)

# The two auditor bindings go through the product surface, not the store.
# That matters here more than anywhere: `role_bindings` is a current-state
# table and an unbound role leaves no row, so `role.bound` on the chain is
# the ONLY record that dana held `auditor` today — which is exactly what
# the authority half of section [2] reads.
api "$admin_token" PUT /v1/roles/bindings \
  '{"subject":"dana","role":"auditor"}' >/dev/null
api "$admin_token" PUT "/v1/hierarchy/nodes/$platform_id/roles" \
  '{"subject":"erin","role":"auditor"}' >/dev/null
echo "    dana: auditor tenant-wide.  erin: the SAME role, at platform only."
echo "    Neither is placed anywhere: an auditor is a member of nothing."

before=$(date -u +%Y-%m-%dT%H:%M:%SZ)
sleep 1

echo
echo "==> alice, bob and carol work. Nothing here is about auditing:"
echo "    this is the product being used, and the chain recording it."
observe "$alice_token" aud2-alice-1 alice-1 \
  "We decided the ledger service keeps its reconciliation window at four hours."
wait_for_records 1
observe "$bob_token" aud2-bob-1 bob-1 \
  "We decided the platform on-call rotation moves to a two-week cycle."
wait_for_records 2
observe "$carol_token" aud2-carol-1 carol-1 \
  "We decided settlement cutover happens on the last business day."
wait_for_records 3

# Two sessions for alice, so "who could see it" has more than one occasion
# to report and the knowledge fold has something to fold.
api "$alice_token" POST /v1/inject '{"session_id":"aud2-alice-s1"}' >/dev/null
api "$alice_token" POST /v1/inject '{"session_id":"aud2-alice-s2"}' >/dev/null
api "$bob_token" POST /v1/inject '{"session_id":"aud2-bob-s1"}' >/dev/null
api "$carol_token" POST /v1/inject '{"session_id":"aud2-carol-s1"}' >/dev/null
echo "    four session starts; the chain now holds four context.injected events"

alice_record=$(psql_t "select r.id from records r
                       join identities i on i.id = r.owner_id
                       where r.tenant_id = '$tenant_id' and i.subject = 'alice'
                       limit 1")
echo "    alice's record: $alice_record"

# ── From here on the auditor has no database. ────────────────────────
unset DATABASE_URL
echo
echo "==> DATABASE_URL is now UNSET. Everything below is dana's bearer"
echo "    against $BASE, decided by the PDP."

window_from=$(date -u -d '1 day ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-1d +%Y-%m-%dT%H:%M:%SZ)
window_until=$(date -u -d '1 day' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v+1d +%Y-%m-%dT%H:%M:%SZ)

echo
echo "==> [1/6] Q1: \"who could see X on date D\" — ONE call."
disclosures=$(api "$dana_token" GET \
  "/v1/audit/disclosures?record=$alice_record&from=$window_from&until=$window_until")
echo "$disclosures" | field disclosed |
  table actor_subject action session_id tier seq
served=$(echo "$disclosures" | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const s = new Set(JSON.parse(d).disclosed.map((r) => r.actor_subject));
    console.log([...s].sort().join(","));
  });
')
echo "    served to: $served"
[ "$served" = "alice" ] || {
  echo "demo FAILED: expected alice alone, got '$served'" >&2
  exit 1
}
echo "    Alice, in both her sessions. Not bob, not carol — the record is"
echo "    hers, and the chain says who was actually handed it rather than"
echo "    who might have been. Each row names the version served, so the"
echo "    finding is re-derivable by someone who does not trust dana."

echo
echo "==> [2/6] the answer is TWO lists, and it refuses to merge them."
echo "$disclosures" | field authority | table action actor_subject outcome seq
echo
echo "$disclosures" | field note | fold -s -w 68 | sed 's/^/    /'
echo
echo "    'disclosed' is evidence: who was served it. 'authority' is the"
echo "    state that governed the window — here the role.bound events that"
echo "    made dana an auditor at all, which live NOWHERE else, because"
echo "    role_bindings is current-state and an unbound role leaves no row."
echo "    Merging them into one 'could see' set would mean deciding over"
echo "    reconstructed inputs, and that is the replay ADR-0042 refused."

echo
echo "==> [3/6] Q2: \"what did agent A know at time T\" — ONE call."
# No `at`: it defaults to now, server-side. Passing a shell `date` here
# would be wrong by up to a second — the format truncates *down*, and
# the window is `occurred_at <= at`, so a just-served record would fall
# outside an instant meant to include it.
knowledge=$(api "$dana_token" GET "/v1/audit/knowledge?subject=alice")
echo "$knowledge" | field known |
  table record_id action occasions seq occurred_at
now=$(echo "$knowledge" | field at)
known_now=$(echo "$knowledge" | field known | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).length));
')
echo "    records alice had been served by $now: $known_now"
[ "$known_now" != "0" ] || {
  echo "demo FAILED: alice was served material and the answer is empty" >&2
  exit 1
}

echo
echo "    The SAME call, asked at an instant before her first session:"
earlier=$(api "$dana_token" GET "/v1/audit/knowledge?subject=alice&at=$before")
known_before=$(echo "$earlier" | field known | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).length));
')
echo "    records alice had been served by $before: $known_before"
[ "$known_before" = "0" ] || {
  echo "demo FAILED: alice knew $known_before records before she started" >&2
  exit 1
}
echo "    One parameter changed. That is the AC's 'uses bitemporal': the"
echo "    answer names versions by hash, and every id in it resolves in the"
echo "    bitemporal pair at the instant asked at — so the audit answer and"
echo "    the corpus agree, and neither has to be taken on trust."

echo
echo "==> [4/6] the refusals, in the product's own words."
erin_code=$(code "$erin_token" GET "/v1/audit/knowledge?subject=alice")
[ "$erin_code" = "403" ] || {
  echo "demo FAILED: erin got $erin_code, want 403" >&2
  exit 1
}
echo "    erin (auditor AT PLATFORM) asking the tenant's chain: $erin_code"
echo "      There is one chain per tenant, and an event's resource column is"
echo "      a scope for some actions and a binding or a tenant for others."
echo "      A subtree answer could only be 'the events we could attribute to"
echo "      your subtree', which silently omits the rest. So AuditRead names"
echo "      only the Tenant resource in the Cedar schema — a scope-scoped"
echo "      audit request does not type-check, let alone decide."

alice_code=$(code "$alice_token" GET "/v1/audit/events")
[ "$alice_code" = "403" ] || {
  echo "demo FAILED: alice got $alice_code, want 403" >&2
  exit 1
}
echo "    alice (the SUBJECT of the answers above) reading the trail: $alice_code"
echo "      She can be asked about and cannot ask. No role, no admin power."

typo_code=$(code "$dana_token" GET "/v1/audit/events?action=context.injcted")
[ "$typo_code" = "400" ] || {
  echo "demo FAILED: a misspelled action got $typo_code, want 400" >&2
  exit 1
}
echo "    dana misspelling an action name: $typo_code"
echo "      'No events' and 'you spelled it wrong' are different facts, and"
echo "      only one of them is an audit finding. Same for a limit over the"
echo "      cap, which is refused rather than quietly trimmed:"
cap_code=$(code "$dana_token" GET "/v1/audit/events?limit=99999")
[ "$cap_code" = "400" ] || {
  echo "demo FAILED: an over-cap limit got $cap_code, want 400" >&2
  exit 1
}
echo "      limit=99999 -> $cap_code"

echo
echo "==> [5/6] no record content reaches an audit answer."
echo "    dana holds AuditRead and no MemoryRead. Sweeping every response"
echo "    for the text of the memories the demo wrote:"
sweep=$(
  api "$dana_token" GET "/v1/audit/events?limit=500"
  api "$dana_token" GET \
    "/v1/audit/disclosures?record=$alice_record&from=$window_from&until=$window_until"
  api "$dana_token" GET "/v1/audit/knowledge?subject=alice"
  api "$dana_token" GET "/v1/audit/verify"
)
hits=0
for fragment in "reconciliation window" "on-call rotation" "settlement cutover"; do
  n=$(printf '%s' "$sweep" | grep -c "$fragment" || true)
  echo "      \"$fragment\": $n hits"
  hits=$((hits + n))
done
[ "$hits" = "0" ] || {
  echo "demo FAILED: $hits fragments of record content reached an audit answer" >&2
  exit 1
}
echo "    zero. The surface has no content path to forget to gate: it"
echo "    returns ids, addresses, channels and tiers, and resolving any of"
echo "    them to a body is MemoryRead through /v1/recall — a different"
echo "    call, a different decision (seed §5: an auditor touches no content)."

echo
echo "==> [6/6] reading the trail is itself on the trail, and it verifies."
reads=$(api "$dana_token" GET \
  "/v1/audit/events?action=authz.decision&outcome=allow&actor=dana&limit=200")
echo "$reads" | field events | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const rows = JSON.parse(d).filter(
      (e) => e.payload?.authz?.action === "audit.read");
    console.log("      dana'"'"'s own audit reads on the chain: " + rows.length);
    for (const r of rows.slice(0, 5))
      console.log("        seq " + r.seq + "  op=" + (r.payload.op ?? "-"));
  });
'
echo "    Every allowed admin-plane read chains its decision, this one"
echo "    included. 'Who has been reading the trail' is a question a"
echo "    regulator asks — and it is why the pages are cursor-paginated:"
echo "    the chain grows underneath a reader who is reading it."
echo
verify=$(api "$dana_token" GET /v1/audit/verify)
valid=$(echo "$verify" | field valid)
events=$(echo "$verify" | field events)
head_hash=$(echo "$verify" | field head_hash)
echo "    chain verify: valid=$valid over $events events"
echo "    head: $head_hash"
[ "$valid" = "true" ] || {
  echo "demo FAILED: the chain does not verify" >&2
  exit 1
}
echo "    Recomputed from the rows, through the gateway, by an auditor with"
echo "    no database credentials — the surface 'synveda audit verify' has"
echo "    been standing in for since AUD-1."

echo
echo "==> the end-to-end acceptance suite over the real product path"
DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda \
  cargo test -p synveda-gateway --test audit_query -- --test-threads=1 2>&1 | tail -6

echo
echo "AUD-2 demo complete."
echo "  - \"who could see X on date D\": one call, answered from recorded"
echo "    disclosures, with the authority that governed them beside it and"
echo "    deliberately not merged into it"
echo "  - \"what did agent A know at time T\": one call, folded to one row"
echo "    per record, naming versions that resolve bitemporally"
echo "  - tenant-complete or refused: a subtree-bound auditor is denied,"
echo "    because a partial audit answer is a misleading one"
echo "  - an auditor reads no content, and the route has none to leak"
echo "  - the audit log records reads of the audit log, and verifies"
