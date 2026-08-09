#!/usr/bin/env node
// Publishes a LongMemEval score (EVAL-3, ADR-0061 decision 11), and
// checks that the published table still says what the score files say.
//
//   node scripts/publish-benchmark.mjs <report.json>   publish a run
//   node scripts/publish-benchmark.mjs                 check (what CI runs)
//
// Decision 11: "`evals/scores/longmemeval-<version>.json` plus a rendered
// table in `docs/BENCHMARKS.md`, each row carrying the score, the judge's
// agreement rate, both model versions as served, the instance count and
// the commit. 'Tracked per release' is a file that accumulates rows, not a
// number somebody edits."
//
// This lives in a script rather than in `synveda-eval --publish` for a
// reason ADR-0028 decision 1 already fixed: the harness holds no Synveda
// dependency and knows nothing about this repository, which is what lets
// it be pointed at a deployment whose source it does not have. It has no
// business running `git rev-parse`. So the harness measures and writes a
// report, and publishing — which needs the version, the commit and the
// working tree — is a separate, deliberate act. That split is also what
// the ADR's own consequence predicted: "the published number will
// sometimes be older than `main`."
//
// Only the region between the markers in docs/BENCHMARKS.md is rewritten.
// The generator that used to write docs/backlog/*.md whole was removed on
// 2026-08-05 because regenerating discarded the hand-written narrative; a
// marketing artefact is *mostly* narrative, so the same mistake here would
// cost more.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const SCORES = "evals/scores";
const DOC = "docs/BENCHMARKS.md";
const BEGIN = "<!-- benchmarks:begin -->";
const END = "<!-- benchmarks:end -->";

/// The corpus files a published LongMemEval score may be computed from,
/// and the instance count the benchmark has.
///
/// This is the guard that matters most in this script. Every other check
/// here catches a mistake; this one catches the failure ADR-0061 is about
/// — a claim that outruns its evidence. A run against a hand-made corpus
/// in LongMemEval's *shape* produces a report indistinguishable from a
/// real one at a glance, and publishing it would put a number under the
/// heading "LongMemEval" that LongMemEval never produced. The digest in
/// the row would technically disclose it. Nobody reads a digest.
///
/// EVAL-7's second corpus adds a line here, in a diff somebody reviews.
const PUBLISHABLE = {
  "longmemeval_s_cleaned.json": "full haystacks, ~115k tokens each — the benchmark as published",
  "longmemeval_m_cleaned.json": "longer haystacks, the same questions under a harder retrieval load",
  "longmemeval_oracle.json": "evidence sessions only — reading and judging with retrieval removed",
};
const CORPUS_INSTANCES = 500;

const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exit(1);
};

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

function workspaceVersion() {
  const raw = readFileSync("Cargo.toml", "utf8");
  const match = raw.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!match) fail("Cargo.toml declares no workspace version");
  return match[1];
}

/// One published row, from one judged run.
function row(report) {
  const metric = (name) => {
    const value = report.metrics?.[name];
    if (typeof value !== "number") {
      fail(
        `the report measures no \`${name}\`. A published row carries the score, the judge's ` +
          `agreement rate, both models, the instance count and the commit (decision 11) — a ` +
          `row missing one of them is a claim with a piece of its provenance removed.`,
      );
    }
    return value;
  };

  if (report.tier !== "judged") {
    fail(
      `this report is the \`${report.tier}\` tier. The published figure is the model-judged ` +
        `one; publishing a retrieval-only run under the benchmark's name would be publishing ` +
        `half the claim as the whole of it.`,
    );
  }
  const corpus = report.slice?.file;
  if (!(corpus in PUBLISHABLE)) {
    fail(
      `the corpus was \`${corpus}\`, which is not a LongMemEval release ` +
        `(${Object.keys(PUBLISHABLE).join(", ")}). A run against a corpus in LongMemEval's ` +
        `shape produces a report that looks exactly like a real one, and publishing it would ` +
        `put a number under this benchmark's name that this benchmark never produced. Run it ` +
        `against the fetched corpus — see evals/fixtures/longmemeval/NOTICE.md.`,
    );
  }
  if (report.slice.corpus_instances !== CORPUS_INSTANCES) {
    fail(
      `the corpus holds ${report.slice.corpus_instances} instances and LongMemEval has ` +
        `${CORPUS_INSTANCES}. Either the file is truncated or it is not the corpus it is named ` +
        `after; a score over a subset published as the benchmark's is the same claim outrunning ` +
        `its evidence.`,
    );
  }
  for (const role of ["reader", "judge"]) {
    if (!report.models?.[role]) {
      fail(
        `the report names no ${role} model. Decision 6: the score is a joint property of the ` +
          `block, the reader and the judge, and a memory benchmark figure quoted without its ` +
          `reader model is not reproducible by anyone including us.`,
      );
    }
  }
  if (metric("longmemeval_qa_ungraded") > 0) {
    fail(
      `${report.metrics.longmemeval_qa_ungraded} instance(s) could not be graded. An accuracy ` +
        `whose denominator shrank because the reader or the judge errored is an accuracy over ` +
        `the easy half — re-run before publishing.`,
    );
  }

  const model = (role) =>
    report.models[`${role}_effort`]
      ? `${report.models[role]} (${report.models[`${role}_effort`]})`
      : report.models[role];

  return {
    benchmark: "longmemeval",
    version: workspaceVersion(),
    commit: git("rev-parse", "HEAD"),
    measured_at: report.started_at,
    corpus: {
      file: corpus,
      digest: report.slice.digest,
      instances: report.slice.corpus_instances,
    },
    // The slice, in full, because decision 7's rule travels with the
    // number it produced: a suite that bounds its coverage says what it
    // bounded, and a published row is the last place that can still be
    // said.
    slice: report.slice,
    models: report.models,
    reader: model("reader"),
    judge: model("judge"),
    // Decision 4: no claim EVAL-3 publishes may be tighter than its
    // judge's own agreement, so the two travel together or not at all.
    judge_agreement: metric("judge_agreement"),
    judge_false_accept_rate: metric("judge_false_accept_rate"),
    judge_false_reject_rate: metric("judge_false_reject_rate"),
    qa_accuracy: metric("longmemeval_qa_accuracy"),
    qa_per_type: metric("longmemeval_qa_per_type"),
    retrieval_recall: metric("longmemeval_retrieval_recall"),
    bound_instances: metric("longmemeval_bound_instances"),
    // The self-reading bias, when the run had it (option 7). It rides
    // into the row rather than the narrative, because a reader comparing
    // two rows needs to know which of them has it.
    ...(report.independence ? { independence: report.independence } : {}),
    metrics: report.metrics,
  };
}

