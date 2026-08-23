#!/usr/bin/env sh
# EVAL-2 acceptance demo: the extraction quality suite (ADR-0046).
# AC (docs/backlog/EVAL-2.md): one labelled corpus read by both the eval
# harness and MEM-3's unit test; `make eval` reports per-class precision
# and recall for every RecordClass the corpus exercises, plus macro
# averages, measured over the real observe->extract->serve path; the report
# carries produced/expected/matched per class, the unmatched-record list,
# and the pipeline's own committed counts read from the audit chain, so a
# shortfall between what was committed and what a reader is served is its
# own number rather than absorbed into recall; hallucination_rate measured
# from fixture-declared bait and gated at zero; a real product change that
# degrades quality fails the gate naming the axis, the baseline, the
# measurement, and the delta, and the attribution column says why; the
# >2pt tolerance is a declared slack in the committed baseline.
#
# Flow: a stack on a scratch database with one actor per fixture group and
# an auditor who is a member of nothing -> the suite runs green, and the
# two lenses agree (what the pipeline committed == what the reader was
# served) -> a RETENTION HORIZON is applied for real, which is the change
# this demo exists for: the pipeline still commits every record and the
# read path stops serving a whole class of them -> the very next run fails
# the gate naming the axis and the delta, AND the attribution column shows
# committed > served, which is what distinguishes "the reader was not
# shown it" from "the extractor never found it" -> the horizon is
# withdrawn and the gate holds again, which is what makes the failure a
# measurement rather than a broken demo.
#
# The change is deliberately surgical. The pack applied is the *default
# pack's own source* (crates/synveda-policy/src/packs/regulated-strict
# .cedar) with one retention horizon added, so not one permit differs and
# the only thing that changed is a horizon. A demo that also rewrote the
# policy would have two reasons to fail and would prove neither.
#
# The harness holds no Synveda crate dependency: it reaches the stack
# through /v1 only — observe to seed, recall's sweep to enumerate, and the
# audit search to read what the pipeline committed — so every number is a
# number a caller could have measured for themselves, PDP included
# (ADR-0028 decision 1, ADR-0046 decisions 1 and 4).
#
# Every phase below gets a FRESH TENANT, which is ADR-0028 decision 7's
# rule ("a fresh tenant per run is what makes two runs comparable") and
# not a formality. `wait_for_seed` deliberately waits only for the
# material a scenario is graded on, so a first run against a fresh tenant
# composes over a partly-populated corpus and a second run over a
# complete one: `tokens_mean` rises from 129.8 to 157 between two
# byte-identical runs with no product change and no new records admitted
# (the observe buffer dedups them). Comparing phases on one tenant would
# therefore have compared two different corpora and blamed the difference
# on the horizon.
#
# On Windows, run via Git Bash. Needs postgres from the dev compose, plus
# node. No IdP and no model server: dev-mode bearers and the deterministic
# extractor keep a nightly run about the code (ADR-0028 decision 6). The
# live-model measurement is `make eval-extraction-live`, against its own
# baseline, deliberately not here.
set -eu

cd "$(dirname "$0")/.."
. evals/lib.sh

EVAL_KEEP_STATE=1
export EVAL_KEEP_STATE

trap eval_down EXIT INT TERM

# A stack nobody has measured yet. Tearing the previous one down first is
# what makes the phases comparable (see the header) — and it is cheap,
# because the compose services stay up and only the scratch database and
# the two gateways are rebuilt.
fresh_stack() {
  if [ -n "${EVAL_DB:-}" ]; then
    eval_down
  fi
  eval_up
  admin=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject eval-admin)
}

report_extraction() {
  node -e '
    const { readFileSync } = require("node:fs");
    const report = JSON.parse(readFileSync(process.argv[1], "utf8"));
    for (const group of report.extraction) {
      const gap = group.committed_records - group.served_records;
      const mark = group.passed ? "✓" : "✗";
      console.log(
        `    ${mark} ${group.group.padEnd(8)} committed ${group.committed_records}` +
        ` -> served ${group.served_records}` +
        (gap > 0 ? `  (${gap} withheld by the read path, not missed by the extractor)` : "  (nothing withheld)") +
        `  chain ${group.chain_from}..${group.chain_to}`
      );
    }
    const axes = Object.entries(report.metrics)
      .filter(([name]) => name.startsWith("extraction_") || name === "hallucination_rate");
    console.log("    axes:");
    for (const [name, value] of axes) console.log(`      ${name.padEnd(32)} ${value}`);
    console.log(`    gate: ${report.gate.passed ? "held" : "FAILED"}`);
    for (const breach of report.gate.breaches) {
      console.log(`      ${breach.metric}: baseline ${breach.baseline}, measured ${breach.measured}, delta ${breach.delta}`);
    }
  ' "$EVAL_STATE/report.json"
}

echo "==> a stack on a scratch database: acme/eng/{platform,payments}, one"
echo "    actor per fixture group, and an auditor placed nowhere at all"
fresh_stack
echo "    tenant $EVAL_TENANT on $EVAL_GATEWAY_URL"
echo "    extract-{alpha,beta,gamma,delta,epsilon} at acme/eng/platform"
echo "    eval-auditor: the 'auditor' role tenant-wide, no placement, no"
echo "    identity row. AuditRead declares resource: [Tenant] and admits"
echo "    nothing narrower, and AUTH-3 denies the tenant plane to every"
echo "    service identity however bound — so the five actors above could"
echo "    not have read the chain even if they wanted to (ADR-0045)."

