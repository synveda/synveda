import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { installationId } from "./install-id.mjs";

test("an unwritable installation identity never becomes an ephemeral conversation namespace", () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-install-id-"));
  writeFileSync(join(root, "synveda"), "occupied");
  const configHome = process.env.XDG_CONFIG_HOME;
  const stateHome = process.env.XDG_STATE_HOME;
  process.env.XDG_CONFIG_HOME = root;
  process.env.XDG_STATE_HOME = root;
  try {
    assert.equal(installationId(), undefined);
  } finally {
    if (configHome === undefined) delete process.env.XDG_CONFIG_HOME;
    else process.env.XDG_CONFIG_HOME = configHome;
    if (stateHome === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = stateHome;
    rmSync(root, { recursive: true, force: true });
  }
});
