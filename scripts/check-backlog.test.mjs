import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";

const checker = resolve(import.meta.dirname, "check-backlog.mjs");
const sections = [
  "Problem and evidence",
  "Scope",
  "Non-goals",
  "Architecture seam",
  "Acceptance criteria",
  "Required tests",
  "Rollout and rollback",
  "Dependencies",
];

function run(status, brief = null) {
  const root = mkdtempSync(join(tmpdir(), "synveda-backlog-"));
  try {
    mkdirSync(join(root, "docs/backlog"), { recursive: true });
    writeFileSync(join(root, "docs/backlog/STATUS.md"), status);
    if (brief !== null) writeFileSync(join(root, "docs/backlog/TEST-1.md"), brief);
    return execFileSync(process.execPath, [checker], {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

function openStatus(check = " ") {
  return `# Feature inventory

1 features in this index.

0 delivered; 1 open.

## Phase 0

- [${check}] [TEST-1: Test feature](TEST-1.md) — open
`;
}

function brief(body = "Current evidence.") {
  return `# TEST-1: Test feature

${sections.map((section) => `## ${section}\n\n${body}`).join("\n\n")}
`;
}

test("accepts one canonical open brief", () => {
  assert.match(run(openStatus(), brief()), /1 features/u);
});

test("rejects non-canonical checklist markers", () => {
  assert.throws(() => run(openStatus("X"), brief()), /Command failed/u);
});

test("rejects an empty or fenced-only required section", () => {
  const source = brief().replace(
    "## Scope\n\nCurrent evidence.",
    "## Scope\n\n```md\ncontent hidden in a fence\n```",
  );
  assert.throws(() => run(openStatus(), source), /Command failed/u);
});
