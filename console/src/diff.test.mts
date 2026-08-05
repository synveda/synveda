import assert from "node:assert/strict";
import { test } from "node:test";

import { diffLines, lines } from "./diff.mjs";

function shape(before: string | null, after: string): string {
  return diffLines(before, after)
    .map((row) => `${{ added: "+", removed: "-", same: " " }[row.mark]}${row.text}`)
    .join("\n");
}

test("an addition is an addition of every line, not a removal of nothing", () => {
  // ADR-0035's rule, and the CLI's: a reviewer admitting new content should
  // not have to read a column of absences to learn there was no old version.
  assert.equal(shape(null, "one\ntwo\n"), "+one\n+two");
});

test("an unchanged member renders as unchanged rather than as a rewrite", () => {
  assert.equal(shape("one\ntwo\n", "one\ntwo\n"), " one\n two");
});

test("one changed line changes one line", () => {
  // The property that makes a diff worth showing at all. A renderer that
  // marked the whole text removed and the whole text added would satisfy
  // "every line is named" and tell a reviewer nothing about what moved.
  assert.equal(
    shape("check the rota\nrotate the key\nfile the record\n", "check the rota\nrotate the key every 90 days\nfile the record\n"),
    " check the rota\n-rotate the key\n+rotate the key every 90 days\n file the record",
  );
});

test("a removal before an addition, so a change reads old-then-new", () => {
  const rows = diffLines("a\n", "b\n");
  assert.deepEqual(
    rows.map((row) => row.mark),
    ["removed", "added"],
  );
});

test("an insertion keeps the lines around it", () => {
  assert.equal(shape("a\nc\n", "a\nb\nc\n"), " a\n+b\n c");
});

test("a trailing newline is not an empty final line", () => {
  // Every file in the corpus ends in one; rendering a blank row at the
  // bottom of every diff would be an artefact of the format rather than
  // anything about the change.
  assert.deepEqual(lines("a\nb\n"), ["a", "b"]);
  // A genuinely blank last line is kept, because that *is* a difference.
  assert.deepEqual(lines("a\n\n"), ["a", ""]);
  assert.deepEqual(lines(""), [""]);
});

test("past the table's ceiling it degrades to removed-then-added rather than to slow", () => {
  const before = `${"x\n".repeat(2001)}`;
  const after = `${"y\n".repeat(2001)}`;
  const rows = diffLines(before, after);
  assert.equal(rows.length, 4002);
  assert.ok(rows.every((row) => row.mark !== "same"));
  // Honest rather than wrong: it is what a diff of two unrelated files
  // looks like anyway.
  assert.equal(rows[0].mark, "removed");
  assert.equal(rows[rows.length - 1].mark, "added");
});
