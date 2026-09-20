import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  cp,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const website = fileURLToPath(new URL(".", import.meta.url));
const rasters = [
  "synveda-avatar-512.png",
  "synveda-avatar-256.png",
  "favicon-32.png",
  "social-preview.png",
];

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), "synveda-brand-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const brand = join(root, "assets/brand");
  await cp(new URL("../assets/brand/", import.meta.url), brand, {
    recursive: true,
  });
  for (const name of ["brand.mjs", "rasterize.mjs"]) {
    await cp(join(website, name), join(root, "website", name), {
      recursive: true,
    });
  }
  await symlink(
    join(website, "node_modules"),
    join(root, "website/node_modules"),
  );
  return {
    brand,
    run(...args) {
      return spawnSync(
        process.execPath,
        [join(root, "website/brand.mjs"), ...args],
        {
          encoding: "utf8",
          timeout: 20_000,
        },
      );
    },
  };
}

function passed(result) {
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr || result.stdout);
}

test("exact export checks reject a modified PNG without rewriting it", async (t) => {
  const { brand, run } = await fixture(t);
  passed(run("--check"));
  const path = join(brand, rasters[0]);
  const original = await readFile(path);
  const changed = Buffer.from(original);
  changed[changed.length - 1] ^= 1;
  await writeFile(path, changed);
  const result = run("--check");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /synveda-avatar-512\.png is stale/);
  assert.deepEqual(await readFile(path), changed);
  passed(run());
  assert.deepEqual(await readFile(path), original);
  passed(run("--check"));
});

test("a canonical mark edit requires regeneration and reaches every PNG", async (t) => {
  const { brand, run } = await fixture(t);
  const originals = await Promise.all(
    rasters.map((name) => readFile(join(brand, name))),
  );
  const path = join(brand, "synveda-mark.svg");
  const original = await readFile(path, "utf8");
  await writeFile(path, original.replace("#2563EB", "#EF4444"));
  const result = run("--check");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /synveda-mark-dark\.svg is stale/);
  passed(run());
  passed(run("--check"));
  for (const [index, name] of rasters.entries()) {
    assert.notDeepEqual(
      await readFile(join(brand, name)),
      originals[index],
      name,
    );
  }
});
