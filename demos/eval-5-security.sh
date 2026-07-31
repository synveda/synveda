#!/usr/bin/env sh
# EVAL-5 acceptance demo: security evals (ADR-0048).
# AC (docs/backlog/EVAL-5.md): one security corpus whose every (record,
# reader) pair is declared readable or forbidden and refused at parse time
# if a pair is left out; a corpus governed into place rather than seeded —
# the `restricted` tier through a classification proposal, the team
# material through a real climb; every reader asked every generated variant
# over four read surfaces; leaks reported as COUNTS gated at zero, above
# FLOORS on the probe and variant counts and a positive control at 1.0; the
# prompt-injection half as an invariant about the block's lines; and a real,
# governed relaxation that fails the gate naming the axis, the baseline, the
# measurement and the delta.
#
# Flow: a stack with TWO admitted tenants -> the corpus climbs and gets
# classified through real review, and the suite runs green with every zero
# sitting on a denominator the report prints -> a LAPSE is granted for real,
# proposed on the disclosing side and approved by two distinct stewards,
# which is the change this demo exists for: nothing is deleted, no pack
# changes, no role is bound that widens anything, and one team can now read
# another's -> the very next run fails the gate, and the leak line names the
# record, the reader, the surface, the phrasing and the probe index ->
# a third fresh tenant with no grant on it is green again, which is what
# makes the failure a measurement rather than a broken demo.
#
# WHY A LAPSE AND NOT A PACK FLIP. `open-collaboration` at the org would
# have been the obvious change and it discloses nothing here: a pack cannot
# put a sibling team's material into anybody's block, because the candidate
# universe is the caller's placement chain and "widens by lapse and by
# nothing else" (ADR-0037 decision 13). `recall` does widen with the pack,
# but promoted material never left its author's personal leaf (ADR-0034
# decision 3), personal scopes are excluded under every pack including the
# open one, and a query-shaped recall does not follow published channels
# (ADR-0047 reversal trigger (g)). That is a good product property and a
# demo that proves nothing. The lapse is the one mechanism that widens a
# universe, so it is the one change this gate can be shown failing on.
#
# WHAT DOES NOT MOVE, AND WHY THEY ARE SEPARATE AXES. On the failing run
# `security_leaks_sensitivity` and `security_leaks_tenant` stay at zero, and
# they stay there for two different reasons. `supplier-terms` is
# `confidential` and the grant declares only the working tier, so the
# lapse's own ceiling withholds it (ADR-0038 decision 9). `vault-ceremony`
# is `restricted` at a personal leaf, and NO grant can reach it at all:
# base.cedar's forbid has no owner carve-out and the one base-layer permit
# that could lift it carries `resource.kind != "user"`.
#
# The harness holds no Synveda crate dependency: it reaches the stack
# through /v1 only — observe to seed, the recall sweep to find where the
# records landed, proposals and approvals to classify and to climb, then
# inject and all three recall shapes to probe — so every zero is a zero a
# caller could have measured for themselves, PDP included (ADR-0028
# decision 1).
#
# Every phase gets a FRESH TENANT PAIR, which is ADR-0028 decision 7's rule
# and not a formality — see demos/eval-2-extraction.sh for the run that
# proved it.
#
# On Windows, run via Git Bash. Needs postgres from the dev compose, plus
# node. No IdP and no model server: dev-mode bearers and the deterministic
# extractor keep a nightly run about the code (ADR-0028 decision 6).
set -eu

cd "$(dirname "$0")/.."
. evals/lib.sh

EVAL_KEEP_STATE=1
export EVAL_KEEP_STATE

# The demo runs the pull-request slice rather than the nightly's 10,000:
# the claim under demonstration is that the gate fires, and it fires on the
# corpus's own hand-written phrasings long before the combinatorial tail.
# `make eval-security` is the full budget.
EVAL_SECURITY_VARIANTS=${EVAL_SECURITY_VARIANTS:-400}
export EVAL_SECURITY_VARIANTS

# Long enough to outlast a whole measured run, with room to spare. The
# first attempt used 150s and the gate held in phase 3 — because the
# security corpus runs LAST, after the scenarios, five extraction groups
# and the Q&A corpus, so the grant had already expired by the time
# anything probed it. That is the product's expiry working exactly as
# AUTHZ-4 built it and a demo measuring the wrong thing; under
# `regulated-strict` the ceiling is thirty days, so the window is this
# demo's choice and not the pack's.
LAPSE_SECS=${LAPSE_SECS:-1800}

trap eval_down EXIT INT TERM

fresh_stack() {
  if [ -n "${EVAL_DB:-}" ]; then
    eval_down
  fi
  eval_up
  admin=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject eval-admin)
}

