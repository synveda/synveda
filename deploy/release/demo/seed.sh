#!/usr/bin/env sh
# Builds the ACME demo organisation (OPS-9, ADR-0066).
#
# `synveda init --demo` puts ACME's people in the bundled IdP and stops
# there, because ADR-0055 decision 1 will not let an installer create
# governed objects: they need the operator's own bearer, and the operator has
# not logged in when `init` runs. So `init` printed the commands and a tester
# logged in to an empty product. This is where those commands went.
#
# Everything below is an ordinary governed act under your own bearer — the
# same CLI verbs and `/v1` routes a person or a harness already drives. No
# database connection, no break-glass, no seeded row. A demo organisation
# assembled by bypassing governance would be the one artefact in this product
# whose existence nothing can account for, and the first thing the tour asks
# you to do is verify the chain.
#
# It ships beside the product rather than inside it (ADR-0066 decision 1):
# the demo is not the product, and `synveda --help` is identical either way.
#
# Prerequisites are the product's own — `synveda` on PATH and a completed
# `synveda login`. No `jq` and no `node`: a tester's only stated prerequisite
# is Docker (OPS-8) and this must not quietly add a second. Everything it
# reads is either the CLI's plain-text output or its own pipe-delimited
# shape file.
#
# Usage:
#   ./seed.sh                                     build it
#   ./seed.sh --dry-run                           print it, change nothing
#   ./seed.sh --i-know-this-is-not-a-demo-tenant  seed a non-empty tenant
set -eu

here=$(cd "$(dirname "$0")" && pwd)
SHAPE="${SYNVEDA_DEMO_SHAPE:-$here/organisation.txt}"
SYNVEDA="${SYNVEDA_BIN_PATH:-synveda}"
GATEWAY="${SYNVEDA_GATEWAY:-http://127.0.0.1:8120}"

dry_run=no
force=no
for arg in "$@"; do
  case "$arg" in
    --dry-run) dry_run=yes ;;
    --i-know-this-is-not-a-demo-tenant) force=yes ;;
    -h|--help) sed -n '2,29p' "$0" | sed 's/^#\{1,\} \{0,1\}//'; exit 0 ;;
    *) echo "seed.sh: unknown option $arg (try --help)" >&2; exit 2 ;;
  esac
done

