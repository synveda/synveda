import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const HELPERS = [
  "check-runtime-smoke.mjs",
  "check-network-preflight.mjs",
  "check-host-resolution.mjs",
  "check-tls-inputs.mjs",
  "reset-runtime-state.mjs",
];

test("Compose helper entrypoints execute from canonical paths containing spaces", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda compose entrypoints "));
  try {
    for (const name of HELPERS) {
      const source = join(ROOT, "deploy/compose/scripts", name);
      const copied = join(scratch, `copied ${basename(name)}`);
      copyFileSync(source, copied);
      const result = spawnSync(process.execPath, [copied], { encoding: "utf8" });
      assert.equal(result.status, 64, `${name}: ${result.stdout}${result.stderr}`);
      assert.match(result.stderr, /configuration was refused/, name);
      assert.equal(result.stdout, "", name);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});