report_security() {
  node -e '
    const { readFileSync } = require("node:fs");
    const report = JSON.parse(readFileSync(process.argv[1], "utf8"));
    for (const corpus of report.security) {
      const mark = corpus.passed ? "✓" : "✗";
      console.log(`    ${mark} ${corpus.corpus}: ${corpus.variants_asked} of ${corpus.variants_generated} variant(s) over ${corpus.probes} probe(s) in ${Math.round(corpus.probe_ms)}ms`);
      for (const entry of corpus.premise) console.log(`        governed: ${entry}`);
      console.log(`        controls ${corpus.controls_met}/${corpus.controls_expected} — material a reader is supposed to have, that reached them`);
      const surfaces = Object.entries(corpus.probes_by_surface).map(([s, n]) => `${s} ${n}`).join(", ");
      if (surfaces) console.log(`        surfaces: ${surfaces}`);
      for (const missed of corpus.controls_missed ?? []) console.log(`        MISSED CONTROL: ${missed}`);
      for (const leak of corpus.leaks ?? []) {
        console.log(`        LEAK [${leak.boundary}] ${leak.record} → ${leak.reader} via ${leak.surface} (${leak.predicate}) at probe ${leak.probe}${leak.query ? ` asking ${JSON.stringify(leak.query)}` : ""}`);
      }
      for (const line of corpus.unattributed ?? []) {
        console.log(`        UNATTRIBUTED LINE for ${line.reader} at probe ${line.probe}: ${JSON.stringify(line.line)}`);
      }
      if (corpus.marker_echo_lines?.length) {
        console.log(`        ${corpus.marker_echo_lines.length} distinct marker echo(es) — reported, bounded by nothing (ADR-0048 decision 11)`);
        for (const line of corpus.marker_echo_lines ?? []) console.log(`          ${line}`);
      }
      for (const failure of corpus.failures ?? []) console.log(`        FAILURE: ${failure}`);
    }
    const axes = Object.entries(report.metrics).filter(([name]) => name.startsWith("security_"));
    console.log("    axes:");
    for (const [name, value] of axes) console.log(`      ${name.padEnd(30)} ${value}`);
    console.log(`    gate: ${report.gate.passed ? "held" : "FAILED"}`);
    for (const breach of report.gate.breaches) {
      console.log(`      ${breach.metric}: baseline ${breach.baseline}, measured ${breach.measured}, delta ${breach.delta}`);
    }
  ' "$EVAL_STATE/report.json"
}

api() {
  # api <bearer> <method> <path> [body]
  if [ "$#" -ge 4 ]; then
    curl -fsS -X "$2" "$EVAL_GATEWAY_URL$3" \
      -H "Authorization: Bearer $1" -H 'Content-Type: application/json' -d "$4"
  else
    curl -fsS -X "$2" "$EVAL_GATEWAY_URL$3" -H "Authorization: Bearer $1"
  fi
}

echo "==> a stack on a scratch database, with TWO admitted tenants"
fresh_stack
echo "    tenant $EVAL_TENANT on $EVAL_GATEWAY_URL"
echo "    and a second: $EVAL_TENANT_B"
echo "    acme/sec/{vault,desk} holds the security estate — sec-owner and"
echo "    sec-mate at the vault team, sec-neighbour at the settlement desk,"
echo "    sec-compliance at the org. northwind/clearing in the other tenant"
echo "    holds xt-reader. The runner never sends a tenant: the token"
echo "    carries one, which is what makes a foreign probe a real probe."

echo
echo "==> [1/4] the suite. The corpus is GOVERNED into place before a single"
echo "    probe is asked: vault-ceremony reaches \`restricted\` through a"
echo "    classification proposal its own author opened and two distinct"
echo "    approvers signed, one of them compliance — the only mechanism in"
echo "    this product that mints the tier — and bridge-rota and"
echo "    supplier-terms climb to the vault team through real proposals and"
echo "    real approvals, because observe lands records at the caller's home"
echo "    leaf and nothing else can put material above one."
eval_run
echo
report_security
echo
echo "    Every leak count is zero and every one of them sits on a"
echo "    denominator the report prints. That is the whole design: a"
echo "    zero-tolerance gate over a denominator the run chooses passes by"
echo "    measuring less, so the probe and variant counts are FLOORS and the"
echo "    controls line is what separates this from an empty corpus."

echo
echo "==> [2/4] now a real, governed relaxation on a fresh tenant pair: a"
echo "    LAPSE granting the settlement desk read of the vault team's"
echo "    material — proposed on the disclosing side, approved by two"
echo "    distinct stewards, time-boxed to ${LAPSE_SECS}s and audited."
fresh_stack
echo "    tenant $EVAL_TENANT"
vault=$(node -e '
  const { readFileSync } = require("node:fs");
  console.log(JSON.parse(readFileSync(process.argv[1], "utf8")).scopes.vault);
' "$EVAL_ENV")
desk=$(node -e '
  const { readFileSync } = require("node:fs");
  console.log(JSON.parse(readFileSync(process.argv[1], "utf8")).scopes.desk);
' "$EVAL_ENV")
# Two distinct stewards with authority over the disclosing scope. Binding a
# role is NOT the change under demonstration and cannot be: a content-role
# binding at another team does not bring that team's material into anybody's
# block, because the candidate universe is the caller's chain (ADR-0038's
# own correction). It is here so the matrix has two people to ask.
./target/debug/synveda role bind --tenant "$EVAL_TENANT" \
  --subject sec-compliance --role steward --scope "$EVAL_ORG" >/dev/null
sec_steward=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject sec-compliance)
qa_steward=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject qa-steward)

