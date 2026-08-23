#!/usr/bin/env sh
# EVAL-4 acceptance demo: retrieval & injection quality (ADR-0047).
# AC (docs/backlog/EVAL-4.md): one Q&A corpus whose material sits at four
# scope tiers because the suite PROMOTED it there through the governed
# path; grading joins seed to block by record identity; `make eval` reports
# qa_answer_rate and qa_body_rate per scope tier and tokens_per_answer as
# the exchange rate a composition change moves; every question declares
# what it needs from the embedder and the semantic ones are skipped and
# counted rather than scored zero where the embedder cannot rank; the
# deterministic gate runs on the pull-request path; and a real composition
# change that degrades quality fails the gate naming the axis, the
# baseline, the measurement and the delta, with the scope tier that fell
# saying which end of the gradient paid for it.
#
# Flow: a stack on a scratch database -> the corpus climbs from three
# personal leaves to a team, a department and the org through
# POST /v1/proposals and this level's own approvers, and the suite runs
# green with all four tiers reaching the reader -> the COMPOSITION BUDGET
# is narrowed for real, which is the change this demo exists for: nothing
# is deleted, no permit changes, every record is still committed and still
# served, and the block simply cannot hold it all -> the very
# next run fails the gate naming the axis and the delta, and the per-tier
# table shows the FURTHEST material going first while the reader's own
# survives, which is the seed §4.4 gradient doing exactly what it is for
# -> the budget is restored and the gate holds again, which is what makes
# the failure a measurement rather than a broken demo.
#
# The change is deliberately surgical. The pack applied is the *default
# pack's own source* (crates/synveda-policy/src/packs/regulated-strict
# .cedar) with one composition budget changed, so not one permit differs
# and nothing about who may read what moves. A demo that also rewrote the
# policy would have two reasons to fail and would prove neither.
#
# Why a promotion rather than a placement: observe lands records at the
# caller's home scope (ADR-0020) and a service identity's home is a
# `principal`-shaped scope under its anchor (ADR-0018 decision 2), so NOTHING
# can write to a team, a department or an org node. A corpus that spans
# scope tiers is a corpus that climbed through review, which makes a
# per-scope answer rate an assertion about FLOW-5 as much as about CTX-2.
# The approvers are the target scope's own: under the zero-config
# regulated-strict a team publication takes one curator and a department
# or org publication takes a curator AND a steward, two distinct people.
#
# The harness holds no Synveda crate dependency: it reaches the stack
# through /v1 only — observe to seed, the recall sweep to find where the
# records landed, proposals and approvals to climb, inject to ask — so
# every number is a number a caller could have measured for themselves,
# PDP included, and every approval is a real approval (ADR-0028
# decision 1, ADR-0047 decision 3).
#
# Every phase below gets a FRESH TENANT, which is ADR-0028 decision 7's
# rule and not a formality — see demos/eval-2-extraction.sh for the run
# that proved it: two byte-identical runs on one tenant measured
# tokens_mean 129.8 and then 157 with no product change at all.
#
# On Windows, run via Git Bash. Needs postgres from the dev compose, plus
# node. No IdP and no model server: dev-mode bearers and the deterministic
# extractor keep a nightly run about the code (ADR-0028 decision 6). The
# real-embedding half is `make eval-retrieval`, against its own baseline,
# deliberately not here — its `semantic` questions are the two this run
# skips and counts.
set -eu

cd "$(dirname "$0")/.."
. evals/lib.sh

EVAL_KEEP_STATE=1
export EVAL_KEEP_STATE

trap eval_down EXIT INT TERM

# The narrowed budget. The full block measures ~435 estimated tokens over
# 12 records — six at the reader's own leaf, then payments, then
# engineering, then acme — so this holds the near end of the chain and
# cannot reach the far end. Nothing is deleted and nothing is forbidden;
# there is simply less room.
NARROW_BUDGET=320

fresh_stack() {
  if [ -n "${EVAL_DB:-}" ]; then
    eval_down
  fi
  eval_up
  admin=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject eval-admin)
}

report_qa() {
  node -e '
    const { readFileSync } = require("node:fs");
    const report = JSON.parse(readFileSync(process.argv[1], "utf8"));
    for (const corpus of report.qa) {
      const mark = corpus.passed ? "✓" : "✗";
      console.log(`    ${mark} ${corpus.corpus}: ${corpus.served_records} record(s) served to ${corpus.reader}`);
      for (const climb of corpus.promotions) console.log(`        climbed ${climb}`);
      const tiers = new Map();
      for (const q of corpus.questions) {
        if (q.skipped) continue;
        for (const [tier, c] of Object.entries(q.per_tier)) {
          const slot = tiers.get(tier) ?? { expected: 0, reached: 0, body: 0 };
          slot.expected += c.expected; slot.reached += c.reached; slot.body += c.body;
          tiers.set(tier, slot);
        }
      }
      console.log("        scope tier   reached   whole");
      for (const tier of ["user", "team", "department", "org"]) {
        const c = tiers.get(tier);
        if (!c) continue;
        console.log(`        ${tier.padEnd(12)} ${String(c.reached + "/" + c.expected).padEnd(9)} ${c.body}/${c.expected}`);
      }
      const skipped = corpus.questions.filter((q) => q.skipped).length;
      if (skipped) console.log(`        ${skipped} semantic question(s) skipped: this embedder cannot rank a paraphrase`);
    }
    const axes = Object.entries(report.metrics)
      .filter(([name]) => name.startsWith("qa_") || ["tokens_per_answer", "retrieval_precision", "estimator_bias_p95", "staleness_p50_permille"].includes(name));
    console.log("    axes:");
    for (const [name, value] of axes) console.log(`      ${name.padEnd(26)} ${value}`);
    console.log(`    gate: ${report.gate.passed ? "held" : "FAILED"}`);
    for (const breach of report.gate.breaches) {
      console.log(`      ${breach.metric}: baseline ${breach.baseline}, measured ${breach.measured}, delta ${breach.delta}`);
    }
  ' "$EVAL_STATE/report.json"
}

