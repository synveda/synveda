#!/usr/bin/env sh
# FLOW-7 acceptance demo: rollback & pinning (ADR-0036).
# AC (docs/backlog/FLOW-7.md): bad-prompt rollback demo <60s to fleet-wide
# effect; a rewind can only install a state the channel has held; a pinned
# scope serves its pinned commit while publications keep landing.
#
# The clock is on the incident, not on the estate. What is timed starts the
# moment a bad instruction is live in every agent's context and ends when
# every one of them has stopped receiving it — which is one operator reading
# the channel's history and running one rewind. Everything before that
# banner (a tenant, a hierarchy, principals, and the reviewed publication
# that went wrong) is setup, and is untimed on purpose: it is the world the
# incident happens in, not the response.
#
# "Bad prompt" is the tech plan's example (§2.5). Prompts become governed
# assets with PRMT-1; today the asset kind with a writer is memory, so what
# ships here is a memory record carrying an operational instruction — which
# is what reaches an agent's context either way, through the same channel,
# and the rollback route is asset-kind generic.
#
# Flow: migrate -> tenant -> acme/eng/{platform,payments} -> principals ->
# the runbook and the bad line authored at the platform team -> both climb
# to Engineering through review (FLOW-3/FLOW-5), which is how bad content
# becomes trusted -> THE FLEET receives it: two engineers in different
# teams and a headless agent -> THE AC: `synveda channel history` and
# `synveda channel rollback`, then the same three agents' next session ->
# THE TRAIL -> WHAT A REWIND MAY INSTALL (the refusals) -> THE PIN.
#
# Needs postgres only; the gateway runs in-process here and the reviewers
# carry dev tokens through SYNVEDA_TOKEN (the ADR-0027 override kept for CI
# and demos). On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

BUDGET_SECS=60

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8147
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=flow-7-demo-secret
export SYNVEDA_DEV_JWT_SECRET
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

BASE="http://127.0.0.1:8147"
CLI=./target/debug/synveda

GOOD="deploys go out on tuesdays after the release review"
BAD="skip the staging soak when the release is running late"
EXTRA="the release calendar lives in the platform wiki"

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

now_ms() { node -e 'console.log(Date.now())'; }

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

# as <token> <args...> — the CLI as one principal. The bearer is the only
# thing that changes; the PDP does the rest.
as() {
  tok=$1
  shift
  SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

# refused <token> <args...> — a CLI command that must fail, with the
# refusal printed in the product's own words.
refused() {
  tok=$1
  shift
  if out=$(SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@" 2>&1); then
    echo "demo FAILED: '$*' should have been refused, got:" >&2
    echo "$out" >&2
    exit 1
  fi
  echo "$out" | sed 's/^/    /'
}

# session <token> — one agent's session start, printing what it received.
session() {
  api "$1" POST /v1/inject '{"session_id":"flow-7-demo"}'
}

# arrival <token> <line> — how that line reached this agent, in one word:
# `reviewed` (unmarked, published somewhere on their chain), `unreviewed`
# (present and marked), or `absent`. Per line rather than per block,
# because a block almost always has some other unreviewed material in it.
arrival() {
  session "$1" | node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const text = JSON.parse(d).text;
      const line = process.argv[1];
      if (text.includes("- [procedure] " + line + "\n")) console.log("reviewed");
      else if (text.includes(line)) console.log("unreviewed");
      else console.log("absent");
    });
  ' "$2"
}

