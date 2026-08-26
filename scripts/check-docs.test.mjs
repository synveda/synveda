import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import {
  anchorsForMarkdown,
  checkRepository,
  codeSpanReferenceFindings,
  extractMarkdownLinks,
  isCurrentDocument,
  maskFencedBlocks,
  staleProseFindings,
  validateDocuments,
} from "./check-docs.mjs";

function catalogue(entries, extraPaths = []) {
  const documents = new Map(entries);
  return {
    documents,
    trackedPaths: new Set([...documents.keys(), ...extraPaths]),
  };
}

test("fenced examples are invisible without changing source line numbers", () => {
  const source = `before 🚀
\`\`\`md
[missing](gone.md)
\`docs/implementation/synveda-context-platform.md\`
\`\`\`
[present](guide.md)
`;
  const masked = maskFencedBlocks(source);
  assert.equal(masked.split("\n").length, source.split("\n").length);
  assert.deepEqual(extractMarkdownLinks(source), [{ target: "guide.md", line: 6 }]);
});

test("relative files, generated heading ids and explicit anchors are checked", () => {
  const fixture = catalogue([
    ["docs/index.md", "[Heading](guide.md#repeated-heading-1)\n[Explicit](guide.md#stable)\n"],
    [
      "docs/guide.md",
      "# Repeated heading\n\n# Repeated heading\n\n<a id=\"stable\"></a>\n",
    ],
  ]);
  assert.deepEqual(validateDocuments(fixture), []);
  assert.deepEqual(
    [...anchorsForMarkdown(fixture.documents.get("docs/guide.md"))].sort(),
    ["repeated-heading", "repeated-heading-1", "stable"],
  );
});

test("heading ids avoid collisions with explicit numeric-looking slugs", () => {
  assert.deepEqual(
    [...anchorsForMarkdown("# Foo\n\n# Foo-1\n\n# Foo\n")],
    ["foo", "foo-1", "foo-2"],
  );
});

test("missing relative targets and explicit anchors fail with file and line", () => {
  const fixture = catalogue([
    ["README.md", "[missing](docs/missing.md)\n[anchor](docs/guide.md#absent)\n"],
    ["docs/guide.md", "# Present\n"],
  ]);
  const findings = validateDocuments(fixture).join("\n");
  assert.match(findings, /README\.md:1: missing documentation target/u);
  assert.match(findings, /README\.md:2: missing anchor #absent/u);
});

test("unambiguous current repository paths in code spans are checked", () => {
  const trackedPaths = new Set([
    "README.md",
    "crates/example/src/lib.rs",
    "pnpm-workspace.yaml",
  ]);
  assert.deepEqual(
    codeSpanReferenceFindings({
      file: "README.md",
      source:
        "See `crates/example/src/lib.rs:10-12` and `pnpm-workspace.yaml`; run `cargo test`.",
      trackedPaths,
    }),
    [],
  );
  assert.match(
    codeSpanReferenceFindings({
      file: "README.md",
      source: "See `scripts/missing-check.mjs:7`.",
      trackedPaths,
    }).join("\n"),
    /README\.md:1: code span names no current repository path/u,
  );
  assert.match(
    codeSpanReferenceFindings({
      file: "README.md",
      source: "See `crates/example/src/lib.rs:10-12`.",
      trackedPaths,
      lineCounts: new Map([["crates/example/src/lib.rs", 9]]),
    }).join("\n"),
    /cited line 12 exceeds .* 9 lines/u,
  );
});

test("retired-ledger prose fails current docs and open briefs but not ADR history", () => {
  const source =
    "See [`docs/implementation/synveda-context-platform.md`](docs/implementation/synveda-context-platform.md).";
  const current = staleProseFindings({ file: "AGENTS.md", source });
  assert.equal(current.length, 1);
  assert.match(
    current.join("\n"),
    /retired implementation ledger/u,
  );
  assert.deepEqual(staleProseFindings({ file: "docs/adr/adr-0001.md", source }), []);
  assert.equal(staleProseFindings({ file: "docs/backlog/CPR-44.md", source }).length, 1);
  assert.equal(isCurrentDocument("README.md"), true);
  assert.equal(isCurrentDocument("docs/backlog/STATUS.md"), true);
  assert.equal(isCurrentDocument("docs/adr/README.md"), true);
  assert.equal(isCurrentDocument("docs/backlog/CPR-44.md"), true);
});

test("reference definitions are validated and external links are ignored", () => {
  const fixture = catalogue([
    [
      "docs/index.md",
      "[local][guide]\n[web](https://example.com/missing.md)\n\n[guide]: guide.md \"Guide\"\n",
    ],
    ["docs/guide.md", "# Guide\n"],
  ]);
  assert.deepEqual(validateDocuments(fixture), []);
});

test("repository checks include untracked documents and omit tracked deletions", () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-docs-"));
  try {
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    writeFileSync(join(root, "README.md"), "# Root\n");
    writeFileSync(join(root, "removed.md"), "# Removed\n");
    execFileSync("git", ["add", "README.md", "removed.md"], { cwd: root });
    unlinkSync(join(root, "removed.md"));
    writeFileSync(join(root, "CONTRIBUTING.md"), "[root](README.md)\n");

    const result = checkRepository(root);
    assert.equal(result.documents, 2);
    assert.deepEqual(result.findings, []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