echo "==> a stack on a scratch database: acme/eng/{platform,payments}, a"
echo "    reader and three authors placed where they must propose, and two"
echo "    reviewers at the org"
fresh_stack
echo "    tenant $EVAL_TENANT on $EVAL_GATEWAY_URL"
echo "    qa-reader, qa-team at acme/eng/payments; qa-dept at acme/eng;"
echo "    qa-org, qa-curator, qa-steward at acme."
echo "    The anchors are forced, not chosen: AUTH-3's confinement forbid"
echo "    denies a service identity every resource outside its anchor"
echo "    subtree, and a climb names its target scope — so the author of"
echo "    department material has to sit at the department (ADR-0018"
echo "    decision 4, ADR-0047 decision 3)."

echo
echo "==> [1/4] the suite: a corpus seeded at three personal leaves, then"
echo "    climbed to acme/eng/payments, acme/eng and acme through"
echo "    POST /v1/proposals under each level's own approvers, then ten"
echo "    questions asked of the reader's own context-run block and graded"
echo "    by record identity — never by string containment, because an"
echo "    index entry carries a truncated head and 'demoted' and 'absent'"
echo "    would otherwise be one measurement"
eval_run
echo
report_qa
echo
echo "    Four tiers, all reached. The reader could not see a byte of any"
echo "    author's leaf before the climb — no pack opens another"
echo "    principal's personal scope — so every row above the first is"
echo "    review having happened."

echo
echo "==> [2/4] now a real product change: the COMPOSITION BUDGET narrowed"
echo "    to $NARROW_BUDGET estimated tokens, on a tenant nobody has measured yet,"
echo "    so the only difference between this phase and the last one is"
echo "    the budget itself."
fresh_stack
echo "    tenant $EVAL_TENANT"
echo "    The pack is the default pack's OWN source with one number"
echo "    changed, so not one permit differs. Nothing is deleted. Every"
echo "    record is still committed, still admitted, and still returned by"
echo "    a context run — there is simply less room in the block."
./target/debug/synveda policy apply --tenant "$EVAL_TENANT" \
  --name eval-narrow-budget \
  --composition-budget "$NARROW_BUDGET" \
  --composition-channels published-and-derived \
  crates/synveda-policy/src/packs/regulated-strict.cedar >/dev/null
curl -fsS -X PUT "$EVAL_GATEWAY_URL/v1/policy/default" \
  -H "Authorization: Bearer $admin" -H 'Content-Type: application/json' \
  -d '{"name":"eval-narrow-budget"}' >/dev/null
# The gateway polls for stored-pack changes (SYNVEDA_POLICY_REFRESH_SECS,
# default 5s); wait for the budget to be in force before measuring.
sleep 8
echo "    a ${NARROW_BUDGET}-token composition budget is in force tenant-wide"

echo
echo "==> [3/4] the same suite against the same product, changed. The AC's"
echo "    claim is that this fails, and says which end of the chain paid:"
if eval_run; then
  echo "demo FAILED: the gate held through a budget that cannot carry the" >&2
  echo "             far end of the reader's own chain — a gate that" >&2
  echo "             cannot fail is a dashboard" >&2
  exit 1
fi
echo
echo "    ^ that non-zero exit is the acceptance criterion"
echo
report_qa
echo
echo "    Read the per-tier table. The reader's own material is untouched"
echo "    and the material furthest from it is gone, in gradient order:"
echo "    scopes are placed nearest-first and totally ordered (seed §4.4,"
echo "    ADR-0025 decision 5), so a budget that binds spends itself on"
echo "    the near end and never reaches the far one. That is why the"
echo "    tiers are separate axes — 'quality fell' would have been true"
echo "    and useless, where 'qa_scope_org fell to 0.0' says a promotion"
echo "    that a department signed off is no longer reaching anybody."

echo
echo "==> [4/4] a third fresh tenant at the product default: the same"
echo "    suite is green again, which is what makes the failure above a"
echo "    measurement rather than a broken demo"
fresh_stack
echo "    tenant $EVAL_TENANT"
eval_run
echo
report_qa

echo
echo "EVAL-4 retrieval & injection quality: acceptance criteria pass."
echo "One Q&A corpus at evals/fixtures/qa/ whose material spans four scope"
echo "tiers because the suite climbed it there through real proposals and"
echo "real approvals — the only route there is, since observe lands"
echo "records at the caller's home scope and a service identity's home is"
echo "a leaf. Grading joins observe's event_id to the sweep's provenance"
echo "to the block's record_ids and tiers, so a demotion to the index tier"
echo "and an absence are two different numbers: qa_answer_rate counts what"
echo "reached the reader and qa_body_rate what reached it whole, and the"
echo "gap between them is CTX-4's displacement. tokens_per_answer is the"
echo "exchange rate a composition change moves. Every question declares"
echo "what it needs from the embedder, and the paraphrases are skipped and"
echo "counted here rather than scored zero, because the deterministic hash"
echo "embedder ranks by nothing by construction — 'make eval-retrieval'"
echo "measures them against real BGE-M3 and its own baseline. And a"
echo "narrowed budget — a change no test in this repository would have"
echo "called a failure — drops the axes, fails the gate by name and"
echo "number, and names the end of the gradient that paid for it."
echo "Before merge: the eval job in .github/workflows/ci.yml. Nightly,"
echo "with real embeddings: .github/workflows/eval.yml. Locally: make"
echo "eval, and make eval-retrieval."