echo
echo "==> [1/4] the suite: 50 labelled transcripts seeded through"
echo "    POST /v1/sessions/{id}/events, the chain waited on until the"
echo "    pipeline is done with every one, then /v1/recall's sweep to"
echo "    enumerate what a reader is actually served — and the per-class"
echo "    table scored off that."
echo
echo "    THE SWEEP NO LONGER EXISTS. CPR-12 deleted /v1/recall (ADR-0078"
echo "    decision 5) and a context run cannot stand in: it ranks and"
echo "    budgets where a sweep enumerates, so what it left out would be a"
echo "    property of the budget rather than of extraction. The seed leg is"
echo "    re-pointed onto the session plane and the sweep leg refuses by"
echo "    name, so this run fails with the reason rather than reporting a"
echo "    number measured against a different question. Prompt 18 re-cuts"
echo "    recall; Prompt 32 re-measures."
eval_run
echo
echo "    the attribution column, which is the half a single lens cannot give:"
report_extraction

echo
echo "==> [2/4] now a real product change: a RETENTION HORIZON, on a"
echo "    tenant nobody has measured yet, so the only difference between"
echo "    this phase and the last one is the horizon itself."
fresh_stack
echo "    tenant $EVAL_TENANT"
echo "    The pack is the default pack's OWN source with one horizon"
echo "    added, so not one permit differs. Nobody touches a record."
echo "    No sweep runs. Nothing restarts (MEM-6, ADR-0040)."
cat >"$EVAL_STATE/retention.json" <<'JSON'
{
  "mode": "enforce",
  "ttl": { "episode": 1 },
  "destroy_after_days": 0,
  "staging_days": 7,
  "staleness_half_life_days": 90
}
JSON
# The corpus dates its fixtures in the past on purpose, and this is one of
# the things that depends on it: a horizon is `valid_from <= now - ttl`
# (ADR-0040 decision 8), and `valid_from` is the observed instant. Rebasing
# the corpus onto `now` would silently make this step a no-op.
./target/debug/synveda policy apply --tenant "$EVAL_TENANT" \
  --name eval-episode-horizon \
  --retention "$EVAL_STATE/retention.json" \
  crates/synveda-policy/src/packs/regulated-strict.cedar >/dev/null
curl -fsS -X PUT "$EVAL_GATEWAY_URL/v1/policy/default" \
  -H "Authorization: Bearer $admin" -H 'Content-Type: application/json' \
  -d '{"name":"eval-episode-horizon"}' >/dev/null
# The gateway polls for stored-pack changes (SYNVEDA_POLICY_REFRESH_SECS,
# default 5s); wait for the horizon to be in force before measuring.
sleep 8
echo "    a one-day horizon on 'episode' is in force tenant-wide"
echo "    (the scenario suite seeds transcript deltas at the current"
echo "     instant, so it is untouched — this cuts one class of the"
echo "     extraction corpus and nothing else)"

echo
echo "==> [3/4] the same suite against the same product, changed. The AC's"
echo "    claim is that this fails, and says why:"
if eval_run; then
  echo "demo FAILED: the gate held through a horizon that withholds an" >&2
  echo "             entire record class from every reader — a gate that" >&2
  echo "             cannot fail is a dashboard" >&2
  exit 1
fi
echo
echo "    ^ that non-zero exit is the acceptance criterion"
echo
report_extraction
echo
echo "    Read the two columns together. committed > served says the"
echo "    pipeline extracted exactly what it extracted before and the"
echo "    READ PATH withheld it — so the recall drop is a policy outcome,"
echo "    not the extractor getting worse. Without that column the same"
echo "    number would have read as an extraction regression, and someone"
echo "    would have gone looking for a bug in MEM-3 (ADR-0046 decision 4)."

echo
echo "==> [4/4] a third fresh tenant with NO horizon: the same suite is"
echo "    green again, which is what makes the failure above a measurement"
echo "    rather than a broken demo"
fresh_stack
echo "    tenant $EVAL_TENANT"
eval_run
echo
report_extraction

echo
echo "EVAL-2 extraction quality suite: acceptance criteria pass."
echo "One labelled corpus at evals/fixtures/extraction/ — 50 transcripts,"
echo "54 expectations, every RecordClass — read by this harness over /v1"
echo "and by crates/synveda-ingest/tests/extraction_precision.rs with no"
echo "stack at all, both refusing an unknown field so a change for one"
echo "reader cannot be silently dropped by the other. Per-class precision"
echo "and recall are measured over the real observe->extract->serve path"
echo "and reported with produced/expected/matched beside every ratio;"
echo "hallucination_rate is fixture-declared bait, gated at zero, which"
echo "asserts that a span-copying extractor cannot invent. The gate's 2pt"
echo "tolerance is a declared 'slack' in evals/baseline.json, so it lands"
echo "in a number a reviewer sees rather than in a comparison nobody"
echo "reads. And a horizon that withholds a class — a change no test in"
echo "this repository would have called a failure — drops the axis, fails"
echo "the gate by name and number, and is attributable to the read path"
echo "rather than to the extractor. Nightly: .github/workflows/eval.yml."
echo "Locally: make eval. Against a real model: make eval-extraction-live."
