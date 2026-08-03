#!/usr/bin/env sh
# SKIL-3 acceptance demo: skill quality scoring (ADR-0053).
# AC (docs/SYNVEDA_FEATURES.md): score displayed at review and in the
# registry; low-score publish requires override.
#
# The score is TWO numbers and the demo never averages them, because that is
# the design: an automated rubric over the bundle's bytes, and a reviewer's
# checklist, which no machine can supply. Summing them would let a
# well-formatted bundle nobody worked through read the same as one somebody
# did.
#
# Flow:
#
#   postgres -> scratch db -> tenant, hierarchy, five principals
#   [1/6] alice authors a thin bundle. It is SCORED and NOT refused —
#         a draft is where a skill is supposed to be unfinished — and the
#         rubric names what it lost and why, so she can fix it.
#   [2/6] THE AC's SECOND SURFACE: the score in the registry listing. She
#         fixes the bundle and the number moves, which is the rubric being
#         about the bundle rather than about her.
#   [3/6] THE AC's FIRST SURFACE: `synveda proposal show` renders the score
#         at review — recomputed from the proposal's own bytes, not read
#         from the listing's cache — with the checklist missing and said so.
#   [4/6] sam records the checklist and answers one item `no`. The
#         publication is refused naming the objection: a pack decides
#         whether a checklist is mandatory, and no pack decides that a
#         written-down `no` counts for nothing.
#   [5/6] THE AC's THIRD CLAUSE. The thin bundle is below the bar. cora, who
#         publishes everything else here, CANNOT override; sam, who can,
#         holds no content read and so could not publish it himself. Two
#         acts, two people — and the override binds the bytes, so alice's
#         next edit does not inherit it.
#   [6/6] the trail: skill.checklist.recorded and skill.quality.overridden
#         with the digest, the reason and which bar was missed; the chain
#         verifying; no file content in any payload.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1/MEM-6/PRMT-1/PRMT-2
# discipline.
SKIL_DB=skil3_$$
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
SYNVEDA_SEARCH_INDEX_DIR="./data/skil3-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8153
export SYNVEDA_LISTEN_ADDR
BASE="http://127.0.0.1:8153"
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=skil-3-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER

# Built against the committed `.sqlx` cache, not against DATABASE_URL: the
# scratch database above is empty until `db migrate` runs. Same build CI does.
SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli
CLI=$PWD/target/debug/synveda

WORK="${TMPDIR:-/tmp}/skil3-$$"
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
  --slug "skil3-demo-$$" --name "SKIL-3 Demo Tenant" | field id)
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

# From here on: no psql and no DATABASE_URL. Every governed act is a gateway
# call under a principal's own bearer (ADR-0035 decision 1).
unset DATABASE_URL

# bundle <name> <manifest-body> [script-path] [script-body] — writes a
# directory in the anthropics/skills layout, which is what `skill import`
# reads.
bundle() {
  name=$1
  body=$2
  script_path=${3:-}
  script_body=${4:-}
  rm -rf "$WORK/bundle/$name"
  mkdir -p "$WORK/bundle/$name"
  cat >"$WORK/bundle/$name/SKILL.md" <<DOC
---
name: $name
description: Sets up and runs the project toolchain. Use when asked to set up or build.
---

# $name

$body
DOC
  if [ -n "$script_path" ]; then
    mkdir -p "$(dirname "$WORK/bundle/$name/$script_path")"
    printf '%s\n' "$script_body" >"$WORK/bundle/$name/$script_path"
  fi
}

