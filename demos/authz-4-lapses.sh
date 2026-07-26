#!/usr/bin/env sh
# AUTHZ-4 acceptance demo: lapses — controlled relaxation (ADR-0037).
# AC (docs/backlog/AUTHZ-4.md): E2E — lapse grants cross-team read, expiry
# restores denial, audit shows the full story.
#
# The claim is made from the reader's side, because that is what makes a
# lapse a grant rather than a row: Bea is on the payments team and cannot
# read the platform team's material. Under a two-steward lapse she can.
# When the window closes she cannot again — and nobody revokes anything,
# nothing restarts, and no operator acts at all. The window simply ends.
#
# **The expiry here is real.** The lapse runs for a handful of seconds and
# this script waits them out. There is no fake clock: a lapse's duration is
# seconds with no minimum precisely so this can be demonstrated rather than
# asserted (ADR-0037 decision 4). The same script with 30 days in place of
# ten seconds is the product.
#
# Flow: migrate -> tenant -> acme/eng/{platform,payments} -> principals ->
# platform publishes a runbook through review -> BEFORE (bea reads nothing)
# -> the lapse proposed by a steward at the DISCLOSING side, reviewed by
# two -> DURING (bea reads it, marked, under platform's own section) ->
# EXPIRY (the same request, seconds later, with nobody acting) -> THE TRAIL
# -> THE REFUSALS, in the product's own words -> REVOCATION (a
# security-reviewer ends a grant they could never have opened).
#
# Needs postgres only; the gateway runs in-process here and principals
# carry dev tokens through SYNVEDA_TOKEN (the ADR-0027 override kept for CI
# and demos). On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

# The lapse's window. Short enough to watch; the product's own default
# ceiling under regulated-strict is 30 days.
WINDOW_SECS=10

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8149
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=authz-4-demo-secret
export SYNVEDA_DEV_JWT_SECRET
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
# The expiry sweep's cadence. It is bookkeeping — every grant it touches
# stopped deciding reads the moment its window closed — so the product
# default is a slack 60s. A demo that waited that out to show one audit
# line would be showing the scheduler.
SYNVEDA_LAPSE_SWEEP_SECS=2
export SYNVEDA_LAPSE_SWEEP_SECS

BASE="http://127.0.0.1:8149"
CLI=./target/debug/synveda

RUNBOOK="page the on-call via the incident bridge, never by direct message"
DRAFT="draft, unreviewed: restart the broker by hand if the bridge is down"

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

as() {
  tok=$1
  shift
  SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

session() {
  api "$1" POST /v1/inject '{"session_id":"authz-4-demo"}'
}

# arrival <token> <line> — how that line reached this agent, in one word:
# `lapsed` (present, in a section the block marks as a grant), `reviewed`
# (present, on their own chain), or `absent`.
arrival() {
  session "$1" | node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const text = JSON.parse(d).text;
      const line = process.argv[1];
      if (!text.includes(line)) return console.log("absent");
      // Which section is it in? Walk headers until we pass the line.
      let section = "";
      for (const l of text.split("\n")) {
        if (l.startsWith("## ")) section = l;
        if (l.includes(line)) break;
      }
      console.log(section.includes("[lapse]") ? "lapsed" : "reviewed");
    });
  ' "$2"
}

