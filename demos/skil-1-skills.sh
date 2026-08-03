#!/usr/bin/env sh
# SKIL-1 acceptance demo: the skills registry (ADR-0051).
# AC (docs/SYNVEDA_FEATURES.md): a skill authored at a scope reaches a client
# only through the review the pack in force asks for — and under EVERY pack
# that is two distinct people, one a security-reviewer; "installs unmodified"
# is a hash comparison rather than a claim; the spec's rules are enforced at
# authoring; a bundle carrying a credential never reaches a laptop.
#
# PRMT-1 showed its claims at the registry's own route and PRMT-2 at
# /v1/inject. Almost nothing here can be shown at either: a skill's whole
# point is that it leaves the product and is loaded by somebody else's
# client, so the load-bearing beats below are measured **on a filesystem**.
# Flow:
#
#   postgres -> scratch db -> tenant, hierarchy, five principals
#   [1/8] alice imports a real anthropics/skills directory at the DEPARTMENT.
#         DATABASE_URL is unset from here on, so every governed act is a
#         gateway call under that principal's own bearer.
#   [2/8] the direct publish route refuses — and refuses again under
#         `standard`, which is ADR-0051 decision 18: the invariant floor had
#         required the security-reviewer ROLE without ever requiring a second
#         SIGNATURE, so under the SMB pack one person shipped code alone.
#   [3/8] the review carries it: a steward and a security reviewer, two
#         distinct people, and a curator runs the effect.
#   [4/8] THE AC. Install into TWO clients' own skills roots. The trees are
#         byte-identical, every file's address recomputes to the one the
#         commit named, the bundle holds exactly the reviewed files, and the
#         receipt is OUTSIDE it — because a receipt inside the bundle is the
#         modification "unmodified" forbids.
#   [5/8] the live runs, when the binaries are present: `claude` and `codex`
#         against the installed bundles. Reported, never gated — whether a
#         MODEL reaches for a skill is a property of the model.
#   [6/8] a rewind refuses the pinned install naming both commits, so
#         FLOW-7's "<60s to fleet-wide effect" stays true of an asset that
#         lives on laptops.
#   [7/8] a bundle carrying a live credential is stopped at authoring. No
#         secret reaches a client's disk.
#   [8/8] the trail: skill.authored, skill.resolved, skill.quarantined and
#         the same vedaflow.channel.published a memory publication emits,
#         with no file content in any payload, and the chain verifying.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1/MEM-6/PRMT-1/PRMT-2
# discipline.
SKIL_DB=skil1_$$
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
SYNVEDA_SEARCH_INDEX_DIR="./data/skil1-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8147
export SYNVEDA_LISTEN_ADDR
BASE="http://127.0.0.1:8147"
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=skil-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER

# Built against the committed `.sqlx` cache, not against DATABASE_URL: the
# scratch database above is empty until `db migrate` runs. Same build CI does.
SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli
CLI=$PWD/target/debug/synveda

WORK="${TMPDIR:-/tmp}/skil1-$$"
mkdir -p "$WORK"
# A scratch HOME, so the two clients' real skills directories are never
# touched and the demo's own installs are the only thing in them (ADPT-1's
# discipline).
DEMO_HOME="$WORK/home"
mkdir -p "$DEMO_HOME"

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

# as <token> <args...> — the CLI as one principal, inside the scratch HOME.
as() {
  tok=$1
  shift
  HOME="$DEMO_HOME" XDG_CONFIG_HOME="$DEMO_HOME/.config" \
    SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

# refused <token> <args...> — a CLI command that must fail, printed.
refused() {
  tok=$1
  shift
  if out=$(HOME="$DEMO_HOME" XDG_CONFIG_HOME="$DEMO_HOME/.config" \
    SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@" 2>&1); then
    echo "demo FAILED: '$*' should have been refused, got:" >&2
    echo "$out" >&2
    exit 1
  fi
  echo "$out" | sed 's/^/    /'
}

# refused_http <token> <method> <path> [body]
refused_http() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if out=$(curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
    -H "Content-Type: application/json" \
    ${body:+-d "$body"} "$BASE$path" 2>&1); then
    echo "demo FAILED: $method $path should have been refused, got:" >&2
    echo "$out" >&2
    exit 1
  fi
  curl -sS -X "$method" -H "Authorization: Bearer $tok" \
    -H "Content-Type: application/json" \
    ${body:+-d "$body"} "$BASE$path" 2>/dev/null | sed 's/^/    /'
}

# says <needle> <haystack> / silent <needle> <haystack>
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
  --slug "skil1-demo-$$" --name "SKIL-1 Demo Tenant" | field id)
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

