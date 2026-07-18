#!/usr/bin/env node
// Generates docs/backlog/<ID>.md (one file per feature) and docs/backlog/STATUS.md
// from Part B of docs/SYNVEDA_FEATURES.md.
//
// Phases are transcribed from the "Sequencing (features → phases)" section below;
// the script fails if the transcription and the parsed features drift apart, and
// labels any feature absent from Sequencing as phase:unscheduled.
//
// Usage: node scripts/generate-backlog.mjs

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const SRC = "docs/SYNVEDA_FEATURES.md";
const OUT = "docs/backlog";

// ── Transcribed from the Sequencing section ─────────────────────────────────
const PHASES = [
  {
    n: 0,
    title: "Phase 0 — Foundation (wk 1)",
    demo: null,
    ids: ["FND-1", "FND-2", "FND-3", "FND-4", "FND-5", "FND-6"],
  },
  {
    n: 1,
    title: "Phase 1 — The spine (wk 2–5)",
    demo:
      "SSO login → auto-scoped → live Claude Code session writes and receives governed memory, fully audited.",
    ids: [
      "TEN-1", "TEN-2",
      "AUTH-1", "AUTH-2", "AUTH-3",
      "AUTHZ-1", "AUTHZ-2", "AUTHZ-3",
      "HIER-1", "HIER-2", "HIER-3",
      "MEM-1", "MEM-2", "MEM-3", "MEM-4",
      "CTX-1", "CTX-2", "CTX-3",
      "AUD-1", "ADPT-1", "EVAL-1",
    ],
  },
  {
    n: 2,
    title: "Phase 2 — Governance (wk 6–10)",
    demo: "promotion pipeline, lapse lifecycle, as-of inject, bank-mode switch.",
    ids: [
      "FLOW-1", "FLOW-2", "FLOW-3", "FLOW-4", "FLOW-5", "FLOW-6", "FLOW-7",
      "AUTHZ-4", "AUTHZ-5",
      "MEM-5", "MEM-6",
      "CTX-4", "CTX-5",
      "GRPH-1", "GRPH-2", "GRPH-4",
      "AUD-2",
      "EVAL-2", "EVAL-4", "EVAL-5",
      "PRMT-1", "PRMT-2",
    ],
  },
  {
    n: 3,
    title: "Phase 3 — Enterprise (wk 11–16)",
    demo:
      "Entra/Okta live, spec-compliant governed skills into Claude Code + Cursor, LoCoMo/LongMemEval scores published, Helm install.",
    ids: [
      "AUTH-4", "AUTH-5",
      "TEN-3", "TEN-4", "TEN-5", "TEN-6",
      "SKIL-1", "SKIL-2", "SKIL-3", "SKIL-4",
      "GRPH-3",
      "AUD-3", "AUD-4",
      "EVAL-3", "EVAL-6",
      "OPS-1", "OPS-2", "OPS-3", "OPS-4",
      "CNSL-1", "CNSL-2",
      "ADPT-2", "ADPT-3",
      "CTX-6", "FLOW-8",
    ],
  },
  {
    n: 4,
    title: "Phase 4 — Ecosystem",
    demo: null,
    ids: [
      "ADPT-4", "ADPT-5",
      "PRMT-3", "SKIL-5", "MEM-7",
      "OPS-5", "OPS-6",
      "CNSL-3", "CNSL-4",
      "AUD-5", "AUTHZ-6",
    ],
  },
];