function scoreFiles() {
  if (!existsSync(SCORES)) return [];
  return readdirSync(SCORES)
    .filter((name) => name.startsWith("longmemeval-") && name.endsWith(".json"))
    .sort()
    .map((name) => ({ name, row: JSON.parse(readFileSync(join(SCORES, name), "utf8")) }));
}

/// The generated region. Newest first, because the question a reader
/// arrives with is "what does it score now".
function table(scores) {
  if (scores.length === 0) {
    return [
      "_No score has been published yet._ The corpus is fetched rather than committed",
      "(`evals/fixtures/longmemeval/NOTICE.md`), and `scripts/publish-benchmark.mjs` refuses",
      "any run that did not measure a LongMemEval release — so this table stays empty until",
      "somebody runs the real thing, rather than filling with numbers from a corpus that",
      "merely has the right shape.",
    ].join("\n");
  }
  const rows = [...scores].reverse().map(({ name, row }) => {
    const cells = [
      `[${row.version}](../${SCORES}/${name})`,
      row.qa_accuracy.toFixed(3),
      row.retrieval_recall.toFixed(3),
      row.judge_agreement.toFixed(3),
      row.reader,
      row.judge,
      `${row.slice.instances} of ${row.corpus.instances}`,
      `\`${row.corpus.file}\` \`${row.corpus.digest.slice(0, 12)}\``,
      `\`${row.commit.slice(0, 12)}\``,
      row.measured_at.slice(0, 10),
    ];
    return `| ${cells.join(" | ")} |`;
  });
  return [
    "| Release | QA accuracy | Retrieval recall | Judge agreement | Reader | Judge | Instances | Corpus | Commit | Measured |",
    "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ...rows,
  ].join("\n");
}

function render(scores) {
  const doc = readFileSync(DOC, "utf8");
  const begin = doc.indexOf(BEGIN);
  const end = doc.indexOf(END);
  if (begin === -1 || end === -1 || end < begin) {
    fail(`${DOC} is missing its ${BEGIN} / ${END} markers`);
  }
  return `${doc.slice(0, begin + BEGIN.length)}\n\n${table(scores)}\n\n${doc.slice(end)}`;
}

const [, , reportPath] = process.argv;

if (reportPath) {
  // The report is judged before the environment is. Both refusals are
  // real, but a report that is the wrong tier is wrong on any machine,
  // while a dirty tree is a fact about this one — and checking the tree
  // first made every other refusal print "dirty working tree" instead of
  // its own reason, which is a check that looks like coverage and is not.
  const report = JSON.parse(readFileSync(reportPath, "utf8"));
  const published = row(report);

  // A published score names a commit, so the working tree has to *be*
  // that commit. Otherwise the row attributes a number to code that was
  // never what produced it — which is the one lie a reproducibility
  // column cannot survive.
  const dirty = git("status", "--porcelain");
  if (dirty) {
    fail(
      `the working tree is dirty, so the commit this row would name is not the code that ` +
        `produced the score:\n${dirty}\nCommit first, then publish.`,
    );
  }
  const name = `longmemeval-${published.version}-${published.commit.slice(0, 12)}.json`;
  mkdirSync(SCORES, { recursive: true });
  writeFileSync(join(SCORES, name), `${JSON.stringify(published, null, 2)}\n`);
  writeFileSync(DOC, render(scoreFiles()));
  console.log(
    `published ${SCORES}/${name} — QA ${published.qa_accuracy.toFixed(3)}, ` +
      `retrieval ${published.retrieval_recall.toFixed(3)}, ` +
      `judge agreement ${published.judge_agreement.toFixed(3)}; ${DOC} re-rendered`,
  );
} else {
  // Check mode. "A number somebody edits" is exactly what this catches:
  // the table is a function of the score files, and a hand-typed cell
  // makes the two disagree.
  const scores = scoreFiles();
  if (readFileSync(DOC, "utf8") !== render(scores)) {
    fail(
      `${DOC}'s table does not match ${SCORES}. It is generated, not written: re-run ` +
        `\`node scripts/publish-benchmark.mjs <report.json>\`, or revert the edit. A published ` +
        `score is a measurement, and a cell somebody typed is not one.`,
    );
  }
  console.log(
    `${DOC} matches ${scores.length} published score(s) in ${SCORES}`,
  );
}
