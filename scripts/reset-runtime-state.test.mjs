import assert from "node:assert/strict";
import { chmod, lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  applyResetState,
  resetStatePlan,
} from "../deploy/compose/scripts/reset-runtime-state.mjs";

const project = "synveda-development-acceptance-reset";

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "synveda-reset-state-"));
  const runtime = join(root, project);
  const authorityDir = join(runtime, "database-authority");
  const gateDir = join(runtime, "keycloak-public-gate");
  await mkdir(authorityDir, { recursive: true, mode: 0o700 });
  await mkdir(gateDir, { mode: 0o700 });
  for (const directory of [authorityDir, gateDir]) {
    const marker = join(directory, ".synveda-private-directory");
    await writeFile(marker, `project:${project}\n`, { mode: 0o600 });
    await chmod(marker, 0o600);
  }
  const witness = join(authorityDir, "keycloak-cluster.json");
  await writeFile(witness, "{}\n", { mode: 0o600 });
  const generationName = ".generation-AbCdEf012345";
  const generation = join(gateDir, generationName);
  await mkdir(generation, { mode: 0o700 });
  const ready = join(generation, "cpr45-keycloak-realm-v3.ready");
  await writeFile(ready, "cpr45-keycloak-realm-v3\n", { mode: 0o400 });
  await chmod(ready, 0o400);
  await symlink(generationName, join(gateDir, "current"));
  return { root, authorityDir, gateDir, witness, generation };
}

test("reset removes only generated authority and gate state", async () => {
  const state = await fixture();
  try {
    const plan = await resetStatePlan({ project, ...state });
    await applyResetState(plan);
    await assert.rejects(lstat(state.witness), { code: "ENOENT" });
    await assert.rejects(lstat(state.generation), { code: "ENOENT" });
    assert.equal(
      await readFile(join(state.authorityDir, ".synveda-private-directory"), "utf8"),
      `project:${project}\n`,
    );
    assert.equal(
      await readFile(join(state.gateDir, ".synveda-private-directory"), "utf8"),
      `project:${project}\n`,
    );
  } finally {
    await rm(state.root, { recursive: true, force: true });
  }
});

test("unknown runtime state blocks reset before deletion", async () => {
  const state = await fixture();
  try {
    await writeFile(join(state.gateDir, "unknown"), "sentinel\n", { mode: 0o600 });
    await assert.rejects(resetStatePlan({ project, ...state }), {
      message: "Keycloak public gate contains an unknown leaf",
    });
    assert.equal((await lstat(state.witness)).isFile(), true);
    assert.equal((await lstat(state.generation)).isDirectory(), true);
  } finally {
    await rm(state.root, { recursive: true, force: true });
  }
});
