#!/usr/bin/env sh
# SKIL-4 acceptance demo: scope-targeted distribution (ADR-0054).
# AC (docs/SYNVEDA_FEATURES.md): user in team A sees team A's skills;
# team B's are absent; org skills present for both.
#
# The criterion's three clauses are THREE DIFFERENT MECHANISMS and only one
# of them is a policy decision, which is why this demo asserts them
# separately: the org's skills reach both readers because the org is on both
# chains; team A's reach a team A reader because team A is on that chain;
# team B's are absent because team B is on NO chain that reader has — the
# same reason another tenant's records are absent, one level down.
#
# Flow:
#
#   postgres -> scratch db -> tenant, acme > eng > {platform, payments}
#   [1/6] three publications: `house-style` at the org, `deploy-platform`
#         at team A, `settle-ledger` at team B. Each through the review
#         every pack asks for — a steward and a security reviewer.
#   [2/6] THE AC, first surface: `synveda skill available`, run as a
#         reader in each team. Two shelves, one shared entry, and neither
#         holds the other team's.
#   [3/6] THE AC, second surface: the composed block names what the
#         identity may install and NOT what it says — a skill's body still
#         never composes (ADR-0051 decision 9); its name now does.
#   [4/6] the materialisation: `synveda skill sync` into a governed root,
#         the two readers' roots compared, and the receipt that lives
#         OUTSIDE the bundle.
#   [5/6] DISTRIBUTION IS A RECONCILE. A FLOW-7 rewind withdraws a skill;
#         the next sync REMOVES it from the laptop. A materialisation that
#         only ever writes is not a distribution — and this is what makes
#         "<60s to fleet-wide effect" true of a directory.
#   [6/6] the trail: one `skill.resolved` per bundle that reached a disk,
#         the advertisement on the inject event, the chain verifying, and
#         a sweep proving no description text is in any payload.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the house discipline.
SKIL_DB=skil4_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $SKIL_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$SKIL_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/skil4-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8154
export SYNVEDA_LISTEN_ADDR
BASE="http://127.0.0.1:8154"
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=skil-4-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER

SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli
CLI=$PWD/target/debug/synveda

WORK="${TMPDIR:-/tmp}/skil4-$$"
mkdir -p "$WORK"
# A scratch HOME and a scratch "plugin root", so nothing this demo installs
# can reach a developer's own skills directories. The governed root is the
# one the adapter passes: `${CLAUDE_PLUGIN_ROOT}/skills`, a directory this
# product created and may therefore prune (ADR-0054 decision 16).
DEMO_HOME="$WORK/home"
mkdir -p "$DEMO_HOME"
PLUGIN_A="$WORK/plugin-a/skills"
PLUGIN_B="$WORK/plugin-b/skills"
mkdir -p "$PLUGIN_A" "$PLUGIN_B"

REAL_HOME="$HOME"

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  sleep 1
  HOME="$REAL_HOME" $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $SKIL_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR" "$WORK"
}
trap cleanup EXIT INT TERM

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

# as <token> [home] <args...> — the CLI as one principal, in a scratch HOME.
as() {
  tok=$1
  shift
  HOME="$DEMO_HOME" XDG_CONFIG_HOME="$DEMO_HOME/.config" \
    SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

# as_home <token> <config-home> <args...> — the same, with its own config
# directory, so two readers' receipts never mingle.
as_home() {
  tok=$1
  cfg=$2
  shift 2
  HOME="$DEMO_HOME" XDG_CONFIG_HOME="$cfg" \
    SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

says() {
  if ! printf '%s' "$2" | grep -q "$1"; then
    echo "demo FAILED: expected '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}
silent() {
  if printf '%s' "$2" | grep -q "$1"; then
    echo "demo FAILED: did not expect '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}

echo "==> migrate + admit a tenant"
$CLI db migrate
tenant_id=$($CLI tenant create \
  --slug "skil4-demo-$$" --name "SKIL-4 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$($CLI token issue --tenant "$tenant_id" --subject demo-admin)
$CLI role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="$WORK/gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
    echo "demo FAILED: the gateway exited; see $GATEWAY_LOG" >&2
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
team_a=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
team_b=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}" |
  field id)
# The cast is anchored at the ORG, because this demo publishes at the org and
# at both teams, and AUTH-3's confinement forbids a write outside a service
# identity's own anchor subtree.
for who in alice cora sam sec; do
  $CLI service register --tenant "$tenant_id" \
    --subject "$who" --scope "$org_id" >/dev/null
done
# The two readers, each anchored in their own team. They hold no role at all:
# what reaches them is what their placement plus the pack's read grants say,
# and nothing else.
$CLI service register --tenant "$tenant_id" --subject bea --scope "$team_a" >/dev/null
$CLI service register --tenant "$tenant_id" --subject dan --scope "$team_b" >/dev/null
for who in alice cora sam sec; do
  case $who in
  alice) role=contributor ;;
  cora) role=curator ;;
  sam) role=steward ;;
  sec) role=security-reviewer ;;
  esac
  $CLI role bind --tenant "$tenant_id" --subject "$who" --role "$role" --scope "$org_id" >/dev/null