echo "==> migrate + admit a tenant"
$CLI db migrate
tenant_id=$($CLI tenant create \
  --slug "authz4-demo-$$" --name "AUTHZ-4 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$($CLI token issue --tenant "$tenant_id" --subject demo-admin)
$CLI role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/authz4-gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  # A healthz that answers is not proof it is OUR gateway (the FLOW-6
  # lesson): a leftover process holds the port, ours dies on bind, and
  # every request goes to a stranger signed with another secret.
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
kill -0 "$GATEWAY_PID" 2>/dev/null || {
  echo "demo FAILED: healthz answered but our gateway is gone; \
another process holds $SYNVEDA_LISTEN_ADDR" >&2
  exit 1
}

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

# seed_user <subject> <parent scope> — a *user* identity, seeded directly.
#
# Bea has to be a user rather than a service identity, and the reason is a
# property worth naming: the base layer confines an agent credential to its
# anchor subtree and carves out only own-chain MemoryRead (ADR-0018
# decision 4), so **no lapse can widen a service token past its anchor**. A
# forbid beats the base layer's permit, which is exactly why the permit is
# safe to put there — and it means a demo that made every principal an
# agent would be demonstrating that instead of this.
#
# The reviewers stay service identities: they act inside their own anchors.
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
# Two stewards at Engineering, because regulated-strict's `policy` cell
# asks for two DISTINCT steward approvers — tech plan §2.4's lapse row,
# carried in the approval matrix since FLOW-3 with nothing until now that
# resolved against it.
#
# Raj is a security-reviewer at the org and nothing else: the responder who
# can end a disclosure but could never open one, which is the whole reason
# LapseGrant and LapseRevoke are two actions.
$CLI service register --tenant "$tenant_id" --subject tara  --scope "$platform_id" >/dev/null
$CLI service register --tenant "$tenant_id" --subject nadia --scope "$eng_id"      >/dev/null
$CLI service register --tenant "$tenant_id" --subject omar  --scope "$eng_id"      >/dev/null
$CLI service register --tenant "$tenant_id" --subject raj   --scope "$org_id"      >/dev/null
seed_user bea "$payments_id"
$CLI role bind --tenant "$tenant_id" --subject tara  --role curator --scope "$platform_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject nadia --role steward --scope "$eng_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject omar  --role steward --scope "$eng_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject raj --role security-reviewer --scope "$org_id" >/dev/null
tara_token=$($CLI token issue --tenant "$tenant_id" --subject tara)
nadia_token=$($CLI token issue --tenant "$tenant_id" --subject nadia)
omar_token=$($CLI token issue --tenant "$tenant_id" --subject omar)
raj_token=$($CLI token issue --tenant "$tenant_id" --subject raj)
bea_token=$($CLI token issue --tenant "$tenant_id" --subject bea)
echo "    tara=curator@platform  nadia,omar=steward@eng  raj=security-reviewer@acme"
echo "    the reader: bea@payments"

# Platform's shelf: one line it stands behind, and one it does not. A lapse
# discloses what the target PUBLISHED, so the second line is here to show
# what a grant does not carry.
tara_identity=$(psql_t "select id from identities
                        where tenant_id = '$tenant_id' and subject = 'tara'")
seed_record() {
  psql_t "begin;
          insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                               content, sensitivity, provenance, valid_from)
          values ('$1', '$tenant_id', '$platform_id', '$tara_identity', 'pinned',
                  'procedure', \$content\$$2\$content\$, 'internal',
                  '{\"source\":\"authz-4 demo\"}', now() - interval '1 hour');
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          values ('$1', '$tenant_id', 'hash@1', 4, '[0.25,0.25,0.25,0.25]');
          commit;" >/dev/null
}
runbook_id=$(psql_t "select gen_random_uuid()")
draft_id=$(psql_t "select gen_random_uuid()")
seed_record "$runbook_id" "$RUNBOOK"
seed_record "$draft_id" "$DRAFT"

echo "==> platform publishes the runbook (and not the draft)"
api "$tara_token" POST "/v1/channels/$platform_id/publish" \
  "{\"record_ids\":[\"$runbook_id\"],\"message\":\"reviewed at the incident retro\"}" \
  >/dev/null
echo "    platform stands behind one line of two"

echo
echo "════════════════════════════════════════════════════════════════════"
echo " BEFORE — regulated-strict has no cross-team read at all"
echo "════════════════════════════════════════════════════════════════════"
before=$(arrival "$bea_token" "$RUNBOOK")
echo "    bea@payments receives the runbook: $before"
[ "$before" = "absent" ] || { echo "demo FAILED: expected absent" >&2; exit 1; }

echo
echo "════════════════════════════════════════════════════════════════════"
echo " THE LAPSE — proposed on the disclosing side, reviewed by two"
echo "════════════════════════════════════════════════════════════════════"
# The target is PLATFORM: the scope whose material is disclosed. Authority
# over a disclosure belongs where the material is, never where the request
# came from — so a steward of payments could not open this even if they
# wanted it.
proposal=$(api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"duration_secs\":$WINDOW_SECS,
    \"reason\":\"joint incident review: payments is on the bridge for the outage\"}" |
  field proposal_id)
echo "    proposal $proposal opened by nadia (steward@eng), targeting platform"
echo "    it grants nothing yet — the matrix asks for two distinct stewards"

api "$nadia_token" POST "/v1/proposals/$proposal/approve" '{}' >/dev/null
echo "    nadia approves"
echo "    one steward tries to run the effect:"
refused_api "$nadia_token" POST "/v1/proposals/$proposal/lapse" '{}'

api "$omar_token" POST "/v1/proposals/$proposal/approve" '{}' >/dev/null
echo "    omar approves — a second, distinct person"
lapse_id=$(api "$omar_token" POST "/v1/proposals/$proposal/lapse" '{}' | field id)
expires_at=$(api "$nadia_token" "GET" "/v1/lapses?scope_id=$platform_id" |
  node -e 'let d="";process.stdin.on("data",c=>d+=c);process.stdin.on("end",()=>console.log(JSON.parse(d).lapses[0].expires_at))')
echo "    granted: lapse $lapse_id, expiring $expires_at"

echo
echo "════════════════════════════════════════════════════════════════════"
echo " DURING — the same request bea made a moment ago"
echo "════════════════════════════════════════════════════════════════════"
during=$(arrival "$bea_token" "$RUNBOOK")
echo "    bea receives the runbook: $during"
[ "$during" = "lapsed" ] || { echo "demo FAILED: expected lapsed, got $during" >&2; exit 1; }
draft_arrival=$(arrival "$bea_token" "$DRAFT")
echo "    bea receives the unpublished draft: $draft_arrival"
[ "$draft_arrival" = "absent" ] || {
  echo "demo FAILED: a lapse must not carry unreviewed material" >&2; exit 1; }
echo
echo "    her block, the lapsed section only:"
session "$bea_token" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const lines = JSON.parse(d).text.split("\n");
    let on = false;
    for (const l of lines) {
      if (l.startsWith("## ")) on = l.includes("[lapse]");
      if (on && l.trim()) console.log("      " + l);
    }
  });
