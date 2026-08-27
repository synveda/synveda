/** CPR-14 placement settings: a checkout may name both governed subtypes. */

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

import { loadConfig } from "./config.mjs";

const savedWorkspace = process.env.SYNVEDA_WORKSPACE;
const savedProject = process.env.SYNVEDA_PROJECT;

afterEach(() => {
  if (savedWorkspace === undefined) delete process.env.SYNVEDA_WORKSPACE;
  else process.env.SYNVEDA_WORKSPACE = savedWorkspace;
  if (savedProject === undefined) delete process.env.SYNVEDA_PROJECT;
  else process.env.SYNVEDA_PROJECT = savedProject;
});

test("a project file can bind a checkout to a workspace and project", () => {
  delete process.env.SYNVEDA_WORKSPACE;
  delete process.env.SYNVEDA_PROJECT;
  const root = mkdtempSync(join(tmpdir(), "synveda-config-"));
  mkdirSync(join(root, ".synveda"));
  writeFileSync(
    join(root, ".synveda/config.json"),
    JSON.stringify({
      workspace_id: "11111111-1111-1111-1111-111111111111",
      project_id: "22222222-2222-2222-2222-222222222222",
    }),
  );
  try {
    const config = loadConfig(root);
    assert.equal(config.workspaceId, "11111111-1111-1111-1111-111111111111");
    assert.equal(config.projectId, "22222222-2222-2222-2222-222222222222");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("explicit placement environment wins over the checkout", () => {
  process.env.SYNVEDA_WORKSPACE = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
  process.env.SYNVEDA_PROJECT = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
  const config = loadConfig(undefined);
  assert.equal(config.workspaceId, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
  assert.equal(config.projectId, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
});