done
alice_token=$($CLI token issue --tenant "$tenant_id" --subject alice)
cora_token=$($CLI token issue --tenant "$tenant_id" --subject cora)
sam_token=$($CLI token issue --tenant "$tenant_id" --subject sam)
sec_token=$($CLI token issue --tenant "$tenant_id" --subject sec)
bea_token=$($CLI token issue --tenant "$tenant_id" --subject bea)
dan_token=$($CLI token issue --tenant "$tenant_id" --subject dan)
echo "    org=$org_id  team A (platform)=$team_a  team B (payments)=$team_b"
echo "    bea reads in team A; dan reads in team B; neither holds a role"

unset DATABASE_URL

# bundle <name> <description> <body> — a directory in the anthropics/skills
# layout, carrying the structure and the worked example SKIL-3's rubric
# asks for. A fleet installs what a review passed.
bundle() {
  name=$1
  description=$2
  body=$3
  rm -rf "$WORK/bundle/$name"
  mkdir -p "$WORK/bundle/$name"
  cat >"$WORK/bundle/$name/SKILL.md" <<DOC
---
name: $name
description: $description
---

# $name

$body

## Steps

1. Read what is in front of you.
2. Do the thing this skill is for.
3. Report what changed.

## Example

\`\`\`sh
echo 'ran $name'
\`\`\`
DOC
}

# publish <name> <scope> — author, review and publish one bundle. The
# review is the one every pack asks for: a steward and a security reviewer,
# two distinct people (ADR-0051 decision 18).
publish() {
  name=$1
  scope=$2
  # Quiet: the review itself is SKIL-1's demo, and this one is about where
  # a published skill goes afterwards.
  as "$alice_token" skill import "$WORK/bundle/$name" --scope "$scope" >/dev/null 2>&1
  as "$alice_token" skill propose "$name" --scope "$scope" --title "$name" >/dev/null 2>&1
  pid=$(api "$sec_token" GET "/v1/proposals?scope_id=$scope&state=open" | field proposals 0 id)
  api "$sam_token" POST "/v1/proposals/$pid/checklist" \
    '{"answers":{"instructions-correct":"yes","scope-appropriate":"yes","not-duplicate":"yes","dependencies-available":"yes","tested":"yes"}}' >/dev/null
  as "$sam_token" proposal approve "$pid" >/dev/null 2>&1
  as "$sec_token" proposal approve "$pid" >/dev/null 2>&1
  as "$cora_token" proposal publish "$pid" >/dev/null 2>&1
}

echo
echo "================================================================"
echo "  [1/6] three publications, each through the review every pack"
echo "  asks for: the ORG's house style, team A's deploy runbook, and"
echo "  team B's settlement procedure."
echo "================================================================"
bundle house-style \
  "The house code style. Use when writing or reviewing any code in this org." \
  "Two spaces, no tabs, and one assertion per test."
publish house-style "$org_id"
echo "    published house-style at the org"

bundle deploy-platform \
  "Deploy the platform service. Use when shipping a platform change." \
  "Run the platform pipeline and watch the canary."
publish deploy-platform "$team_a"
# The state team A's channel held before it published a second skill — the
# rewind target beat 5 uses, captured here rather than derived later,
# because a rewind is a decision about which state to leave (ADR-0036).
platform_first=$(api "$bea_token" GET "/v1/skills/deploy-platform" | field commit)
echo "    published deploy-platform at team A (platform)"

bundle rotate-keys \
  "Rotate the platform signing keys. Use at the quarterly rotation." \
  "Mint the new pair, publish it, retire the old one after a week."
publish rotate-keys "$team_a"
echo "    published rotate-keys at team A (platform)"

bundle settle-ledger \
  "Settle the payments ledger. Use at close of business." \
  "Reconcile the acquirer statement against the ledger tail."
publish settle-ledger "$team_b"
echo "    published settle-ledger at team B (payments)"

echo
echo "================================================================"
echo "  [2/6] THE ACCEPTANCE CRITERION, first surface. \`skill"
echo "  available\` is the plural of \`skill show\`: the SAME chain walk,"
echo "  the same SkillRead decision per scope, over a whole shelf. Team"
echo "  B's skills are absent from bea's shelf for a reason no filter"
echo "  had to apply — team B is on no chain she has."
echo "================================================================"
bea_shelf=$(as "$bea_token" skill available 2>&1)
printf '%s\n' "$bea_shelf" | sed 's/^/    /'
says "deploy-platform" "$bea_shelf"
says "rotate-keys" "$bea_shelf"
says "house-style" "$bea_shelf"
silent "settle-ledger" "$bea_shelf"

echo
dan_shelf=$(as "$dan_token" skill available 2>&1)
printf '%s\n' "$dan_shelf" | sed 's/^/    /'
says "settle-ledger" "$dan_shelf"
says "house-style" "$dan_shelf"
silent "deploy-platform" "$dan_shelf"
silent "rotate-keys" "$dan_shelf"
echo
echo "    the org's skill is on both shelves; neither team's is on the other's"

echo
echo "================================================================"
echo "  [3/6] THE ACCEPTANCE CRITERION, second surface. The composed"
echo "  block names what this identity may install — and says nothing"
echo "  about what those skills contain, because the client's own"
echo "  progressive disclosure is the loader (ADR-0051 decision 9)."
echo "================================================================"
block=$(api "$bea_token" POST /v1/inject '{"session_id":"skil4-demo"}')
printf '%s' "$block" | field text | sed 's/^/    /'
text=$(printf '%s' "$block" | field text)
says "Skills available" "$text"
says "deploy-platform" "$text"
says "house-style" "$text"
silent "settle-ledger" "$text"
silent "watch the canary" "$text"
echo "    section cost: $(printf '%s' "$block" | field skill_tokens) tokens, \
$(printf '%s' "$block" | field skills | node -e 'let d="";process.stdin.on("data",c=>d+=c);process.stdin.on("end",()=>console.log(JSON.parse(d).length))') named, \
$(printf '%s' "$block" | field skills_omitted) omitted"
echo "    and the citation — scope, commit, address — rides the response"
echo "    rather than the token budget:"
printf '%s' "$block" | field skills | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    for (const s of JSON.parse(d)) {
      console.log(`      ${s.name.padEnd(18)} ${s.commit.slice(0, 12)}  ${s.sensitivity}`);
    }
  });
'

echo
echo "================================================================"
echo "  [4/6] the materialisation. \`skill sync\` writes the shelf into"
echo "  the client's GOVERNED root — the adapter's own plugin directory,"
echo "  never a person's ~/.claude/skills, because a reconcile prunes"
echo "  and the only directory this product may prune is one it created."
echo "================================================================"
as_home "$bea_token" "$DEMO_HOME/.config-bea" \
  skill sync --client claude-code --root "$PLUGIN_A" | sed 's/^/    /'
echo
echo "    team A's governed root:"
ls -1 "$PLUGIN_A" | sed 's/^/      /'
as_home "$dan_token" "$DEMO_HOME/.config-dan" \
  skill sync --client claude-code --root "$PLUGIN_B" >/dev/null
echo "    team B's governed root:"
ls -1 "$PLUGIN_B" | sed 's/^/      /'

# The criterion as a filesystem fact rather than a claim.
[ -d "$PLUGIN_A/deploy-platform" ] || { echo "demo FAILED: team A missing its own skill" >&2; exit 1; }
[ -d "$PLUGIN_A/house-style" ] || { echo "demo FAILED: team A missing the org's" >&2; exit 1; }
[ -d "$PLUGIN_A/settle-ledger" ] && { echo "demo FAILED: team B's skill reached team A" >&2; exit 1; }
[ -d "$PLUGIN_B/settle-ledger" ] || { echo "demo FAILED: team B missing its own skill" >&2; exit 1; }
[ -d "$PLUGIN_B/house-style" ] || { echo "demo FAILED: team B missing the org's" >&2; exit 1; }
[ -d "$PLUGIN_B/deploy-platform" ] && { echo "demo FAILED: team A's skill reached team B" >&2; exit 1; }
echo
echo "    the one skill both roots hold is byte-identical, file for file:"
diff -r "$PLUGIN_A/house-style" "$PLUGIN_B/house-style" && echo "      identical"
echo
echo "    and the bundle directory holds EXACTLY the reviewed files —"
echo "    the receipt is outside it, in the CLI's own config directory:"
find "$PLUGIN_A/deploy-platform" -type f | sed "s#$PLUGIN_A/#      #"
ls -1 "$DEMO_HOME/.config-bea/synveda/skills/claude-code" | sed 's/^/      receipt: /'

echo
echo "    a second sync writes nothing: the receipt records the commit and"
echo "    every file's address, so an unchanged bundle costs one listing"
echo "    call and no resolves at all."
again=$(as_home "$bea_token" "$DEMO_HOME/.config-bea" \
  skill sync --client claude-code --root "$PLUGIN_A" 2>&1)
printf '%s\n' "$again" | sed 's/^/    /'
says "unchanged" "$again"

echo
echo "================================================================"
echo "  [5/6] DISTRIBUTION IS A RECONCILE. A FLOW-7 rewind withdraws"
echo "  rotate-keys from team A's channel. The next sync REMOVES the"
echo "  directory — because a materialisation that only ever writes"
echo "  leaves a withdrawn skill running on a laptop forever, and"
echo "  '<60s to fleet-wide effect' would stop at the network."
echo "================================================================"
head_commit=$(api "$bea_token" GET "/v1/skills/rotate-keys" | field commit)
as "$cora_token" channel rollback "$team_a" --channel skill/published \
  --from "$head_commit" --to "$platform_first" \
  --message "the rotation runbook was wrong" | sed 's/^/    /'
echo
after=$(as "$bea_token" skill available 2>&1)
silent "rotate-keys" "$after"
echo "    bea's shelf no longer lists it"
synced=$(as_home "$bea_token" "$DEMO_HOME/.config-bea" \
  skill sync --client claude-code --root "$PLUGIN_A" 2>&1)
printf '%s\n' "$synced" | sed 's/^/    /'
says "rotate-keys" "$synced"
[ -d "$PLUGIN_A/rotate-keys" ] && { echo "demo FAILED: the withdrawn skill is still on disk" >&2; exit 1; }
echo "    and the directory is gone from the governed root:"
ls -1 "$PLUGIN_A" | sed 's/^/      /'
# The receipt goes with it, so the two can never disagree about what is
# installed.
ls -1 "$DEMO_HOME/.config-bea/synveda/skills/claude-code" | sed 's/^/      receipt: /'

echo
echo "================================================================"
echo "  [6/6] the trail. Every bundle that reached a disk was served by"
echo "  an audited resolve, the block's advertisement is on the inject"
echo "  event, and no payload anywhere carries a line of a SKILL.md."
echo "================================================================"
trail=$(DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $CLI audit tail --tenant "$tenant_id" --limit 300)
printf '%s\n' "$trail" | node -e '
  const counted = {};
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    for (const line of d.split("\n").filter(Boolean)) {
      const event = JSON.parse(line);
      counted[event.action] = (counted[event.action] ?? 0) + 1;
    }
    for (const action of Object.keys(counted).sort()) {
      console.log(`    ${String(counted[action]).padStart(3)}  ${action}`);
    }
  });
'
echo
echo "    the inject event names what bea was told she could install,"
echo "    with the commit and the address, and never the description:"
printf '%s\n' "$trail" | grep "context.injected" | head -1 |
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      for (const s of JSON.parse(d).payload.skills ?? []) {
        console.log(`      ${s.name.padEnd(18)} ${s.commit.slice(0, 12)}  ${s.sensitivity}`);
      }
    });
  '

echo
echo "    and the sweep: no payload carries a bundle's text or the"
echo "    description the block rendered."
leaked=$(printf '%s\n' "$trail" |
  grep -cE "watch the canary|Reconcile the acquirer|one assertion per test|Use when shipping" || true)
[ "$leaked" = "0" ] || {
  echo "demo FAILED: $leaked audit rows carry skill text" >&2
  exit 1
}
echo "    0 rows"

echo
chain=$(DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $CLI audit verify --tenant "$tenant_id")
printf '%s\n' "$chain" | sed 's/^/    /'
says "chain valid" "$chain"

echo
echo "================================================================"
echo "  SKIL-4 demonstrated: a reader's shelf is their own chain and"
echo "  nothing else — team A's skills and the org's, never team B's,"
echo "  on both the surface a client installs from and the block a"
echo "  session is given; the two agree because they are one walk; the"
echo "  materialisation writes byte-identical bundles into a root this"
echo "  product owns; and a withdrawal REMOVES one, which is the half"
echo "  of distribution that makes a rollback mean anything on a laptop."
echo "================================================================"
