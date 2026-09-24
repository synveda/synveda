/** CPR-14 placement settings: a checkout may name both governed subtypes. */

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

import { loadConfig } from "./config.mjs";

const savedWorkspace = process.env.SYNVEDA_WORKSPACE;
const savedProject = process.env.SYNVEDA_PROJECT;
const savedRepository = process.env.SYNVEDA_REPOSITORY;
const savedProfile = process.env.SYNVEDA_PROFILE;
const savedConfig = process.env.XDG_CONFIG_HOME;

afterEach(() => {
  if (savedWorkspace === undefined) delete process.env.SYNVEDA_WORKSPACE;
  else process.env.SYNVEDA_WORKSPACE = savedWorkspace;
  if (savedProject === undefined) delete process.env.SYNVEDA_PROJECT;
  else process.env.SYNVEDA_PROJECT = savedProject;
  if (savedRepository === undefined) delete process.env.SYNVEDA_REPOSITORY;
  else process.env.SYNVEDA_REPOSITORY = savedRepository;
  if (savedProfile === undefined) delete process.env.SYNVEDA_PROFILE;
  else process.env.SYNVEDA_PROFILE = savedProfile;
  if (savedConfig === undefined) delete process.env.XDG_CONFIG_HOME;
  else process.env.XDG_CONFIG_HOME = savedConfig;
});

test("managed observation requires matching private local consent and profile", () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "synveda-private-consent-")));
  try {
    delete process.env.SYNVEDA_WORKSPACE;
    delete process.env.SYNVEDA_PROJECT;
    process.env.SYNVEDA_PROFILE = "chosen";
    process.env.XDG_CONFIG_HOME = join(root, "private");
    mkdirSync(join(root, ".git"));
    mkdirSync(join(root, ".synveda"));
    const config = join(root, ".synveda/config.json");
    writeFileSync(config, JSON.stringify({ managed_observation: true, observe: true,
      workspace_id: "workspace", project_id: "project" }));
    // Repository bytes alone cannot turn recording on.
    assert.equal(loadConfig(root).observe, false);
    const receipts = join(root, "private/synveda/consumer");
    mkdirSync(receipts, { recursive: true });
    const receipt = join(receipts, `setup-${createHash("sha256").update(root).digest("hex")}.json`);
    const value = { version: 1, root, selection: { profile: "chosen", workspace: "workspace",
      project: "project", observation: "on" }, previous: {} };
    writeFileSync(receipt, JSON.stringify(value), { mode: 0o600 });
    assert.equal(loadConfig(root).observe, true);
    process.env.SYNVEDA_PROFILE = "different";
    assert.equal(loadConfig(root).observe, false);
    process.env.SYNVEDA_PROFILE = "chosen";
    process.env.SYNVEDA_PROJECT = "different";
    assert.equal(loadConfig(root).observe, false);
    delete process.env.SYNVEDA_PROJECT;
    writeFileSync(receipt, JSON.stringify({ ...value, selection: { ...value.selection, observation: "off" } }));
    assert.equal(loadConfig(root).observe, false);
    writeFileSync(receipt, JSON.stringify(value));
    writeFileSync(config, "invalid");
    assert.equal(loadConfig(root).observe, false);
    rmSync(config);
    assert.equal(loadConfig(root).observe, false);
    writeFileSync(receipt, "invalid");
    assert.equal(loadConfig(root).observe, false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("a project file can bind a checkout to a workspace and project", () => {
  delete process.env.SYNVEDA_WORKSPACE;
  delete process.env.SYNVEDA_PROJECT;
  delete process.env.SYNVEDA_REPOSITORY;
  const root = mkdtempSync(join(tmpdir(), "synveda-config-"));
  mkdirSync(join(root, ".synveda"));
  writeFileSync(
    join(root, ".synveda/config.json"),
    JSON.stringify({
      workspace_id: "11111111-1111-1111-1111-111111111111",
      project_id: "22222222-2222-2222-2222-222222222222",
      repository_id: "33333333-3333-3333-3333-333333333333",
    }),
  );
  try {
    const config = loadConfig(root);
    assert.equal(config.workspaceId, "11111111-1111-1111-1111-111111111111");
    assert.equal(config.projectId, "22222222-2222-2222-2222-222222222222");
    assert.equal(config.repositoryId, "33333333-3333-3333-3333-333333333333");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("explicit placement environment wins over the checkout", () => {
  process.env.SYNVEDA_WORKSPACE = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
  process.env.SYNVEDA_PROJECT = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
  process.env.SYNVEDA_REPOSITORY = "cccccccc-cccc-cccc-cccc-cccccccccccc";
  const config = loadConfig(undefined);
  assert.equal(config.workspaceId, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
  assert.equal(config.projectId, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
  assert.equal(config.repositoryId, "cccccccc-cccc-cccc-cccc-cccccccccccc");
});

test("nested hook working directories retain the Git root's observation consent", () => {
  delete process.env.SYNVEDA_WORKSPACE;
  delete process.env.SYNVEDA_PROJECT;
  const root = mkdtempSync(join(tmpdir(), "synveda-config-consent-"));
  try {
    mkdirSync(join(root, ".synveda"));
    mkdirSync(join(root, "nested", "deep"), { recursive: true });
    // A worktree's .git is a file, not necessarily a directory.
    writeFileSync(join(root, ".git"), "gitdir: /unused/worktree\n");
    writeFileSync(join(root, ".synveda/config.json"), JSON.stringify({
      observe: false, workspace_id: "workspace", project_id: "project",
    }));
    const config = loadConfig(join(root, "nested", "deep"));
    assert.equal(config.observe, false);
    assert.equal(config.workspaceId, "workspace");
    assert.equal(config.projectId, "project");
    // A nested repository is its own project; outer consent cannot select it.
    mkdirSync(join(root, "nested", ".git"));
    assert.equal(loadConfig(join(root, "nested", "deep")).projectId, undefined);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