// Features already delivered (kept checked in STATUS.md across regenerations).
const DONE = new Map([
  ["FND-1", "done 2026-07-16, demo: demos/fnd-1-scaffold.sh"],
  ["FND-2", "done 2026-07-17, demo: demos/fnd-2-dev-env.sh"],
  ["FND-3", "done 2026-07-18, AC test: crates/synveda-types/tests/serde_roundtrip.rs"],
  ["FND-4", "done 2026-07-18, AC test: crates/synveda-store/tests/bitemporal.rs, demo: demos/fnd-4-bitemporal.sh"],
  ["FND-5", "done 2026-07-18, AC test: crates/synveda-gateway/tests/observability.rs, demo: demos/fnd-5-observability.sh"],
  ["FND-6", "done 2026-07-18, demo: demos/fnd-6-adrs.sh (adr-0001..0004 in docs/adr/)"],
  ["TEN-1", "done 2026-07-18, AC test: crates/synveda-gateway/tests/tenant_resolution.rs, demo: demos/ten-1-tenant-resolution.sh"],
  ["TEN-2", "done 2026-07-18, AC test: crates/synveda-store/tests/rls.rs, demo: demos/ten-2-rls.sh"],
]);

// Phase-level notes appended after a phase's checklist (kept across
// regenerations, like DONE).
const PHASE_NOTES = new Map([
  [
    0,
    "_Phase 0 complete: exit gate `make dev-up && make smoke` passed 2026-07-18\n" +
      "(all services healthy incl. AGE/PGMQ/pgvector, Rauthy, Temporal, TEI BGE-M3,\n" +
      "Jaeger). Phase 1 may start._",
  ],
  [
    1,
    "_TEN-1 deferral (ADR-0008): tenant-resolution decisions are an audit\n" +
      "emission point; events are wired when AUD-1's hash-chained log lands. Until\n" +
      "then they are visible in traces and `synveda_tenant_resolutions_total` only._\n" +
      "\n" +
      "_TEN-2 deferrals (ADR-0009): RLS-backstop trips (SQLSTATE 42501 →\n" +
      "`Error::Internal`) are an AUD-1 emission point; data-path features must\n" +
      "reach tenant-scoped tables via `synveda_store::rls::begin_tenant_tx`, and\n" +
      "deployment profiles (OPS-1/OPS-2) must connect as a non-superuser\n" +
      "`synveda_app` login — the dev compose superuser bypasses RLS._",
  ],
]);

// ── Parse Part B ─────────────────────────────────────────────────────────────
const lines = readFileSync(SRC, "utf8").split(/\r?\n/);

const epicRe = /^EPIC ([A-Z]+) — (.+)$/;
// "FND-1  Workspace scaffold (S)" / "TEN-6  Cross-tenant isolation test harness (M) [continuous]"
const blockRe = /^([A-Z]+-\d+)\s+(.+)\s\((S|M|L)\)(?:\s+\[([^\]]+)\])?\s*$/;
// "CNSL-2 Hierarchy & policy explorer (M) — visualise scopes, packs, roles, active lapses."
const inlineRe = /^([A-Z]+-\d+)\s+(.+?)\s\((S|M|L)\)\s+—\s+(.*)$/;

const features = new Map();
let currentEpic = null;
let current = null;

function close() {
  if (current) {
    features.set(current.id, current);
    current = null;
  }
}

for (const raw of lines) {
  const line = raw.trimEnd();
  if (/^─+$/.test(line) || line.startsWith("Sequencing")) {
    close();
    if (line.startsWith("Sequencing")) currentEpic = null;
    continue;
  }
  const epicM = line.match(epicRe);
  if (epicM) {
    close();
    currentEpic = { code: epicM[1], title: epicM[2] };
    continue;
  }
  if (!currentEpic) continue;
  const inlineM = line.match(inlineRe);
  const blockM = inlineM ? null : line.match(blockRe);
  if (inlineM || blockM) {
    close();
    const m = inlineM ?? blockM;
    current = {
      id: m[1],
      name: m[2],
      size: m[3],
      marker: inlineM ? null : (m[4] ?? null),
      epic: currentEpic.code,
      epicTitle: currentEpic.title,
      body: inlineM ? [m[4]] : [],
    };
    continue;
  }
  if (current && /^\s{2,}\S/.test(raw)) {
    current.body.push(line.trim());
    continue;
  }
  if (line !== "") close();
}
close();