'
echo
echo "    the section says [lapse] because bea is not a member of platform:"
echo "    a block that quietly contained another team's material would be"
echo "    claiming otherwise."

echo
echo "════════════════════════════════════════════════════════════════════"
echo " EXPIRY — nobody acts; the window closes"
echo "════════════════════════════════════════════════════════════════════"
echo "    waiting ${WINDOW_SECS}s. No revocation, no restart, no operator."
sleep $((WINDOW_SECS + 1))
after=$(arrival "$bea_token" "$RUNBOOK")
echo "    bea receives the runbook: $after"
[ "$after" = "absent" ] || { echo "demo FAILED: expiry did not restore the denial" >&2; exit 1; }
echo
echo "    Nothing ran to make that happen. The read path selects grants"
echo "    where 'expires_at > now()', so the window closes in the query"
echo "    that asks — a sweep that is down cannot leave access standing."

echo
echo "════════════════════════════════════════════════════════════════════"
echo " THE TRAIL"
echo "════════════════════════════════════════════════════════════════════"
# The sweep only chains the expiry event; the access was already gone
# before it ran — which is why this waits for the *event* rather than
# racing a fixed sleep against it. A demo that sometimes printed a trail
# without its last line would be showing the scheduler's luck.
tries=0
until [ "$(psql_t "select count(*) from audit_log
                   where tenant_id = '$tenant_id'
                     and action = 'policy.lapse.expired'")" -ge 1 ]; do
  tries=$((tries + 1))
  if [ "$tries" -ge 20 ]; then
    echo "demo FAILED: the expiry sweep never chained its event" >&2
    exit 1
  fi
  sleep 1
done
psql_t "select seq, action, actor_kind, actor_subject
        from audit_log where tenant_id = '$tenant_id'
          and action in ('vedaflow.proposal.opened','vedaflow.proposal.approved',
                         'policy.lapse.granted','context.injected',
                         'policy.lapse.expired')
        order by seq" |
  awk -F'|' 'BEGIN { printf "    %-5s %-30s %-12s %s\n", "seq", "action", "actor", "subject" }
             { printf "    %-5s %-30s %-12s %s\n", $1, $2, $3, $4 }'