# quality <json> — the score block from an author or a review response.
quality() {
  printf '%s' "$1" | node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const q = JSON.parse(d).quality;
      if (!q) { console.log("    (no quality block)"); return; }
      const bar = q.min_score ? `this pack asks for ${q.min_score}` : "this pack sets no bar";
      console.log(`    quality  ${q.score}/100   (rubric v${q.rubric_version}, ${bar})`);
      for (const c of q.checks.filter((c) => !c.passed)) {
        console.log(`      -${String(c.weight).padEnd(3)} ${c.check.padEnd(24)} ${c.title}`);
        if (c.detail) console.log(`             ${c.detail}`);
      }
      if (q.checklist) {
        console.log(`      checklist  ${q.checklist.complete ? "complete" : "PARTIAL"}` +
          (q.checklist.concerns.length ? `  concerns: ${q.checklist.concerns.join(", ")}` : ""));
      } else {
        console.log(`      checklist  ${q.requires_checklist ? "NONE recorded — this pack requires one" : "none; not required here"}`);
      }
      if (q.needs_override) console.log(`      -> publishing this needs an override`);
    });
  '
}

# author <dir> — POSTs a bundle directory to /v1/skills as alice and prints
# the response. The CLI's `skill import` does the same thing; this demo goes
# through the API where it needs the authoring response's own quality block.
author() {
  dir=$1
  body=$(node -e '
    const fs = require("fs"), path = require("path");
    const root = process.argv[1], scope = process.argv[2];
    const files = [];
    (function walk(d) {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) walk(p);
        else files.push({
          path: path.relative(root, p).split(path.sep).join("/"),
          content: fs.readFileSync(p, "utf8"),
        });
      }
    })(root);
    console.log(JSON.stringify({ scope_id: scope, name: path.basename(root), files }));
  ' "$dir" "$eng_id")
  api "$alice_token" POST /v1/skills "$body"
}

echo
echo "================================================================"
echo "  [1/6] alice authors a thin bundle. It is SCORED, and it is NOT"
echo "  refused: a draft is where a skill is supposed to be unfinished,"
echo "  and a registry that refuses work in progress is one where the"
echo "  work happens in a text editor instead (ADR-0053 option 12)."
echo "================================================================"
mkdir -p "$WORK/bundle/changelog"
cat >"$WORK/bundle/changelog/SKILL.md" <<'DOC'
---
name: changelog
description: A generator of formatted release output from repository data.
---

# changelog

TODO: write this up properly.
DOC
authored=$(author "$WORK/bundle/changelog")
quality "$authored"
thin_score=$(printf '%s' "$authored" | field quality score)
says "TODO" "$(cat "$WORK/bundle/changelog/SKILL.md")"

echo
echo "    every check reports, passing ones included: \"this passed\" and"
echo "    \"this is not checked\" must not look the same to somebody"
echo "    deciding whether to trust the number."
printf '%s' "$authored" | field quality checks | node -e '
  let d=""; process.stdin.on("data",c=>d+=c); process.stdin.on("end",()=>{
    const cs = JSON.parse(d);
    console.log(`    ${cs.length} checks, ${cs.filter(c=>c.passed).length} passed`);
  });'

echo
echo "================================================================"
echo "  [2/6] THE SCORE IN THE REGISTRY — the AC's second surface. The"
echo "  listing reads a cache written at authoring, because a scope with"
echo "  forty skills would otherwise read every object of every bundle"
echo "  to draw one column. It is a cache and never a truth: nothing"
echo "  that decides anything reads it (ADR-0053 decision 3)."
echo "================================================================"
listed=$(api "$alice_token" GET "/v1/skills?scope_id=$eng_id")
printf '%s' "$listed" | node -e '
  let d=""; process.stdin.on("data",c=>d+=c); process.stdin.on("end",()=>{
    for (const s of JSON.parse(d).skills) {
      const q = s.quality;
      console.log(`    ${String(q ? q.score : "--").padStart(3)}/100  ${s.name}` +
        (q && q.stale ? "   (stale: an older rubric wrote it)" : ""));
    }
  });'

echo
echo "    alice fixes it — a description that says WHEN to reach for the"
echo "    skill, sections, an example, no leftover marker — and the"
echo "    number moves. The rubric is about the bundle, not about her."
cat >"$WORK/bundle/changelog/SKILL.md" <<'DOC'
---
name: changelog
description: Drafts release notes from a changelog. Use when preparing a release or when asked for release notes.
---

