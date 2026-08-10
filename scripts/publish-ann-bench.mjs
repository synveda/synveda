#!/usr/bin/env node
// Publishes TEN-3's dense-leg rows (ADR-0063 decision 1), and checks that
// the rows already published still say what they said.
//
//   node scripts/publish-ann-bench.mjs <run-dir>   publish a sweep
//   node scripts/publish-ann-bench.mjs             check (what CI runs)
//
// Decision 1: "It records like EVAL-3's scores do: a file that accumulates
// rows with the corpus digest, the pgvector version and the commit in
// each, because 'benchmark vs unpartitioned **recorded**' is the AC's own
// word."
//
// The split between measuring and publishing is ADR-0028 decision 1's, for
// its reason: the harness knows nothing about this repository and has no
// business running `git rev-parse`, so it measures and writes a report,
// and publishing — which needs the commit and a clean tree — is a separate,
// deliberate act. `scripts/publish-benchmark.mjs` is the same split for
// LongMemEval.
//
// This is a sibling of that script and not an extension of it, because the
// two publish different kinds of claim. A LongMemEval score is a marketing
// artefact and its refusals guard a claim that could outrun its evidence in
// public. These rows are engineering evidence for an ADR gate, and their
// refusals guard something narrower and more specific: **a row that cannot
// be reproduced, or that was never measured more than once.** ADR-0063's
// first table was n=1 in every row and three of its four findings have
// since been withdrawn. That is the mistake this file exists to make
// unrepeatable.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const SCORES = "evals/scores";
const FILE = join(SCORES, "ten3-dense-leg.json");

/// Three, not two. The median of two runs is their mean, and a mean is the
/// statistic this file exists to avoid: recall carries ±3 points of
/// run-to-run variance at this corpus size, so one unlucky HNSW graph moves
/// a two-run row by half the variance and nothing in the row says so.
const MIN_RUNS = 3;

const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exit(1);
};

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

/// The corpus is *generated*, so there is no file to hash. What identifies
/// it is its specification: the same parameters and the harness's fixed LCG
/// seed produce the same vectors every time.
///
/// It is named `corpus_spec_digest` rather than `corpus_digest` because it
/// is not the corpus. Record ids are UUIDv7 and pgvector assigns HNSW
/// levels at random, so two runs of one specification build different
/// graphs — which is exactly why a row carries a spread and why MIN_RUNS
/// exists. A digest that claimed to pin the corpus would be promising a
/// determinism this benchmark does not have.
function corpusSpecDigest(report) {
  const spec = {
    records: report.corpus.records,
    tenants: report.corpus.tenants,
    scopes_per_tenant: report.corpus.scopes_per_tenant,
    queries: report.corpus.queries,
    dim: report.dim,
    k: report.k,
  };
  return createHash("sha256").update(JSON.stringify(spec)).digest("hex").slice(0, 12);
}

const bound = (arm) => arm.max_scan_tuples ?? "default";
const armId = (arm) =>
  `${arm.iterative_scan}|${arm.ef_search}|${bound(arm)}|${arm.plan_cache_mode}`;
const rowId = (row) =>
  `${row.commit}|${row.corpus_spec_digest}|${armId(row.arm)}|${row.regime}`;

const NODE = /(Seq Scan on|Index Scan using|Index Only Scan using|Bitmap Index Scan on) ([a-z0-9_]+)/g;
const accessPaths = (plan) =>
  [...new Set([...(plan ?? "").matchAll(NODE)].map(([, node, name]) => `${node} ${name}`))].sort();

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

const spread = (values) => ({
  median: median(values),
  min: Math.min(...values),
  max: Math.max(...values),
});

function readPublished() {
  if (!existsSync(FILE)) return [];
  let parsed;
  try {
    parsed = JSON.parse(readFileSync(FILE, "utf8"));
  } catch (error) {
    fail(`${FILE} does not parse: ${error.message}`);
  }
  if (!Array.isArray(parsed?.rows)) fail(`${FILE} has no \`rows\` array`);
  return parsed.rows;
}

// ── check ────────────────────────────────────────────────────────────────────
function check() {
  const rows = readPublished();
  if (rows.length === 0) {
    console.log(`ok: ${FILE} holds no rows yet`);
    return;
  }
  const seen = new Set();
  for (const [index, row] of rows.entries()) {
    const where = `${FILE} row ${index}`;
    for (const field of ["commit", "pgvector", "postgres", "corpus_spec_digest", "regime"]) {
      if (!row[field]) fail(`${where}: no \`${field}\`. A row without its provenance is a number nobody can reproduce.`);
    }
    if (!row.arm?.plan_cache_mode) {
      fail(
        `${where}: no \`arm.plan_cache_mode\`. It is part of the arm's identity — TEN-3's ` +
          `withdrawn table is what a row that omitted it looks like.`,
      );
    }
    if (row.runs < MIN_RUNS) {
      fail(`${where}: ${row.runs} run(s), and a published row needs ${MIN_RUNS}.`);
    }
    for (const metric of ["recall_at_k", "p50_ms", "p95_ms"]) {
      const value = row[metric];
      if (typeof value?.median !== "number") fail(`${where}: \`${metric}\` has no median`);
      if (!(value.min <= value.median && value.median <= value.max)) {
        fail(`${where}: \`${metric}\` median ${value.median} is outside [${value.min}, ${value.max}]`);
      }
    }
    const id = rowId(row);
    if (seen.has(id)) fail(`${where}: duplicates an earlier row (${id})`);
    seen.add(id);
  }
  console.log(`ok: ${rows.length} rows in ${FILE}, each with its commit, pgvector and corpus`);
}