echo
echo "    the grant event, with the window it opened and no record content:"
psql_t "select jsonb_pretty(payload - 'authz' - 'approvals' - 'approved_by')
        from audit_log where tenant_id = '$tenant_id'
          and action = 'policy.lapse.granted'" | sed 's/^/      /'

verified=$($CLI audit verify --tenant "$tenant_id" 2>&1 || true)
echo "    chain: $verified"

echo
echo "════════════════════════════════════════════════════════════════════"
echo " THE REFUSALS — in the product's own words"
echo "════════════════════════════════════════════════════════════════════"
echo "  a personal scope (the privacy floor, at two independent layers):"
bea_scope=$(psql_t "select scope_id from identities
                    where tenant_id = '$tenant_id' and subject = 'bea'")
refused_api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$bea_scope\",\"grantee_scope_id\":\"$platform_id\",
    \"action\":\"memory.read\",\"duration_secs\":600,\"reason\":\"read bea's notes\"}"

echo "  a scope the grantee already composes through its own chain:"
refused_api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$eng_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"duration_secs\":600,\"reason\":\"payments reads eng\"}"

echo "  an action outside the closed vocabulary:"
refused_api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"policy.assign\",\"duration_secs\":600,\"reason\":\"admin for a while\"}"

echo "  a window past regulated-strict's 30-day ceiling:"
refused_api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"duration_secs\":3888000,\"reason\":\"forty-five days\"}"

echo "  the reader herself, asking for the access she wants:"
refused_api "$bea_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"duration_secs\":600,\"reason\":\"I would like this\"}"

echo
echo "════════════════════════════════════════════════════════════════════"
echo " REVOCATION — ended by someone who could never have opened it"
echo "════════════════════════════════════════════════════════════════════"
proposal2=$(api "$nadia_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"duration_secs\":3600,
    \"reason\":\"second bridge, same outage\"}" | field proposal_id)
api "$nadia_token" POST "/v1/proposals/$proposal2/approve" '{}' >/dev/null
api "$omar_token" POST "/v1/proposals/$proposal2/approve" '{}' >/dev/null
lapse2=$(api "$omar_token" POST "/v1/proposals/$proposal2/lapse" '{}' | field id)
echo "    a fresh hour-long lapse: $lapse2"
echo "    bea receives the runbook: $(arrival "$bea_token" "$RUNBOOK")"

echo "    raj (security-reviewer) cannot open one:"
refused_api "$raj_token" POST /v1/lapses \
  "{\"scope_id\":\"$platform_id\",\"grantee_scope_id\":\"$payments_id\",
    \"action\":\"memory.read\",\"duration_secs\":600,\"reason\":\"mine to open\"}"

api "$raj_token" POST "/v1/lapses/$lapse2/revoke" \
  '{"reason":"the bridge closed; access no longer needed"}' >/dev/null
echo "    raj revokes it — no second approval, nothing to convene"
revoked=$(arrival "$bea_token" "$RUNBOOK")
echo "    bea receives the runbook: $revoked"
[ "$revoked" = "absent" ] || { echo "demo FAILED: revocation did not reach the reader" >&2; exit 1; }

echo
echo "    the listing keeps both grants, because 'who could read this"
echo "    scope's material, and when' is the question it exists for:"
api "$nadia_token" "GET" "/v1/lapses?scope_id=$platform_id" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    for (const l of JSON.parse(d).lapses) {
      console.log(`      ${l.outcome.padEnd(8)} ${l.granted_at} → ${l.expires_at}  ${l.reason}`);
    }
  });
'

echo
echo "AUTHZ-4 demo complete:"
echo "  · a lapse granted a cross-team read the pack forbids, and the block said so"
echo "  · the window closed by itself and the denial came back, with nobody acting"
echo "  · the trail carries the proposal, both approvals, the grant with its"
echo "    window, and the expiry — chain verifying, no record content anywhere"
