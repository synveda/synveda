#!/usr/bin/env node
// STATUS is the concise feature inventory. Delivered implementation history
// stays in git; only open features retain mutable implementation briefs.

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { maskFencedBlocks } from "./check-docs.mjs";

const STATUS = "docs/backlog/STATUS.md";
const BACKLOG = "docs/backlog";
const REQUIRED_SECTIONS = [
  "Problem and evidence",
  "Scope",
  "Non-goals",
  "Architecture seam",
  "Acceptance criteria",
  "Required tests",
  "Rollout and rollback",
  "Dependencies",
];

const problems = [];
const fail = (message) => problems.push(message);
const status = readFileSync(STATUS, "utf8");

const deliveredPattern =
  /^- \[x\] ([A-Z]+-\d+): (.+?) — delivered(?:.*)$/gm;
const openPattern =
  /^- \[ \] \[([A-Z]+-\d+): ([^\]\n]+)\]\(([^)\n]+)\) — open$/gm;

const entries = [];
for (const match of status.matchAll(deliveredPattern)) {
  entries.push({ id: match[1], title: match[2], delivered: true, target: null });
}
for (const match of status.matchAll(openPattern)) {
  entries.push({ id: match[1], title: match[2], delivered: false, target: match[3] });
}

const parsedLines = new Set([
  ...status.matchAll(deliveredPattern),
  ...status.matchAll(openPattern),
].map((match) => match[0]));
for (const line of status.match(/^- \[[^\]\n]*\] .+$/gm) ?? []) {
  if (!parsedLines.has(line)) {
    fail(`${STATUS}: malformed checklist line: ${line}`);
  }
}

const byId = new Map();
for (const entry of entries) {
  if (byId.has(entry.id)) {
    fail(`${STATUS}: duplicate feature ${entry.id}`);
  } else {
    byId.set(entry.id, entry);
  }
}

const openIds = new Set();
for (const entry of entries) {
  const expected = `${entry.id}.md`;
  const path = join(BACKLOG, expected);
  if (entry.delivered) {
    if (existsSync(path)) {
      fail(`${path}: delivered planning prose must be removed; git is the archive`);
    }
    continue;
  }

  openIds.add(entry.id);
  if (entry.target !== expected) {
    fail(`${STATUS}: ${entry.id} links to ${entry.target}, expected ${expected}`);
  }
  if (!existsSync(path)) {
    fail(`${entry.id}: no open-feature brief at ${path}`);
    continue;
  }

  const source = readFileSync(path, "utf8");
  const visible = maskFencedBlocks(source);
  const heading = source.match(/^# ([A-Z]+-\d+): (.+)$/m);
  if (!heading || heading[1] !== entry.id || heading[2] !== entry.title) {
    fail(`${path}: H1 must be exactly "# ${entry.id}: ${entry.title}"`);
  }
  const sectionMatches = [...visible.matchAll(/^## (.+)$/gm)];
  const sections = sectionMatches.map((match) => match[1]);
  if (sections.join("\n") !== REQUIRED_SECTIONS.join("\n")) {
    fail(
      `${path}: H2 sections must be exactly, in order: ${REQUIRED_SECTIONS.join("; ")}`,
    );
  }
  for (const [index, match] of sectionMatches.entries()) {
    const bodyStart = match.index + match[0].length;
    const bodyEnd = sectionMatches[index + 1]?.index ?? visible.length;
    if (visible.slice(bodyStart, bodyEnd).trim() === "") {
      fail(`${path}: "## ${match[1]}" has no content`);
    }
  }
}

for (const name of readdirSync(BACKLOG)) {
  const match = name.match(/^([A-Z]+-\d+)\.md$/);
  if (match && !openIds.has(match[1])) {
    fail(`${BACKLOG}/${name}: no matching open STATUS entry`);
  }
}

const totalClaim = status.match(/^(\d+) features in this index\./m);
if (!totalClaim || Number(totalClaim[1]) !== entries.length) {
  fail(`${STATUS}: header must report ${entries.length} features in this index`);
}
const delivered = entries.filter((entry) => entry.delivered).length;
const open = entries.length - delivered;
const summary = status.match(/^(\d+) delivered; (\d+) open\./m);
if (!summary || Number(summary[1]) !== delivered || Number(summary[2]) !== open) {
  fail(`${STATUS}: summary must report ${delivered} delivered; ${open} open`);
}

if (problems.length > 0) {
  for (const problem of problems) {
    console.error(`FAIL ${problem}`);
  }
  console.error(`\n${problems.length} feature-inventory problem(s).`);
  process.exit(1);
}

const phases = new Set(
  [...status.matchAll(/^## Phase (\d+)\b/gm)].map((match) => match[1]),
);
console.log(
  `ok: ${entries.length} features across ${phases.size} phases — ${delivered} delivered in git, ${open} current briefs`,
);
