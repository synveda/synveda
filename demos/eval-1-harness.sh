#!/usr/bin/env sh
# EVAL-1 acceptance demo: the eval harness (ADR-0028).
# AC (docs/backlog/EVAL-1.md): `make eval` runs the scenario suite against
# a live stack and reports all five axes as machine-readable JSON plus a
# human summary; a committed baseline gates the run; a real product change
# that degrades quality (a bank-mode pack flip withholding derived memory)
# fails the gate naming the axis, the baseline, the measurement, and the
# delta; nightly workflow; demo script.
#
# Flow: a stack on a scratch database with three actors in two teams ->
# the suite runs green and the gate holds, with every axis reported ->
# the bank-mode switch is thrown for real (a policy pack whose channel
# rule is published-only, assigned at the org, exactly as FLOW-2 will) ->
# the very next run measures the same product answering worse: derived
# memory stops composing, recall and accuracy collapse, and the gate
# fails naming the axis and the delta -> the pack is withdrawn and the
# gate holds again, which is what makes the failure a measurement rather
# than a broken demo.
#
# The harness holds no Synveda crate dependency: it reaches the stack
# through /v1 with each actor's own bearer and nothing else, so what it
# reports is what a caller gets, PDP included (ADR-0028 decision 1).
#
# On Windows, run via Git Bash. Needs postgres from the dev compose, plus
# node. No IdP and no model server: dev-mode bearers and the deterministic
# extractor/embedder keep a nightly run about the code.
set -eu

cd "$(dirname "$0")/.."
. evals/lib.sh

EVAL_KEEP_STATE=1
export EVAL_KEEP_STATE

trap eval_down EXIT INT TERM

echo "==> a stack on a scratch database: acme/eng/{platform,payments},"
echo "    three actors, and nothing in memory yet"
eval_up
echo "    tenant $EVAL_TENANT on $EVAL_GATEWAY_URL"
echo "    curator + newcomer at acme/eng/platform, outsider at acme/eng/payments"

echo
echo "==> the suite: seed through /v1/observe, wait for the pipeline,"
echo "    probe through /v1/inject, grade, gate"
eval_run
echo "    gate held"

echo
echo "==> now a real product change: the bank-mode switch"
echo "    (a pack whose channel rule is published-only, assigned at the"
echo "    org — every scope inherits it, and derived memory stops"
echo "    composing anywhere)"
cat >"$EVAL_STATE/bank.cedar" <<'EOF'
permit (principal, action, resource) when { resource in principal.tenant };
EOF
./target/debug/synveda policy apply --tenant "$EVAL_TENANT" --name eval-bank \
  --composition-budget 1500 --composition-channels published-only \
  "$EVAL_STATE/bank.cedar" >/dev/null
admin=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject eval-admin)
curl -fsS -X PUT "$EVAL_GATEWAY_URL/v1/hierarchy/nodes/$EVAL_ORG/policy" \
  -H "Authorization: Bearer $admin" -H 'Content-Type: application/json' \
  -d '{"name":"eval-bank"}' >/dev/null
# The gateway polls for stored-pack changes (SYNVEDA_POLICY_REFRESH_SECS,
# default 5s); wait for the switch to be in force before measuring.
sleep 8
echo "    published-only in force at the org"

echo
echo "==> the same suite against the same product, changed. The AC's"
echo "    claim is that this fails, and says why:"
# A shorter seed wait for this run only: nothing will ever become
# composable while published-only is in force, and the demo should not
# spend 90 seconds per scenario proving it.
if eval_run --seed-timeout-secs 20; then
  echo "demo FAILED: the gate held through a change that withholds every" >&2
  echo "             derived record — a gate that cannot fail is a dashboard" >&2
  exit 1
fi
echo
echo "    ^ that non-zero exit is the acceptance criterion: a real change"
echo "      in what the product composes, caught by name and by number"

# Every axis and every scenario, as the nightly workflow keeps it.
echo
echo "==> the report the nightly run uploads (abridged):"
node -e '
  const { readFileSync } = require("node:fs");
  const report = JSON.parse(readFileSync(process.argv[1], "utf8"));
  console.log("    metrics:", JSON.stringify(report.metrics));
  console.log("    gate:", report.gate.passed ? "held" : "FAILED");
  for (const breach of report.gate.breaches) {
    console.log(`      ${breach.metric}: baseline ${breach.baseline}, measured ${breach.measured}, delta ${breach.delta}`);
  }
  for (const scenario of report.scenarios) {
    const budget = `${scenario.tokens}/${scenario.budget_tokens} tokens`;
    console.log(`    ${scenario.passed ? "✓" : "✗"} ${scenario.name} (${budget}, block ${scenario.block_hash.slice(0, 12)})`);
    // Omitted entirely when a scenario passed.
    for (const failure of scenario.failures ?? []) console.log(`        ${failure}`);
  }
' "$EVAL_STATE/report.json"

echo
echo "==> withdraw the pack; the same suite is green again"
curl -fsS -X DELETE "$EVAL_GATEWAY_URL/v1/hierarchy/nodes/$EVAL_ORG/policy" \
  -H "Authorization: Bearer $admin" >/dev/null
sleep 8
eval_run
echo "    gate held"

echo
echo "EVAL-1 eval harness: acceptance criteria pass."
echo "The suite runs against a live stack over /v1 only — seeding through"
echo "observe, waiting for the real pipeline, probing through inject — and"
echo "reports accuracy, recall, abstention, tokens, and latency as JSON and"
echo "as a summary. The committed baseline gates it, and a bank-mode pack"
echo "flip — a change no test in this repository would have called a"
echo "failure — collapses recall and accuracy and fails the gate naming the"
echo "axis, the baseline, the measurement, and the delta. Nightly:"
echo ".github/workflows/eval.yml. Locally: make eval."