// ── Validate against the phase transcription ────────────────────────────────
const phaseOf = new Map();
for (const p of PHASES) {
  for (const id of p.ids) {
    if (phaseOf.has(id)) throw new Error(`duplicate in phase map: ${id}`);
    phaseOf.set(id, p.n);
  }
}
const missing = [...phaseOf.keys()].filter((id) => !features.has(id));
if (missing.length) {
  throw new Error(`Sequencing transcription lists unparsed features: ${missing.join(", ")}`);
}
const unscheduled = [...features.keys()].filter((id) => !phaseOf.has(id));
if (unscheduled.length) {
  console.warn(`WARN: not in Sequencing, labelled phase:unscheduled: ${unscheduled.join(", ")}`);
}

// ── Emit one file per feature ────────────────────────────────────────────────
mkdirSync(OUT, { recursive: true });

function splitBody(bodyLines) {
  const body = bodyLines.join(" ").replace(/\s+/g, " ").trim();
  const i = body.search(/\bAC:\s/);
  if (i === -1) return { desc: body, ac: null };
  return {
    desc: body.slice(0, i).trim(),
    ac: body.slice(i).replace(/^AC:\s*/, "").trim(),
  };
}

for (const f of features.values()) {
  const { desc, ac } = splitBody(f.body);
  const phase = phaseOf.get(f.id);
  const phaseLabel = phase === undefined ? "unscheduled" : String(phase);
  const out = [
    "---",
    `title: ${JSON.stringify(`${f.id}: ${f.name}`)}`,
    "labels:",
    `  - epic:${f.epic}`,
    `  - phase:${phaseLabel}`,
    `size: ${f.size}`,
    ...(f.marker ? [`marker: ${JSON.stringify(f.marker)}`] : []),
    "---",
    "",
    `# ${f.id}: ${f.name}`,
    "",
    `**Epic:** ${f.epic} — ${f.epicTitle} · **Phase:** ${phaseLabel} · **Size:** ${f.size}` +
      (f.marker ? ` · **Marker:** ${f.marker}` : ""),
    "",
    "## Description",
    "",
    desc,
    "",
    "## Acceptance criteria",
    "",
    ac ?? "_No acceptance criteria specified in SYNVEDA_FEATURES.md._",
    "",
  ];
  writeFileSync(path.join(OUT, `${f.id}.md`), out.join("\n"));
}

// ── STATUS.md checklist ──────────────────────────────────────────────────────
const status = [
  "# Backlog status",
  "",
  `${features.size} features parsed from docs/SYNVEDA_FEATURES.md — one file per`,
  "feature in this directory. Phases per the Sequencing section. Regenerate with",
  "`node scripts/generate-backlog.mjs` (preserves done-marks listed in the script).",
  "",
  "Phase 1+ must not start until FND is complete and `make dev-up && make smoke`",
  "passes (CLAUDE.md, current phase).",
  "",
];
for (const p of PHASES) {
  status.push(`## ${p.title}`, "");
  if (p.demo) status.push(`_Phase demo goal: ${p.demo}_`, "");
  for (const id of p.ids) {
    const f = features.get(id);
    const done = DONE.get(id);
    status.push(
      `- [${done ? "x" : " "}] [${id}: ${f.name}](${id}.md)${done ? ` — ${done}` : ""}`,
    );
  }
  status.push("");
  const note = PHASE_NOTES.get(p.n);
  if (note) status.push(note, "");
}
if (unscheduled.length) {
  status.push("## Unscheduled — not listed in the Sequencing section", "");
  for (const id of unscheduled) {
    status.push(`- [ ] [${id}: ${features.get(id).name}](${id}.md)`);
  }
  status.push("");
}
writeFileSync(path.join(OUT, "STATUS.md"), status.join("\n"));

console.log(`wrote ${features.size} feature files + STATUS.md to ${OUT}/`);