# changelog

## When to use

Reach for this when cutting a release.

## How

Collect the merged pull requests, then edit what comes out:

```sh
git log --oneline "$(git describe --tags --abbrev=0)..HEAD"
```
DOC
authored=$(author "$WORK/bundle/changelog")
quality "$authored"
good_score=$(printf '%s' "$authored" | field quality score)
[ "$good_score" -gt "$thin_score" ] || {
  echo "demo FAILED: fixing the bundle did not raise the score" >&2
  exit 1
}
echo "    $thin_score -> $good_score"

echo
echo "================================================================"
echo "  [3/6] THE SCORE AT REVIEW — the AC's first surface. Recomputed"
echo "  from the proposal's own bytes rather than read from the cache"
echo "  above, so the two agree or the cache is a lie."
echo "================================================================"
proposal=$(api "$alice_token" POST /v1/proposals \
  "{\"scope_id\":\"$eng_id\",\"skill_names\":[\"changelog\"],\"title\":\"publish changelog\"}")
pid=$(printf '%s' "$proposal" | field id)
detail=$(api "$sam_token" GET "/v1/proposals/$pid")
quality "$detail"
review_score=$(printf '%s' "$detail" | field quality score)
[ "$review_score" = "$good_score" ] || {
  echo "demo FAILED: review scored $review_score, the registry cached $good_score" >&2
  exit 1
}
echo "    the recomputed score equals the cached one ($review_score)"

echo
echo "================================================================"
echo "  [4/6] sam works through the checklist and answers one item \`no\`."
echo "  A pack decides whether a checklist is MANDATORY; no pack decides"
echo "  that a written-down \`no\` counts for nothing (decision 7)."
echo "================================================================"
as "$sam_token" proposal checklist "$pid" \
  --item instructions-correct=yes --item scope-appropriate=yes \
  --item not-duplicate=yes --item dependencies-available=yes \
  --item tested=no --note "nobody has run this against a real release yet" 2>&1 |
  sed 's/^/    /'
for tok in "$sam_token" "$sec_token"; do
  api "$tok" POST "/v1/proposals/$pid/approve" '{}' >/dev/null
done
echo
echo "    approved by both, and still refused:"
refused_http "$cora_token" POST "/v1/proposals/$pid/publish"

echo
echo "    sam looks again, having run it, and re-answers. Re-answering is"
echo "    an ordinary act; the chain keeps every answer either way."
as "$sam_token" proposal checklist "$pid" \
  --item instructions-correct=yes --item scope-appropriate=yes \
  --item not-duplicate=yes --item dependencies-available=yes \
  --item tested=yes 2>&1 | sed 's/^/    /'
published=$(api "$cora_token" POST "/v1/proposals/$pid/publish" '{}')
echo "    published at commit $(printf '%s' "$published" | field commit | cut -c1-12)"

echo
echo "================================================================"
echo "  [5/6] THE OVERRIDE — the AC's third clause. A bundle below the"
echo "  bar needs one, and it is a SEPARATE ACT by a SEPARATE AUTHORITY."
echo "  It has to be: cora holds the SkillRead and ChannelPublish that"
echo "  publishing a skill takes, and sam holds the override and no"
echo "  content read at all, so neither could do both (decision 8)."
echo "================================================================"
mkdir -p "$WORK/bundle/hotfix"
cat >"$WORK/bundle/hotfix/SKILL.md" <<'DOC'
---
name: hotfix
description: A procedure for the production hotfix path.
---

# hotfix

TODO: expand this before the next incident.
DOC
authored=$(author "$WORK/bundle/hotfix")
quality "$authored"
proposal=$(api "$alice_token" POST /v1/proposals \
  "{\"scope_id\":\"$eng_id\",\"skill_names\":[\"hotfix\"],\"title\":\"publish hotfix\"}")