echo "==> hierarchy: acme > eng > platform, five principals"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
# Authors and reviewers are anchored at the DEPARTMENT, where this demo
# publishes: AUTH-3's confinement forbids every write outside a service
# identity's own anchor subtree.
for who in alice cora sam sec; do
  $CLI service register --tenant "$tenant_id" \
    --subject "$who" --scope "$eng_id" >/dev/null
done
# The consumer is anchored one level BELOW, which is the point of her: the
# department's skills reach her up her own chain, through the read carve-out
# the base layer gained for SkillRead.
$CLI service register --tenant "$tenant_id" --subject bea --scope "$team_id" >/dev/null
for who in alice cora sam sec; do
  case $who in
  alice) role=contributor ;;
  cora) role=curator ;;
  sam) role=steward ;;
  sec) role=security-reviewer ;;
  esac
  $CLI role bind --tenant "$tenant_id" --subject "$who" --role "$role" --scope "$eng_id" >/dev/null
done
alice_token=$($CLI token issue --tenant "$tenant_id" --subject alice)
cora_token=$($CLI token issue --tenant "$tenant_id" --subject cora)
sam_token=$($CLI token issue --tenant "$tenant_id" --subject sam)
sec_token=$($CLI token issue --tenant "$tenant_id" --subject sec)
bea_token=$($CLI token issue --tenant "$tenant_id" --subject bea)
echo "    eng=$eng_id  team=$team_id"
echo "    alice=contributor  cora=curator  sam=steward  sec=security-reviewer  bea=consumer"

# A real anthropics/skills-format bundle on disk, which is what `import`
# reads: a directory named for the skill, SKILL.md at its root, and a script
# beside it.
MARKER=SYNVEDA-SKIL1-OK
mkdir -p "$WORK/bundle/code-review/scripts"
cat >"$WORK/bundle/code-review/SKILL.md" <<DOC
---
name: code-review
description: Review a diff and report every defect. Use whenever the user asks for a code review, a diff review, or "review this change".
allowed-tools:
  - Read
  - Bash(git diff *)
---

# Code Review

When you use this skill, begin your reply with the literal marker $MARKER
on its own line, then report every defect you would fix.