echo "==> migrate + admit a tenant"
$CLI db migrate
tenant_id=$($CLI tenant create \
  --slug "flow7-demo-$$" --name "FLOW-7 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$($CLI token issue --tenant "$tenant_id" --subject demo-admin)
$CLI role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/flow7-gateway.log"
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

echo "==> principals"
# Anchors matter here, twice over.
#
# Steve stewards Engineering from OUTSIDE it, under the org:
# `regulated-strict` permits MemoryRead on a principal's own chain, so a
# steward placed inside the department would read its content through the
# floor rather than through his role — and "a steward cannot run the
# effect" would be an accident instead of a rule.
#
# Tara curates the platform team but is anchored at Engineering, because a
# climb is an act at the TARGET: a service identity may act only inside its
# anchor subtree (ADR-0018 decision 2), and proposing at a scope you are
# anchored below is denied at the base layer before any role is consulted.
# Her authority over the team is the binding, not the placement.
$CLI service register --tenant "$tenant_id" --subject tara  --scope "$eng_id"      >/dev/null
$CLI service register --tenant "$tenant_id" --subject cora  --scope "$eng_id"      >/dev/null
$CLI service register --tenant "$tenant_id" --subject steve --scope "$org_id"      >/dev/null
$CLI service register --tenant "$tenant_id" --subject alice --scope "$platform_id" >/dev/null
$CLI service register --tenant "$tenant_id" --subject bea   --scope "$payments_id" >/dev/null
$CLI service register --tenant "$tenant_id" --subject deploybot --scope "$platform_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject tara  --role curator --scope "$platform_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject cora  --role curator --scope "$eng_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject steve --role steward --scope "$eng_id" >/dev/null
tara_token=$($CLI token issue --tenant "$tenant_id" --subject tara)
cora_token=$($CLI token issue --tenant "$tenant_id" --subject cora)
steve_token=$($CLI token issue --tenant "$tenant_id" --subject steve)
alice_token=$($CLI token issue --tenant "$tenant_id" --subject alice)
bea_token=$($CLI token issue --tenant "$tenant_id" --subject bea)
bot_token=$($CLI token issue --tenant "$tenant_id" --subject deploybot)
echo "    tara=curator@platform  cora=curator@eng  steve=steward@eng (anchored at acme)"
echo "    the fleet: alice@platform, bea@payments, deploybot (headless, @platform)"

# The material is authored at the team. Records reach a scope through
# observe (ADR-0020) at their owner's personal node; the team's own shelf
# is seeded directly because the feature under test is the retraction.
tara_identity=$(psql_t "select id from identities
                        where tenant_id = '$tenant_id' and subject = 'tara'")
seed_record() {
  psql_t "begin;
          insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                               content, sensitivity, provenance, valid_from)
          values ('$1', '$tenant_id', '$platform_id', '$tara_identity', 'pinned',
                  'procedure', \$content\$$2\$content\$, 'internal',
                  '{\"source\":\"flow-7 demo\"}', now() - interval '1 hour');
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          values ('$1', '$tenant_id', 'hash@1', 4, '[0.25,0.25,0.25,0.25]');
          commit;" >/dev/null
}
good_id=$(psql_t "select gen_random_uuid()")
bad_id=$(psql_t "select gen_random_uuid()")
extra_id=$(psql_t "select gen_random_uuid()")
seed_record "$good_id" "$GOOD"
seed_record "$bad_id" "$BAD"
seed_record "$extra_id" "$EXTRA"

# The way bad content becomes trusted: a proposal at the department, its
# approvers, and a publication. `regulated-strict` asks for a curator AND a
# steward at a department — two distinct people — which is exactly why the
# rewind below is worth having: getting it in took a quorum, and it is
# reaching agents right now.
promote() {
  pid=$(api "$tara_token" POST /v1/proposals \
    "{\"scope_id\":\"$eng_id\",\"source_scope_id\":\"$platform_id\",\
\"record_ids\":[\"$1\"],\"title\":\"$2\"}" | field id)
  api "$cora_token" POST "/v1/proposals/$pid/approve" >/dev/null
  api "$steve_token" POST "/v1/proposals/$pid/approve" >/dev/null
  api "$cora_token" POST "/v1/proposals/$pid/publish" | field commit
}

echo
echo "==> SETUP — both lines climb to Engineering through review"
good_commit=$(promote "$good_id" "the release runbook")
echo "    published: \"$GOOD\""
echo "               at commit $(echo "$good_commit" | cut -c1-12)"
bad_commit=$(promote "$bad_id" "runbook: late-release exception")
echo "    published: \"$BAD\"   <-- the mistake"
echo "               at commit $(echo "$bad_commit" | cut -c1-12)"

echo
echo "==> the fleet, before: three agents in two teams, none of them"
echo "    configured for any of this"
for who in alice bea deploybot; do
  case $who in
    alice) tok=$alice_token ;;
    bea) tok=$bea_token ;;
    *) tok=$bot_token ;;
  esac
  [ "$(arrival "$tok" "$BAD")" = "reviewed" ] || {
    echo "demo FAILED: $who must be receiving the bad line as reviewed material" >&2
    exit 1
  }
  echo "    $who: receiving it, unmarked — \"$BAD\""
done
echo "    …unmarked, because a department's curator and steward approved it."

echo
echo "================================================================"
echo "  THE AC — from here the clock runs. One operator, two commands,"
echo "  and then the fleet's next sessions. Budget: ${BUDGET_SECS}s."
echo "================================================================"
started=$(now_ms)

echo
echo "--> cora reads the states Engineering's channel has held"
as "$cora_token" channel history "$eng_id" | tee "${TMPDIR:-/tmp}/flow7-history.txt"
grep -q "head" "${TMPDIR:-/tmp}/flow7-history.txt" || {
  echo "demo FAILED: the history must mark where the channel is" >&2; exit 1; }