proposal=$(api "$qa_steward" POST /v1/lapses \
  "{\"scope_id\":\"$vault\",\"grantee_scope_id\":\"$desk\",
    \"action\":\"memory.read\",\"duration_secs\":$LAPSE_SECS,
    \"reason\":\"joint reconciliation review: the desk is on the bridge\"}" |
  eval_json_field proposal_id)
echo "    lapse proposal $proposal opened against acme/sec/vault"
api "$qa_steward" POST "/v1/proposals/$proposal/approve" '{}' >/dev/null
api "$sec_steward" POST "/v1/proposals/$proposal/approve" '{}' >/dev/null
lapse=$(api "$sec_steward" POST "/v1/proposals/$proposal/lapse" '{}' | eval_json_field id)
echo "    granted: lapse $lapse — two distinct stewards, a reason on the record,"
echo "    and an expiry the product enforces without anybody remembering to"

echo
echo "==> [3/4] the same suite against the same product, relaxed. The AC's"
echo "    claim is that this fails, and that it says exactly what crossed:"
if eval_run; then
  echo "demo FAILED: the gate held while a standing grant disclosed one" >&2
  echo "             team's material to another — a zero-tolerance gate" >&2
  echo "             that cannot fail is a dashboard" >&2
  exit 1
fi
echo
echo "    ^ that non-zero exit is the acceptance criterion"
echo
report_security
echo
echo "    Read the leak lines, not just the axis. Each one names the record,"
echo "    the reader, the surface, the predicate that fired and the probe"
echo "    index — the five things needed to reproduce a disclosure, which is"
echo "    why this suite is sequential."
echo
echo "    And read what did NOT move. security_leaks_sensitivity is still"
echo "    zero, and it is held there by two different mechanisms:"
echo "      · supplier-terms is \`confidential\` and this grant declared only"
echo "        the working tier, so the lapse's OWN ceiling withholds it."
echo "      · vault-ceremony is \`restricted\` at a personal leaf, and no"
echo "        grant can reach it at all — base.cedar's forbid has no owner"
echo "        carve-out and the base layer's one permit carries"
echo "        \`resource.kind != \"user\"\`."
echo "    security_leaks_tenant is zero too: nothing about a relaxation"
echo "    inside one tenant is visible from another."

echo
echo "==> [4/4] a third fresh tenant pair, with no grant on it at all: the"
echo "    same suite is green again, which is what makes the failure above"
echo "    a measurement rather than a broken demo. (That a lapse expires on"
echo "    its own timer and restores the denial is AUTHZ-4's own"
echo "    acceptance criterion — demos/authz-4-lapses.sh proves it, and"
echo "    proving it a second time here is what made the first attempt at"
echo "    this demo measure a window rather than a boundary.)"
fresh_stack
echo "    tenant $EVAL_TENANT"
eval_run
echo
report_security

echo
echo "EVAL-5 security evals: acceptance criteria pass."
echo "One security corpus at evals/fixtures/security/ where every (record,"
echo "reader) pair is declared readable or forbidden — the loader refuses a"
echo "file where one is left out, because an undeclared pair still reports"
echo "zero leaks. Its tiers and placements were governed into place through"
echo "classification proposals and real approvals, so a \`restricted\` record"
echo "that does not cross is one the product actually minted. Every reader"
echo "is asked every generated phrasing over four read surfaces, including"
echo "the two no quality suite here has ever graded: recall's wider universe"
echo "and its ids form, which asks the product to refuse a record by name"
echo "with no retrieval in the way. Leaks are COUNTS gated at zero, because"
echo "a rate divides by a denominator the run chooses and three decimals"
echo "round one leak in ten thousand away; the probe and variant counts are"
echo "FLOORS, because a one-sided gate with a free denominator passes by"
echo "measuring less; and the controls axis at 1.0 is what makes a run of"
echo "zeros a measurement rather than an empty corpus. The"
echo "prompt-injection half is an invariant about lines: a record's content"
echo "cannot produce one, so it cannot forge a scope header, an entry no"
echo "record backs, a marker on a line of its own, or a watermark."
echo "Before merge: the eval job in .github/workflows/ci.yml, at a 400-"
echo "variant slice. Nightly, at the full 10,000: the security job in"
echo ".github/workflows/eval.yml. Locally: make eval, and make eval-security."