Run \`python scripts/check.py <file>\` for the mechanical checks.
DOC
cat >"$WORK/bundle/code-review/scripts/check.py" <<'DOC'
import sys

print("checked", sys.argv[1:])
DOC

# From here on: no psql and no DATABASE_URL. Every governed act is a gateway
# call under a principal's own bearer (ADR-0035 decision 1).
unset DATABASE_URL

echo
echo "================================================================"
echo "  [1/8] alice imports the bundle at the DEPARTMENT. The format is"
echo "  the open standard's, so there is nothing to convert — this reads"
echo "  the directory, validates it against the spec, scans every file for"
echo "  secrets, and writes the draft. It moves nothing a client installs."
echo "================================================================"
as "$alice_token" skill import "$WORK/bundle/code-review" --scope "$eng_id"
echo
as "$alice_token" skill list "$eng_id"

echo
echo "================================================================"
echo "  [2/8] the direct publish route refuses — TWICE. Under"
echo "  regulated-strict, and then again under \`standard\`, whose whole"
echo "  content is that publication is cheaper there. That second refusal"
echo "  is ADR-0051 decision 18: the invariant floor asked for the"
echo "  security-reviewer ROLE and never for a second SIGNATURE, so one"
echo "  person holding both roles published executable code alone."
echo "================================================================"
echo "  under regulated-strict (the default):"
refused_http "$cora_token" POST "/v1/channels/$eng_id/publish" \
  '{"skill_names":["code-review"],"message":"ship it"}'
echo
echo "  the same call under \`standard\`:"
api "$admin_token" PUT "/v1/hierarchy/nodes/$org_id/policy" '{"name":"standard"}' >/dev/null
refused_http "$cora_token" POST "/v1/channels/$eng_id/publish" \
  '{"skill_names":["code-review"],"message":"ship it"}'
api "$admin_token" DELETE "/v1/hierarchy/nodes/$org_id/policy" >/dev/null
echo
echo "  and bea installs nothing, because nothing is published:"
refused "$bea_token" skill install code-review --client claude-code

echo
echo "================================================================"
echo "  [3/8] the review the floor asks for: a steward AND a security"
echo "  reviewer, two distinct people. Skills are treated like code"
echo "  because they are."
echo "================================================================"
as "$alice_token" skill propose code-review --scope "$eng_id" \
  --title "code review skill"
proposal_id=$(api "$sec_token" GET "/v1/proposals?scope_id=$eng_id&state=open" |
  field proposals 0 id)
echo
echo "  what the security reviewer sees — a per-file diff:"
as "$sec_token" proposal show "$proposal_id" | sed 's/^/    /'
echo
as "$sam_token" proposal approve "$proposal_id"
as "$sec_token" proposal approve "$proposal_id"
as "$cora_token" proposal publish "$proposal_id"
published_commit=$(api "$bea_token" GET "/v1/skills/code-review" | field commit)
echo "    published at $published_commit"

echo
echo "================================================================"
echo "  [4/8] THE ACCEPTANCE CRITERION. One published commit, installed"
echo "  into TWO clients' own skills roots. 'Unmodified' is measured"
echo "  here, three ways: the two trees are byte-identical, every file's"
echo "  content address recomputes to the one the commit named, and the"
echo "  bundle directory holds exactly the reviewed files — the receipt"
echo "  is outside it, because a file no reviewer approved in a directory"
echo "  a client walks IS the modification the criterion forbids."
echo "================================================================"
as "$bea_token" skill install code-review --client claude-code
echo
as "$bea_token" skill install code-review --client codex
echo

claude_dir="$DEMO_HOME/.claude/skills/code-review"
codex_dir="$DEMO_HOME/.codex/skills/code-review"
echo "  the two trees:"
(cd "$DEMO_HOME" && find .claude/skills .codex/skills -type f | sort | sed 's/^/    /')
echo
if ! diff -r "$claude_dir" "$codex_dir" >/dev/null; then
  echo "demo FAILED: the two clients' trees differ" >&2
  diff -r "$claude_dir" "$codex_dir" >&2
  exit 1
fi
echo "    diff -r: identical. The per-client difference is the ROOT and"
echo "    nothing else, which is what makes portability a hash comparison."

# The bundle holds exactly the reviewed files — nothing added.
# LC_ALL=C, because the check is about *which* files are there and macOS's
# default collation orders `SKILL.md` and `scripts/` differently from Linux's.
installed=$(cd "$claude_dir" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | tr '\n' ' ')
if [ "$installed" != "SKILL.md scripts/check.py " ]; then
  echo "demo FAILED: the installed bundle is not exactly the reviewed files: $installed" >&2
  exit 1
fi
echo "    the directory holds exactly: $installed"

# Every materialised file is non-executable (ADR-0051 decision 8). Mode is
# not in the open spec, so there is nothing to be compliant with — and a
# governed bundle that cannot arrive executable is one less thing SKIL-2 has
# to scan for.
if find "$claude_dir" -type f -perm -u+x | grep -q .; then
  echo "demo FAILED: an installed file carries an execute bit nobody reviewed" >&2
  find "$claude_dir" -type f -perm -u+x >&2
  exit 1
fi
echo "    and not one of them is executable — a skill invokes its scripts"
echo "    through an interpreter, so no mode reaches a laptop unreviewed"

# And the receipt is somewhere else entirely.
receipt="$DEMO_HOME/.config/synveda/skills/claude-code/code-review.json"
if [ ! -f "$receipt" ]; then
  echo "demo FAILED: no receipt at $receipt" >&2
  exit 1
fi
if find "$claude_dir" -name '*.json' | grep -q .; then
  echo "demo FAILED: a receipt-shaped file is inside the bundle" >&2
  exit 1
fi
echo "    receipt: ${receipt#"$DEMO_HOME"/}  (outside every client's root)"
echo "    it records the commit, the scope and every address:"
field commit <"$receipt" | sed 's/^/      commit /'
field files <"$receipt" | sed 's/^/      /'
echo
echo "  the addresses were recomputed by the CLI from what it WROTE, not"
echo "  read back from the server — a materialised bundle carries no"
echo "  watermark of its own, so this hash is its whole provenance."

echo
echo "================================================================"
echo "  [5/8] the live runs. Deferred with a recorded trigger: whether a"
echo "  MODEL reaches for a skill is a property of the model, not of the"
echo "  product's bytes, so this beat REPORTS and never gates."
echo "================================================================"
# GNU `timeout` is not on macOS by default, and the first run of this demo
# found both halves of that: it reported "could not run non-interactively
# (usually unauthenticated)" when the real cause was a missing binary, and
# then `codex exec` ran unbounded and hung. A beat that reports a reason has
# to report the right one, and a demo may not hang — so the bound is POSIX
# and the reason is whatever the client actually said.
#
# run_bounded <seconds> <cmd...> — output in $CLIENT_OUT, 124 on timeout.
CLIENT_OUT="$WORK/client.out"
run_bounded() {
  secs=$1
  shift
  : >"$CLIENT_OUT"
  # stdin from /dev/null: `codex exec` waits on it, and a demo that hangs
  # for ninety seconds on an open pipe would report a timeout where the
  # honest answer is "it wanted input".
  "$@" >"$CLIENT_OUT" 2>&1 </dev/null &
  pid=$!
  waited=0
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge "$secs" ]; then
      kill -TERM "$pid" 2>/dev/null || true
      sleep 2
      kill -KILL "$pid" 2>/dev/null || true
      echo "timed out after ${secs}s without answering" >>"$CLIENT_OUT"
      return 124
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$pid" || return $?
}

run_client() {
  name=$1
  bin=$2
  shift 2
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "    $name: skipped — \`$bin\` is not on PATH"
    return 0
  fi
  echo "    $name: running \`$bin\` against the installed bundle (up to 90s)…"
  # The scratch HOME is what keeps this demo out of the user's real skills
  # directories, and it is also why an authenticated run is unlikely: the
  # client's own credentials live in the real one. That trade is deliberate,
  # and it is why this beat reports rather than gates.
  if HOME="$DEMO_HOME" run_bounded 90 "$@"; then
    if grep -q "$MARKER" "$CLIENT_OUT"; then
      echo "    $name: the skill was loaded and used — marker $MARKER present"
    else
      echo "    $name: ran, but the model did not reach for the skill."
      echo "           That is the deferred half: the bytes are identical in"
      echo "           both roots and the loader accepted them, and what a"
      echo "           model does with a description is the model's."
    fi
  else
    echo "    $name: did not complete. What it said:"
    head -3 "$CLIENT_OUT" | sed 's/^/             /'
    echo "           (the scratch HOME holds the installed bundle and no"
    echo "           client credentials, which is the usual reason)"
  fi
}

run_client "Claude Code" claude \
  claude -p "Review this diff: -foo() { return 1; } +foo() { return 2; }"
run_client "Codex" codex \
  codex exec "Review this diff: -foo() { return 1; } +foo() { return 2; }"

echo
echo "================================================================"
echo "  [6/8] a rewind reaches a laptop. bea pins the commit she"
echo "  installed; a second version publishes and her pin still holds;"
echo "  then a FLOW-7 rewind takes that commit off the channel and the"
echo "  pinned read is REFUSED naming both commits — because '<60s to"
echo "  fleet-wide effect' and a pin that outlives a withdrawal cannot"
echo "  both be true."
echo "================================================================"
sed -i.bak "s/report every defect you would fix/report every defect AND rewrite the tests/" \
  "$WORK/bundle/code-review/SKILL.md"
rm -f "$WORK/bundle/code-review/SKILL.md.bak"
as "$alice_token" skill import "$WORK/bundle/code-review" --scope "$eng_id" >/dev/null
as "$alice_token" skill propose code-review --scope "$eng_id" --title "the rewrite" >/dev/null
second_id=$(api "$sec_token" GET "/v1/proposals?scope_id=$eng_id&state=open" |
  field proposals 0 id)
as "$sam_token" proposal approve "$second_id" >/dev/null
as "$sec_token" proposal approve "$second_id" >/dev/null
as "$cora_token" proposal publish "$second_id" >/dev/null
second_commit=$(api "$bea_token" GET "/v1/skills/code-review" | field commit)
echo "    the channel now serves $second_commit"

pinned=$(api "$bea_token" \
  GET "/v1/skills/code-review?scope_id=$eng_id&commit=$published_commit" | field origin)
echo "    bea's pinned read still resolves: origin=$pinned"

as "$cora_token" channel rollback "$eng_id" --channel skill/published \
  --from "$second_commit" --to "$published_commit" \
  --message "the rewrite was wrong" | sed 's/^/    /'
echo
echo "  and the consumer who pinned the withdrawn version learns on its"
echo "  NEXT CALL rather than its next session:"
refused_http "$bea_token" GET \
  "/v1/skills/code-review?scope_id=$eng_id&commit=$second_commit"

echo
echo "================================================================"
echo "  [7/8] a bundle carrying a live credential is stopped at"
echo "  authoring. MEM-2's scanner runs over every file before anything"
echo "  is stored — and the guarantee is stronger here than for a context"
echo "  pack for having a different destination: a pack's secret would"
echo "  have reached vector space, and a skill's reaches a laptop."
echo "================================================================"
mkdir -p "$WORK/leaky/deploy-helper"
cat >"$WORK/leaky/deploy-helper/SKILL.md" <<'DOC'
---
name: deploy-helper
description: Deploy the service. Use when asked to ship a release.
---

# Deploy Helper

Run the script.
DOC
cat >"$WORK/leaky/deploy-helper/deploy.sh" <<'DOC'
export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
aws s3 sync ./dist s3://acme-releases
DOC
refused "$alice_token" skill import "$WORK/leaky/deploy-helper" --scope "$eng_id"
echo
echo "  and a bundle reaching outside itself is refused before that: a"
echo "  symlink is a reference to bytes rather than bytes, so import does"
echo "  not follow one (ADR-0051 decision 15)."
mkdir -p "$WORK/sneaky/host-reader"
cat >"$WORK/sneaky/host-reader/SKILL.md" <<'DOC'
---
name: host-reader
description: Read the host. Use when asked about the machine.
---

# Host Reader

Read the reference.
DOC
ln -s /etc/hosts "$WORK/sneaky/host-reader/reference.md"
refused "$alice_token" skill import "$WORK/sneaky/host-reader" --scope "$eng_id"
echo
echo "  nothing was stored, so nothing can be published or installed:"
as "$alice_token" skill list "$eng_id" | sed 's/^/    /'

echo
echo "================================================================"
echo "  [8/8] the trail."
echo "================================================================"
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $CLI audit tail --tenant "$tenant_id" --limit 60 |
  grep -E 'skill\.|vedaflow\.channel\.published' | sed 's/^/    /'
echo
echo "  the sweep — no payload carries SKILL.md text or file content:"
leak=$(DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $COMPOSE exec -T postgres psql -tAc \
  "select count(*) from audit_log
    where payload::text like '%every defect%'
       or payload::text like '%import sys%'
       or payload::text like '%AKIAIOSFODNN7EXAMPLE%'" \
  -U synveda -d "$SKIL_DB")
if [ "$(printf '%s' "$leak" | tr -d ' ')" != "0" ]; then
  echo "demo FAILED: $leak audit payload(s) carry file content" >&2
  exit 1
fi
echo "    0 payloads carry file content"
echo
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $CLI audit verify --tenant "$tenant_id" | sed 's/^/    /'

echo
echo "SKIL-1 demo: all checks green."
echo "  a skill reaches a client only through review — under every pack,"
echo "  two distinct people, one of them a security reviewer;"
echo "  one commit installs byte-identically into two clients' roots, with"
echo "  every address recomputed by the client that wrote the files;"
echo "  the bundle holds exactly the reviewed files and the receipt is"
echo "  outside it; a rewind refuses a pinned install by name; and a"
echo "  credential never reaches a laptop."