fail() { echo >&2; echo "seed.sh: $*" >&2; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/synveda-demo-XXXXXX")
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

command -v "$SYNVEDA" >/dev/null 2>&1 ||
  fail "no \`synveda\` on PATH. Install it first — see docs/INSTALL.md."
[ -f "$SHAPE" ] || fail "no organisation shape at $SHAPE"

# ── the shape ───────────────────────────────────────────────────────────
# Comments and blanks out once, so every reader below sees only records.
grep -v '^[[:space:]]*#' "$SHAPE" | grep -v '^[[:space:]]*$' > "$work/shape"
setting() { awk -F'|' -v k="$1" '$1=="setting" && $2==k {print $3; exit}' "$work/shape"; }

WAIT_QUERY=$(setting wait_query)
PROPOSAL_TITLE=$(setting proposal_title)
[ -n "$WAIT_QUERY" ] || fail "$SHAPE names no wait_query"
[ -n "$PROPOSAL_TITLE" ] || fail "$SHAPE names no proposal_title"

# ── a bearer ────────────────────────────────────────────────────────────
# `auth token` refreshes an expired credential and exits non-zero when there
# is nothing to refresh, so this doubles as the "have you logged in" check.
BEARER=$("$SYNVEDA" auth token 2>/dev/null) || fail "no stored login.

  Run \`synveda login\` first. The organisation is created under your own
  identity — that is the whole point (ADR-0055 decision 2) — so there is
  nobody to create it as until you have."

api() { # method path [body]
  if [ $# -ge 3 ]; then
    curl -fsS -X "$1" "$GATEWAY$2" \
      -H "Authorization: Bearer $BEARER" -H 'Content-Type: application/json' -d "$3"
  else
    curl -fsS -X "$1" "$GATEWAY$2" -H "Authorization: Bearer $BEARER"
  fi
}

# ── reading the tree ────────────────────────────────────────────────────
# `synveda hierarchy list --json` serves an array of nodes; `tr '{' '\n'`
# puts one node on each line, after which a node's own fields can be read
# with `grep` and `sed` and no field of one node can be confused with a
# field of another.
#
# **This deliberately does not read the human rendering**, and the reason is
# worth the comment. `hierarchy list` prefixes each line with a kind marker —
# `▪` org, `▸` department, `·` team, space for a personal leaf — and reading
# it means comparing those glyphs in `awk`. macOS's BSD awk reports
# `"▪" == "▸"` as **true**: it coerces multibyte strings numerically, both
# become 0, and every marker compares equal to every other. Concatenating
# `""` to force a string comparison does not fix it. gawk and mawk get it
# right, so a seeder written that way would classify every scope correctly on
# a Linux CI runner and silently mis-classify all of them on the Apple
# Silicon laptop this product's own installer targets first.
#
# The JSON path is ASCII throughout, so it cannot have that bug on any awk.
# `--json` is **pretty-printed** (`serde_json::to_string_pretty`), so each
# field arrives on its own line with a space after the colon. Stripping both
# before splitting is what makes a node's fields land on one line together —
# without it, `kind` and `slug` are on different lines and no per-line grep
# can correlate them. Found by running this: the seeder created both
# departments and then could not find either.
tree_json() {
  "$SYNVEDA" hierarchy list --json 2>/dev/null | tr -d ' \n' | tr '{' '\n' || true
}

# `"id":"` needs a quote before the `i`, so it cannot match inside
# `"tenant_id":"` or `"parent_id":"` — the only `id` field it can find is the
# node's own.
id_of() { # kind slug
  tree_json | grep "\"kind\":\"$1\"" | grep "\"slug\":\"$2\"" |
    sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | head -1
}

slugs_of_kind() { # kind
  tree_json | grep "\"kind\":\"$1\"" | sed -n 's/.*"slug":"\([^"]*\)".*/\1/p'
}

known() { # kind slug -> is it one this seeder creates?
  awk -F'|' -v kind="$1" -v s="$2" '
    (kind=="department" && $1=="department" && $2==s) ||
    (kind=="team"       && $1=="team"       && $3==s) { found=1 }
    END { exit !found }' "$work/shape"
}

root=$("$SYNVEDA" hierarchy root 2>/dev/null) ||
  fail "could not read the org root. Is the gateway up, and have you logged in?"

# The tenant's own id, for the closing summary. `synveda audit verify` takes
# `--tenant` with no default, so a summary that printed the bare verb would
# be handing the reader a command that exits non-zero — which is what the
# first version of this script did.
root_tenant=$("$SYNVEDA" whoami 2>/dev/null | awk '$1=="id" {print $2; exit}')

# ── the guard ───────────────────────────────────────────────────────────
# Content, not provenance (ADR-0066 decision 4). A tenant admitted with
# `--demo` and then filled with real memory is exactly the deployment that
# must be refused; a tenant admitted without it and still empty has nothing
# to lose. So: refuse if a department or team exists that this seeder did not
# create.
#
# Personal scopes are ignored, and can be ignored *correctly* because the
# node's own `kind` says which they are rather than a guess about path depth
# — the operator's own leaf exists before this script runs, and counting it
# as foreign would refuse every fresh install, which is the one case that
# must work.
if [ "$force" = no ]; then
  for slug in $(slugs_of_kind department); do
    known department "$slug" || foreign="department $slug"
  done
  for slug in $(slugs_of_kind team); do
    known team "$slug" || foreign="team $slug"
  done
  if [ -n "${foreign:-}" ]; then
    fail "this tenant already holds a $foreign, which seed.sh did not create.

  A scope somebody built is an organisation, and seeding a fabricated one
  beside it is not something this script will do by accident. If you meant it:

      ./seed.sh --i-know-this-is-not-a-demo-tenant"
  fi
fi

# ── --dry-run ───────────────────────────────────────────────────────────
if [ "$dry_run" = yes ]; then
  echo "seed.sh --dry-run"
  echo
  echo "  gateway        $GATEWAY"
  echo "  org root       $root"
  echo "  shape          $SHAPE"
  echo
  echo "  would create"
  awk -F'|' '
    $1=="department" { printf "    department  %-12s (%s)\n", $2, $3 }
    $1=="team"       { printf "    team        %-12s (%s, under %s)\n", $3, $4, $2 }' "$work/shape"
  echo
  echo "  would assign"
  awk -F'|' '$1=="pack" { printf "    %-18s at %s\n", $3, $2 }' "$work/shape"
  echo "    (inherited)        elsewhere — the tenant default stands"
  echo
  echo "  would observe  $(grep -c '^corpus|' "$work/shape") turns through /v1/observe"
  echo "  would open     one proposal climbing memory to the org root"
  echo
  echo "  would NOT      publish anything. Every publication this organisation"
  echo "                 can make needs two distinct people and there is one"
  echo "                 operator — ADR-0066 decision 2, and the thing the tour"
  echo "                 asks you to go and hit for yourself."
  exit 0
fi

echo "seeding the demo organisation under $root"
echo

# ── 1. the scopes ───────────────────────────────────────────────────────
# Idempotent by lookup rather than by bookkeeping (ADR-0066 decision 5):
# ask what is there, create what is not. A manifest of what a previous run
# wrote is a second source of truth that drifts from the first.
echo "[1/4] scopes"
awk -F'|' '$1=="department" {print $2 "|" $3}' "$work/shape" > "$work/departments"
while IFS='|' read -r slug name; do
  if [ -n "$(id_of department "$slug")" ]; then
    echo "      = $slug"
    continue
  fi
  "$SYNVEDA" hierarchy create --parent "$root" --kind department \
    --slug "$slug" --name "$name" >/dev/null || fail "create the $slug department"
  echo "      + $slug"
done < "$work/departments"

awk -F'|' '$1=="team" {print $2 "|" $3 "|" $4}' "$work/shape" > "$work/teams"
while IFS='|' read -r department slug name; do
  if [ -n "$(id_of team "$slug")" ]; then
    echo "      = $department/$slug"
    continue
  fi
  # Resolved per team rather than once, because the departments may have
  # just been created and a list read before them would send every team to
  # the wrong parent.
  parent=$(id_of department "$department")
  [ -n "$parent" ] || fail "the $department department is missing after creating it"
  "$SYNVEDA" hierarchy create --parent "$parent" --kind team \
    --slug "$slug" --name "$name" >/dev/null || fail "create the $department/$slug team"
  echo "      + $department/$slug"
done < "$work/teams"

# ── 2. the packs ────────────────────────────────────────────────────────
echo "[2/4] policy packs"
awk -F'|' '$1=="pack" {print $2 "|" $3}' "$work/shape" > "$work/packs"
while IFS='|' read -r department pack; do
  node_id=$(id_of department "$department")
  [ -n "$node_id" ] || fail "no $department department to assign $pack to"
  # The one write here the product does not surface as a verb:
  # `synveda hierarchy policy` is a read.
  api PUT "/v1/hierarchy/nodes/$node_id/policy" "{\"name\":\"$pack\"}" >/dev/null ||
    fail "assign $pack at $department"
  echo "      $pack at $department"
done < "$work/packs"
echo "      (inherited) elsewhere — the tenant default stands"

# ── 3. the corpus ───────────────────────────────────────────────────────
# Through /v1/observe like any harness, so the records arrive by extraction
# with provenance rather than by insertion.
echo "[3/4] memory"
grep '^corpus|' "$work/shape" | cut -d'|' -f2- > "$work/corpus"
observed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
events=""
index=0
while IFS= read -r text; do
  # Deterministic keys, so a re-run is a duplicate delivery the product
  # already knows how to refuse (MEM-1, ADR-0020 decision 2) rather than a
  # second copy of the corpus.
  escaped=$(printf '%s' "$text" | sed 's/\\/\\\\/g; s/"/\\"/g')
  [ -n "$events" ] && events="$events,"
  events="$events{\"idempotency_key\":\"synveda-demo-seed-$index\""
  events="$events,\"kind\":\"transcript_delta\",\"occurred_at\":\"$observed_at\""
  events="$events,\"payload\":{\"text\":\"$escaped\"}}"
  index=$((index + 1))
done < "$work/corpus"

observed=$(api POST /v1/observe \
  "{\"session_id\":\"synveda-demo-seed\",\"events\":[$events]}") ||
  fail "observe the corpus"
accepted=$(printf '%s' "$observed" | sed -n 's/.*"accepted":\([0-9]*\).*/\1/p')
echo "      $index turns sent, ${accepted:-0} accepted"

# Wait by asking the product, not by sleeping a guessed interval: the wait is
# as long as the pipeline takes and no longer, and a timeout says how far it
# got rather than leaving a demo that looks empty for no stated reason.
echo "      waiting for extraction and embedding"
recalled() { "$SYNVEDA" recall --query "$WAIT_QUERY" --quiet 2>/dev/null || true; }
tries=0
while :; do
  found=$(recalled | grep -c '^── ' || true)
  [ "$found" -ge "$index" ] && break
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    fail "the extraction worker produced $found of $index records in 120s.

  Check SYNVEDA_EXTRACTOR and the gateway's log. The turns are buffered and
  will extract when it recovers, so re-running this script is safe."
  fi
  sleep 2
done
echo "      $found records extracted and embedded"

# ── 4. the proposal that cannot be finished alone ───────────────────────
# A climb rather than a same-scope publication, because of where the records
# are: extraction lands them in the *observer's* personal scope
# (`observe.rs`: `let home = identity.scope_id`), and FLOW-5's rule is that a
# climb goes up the chain composition walks down — so the org root is the
# only target available. `regulated-strict` prices a memory there at curator
# + steward, two distinct people, which is exactly the refusal the tour asks
# the tester to go and meet.
echo "[4/4] a proposal awaiting review"
if "$SYNVEDA" proposal list 2>/dev/null | grep -qF "$PROPOSAL_TITLE"; then
  echo "      already open — left alone"
else
  recalled > "$work/recalled"
  first=$(grep '^── ' "$work/recalled" | head -1 | awk '{print $2}')
  scope=$(grep '^   scope ' "$work/recalled" | head -1 | awk '{print $2}')
  [ -n "$first" ] && [ -n "$scope" ] ||
    fail "no records to propose — the corpus did not extract"
  opened=$(api POST /v1/proposals \
    "{\"scope_id\":\"$root\",\"source_scope_id\":\"$scope\",\"record_ids\":[\"$first\"],\"title\":\"$PROPOSAL_TITLE\"}") ||
    fail "open the demo proposal"
  id=$(printf '%s' "$opened" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | head -1)
  echo "      opened ${id:-a proposal} — memory climbing to the org root"
fi

echo
echo "ACME is standing."
echo
echo "Look at it:"
echo
echo "    synveda hierarchy list"
echo "    synveda recall --query \"how do we roll out payments\""
# `audit verify` takes a tenant UUID and there is no default, so the id is
# resolved here rather than left as an exercise. Printing a command that
# fails is how the first run of this script ended.
echo "    synveda audit verify --tenant $root_tenant"
echo
echo "Then open the console at $GATEWAY/console/ — the inbox has a proposal in"
echo "it. Try to approve it: you cannot, because publishing to the whole"
echo "organisation takes two distinct people and you are one. That refusal is"
echo "the product working, and it is the thing worth showing somebody."
echo
echo "docs/BETA.md is the guided tour, and the list of what does not work yet."