echo
echo "--> cora rewinds to the state before the mistake"
as "$cora_token" channel rollback "$eng_id" \
  --from "$bad_commit" --to "$good_commit" \
  --message "retract: that exception is not our procedure"

echo
echo "--> the same three agents start their next session"
for who in alice bea deploybot; do
  case $who in
    alice) tok=$alice_token ;;
    bea) tok=$bea_token ;;
    *) tok=$bot_token ;;
  esac
  got=$(arrival "$tok" "$BAD")
  [ "$got" != "reviewed" ] || {
    echo "demo FAILED: $who is still being told the bad line as reviewed" >&2
    exit 1
  }
  [ "$(arrival "$tok" "$GOOD")" = "reviewed" ] || {
    echo "demo FAILED: $who lost the good runbook too" >&2
    exit 1
  }
  case $got in
    unreviewed) echo "    $who: no longer reviewed — back to [unreviewed]" ;;
    *) echo "    $who: gone entirely" ;;
  esac
done

finished=$(now_ms)
elapsed_ms=$((finished - started))
echo
echo "==> AC: bad instruction live in the fleet -> not one agent receiving it"
echo "    $((elapsed_ms / 1000)).$(printf '%03d' $((elapsed_ms % 1000)))s of a ${BUDGET_SECS}s budget"
[ "$elapsed_ms" -lt "$((BUDGET_SECS * 1000))" ] || {
  echo "demo FAILED: over the ${BUDGET_SECS}s budget" >&2; exit 1; }
echo
echo "    The two readers did not get the same answer, and that is the"
echo "    feature rather than a wrinkle in it. Bea is in payments: the"
echo "    record lives at the platform team, off her chain, so with the"
echo "    department's tree no longer naming it there is nothing to"
echo "    compose. Alice is in platform: the record is still in her chain,"
echo "    and what the rewind took away was its TRUST — it composes as"
echo "    [unreviewed]. A rollback moves the boundary; it does not delete."

echo
echo "==> the trail — one act, both commits, and the record that left"
as "$cora_token" channel status "$eng_id"
$CLI audit tail --tenant "$tenant_id" --limit 60 |
  grep 'vedaflow.channel.rolled_back' |
  head -1 |
  node -e '
    let d = ""; process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const e = JSON.parse(d);
      const p = e.payload;
      console.log("    action:  " + e.action + "  actor_kind=" + e.actor.kind);
      console.log("    channel: " + p.channel);
      console.log("    from:    " + p.from.slice(0, 12));
      console.log("    to:      " + p.to.slice(0, 12));
      console.log("    removed: " + JSON.stringify(p.removed));
      console.log("    reason:  " + p.message);
      console.log("    authz:   " + p.authz.action);
      // Ids and addresses, never the record text. The reason above is
      // in the words of whoever rewound, which is why it is carried.
      if (JSON.stringify(p).includes(process.argv[1])) {
        console.error("demo FAILED: an audit payload must not carry content");
        process.exit(1);
      }
    });
  ' "$BAD"
$CLI audit verify --tenant "$tenant_id"

echo
echo "================================================================"
echo "  WHAT A REWIND MAY INSTALL — the refusals, in the product's own"
echo "  words. Every one of them is why a rewind needs no approvals:"
echo "  it can only put the channel back into a state that already had"
echo "  them (ADR-0036 decisions 1-3)."
echo "================================================================"

proposal_commit=$(as "$cora_token" channel history "$eng_id" --json |
  node -e '
    let d = ""; process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const h = JSON.parse(d).history;
      const withProposal = h.find((e) => (e.merge_parents || []).length > 0);
      console.log(withProposal.merge_parents[0]);
    });
  ')
head_commit=$(as "$cora_token" channel history "$eng_id" --json | field head)

echo
echo "--> to the proposal commit that publication came from"
echo "    (reachable from the head — FLOW-1's ancestry test would take it)"
refused "$cora_token" channel rollback "$eng_id" \
  --from "$head_commit" --to "$proposal_commit" --message "back to the proposal"

echo
echo "--> forward, to undo the rewind"
refused "$cora_token" channel rollback "$eng_id" \
  --from "$head_commit" --to "$bad_commit" --message "put it back"

echo
echo "--> by a reader"
refused "$alice_token" channel rollback "$eng_id" \
  --from "$head_commit" --to "$good_commit" --message "let me"

echo
echo "--> by the steward who approved the publication in the first place"
echo "    (he holds ChannelRollback; he reads no content in any pack, and"
echo "     a rewind takes the same read a publication does)"
refused "$steve_token" channel rollback "$eng_id" \
  --from "$head_commit" --to "$good_commit" --message "mine to undo"