hid=$(printf '%s' "$proposal" | field id)
as "$sam_token" proposal checklist "$hid" \
  --item instructions-correct=yes --item scope-appropriate=yes \
  --item not-duplicate=yes --item dependencies-available=yes \
  --item tested=yes >/dev/null 2>&1
for tok in "$sam_token" "$sec_token"; do
  api "$tok" POST "/v1/proposals/$hid/approve" '{}' >/dev/null
done
echo "    a complete checklist and both approvals, and still refused —"
echo "    the only bar left is the score itself:"
refusal=$(refused_http "$cora_token" POST "/v1/proposals/$hid/publish")
printf '%s\n' "$refusal"
says "quality bar" "$refusal"

echo
echo "    cora publishes every other bundle in this demo. She cannot"
echo "    excuse one, and that is the whole content of the action:"
refused_http "$cora_token" POST "/v1/proposals/$hid/quality-override" \
  '{"reason":"we need it for the incident review"}'

echo
echo "    sam can, and says why. He could not publish it himself — a"
echo "    steward holds no content read — which is why this is its own act."
as "$sam_token" proposal override-quality "$hid" \
  --reason "needed for Tuesday's incident review; alice is expanding it this week" 2>&1 |
  sed 's/^/    /'
published=$(api "$cora_token" POST "/v1/proposals/$hid/publish" '{}')
echo "    cora then publishes, with no new privilege and no flag:"
echo "    commit $(printf '%s' "$published" | field commit | cut -c1-12)"

echo
echo "    and the override is bound to THOSE bytes. alice edits the"
echo "    bundle; the next proposal does not inherit the excuse, because"
echo "    nobody agreed to ship whatever it became (decision 4)."
printf '\nStill TODO, differently.\n' >>"$WORK/bundle/hotfix/SKILL.md"
author "$WORK/bundle/hotfix" >/dev/null
proposal=$(api "$alice_token" POST /v1/proposals \
  "{\"scope_id\":\"$eng_id\",\"skill_names\":[\"hotfix\"],\"title\":\"publish the edit\"}")
eid=$(printf '%s' "$proposal" | field id)
as "$sam_token" proposal checklist "$eid" \
  --item instructions-correct=yes --item scope-appropriate=yes \
  --item not-duplicate=yes --item dependencies-available=yes \
  --item tested=yes >/dev/null 2>&1
for tok in "$sam_token" "$sec_token"; do
  api "$tok" POST "/v1/proposals/$eid/approve" '{}' >/dev/null
done
refusal=$(refused_http "$cora_token" POST "/v1/proposals/$eid/publish")
printf '%s\n' "$refusal"
says "quality bar" "$refusal"

echo
echo "================================================================"
echo "  [6/6] the trail. Two new acts, both chained: what a reviewer"
echo "  checked and what somebody excused — with the digest the answers"
echo "  were about, so an auditor can tell exactly which bytes."
echo "================================================================"
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $CLI audit tail --tenant "$tenant_id" --limit 80 |
  grep -E "skill\.(checklist\.recorded|quality\.overridden)" | sed 's/^/    /'

echo
chain=$(DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $CLI audit verify --tenant "$tenant_id")
printf '%s\n' "$chain" | sed 's/^/    /'
says "chain valid" "$chain"

echo
echo "    and the sweep: no payload carries the bundles' bytes."
leaked=$(DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SKIL_DB" \
  $CLI audit tail --tenant "$tenant_id" --limit 200 |
  grep -cE "git log --oneline|describe --tags" || true)
[ "$leaked" = "0" ] || {
  echo "demo FAILED: $leaked audit rows carry file content" >&2
  exit 1
}
echo "    0 rows"

echo
echo "================================================================"
echo "  SKIL-3 acceptance: the score rendered at review and in the"
echo "  registry — the same number from a recompute and from a cache —"
echo "  and a low-scoring bundle reached published only after somebody"
echo "  who could not publish it recorded, in writing and on the chain,"
echo "  why it should ship anyway."
echo "================================================================"
