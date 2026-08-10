#!/usr/bin/env node
// TEN-3 — folds the sweep's per-run reports into the table ADR-0063 reads.
//
//   node scripts/summarise-ann-bench.mjs [run-dir]    (default /tmp/ten3-runs)
//
// It exists because repeats do. ADR-0063's first measurements table is
// n=1 in every row and says so in its own last paragraph: "repeats, and a
// max_scan_tuples axis, come before the gate in decision 3 is applied to
// anything." Folding fifteen terminal stanzas by eye is how that table
// came to be hand-transcribed in the first place, and a hand-transcribed
// table is the one thing `make check-benchmarks` exists to refuse
// everywhere else in this repository.
//
// It reports a **median and a spread**, never a mean. With n=3 a mean is
// one unlucky graph away from moving a row, and the spread is what the
// gate is actually read against: two arms whose spreads overlap have not
// been separated by this corpus, whatever their medians say.
//
// The access-path section is not decoration. TEN-3's first sweep measured
// an arm whose plan changed part-way through the run, and the numbers
// looked ordinary the whole time. An arm whose first execution and sixth
// name different indexes has measured two things and averaged them, so
// both are printed and any difference is the headline.
//
// It does **not** publish. `scripts/publish-benchmark.mjs` puts a number
// in a marketing artefact and earns that with five refusals and a
// clean-tree check; this prints a table into a terminal for somebody
// writing an ADR. TEN-3's recorded rows — corpus digest, pgvector
// version, commit — are a separate, deliberate act for the reason
// ADR-0061 decision 11 already gives.
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2] ?? "/tmp/ten3-runs";

let files;
try {
  files = readdirSync(dir).filter((name) => name.endsWith(".json"));
} catch {
  console.error(`no run directory at ${dir} — run demos/ten-3-dense-leg-sweep.sh first`);
  process.exit(1);
}
if (files.length === 0) {
  console.error(`${dir} holds no reports — run demos/ten-3-dense-leg-sweep.sh first`);
  process.exit(1);
}

const runs = files.map((name) => {
  const report = JSON.parse(readFileSync(join(dir, name), "utf8"));
  // A stray report from another harness would average into a row and move
  // a conclusion. Name it and stop, rather than skipping it quietly.
  if (report.benchmark !== "ten3-dense-leg") {
    console.error(`${name}: not a ten3-dense-leg report (benchmark: ${report.benchmark})`);
    process.exit(1);
  }
  return report;
});

/// Reports are grouped by corpus before anything else. Two sweeps at
/// different sizes in one directory is a normal thing to do and averaging
/// across them would be nonsense — HNSW recall is a function of how much
/// of the index the filter throws away.
const corpusLabel = (c) =>
  `${c.records} records / ${c.tenants} tenants / ${c.scopes_per_tenant} scopes / ${c.queries} queries`;

const bound = (tuning) => tuning.max_scan_tuples ?? "default";

/// `plan_cache_mode` sits beside the tuning rather than inside it, because
/// the product does not set it. It is part of the arm's identity all the
/// same: two runs differing only here can be using different indexes,
/// which is what TEN-3's first sweep found out the hard way.
const planCache = (r) => r.plan_cache_mode ?? "auto";

const armLabel = (r) =>
  `${r.tuning.iterative_scan} · ef ${r.tuning.ef_search} · bound ${bound(r.tuning)} · ${planCache(r)}`;
const armKey = (r) =>
  `${r.tuning.iterative_scan}|${r.tuning.ef_search}|${bound(r.tuning)}|${planCache(r)}`;

/// A stable reading order: what the scan does, then how wide, then how
/// far it may go. Filenames sort alphabetically, which would put ef 1000
/// above ef 100.
const SCAN_ORDER = { off: 0, relaxed_order: 1, strict_order: 2 };
const PLAN_ORDER = { auto: 0, force_custom_plan: 1, force_generic_plan: 2 };
const armRank = (r) => [
  SCAN_ORDER[r.tuning.iterative_scan] ?? 9,
  r.tuning.ef_search,
  r.tuning.max_scan_tuples ?? 0, // `default` is the arm everything else is read against
  PLAN_ORDER[planCache(r)] ?? 9,
];

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