// ── publish ──────────────────────────────────────────────────────────────────
function publish(dir) {
  if (git("status", "--porcelain") !== "") {
    fail(
      "the working tree is dirty. The commit in a row is what says which code produced it, " +
        "so publishing from an uncommitted tree records a commit that never ran this sweep.",
    );
  }
  const commit = git("rev-parse", "HEAD");

  let files;
  try {
    files = readdirSync(dir).filter((name) => name.endsWith(".json"));
  } catch {
    fail(`no run directory at ${dir}`);
  }
  if (files.length === 0) fail(`${dir} holds no reports`);

  const reports = files.map((name) => {
    const report = JSON.parse(readFileSync(join(dir, name), "utf8"));
    if (report.benchmark !== "ten3-dense-leg") {
      fail(`${name}: not a ten3-dense-leg report (benchmark: ${report.benchmark})`);
    }
    if (!report.pgvector_version || !report.postgres_version) {
      fail(
        `${name}: the report does not say which pgvector produced it. ADR-0063's own reversal ` +
          `trigger is "the harness is re-run on a pgvector major bump", which is unenforceable ` +
          `if a row cannot say which version it holds. Re-run the sweep with a harness that ` +
          `records it — do not add it by hand here, which is the edit this script exists to refuse.`,
      );
    }
    return report;
  });

  const digests = new Set(reports.map(corpusSpecDigest));
  if (digests.size > 1) {
    fail(
      `${dir} holds reports from ${digests.size} different corpora. HNSW recall is a function ` +
        `of how much of the index the filter discards, so folding two corpus sizes into one ` +
        `row would average two different questions.`,
    );
  }

  const byArm = new Map();
  for (const report of reports) {
    const arm = {
      iterative_scan: report.tuning.iterative_scan,
      ef_search: report.tuning.ef_search,
      max_scan_tuples: report.tuning.max_scan_tuples ?? null,
      plan_cache_mode: report.plan_cache_mode,
      is_shipped_default: Boolean(report.tuning.is_shipped_default),
    };
    const key = armId(arm);
    if (!byArm.has(key)) byArm.set(key, { arm, reports: [] });
    byArm.get(key).reports.push(report);
  }

  const published = readPublished();
  const existing = new Set(published.map(rowId));
  const fresh = [];

  for (const { arm, reports: armReports } of byArm.values()) {
    if (armReports.length < MIN_RUNS) {
      fail(
        `arm ${armId(arm)} has ${armReports.length} run(s) and a published row needs ` +
          `${MIN_RUNS}. Run the sweep with REPEATS=${MIN_RUNS} rather than publishing a row ` +
          `whose spread is one graph wide.`,
      );
    }
    for (const regime of ["broad", "selective"]) {
      const measurements = armReports
        .map((report) => report.measurements.find((m) => m.regime === regime))
        .filter(Boolean);
      if (measurements.length === 0) continue;

      const empty = measurements.reduce((total, m) => total + (m.empty_slices ?? 0), 0);
      if (empty > 0) {
        fail(
          `arm ${armId(arm)}, ${regime}: ${empty} queries had a slice that admitted nothing. ` +
            `Recall over an empty slice passes because there was nothing to find — EVAL-3's ` +
            `empty-block bug, and not a thing to publish.`,
        );
      }

      const sample = armReports[0];
      const row = {
        commit,
        pgvector: sample.pgvector_version,
        postgres: sample.postgres_version,
        corpus_spec_digest: corpusSpecDigest(sample),
        corpus: sample.corpus,
        dim: sample.dim,
        k: sample.k,
        arm,
        regime,
        runs: measurements.length,
        recall_at_k: spread(measurements.map((m) => m.recall_at_k)),
        p50_ms: spread(measurements.map((m) => m.p50_ms)),
        p95_ms: spread(measurements.map((m) => m.p95_ms)),
        truth_depth: median(measurements.map((m) => m.truth_depth)),
        // Both plans, because an arm whose custom and generic plans name
        // different indexes ran both — and a row that recorded only one of
        // them is how the withdrawn table came to be believed.
        access_paths: {
          custom: accessPaths(measurements[0].plan_custom),
          generic: accessPaths(measurements[0].plan_generic),
        },
      };
      if (existing.has(rowId(row))) {
        fail(
          `arm ${armId(arm)}, ${regime} is already published at commit ${commit.slice(0, 8)} ` +
            `over corpus ${row.corpus_spec_digest}. Remove the old row deliberately if this ` +
            `sweep supersedes it.`,
        );
      }
      fresh.push(row);
    }
  }

  mkdirSync(SCORES, { recursive: true });
  writeFileSync(
    FILE,
    `${JSON.stringify({ benchmark: "ten3-dense-leg", rows: [...published, ...fresh] }, null, 2)}\n`,
  );
  console.log(`published ${fresh.length} rows from ${reports.length} runs to ${FILE}`);
}

const target = process.argv[2];
if (target) publish(target);
else check();
