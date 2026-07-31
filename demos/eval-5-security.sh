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
# sitting on a denominator the report prints -> `open-collaboration`, a pack
# this product ships, is applied unmodified at the security department,
# which is the change this demo exists for: nothing is deleted, no record
# moves, no role is bound, and one tier's worth of material stops being
# withheld -> the very next run fails the gate, and the leak line names the
# record, the reader, the surface, the phrasing and the probe index ->
# a third fresh tenant at the product default is green again, which is what
# makes the failure a measurement rather than a broken demo.
#
# WHY THE PACK AND NOT A LAPSE, which is where this demo was written first
# and the finding is worth more than the demo. A lapse is the one mechanism
# that widens a candidate universe (ADR-0037 decision 13), so it looked like
# the only lever — and it discloses NOTHING here, twice over. Every actor in
# `evals/lib.sh` is a service identity, and base.cedar's confinement forbid
# denies one every resource outside its anchor subtree "regardless of bound
# roles", carving out only own-chain MemoryRead (ADR-0018). Cedar forbids
# beat permits, including the base layer's own lapse permit. So a token
# confined to an anchor cannot be widened by any grant, by anybody, ever —
# which is a strong product property and a demo that proves nothing.
#
# What CAN change what a confined reader composes is its own chain's pack.
# `supplier-terms` is `confidential` and published at the vault team, so
# sec-mate is a member of the scope that names it and is withheld only by
# the tier set: `regulated-strict` admits the working tiers at a team and
# `open-collaboration` admits `confidential` (personal scopes still
# excluded). One pack assignment, and the record crosses.
#
# WHAT DOES NOT MOVE, AND WHY THEY ARE SEPARATE AXES. On the failing run
# `security_leaks_scope` and `security_leaks_tenant` stay at zero, held by
# two mechanisms that are not the pack's. sec-neighbour is at the sibling
# team and the same pack permits it vault's material outright — the
# CONFINEMENT FORBID is what still refuses, not the policy the operator just
# changed. And `vault-ceremony` is `restricted`: base.cedar forbids it to
# every reader without a tier-declaring grant, and no pack can author that
# away.
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

# The pack this product ships, applied unmodified. Not a variant of it and
# not a hand-written permit: what makes this demo a claim about the product
# is that an operator could make exactly this change by name.
OPEN_PACK=crates/synveda-policy/src/packs/open-collaboration.cedar

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
      // Capped for the same reason the harness caps its own: one
      // disclosure recurs under every phrasing that reaches it.
      const leaks = corpus.leaks ?? [];
      for (const leak of leaks.slice(0, 8)) {
        console.log(`        LEAK [${leak.boundary}] ${leak.record} → ${leak.reader} via ${leak.surface} (${leak.predicate}) at probe ${leak.probe}${leak.query ? ` asking ${JSON.stringify(leak.query)}` : ""}`);
      }
      if (leaks.length > 8) {
        const pairs = new Set(leaks.map((l) => `${l.record}→${l.reader}`));
        console.log(`        … and ${leaks.length - 8} more, over ${pairs.size} distinct (record, reader) pair(s): ${[...pairs].join(", ")}`);
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
echo "==> [2/4] now a real, governed policy change on a fresh tenant pair:"
echo "    open-collaboration — a pack this product SHIPS, applied"
echo "    unmodified at the security department. Nothing is deleted, no"
echo "    record moves, no role is bound. One tier stops being withheld."
fresh_stack
echo "    tenant $EVAL_TENANT"
sec=$(node -e '
  const { readFileSync } = require("node:fs");
  console.log(JSON.parse(readFileSync(process.argv[1], "utf8")).scopes.sec);
' "$EVAL_ENV")
# At the DEPARTMENT, not the org: the security estate is the blast radius,
# so EVAL-1's scenarios, EVAL-2's corpus and EVAL-4's Q&A corpus — all of
# them under acme/eng — are measuring the same product they measured in
# phase 1. A demo that also moved those would have several reasons to fail
# and would prove none of them.
./target/debug/synveda policy apply --tenant "$EVAL_TENANT" \
  --name eval-open \
  --composition-budget 1500 \
  --composition-channels published-and-derived \
  "$OPEN_PACK" >/dev/null
curl -fsS -X PUT "$EVAL_GATEWAY_URL/v1/hierarchy/nodes/$sec/policy" \
  -H "Authorization: Bearer $admin" -H 'Content-Type: application/json' \
  -d '{"name":"eval-open"}' >/dev/null
# The gateway polls for stored-pack changes (SYNVEDA_POLICY_REFRESH_SECS,
# default 5s); wait for it to be in force before measuring.
sleep 8
echo "    open-collaboration in force at acme/sec and everything under it"
echo "    Read what it changed: seed §6 calls it \"org-wide read for"
echo "    non-restricted content\", and the half that matters here is the"
echo "    tier — regulated-strict admits the working tiers at a team,"
echo "    this admits confidential too (personal scopes still excluded)."

echo "==> [3/4] the same suite against the same product, opened. The AC's"
echo "    claim is that this fails, and that it says exactly what crossed:"
if eval_run; then
  echo "demo FAILED: the gate held while a pack change disclosed" >&2
  echo "             confidential material to a reader the corpus says" >&2
  echo "             must not have it — a zero-tolerance gate that cannot" >&2
  echo "             fail is a dashboard" >&2
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
echo "    And read what did NOT move, because neither is held by the pack:"
echo "      · security_leaks_scope is zero. This same pack permits"
echo "        sec-neighbour vault's material outright — what still refuses"
echo "        is base.cedar's CONFINEMENT FORBID, which denies a service"
echo "        identity every resource outside its anchor subtree regardless"
echo "        of bound roles (ADR-0018). A token confined to an anchor"
echo "        cannot be widened by policy, by a grant, or by anybody."
echo "      · security_leaks_sensitivity moved for confidential and NOT for"
echo "        restricted: vault-ceremony is forbidden to every reader"
echo "        without a tier-declaring grant, in the base layer, where no"
echo "        pack can author it away. One tier a pack may open, one it may"
echo "        not, on the same run and the same corpus."
echo "      · security_leaks_tenant is zero: nothing about a policy change"
echo "        inside one tenant is visible from another."

echo
echo "==> [4/4] a third fresh tenant pair, at the product default: the"
echo "    same suite is green again, which is what makes the failure above"
echo "    a measurement rather than a broken demo."
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