/// The access-path nodes a plan names. A fact the plan states — what it
/// means for partitioning is the ADR's to say, not this script's.
const NODE = /(Seq Scan on|Index Scan using|Index Only Scan using|Bitmap Index Scan on) ([a-z0-9_]+)/g;
function accessPaths(plan) {
  return [...(plan ?? "").matchAll(NODE)].map(([, node, name]) => `${node} ${name}`);
}

/// Older reports carry a single `plan`; current ones carry the plan built
/// with this query's parameters and the one built without them.
const planPair = (m) => [
  { when: "custom", plan: m.plan_custom ?? m.plan },
  { when: "generic", plan: m.plan_generic ?? m.plan },
];

const byCorpus = new Map();
for (const report of runs) {
  const key = corpusLabel(report.corpus);
  if (!byCorpus.has(key)) byCorpus.set(key, []);
  byCorpus.get(key).push(report);
}

for (const [corpus, reports] of [...byCorpus].sort()) {
  const sample = reports[0];
  console.log(`## ${corpus} — dim ${sample.dim}, k=${sample.k}\n`);

  const byArm = new Map();
  for (const report of reports) {
    const key = armKey(report);
    if (!byArm.has(key)) byArm.set(key, { sample: report, reports: [] });
    byArm.get(key).reports.push(report);
  }
  const arms = [...byArm.values()].sort((a, b) => {
    const [x, y] = [armRank(a.sample), armRank(b.sample)];
    return x[0] - y[0] || x[1] - y[1] || x[2] - y[2] || x[3] - y[3];
  });

  for (const regime of ["broad", "selective"]) {
    console.log(`### ${regime}\n`);
    console.log(
      "| iterative_scan | ef_search | max_scan_tuples | plan_cache | n | recall@k | recall spread | p50 | p95 | depth |",
    );
    console.log("|---|---|---|---|---|---|---|---|---|---|");
    for (const arm of arms) {
      const rows = arm.reports
        .map((r) => r.measurements.find((m) => m.regime === regime))
        .filter(Boolean);
      if (rows.length === 0) continue;
      const recalls = rows.map((m) => m.recall_at_k);
      const shipped = arm.sample.tuning.is_shipped_default ? " *(shipped)*" : "";
      console.log(
        `| ${arm.sample.tuning.iterative_scan} | ${arm.sample.tuning.ef_search}${shipped} ` +
          `| ${bound(arm.sample.tuning)} | ${planCache(arm.sample)} | ` +
          `${rows.length} | ${median(recalls).toFixed(3)} | ` +
          `${Math.min(...recalls).toFixed(3)}–${Math.max(...recalls).toFixed(3)} | ` +
          `${median(rows.map((m) => m.p50_ms)).toFixed(2)}ms | ` +
          `${median(rows.map((m) => m.p95_ms)).toFixed(2)}ms | ` +
          `${median(rows.map((m) => m.truth_depth))} |`,
      );
    }
    console.log("");
  }

  console.log("### access paths, as the plans state them\n");
  console.log(
    "Where the custom and generic plans name different indexes, an arm running under\n" +
      "`plan_cache_mode = auto` used both — the custom one for five executions per pooled\n" +
      "connection, then mostly the other. Its row is an average of two different queries.\n",
  );
  for (const regime of ["broad", "selective"]) {
    for (const arm of arms) {
      const seen = new Map();
      for (const report of arm.reports) {
        const measurement = report.measurements.find((m) => m.regime === regime);
        if (!measurement) continue;
        for (const { when, plan } of planPair(measurement)) {
          if (!seen.has(when)) seen.set(when, new Set());
          for (const path of accessPaths(plan)) seen.get(when).add(path);
        }
      }
      const sets = [...seen.values()].map((paths) => [...paths].sort().join(", "));
      const rendered = [...seen]
        .map(([when, paths]) => `${when}: ${[...paths].sort().join(", ") || "—"}`)
        .join("  ||  ");
      const flipped = sets.length === 2 && sets[0] !== sets[1];
      const flag = flipped ? " **[custom and generic differ]**" : "";
      console.log(`- ${regime} · ${armLabel(arm.sample)}${flag}: ${rendered}`);
    }
  }
  console.log("");
}
