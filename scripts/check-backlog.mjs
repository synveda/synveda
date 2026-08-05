#!/usr/bin/env node
// Checks that docs/SYNVEDA_FEATURES.md, docs/backlog/<ID>.md and
// docs/backlog/STATUS.md agree. Writes nothing, ever.
//
// Replaces scripts/generate-backlog.mjs (removed 2026-08-05), which generated
// those files and could no longer be run: STATUS.md's per-feature narrative and
// several backlog files' hand-added `## Design` sections live only in the
// generated output, so regenerating discarded them. Its own header warned about
// STATUS.md; in practice it also silently rewrote eleven feature files. Its
// `DONE` map was a second copy of STATUS.md's record that had already drifted
// behind it (AUTHZ-5, MEM-5), so a regeneration downgraded those two entries.
//
// What was worth keeping is the other half — the assertion that the three files
// describe the same set of features. That is what this does.
//
// Phases are parsed from the Sequencing section rather than transcribed into a
// constant, so adding a feature touches SYNVEDA_FEATURES.md, its backlog file
// and STATUS.md, and nothing here.
//
// Usage: node scripts/check-backlog.mjs   (exit 0 clean, 1 with findings)

import { existsSync, readFileSync } from "node:fs";

const SRC = "docs/SYNVEDA_FEATURES.md";
const STATUS = "docs/backlog/STATUS.md";
const OUT = "docs/backlog";

const problems = [];
const fail = (message) => problems.push(message);

// ── Parse Part B ─────────────────────────────────────────────────────────────
// Identical to the removed generator's parser, so the feature set this checks
// is the feature set that was being generated.
const lines = readFileSync(SRC, "utf8").split(/\r?\n/);

const epicRe = /^EPIC ([A-Z]+) — (.+)$/;
const blockRe = /^([A-Z]+-\d+)\s+(.+)\s\((S|M|L)\)(?:\s+\[([^\]]+)\])?\s*$/;
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
    current = { id: m[1], name: m[2], epic: currentEpic.code };
    continue;
  }
  if (current && /^\s{2,}\S/.test(raw)) continue;
  if (line !== "") close();
}
close();

if (features.size === 0) fail(`${SRC}: parsed no features at all — the Part B format changed`);

// ── Parse the Sequencing section ─────────────────────────────────────────────
// "Phase 3 enterprise (wk 11–16): SKIL-1..4 · OPS-1 · TEN-3,4,5,6 · ADPT-1 (minimal)"
// continued on indented lines, ending at a prose "(...)" or "→ Demo:" line.
const phaseOf = new Map();
const seqStart = lines.findIndex((l) => l.startsWith("Sequencing"));
if (seqStart === -1) fail(`${SRC}: no "Sequencing" section found`);

function expand(token) {
  // "FND-1..6" → FND-1…FND-6; "TEN-3,4,5,6" → TEN-3…TEN-6; "OPS-1" → OPS-1
  const range = token.match(/^([A-Z]+)-(\d+)\.\.(\d+)$/);
  if (range) {
    const [, epic, from, to] = range;
    const out = [];
    for (let n = Number(from); n <= Number(to); n++) out.push(`${epic}-${n}`);
    return out;
  }
  const list = token.match(/^([A-Z]+)-([\d,]+)$/);
  if (list) return list[2].split(",").filter(Boolean).map((n) => `${list[1]}-${n}`);
  return /^[A-Z]+-\d+$/.test(token) ? [token] : [];
}

for (let i = seqStart; i < lines.length; i++) {
  const m = lines[i].match(/^Phase (\d+)[^:]*:(.*)$/);
  if (!m) continue;
  const phase = Number(m[1]);
  let text = m[2];
  for (let j = i + 1; j < lines.length; j++) {
    const next = lines[j];
    if (!/^\s+\S/.test(next)) break;
    const t = next.trim();
    if (t.startsWith("(") || t.startsWith("→") || /^Phase \d/.test(t)) break;
    text += ` ${t}`;
  }
  for (const chunk of text.split("·")) {
    // drop parentheticals like "ADPT-1 (minimal)"
    for (const token of chunk.replace(/\([^)]*\)/g, " ").trim().split(/\s+/)) {
      for (const id of expand(token)) {
        if (phaseOf.has(id) && phaseOf.get(id) !== phase) {
          fail(`Sequencing: ${id} is listed in both Phase ${phaseOf.get(id)} and Phase ${phase}`);
        }
        phaseOf.set(id, phase);
      }
    }
  }
}

if (phaseOf.size === 0) fail(`${SRC}: the Sequencing section yielded no feature IDs`);

// ── The checks ───────────────────────────────────────────────────────────────
// 1. Every ID named in Sequencing is a feature that actually parses in Part B.
for (const id of phaseOf.keys()) {
  if (!features.has(id)) fail(`Sequencing names ${id}, which does not parse as a feature in ${SRC}`);
}

// 2. Every feature has a backlog file, and 3. a STATUS.md checklist line.
const status = readFileSync(STATUS, "utf8");
for (const id of features.keys()) {
  if (!existsSync(`${OUT}/${id}.md`)) fail(`${id}: no ${OUT}/${id}.md`);
  if (!status.includes(`](${id}.md)`)) fail(`${id}: no checklist line in ${STATUS}`);
}

// 4. STATUS.md's stated count matches what parsed.
const counted = status.match(/^(\d+) features parsed from/m);
if (!counted) {
  fail(`${STATUS}: no "<n> features parsed from" line to check the count against`);
} else if (Number(counted[1]) !== features.size) {
  fail(`${STATUS}: header says ${counted[1]} features, ${SRC} parses ${features.size}`);
}

// Not a failure: a feature deliberately absent from Sequencing. STATUS.md keeps
// an "Unscheduled" section for these, so say so and move on.
const unscheduled = [...features.keys()].filter((id) => !phaseOf.has(id));

// ── Report ───────────────────────────────────────────────────────────────────
if (unscheduled.length) {
  console.log(`note: not in Sequencing, expected under Unscheduled: ${unscheduled.join(", ")}`);
}
if (problems.length) {
  for (const p of problems) console.error(`FAIL ${p}`);
  console.error(`\n${problems.length} problem(s); the backlog does not agree with ${SRC}.`);
  process.exit(1);
}
console.log(`ok: ${features.size} features across ${new Set(phaseOf.values()).size} phases — ${SRC}, ${OUT}/<ID>.md and ${STATUS} agree.`);
