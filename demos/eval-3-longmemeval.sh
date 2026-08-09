#!/usr/bin/env sh
# EVAL-3 acceptance demo: LongMemEval through the governed path (ADR-0061).
# AC (docs/backlog/EVAL-3.md): reproducible scores published in repo,
# tracked per release, as a marketing artefact.
#
# The AC's load-bearing word is *reproducible*, and this demo is about
# that rather than about the score. A number anybody can print is not an
# artefact; a number that names the corpus it was computed from, the two
# models that produced it, the slice it covers and the commit it ran at —
# and that a build refuses to let anyone edit afterwards — is.
#
# Flow: show the published row and the score file behind it -> prove the
# table is generated, by editing a cell and watching `make ci`'s own check
# reject it -> prove the publisher refuses every shape of claim that would
# outrun its evidence -> then, if the corpus is present, run the thing for
# real.
#
# Runnable without the corpus. LongMemEval is 264 MiB and fetched rather
# than committed (evals/fixtures/longmemeval/NOTICE.md), so the first three
# acts assert the publication discipline against the committed artefacts
# and the fourth is skipped with a named reason when the data is absent.
# A demo that could only run on the machine that already did the work
# would demonstrate nothing to anybody else.
set -eu

cd "$(dirname "$0")/.."

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
note() { printf '  %s\n' "$*"; }

SCORES=evals/scores
DOC=docs/BENCHMARKS.md
CORPUS=${EVAL_LONGMEMEVAL_CORPUS:-evals/fixtures/longmemeval/longmemeval_s_cleaned.json}

say "1. What is published"
if ! ls $SCORES/longmemeval-*.json >/dev/null 2>&1; then
  note "No score is published yet. That is a legitimate state — the table"
  note "says so itself — but this demo has nothing to show. Run act 4 first."
  exit 0
fi
row=$(ls $SCORES/longmemeval-*.json | tail -1)
note "score file: $row"
python3 - "$row" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(f"  corpus     {d['corpus']['file']}  blake3 {d['corpus']['digest'][:12]}")
print(f"  slice      {d['slice']['rule']}")
print(f"  reader     {d['reader']}")
print(f"  judge      {d['judge']}  (agreement {d['judge_agreement']:.3f})")
print(f"  commit     {d['commit'][:12]}   measured {d['measured_at'][:10]}")
print(f"  QA {d['qa_accuracy']:.3f}   retrieval {d['retrieval_recall']:.3f}")
PY
note ""
note "Every column a reader needs to reproduce it, and the judge's own"
note "agreement beside the score — decision 4: no claim here may be"
note "tighter than the number that bounds what the judge can tell apart."

say "2. The table is generated, not written"
node scripts/publish-benchmark.mjs
note ""
note "Now edit one cell by hand, as somebody rounding a number up would:"
cp "$DOC" "$DOC.demo-backup"
trap 'mv -f "$DOC.demo-backup" "$DOC" 2>/dev/null || true' EXIT INT TERM
python3 - "$DOC" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
# Nudge the QA accuracy in the rendered row only.
s = re.sub(r"(\| \[0[^|]*\| )(\d\.\d{3})( \|)", lambda m: m.group(1) + "0.900" + m.group(3), s, count=1)
open(p, "w").write(s)
PY
note "  (QA accuracy -> 0.900)"
if node scripts/publish-benchmark.mjs; then
  echo "DEMO FAILED: a hand-edited cell was accepted" >&2
  exit 1
fi
note ""
note "Refused. That check is in \`make ci\`, so the edit fails a build"
note "rather than becoming a claim. Restoring."
mv -f "$DOC.demo-backup" "$DOC"
trap - EXIT INT TERM
node scripts/publish-benchmark.mjs

say "3. What the publisher refuses"
note "Five ways a row could claim more than its run established. Each is"
note "checked against a report that is real apart from the one flaw."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
for case in tier corpus models ungraded; do
  python3 - "$row" "$tmp/$case.json" "$case" <<'PY'
import json, sys
src, dst, case = sys.argv[1], sys.argv[2], sys.argv[3]
d = json.load(open(src))
# Rebuild a report-shaped object from the published row.
run = {"tier": "judged", "slice": dict(d["slice"], file=d["corpus"]["file"],
       corpus_instances=d["corpus"]["instances"]), "models": d["models"],
       "started_at": d["measured_at"], "metrics": dict(d["metrics"])}
if case == "tier":     run["tier"] = "retrieval"
if case == "corpus":   run["slice"]["file"] = "my_own_corpus.json"
if case == "models":   run["models"] = {}
if case == "ungraded": run["metrics"]["longmemeval_qa_ungraded"] = 2.0
json.dump(run, open(dst, "w"))
PY
  printf '  %-9s ' "$case:"
  if node scripts/publish-benchmark.mjs "$tmp/$case.json" >/dev/null 2>&1; then
    echo "DEMO FAILED: $case was published" >&2; exit 1
  fi
  node scripts/publish-benchmark.mjs "$tmp/$case.json" 2>&1 | head -1 | cut -c1-150
done
note ""
note "And the fifth needs no fixture: a dirty working tree is refused,"
note "because a row naming a commit that is not the code which produced"
note "the score is the one lie a reproducibility column cannot survive."

say "4. Producing one"
if [ ! -f "$CORPUS" ]; then
  note "SKIPPED — no corpus at $CORPUS."
  note "It is 264 MiB and fetched rather than committed; see"
  note "evals/fixtures/longmemeval/NOTICE.md for the two curl lines."
  note ""
  note "With it present, and ANTHROPIC_API_KEY set:"
  note "  caffeinate -is make eval-longmemeval-judged   # ~30 min, ~34k tokens"
  note "  node scripts/publish-benchmark.mjs <report>"
  note ""
  note "caffeinate is not decoration: an unattended run on a machine that"
  note "sleeps loses its stack mid-flight, which cost six runs to learn."
  exit 0
fi
if [ "${EVAL_3_DEMO_RUN:-}" != "1" ]; then
  note "Corpus present, but not run: this seeds ~4,900 turns through /v1"
  note "and the PDP, takes ~30 minutes and bills a model per instance. A"
  note "demo that spends money because somebody read the file is a demo"
  note "nobody runs twice. Set EVAL_3_DEMO_RUN=1 to do it for real."
  exit 0
fi
note "Running the judged tier — ~4,900 turns through /v1 and the PDP,"
note "then reading and grading every block."
caffeinate -is make eval-longmemeval-judged
note ""
note "Publish it with: node scripts/publish-benchmark.mjs <report.json>"
