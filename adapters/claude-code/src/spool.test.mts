/**
 * The session spool (ADR-0027 decision 7) and the one-shot project
 * disclosure of decision 13.
 */

import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, utimesSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

process.env.XDG_STATE_HOME = mkdtempSync(join(tmpdir(), "synveda-state-"));

const { claimDisclosure, loadSession, prune, saveSession } = await import("./spool.mjs");
const { sessionDir } = await import("./paths.mjs");

test("a saved cursor round trips", () => {
  saveSession("claude-code:s1", { transcript_path: "/tmp/s1.jsonl", cursor: "u42" });
  const state = loadSession("claude-code:s1");
  assert.equal(state?.session_id, "claude-code:s1");
  assert.equal(state?.transcript_path, "/tmp/s1.jsonl");
  assert.equal(state?.cursor, "u42");
});

test("an unknown session has no state, and that is not an error", () => {
  assert.equal(loadSession("claude-code:never-seen"), undefined);
});

test("a session may have a transcript path before it has a cursor", () => {
  saveSession("claude-code:s2", { transcript_path: "/tmp/s2.jsonl" });
  const state = loadSession("claude-code:s2");
  assert.equal(state?.transcript_path, "/tmp/s2.jsonl");
  assert.equal(state?.cursor, undefined);
});

test("ids that sanitise alike do not share a cursor", () => {
  saveSession("claude-code:a/b", { transcript_path: "/tmp/a.jsonl", cursor: "cursor-a" });
  saveSession("claude-code:a:b", { transcript_path: "/tmp/b.jsonl", cursor: "cursor-b" });
  assert.equal(loadSession("claude-code:a/b")?.cursor, "cursor-a");
  assert.equal(loadSession("claude-code:a:b")?.cursor, "cursor-b");
});

test("disclosure is claimed exactly once per project", () => {
  const project = mkdtempSync(join(tmpdir(), "synveda-project-"));
  assert.equal(claimDisclosure(project), true);
  assert.equal(claimDisclosure(project), false);
  assert.equal(claimDisclosure(project), false);
});

test("a project with no working directory discloses nothing", () => {
  assert.equal(claimDisclosure(undefined), false);
});

test("prune drops state no one will resume and keeps the rest", () => {
  saveSession("claude-code:old", { transcript_path: "/tmp/old.jsonl", cursor: "u1" });
  saveSession("claude-code:fresh", { transcript_path: "/tmp/fresh.jsonl", cursor: "u1" });
  const stale = readdirSync(sessionDir()).find((name) => name.startsWith("claude-code_old"));
  assert.ok(stale);
  const ancient = new Date(Date.now() - 60 * 24 * 60 * 60 * 1000);
  utimesSync(join(sessionDir(), stale), ancient, ancient);

  prune();

  assert.equal(loadSession("claude-code:old"), undefined);
  assert.equal(loadSession("claude-code:fresh")?.cursor, "u1");
});