echo
echo "================================================================"
echo "  THE PIN — the other half. A scope holds what it SERVES without"
echo "  moving where its channel points, so work continues and readers"
echo "  do not (ADR-0036 decision 6)."
echo "================================================================"

echo
echo "--> tara publishes on the platform team's own channel, twice"
before_runbook=$(api "$tara_token" POST "/v1/channels/$platform_id/publish" \
  "{\"record_ids\":[\"$bad_id\"],\"message\":\"the exception, as the team recorded it\"}" |
  field commit)
platform_commit=$(api "$tara_token" POST "/v1/channels/$platform_id/publish" \
  "{\"record_ids\":[\"$good_id\"],\"message\":\"the team's runbook\"}" | field commit)
echo "    platform is at $(echo "$platform_commit" | cut -c1-12)"

echo
echo "--> and holds its readers there for the duration of a migration"
as "$tara_token" channel pin "$platform_id" \
  --commit "$platform_commit" --reason "frozen through the payments migration"

echo
echo "--> work continues: another publication lands"
pinned_publish=$(api "$tara_token" POST "/v1/channels/$platform_id/publish" \
  "{\"record_ids\":[\"$extra_id\"],\"message\":\"the release calendar\"}")
echo "$pinned_publish" | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const r = JSON.parse(d);
    console.log("    the channel advanced to " + r.commit.slice(0, 12));
    if (!r.pinned) {
      console.error("demo FAILED: the publish response must name the standing pin");
      process.exit(1);
    }
    console.log("    …and the response says readers are still at " +
      r.pinned.commit.slice(0, 12));
  });
'

echo
echo "--> alice starts a session: held, and the block says so"
session "$alice_token" | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const r = JSON.parse(d);
    const held = r.channels.find((c) => c.pinned);
    if (!held) {
      console.error("demo FAILED: a frozen citation must say it is frozen");
      process.exit(1);
    }
    console.log("    cites " + held.commit.slice(0, 12) + "  pinned=true");
    // What the pin holds is the TRUST boundary, so the test is whether
    // the line composes REVIEWED — unmarked. It is still derived material
    // living at the team, and derived material still composes where the
    // pack admits it; a pin is not a filter on the corpus.
    if (r.text.includes("- [procedure] " + process.argv[1] + "\n")) {
      console.error("demo FAILED: what landed after the pin must not compose as reviewed");
      process.exit(1);
    }
    console.log("    …and what landed after the pin is unreviewed in the block");
  });
' "$EXTRA"

echo
echo "--> a rewind under a pin would reach nobody, so it refuses and says so"
refused "$tara_token" channel rollback "$platform_id" \
  --from "$(as "$tara_token" channel history "$platform_id" --json | field head)" \
  --to "$platform_commit" --message "retract"

echo
echo "--> release, and the very next session catches up"
as "$tara_token" channel unpin "$platform_id" --reason "migration done"
session "$alice_token" | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const r = JSON.parse(d);
    if (r.channels.some((c) => c.pinned)) {
      console.error("demo FAILED: nothing should be held now");
      process.exit(1);
    }
    if (!r.text.includes("- [procedure] " + process.argv[1] + "\n")) {
      console.error("demo FAILED: the reader should have caught up");
      process.exit(1);
    }
    console.log("    caught up — the newest publication reads as reviewed,");
    console.log("    and no citation is marked pinned");
  });
' "$EXTRA"

echo
echo "==> a climbed record survives its source's rewind (ADR-0034 trigger c)"
echo "    Engineering approved the runbook under its own approvers; a team"
echo "    curator rewinding her own channel does not get to undo that."
platform_head=$(as "$tara_token" channel history "$platform_id" --json | field head)
as "$tara_token" channel rollback "$platform_id" \
  --from "$platform_head" --to "$before_runbook" \
  --message "the team is not the one that stands behind the runbook"
session "$alice_token" | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const r = JSON.parse(d);
    if (!r.text.includes("- [procedure] " + process.argv[1] + "\n")) {
      console.error("demo FAILED: the department publication must stand");
      process.exit(1);
    }
    if (!r.text.includes("## acme/eng (department)")) {
      console.error("demo FAILED: it should now be sectioned under Engineering");
      process.exit(1);
    }
    console.log("    alice still has the runbook, reviewed — and it is now");
    console.log("    sectioned under acme/eng, because Engineering is the");
    console.log("    scope that stands behind it. The remedy at a department");
    console.log("    is a rewind AT the department, by its own principals.");
  });
' "$GOOD"

echo
echo "==> chain still verifies over all of it"
$CLI audit verify --tenant "$tenant_id"

echo
echo "FLOW-7 demo complete."
