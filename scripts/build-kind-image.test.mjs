import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));

function fixture(t) {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-kind-build-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const log = join(scratch, "arguments");
  writeFileSync(join(scratch, "docker"), `#!/bin/sh
printf '%s\\n' "$@" > "$SYNVEDA_BUILD_TEST_ARGS"
exit "\${SYNVEDA_BUILD_TEST_STATUS:-0}"
`, { mode: 0o755 });
  return {
    run(extra = {}, args = ["ghcr.io/synveda/product:0.2.0", "deploy/compose/product/Dockerfile", "product"]) {
      rmSync(log, { force: true });
      const result = spawnSync("bash", ["scripts/build-kind-image.sh", ...args], {
        cwd: root,
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: `${scratch}${delimiter}${process.env.PATH}`,
          SYNVEDA_KIND_GHA_CACHE: "0",
          ACTIONS_RUNTIME_TOKEN: "",
          ACTIONS_RESULTS_URL: "",
          GITHUB_EVENT_NAME: "",
          GITHUB_REF: "",
          SYNVEDA_BUILD_TEST_ARGS: log,
          SYNVEDA_BUILD_TEST_STATUS: "0",
          ...extra,
        },
      });
      return { ...result, args: existsSync(log) ? readFileSync(log, "utf8").trimEnd().split("\n") : [] };
    },
  };
}

const cacheRuntime = {
  SYNVEDA_KIND_GHA_CACHE: "1",
  ACTIONS_RUNTIME_TOKEN: "test-only-runtime-token",
  ACTIONS_RESULTS_URL: "https://cache.example.invalid/",
  GITHUB_EVENT_NAME: "push",
  GITHUB_REF: "refs/heads/main",
};

test("local acceptance builds the requested Dockerfile without GitHub credentials", (t) => {
  const result = fixture(t).run();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.args, ["build", "-t", "ghcr.io/synveda/product:0.2.0",
    "-f", "deploy/compose/product/Dockerfile", "."]);
});

test("CI builds and loads each image with a separate intermediate-layer cache", (t) => {
  const f = fixture(t);
  for (const [scope, dockerfile] of [
    ["product", "deploy/compose/product/Dockerfile"],
    ["cnpg-postgres", "deploy/helm/postgres/Dockerfile"],
    ["keycloak", "deploy/compose/keycloak/Dockerfile"],
  ]) {
    const image = `ghcr.io/synveda/${scope}:test`;
    const result = f.run(cacheRuntime, [image, dockerfile, scope]);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.args, ["buildx", "build", "--load", "-t", image, "-f", dockerfile,
      "--cache-from", `type=gha,version=2,scope=kind-${scope},timeout=2m`,
      "--cache-to", `type=gha,version=2,scope=kind-${scope},mode=max,ignore-error=true,timeout=2m`, "."]);
    assert.ok(!result.args.join(" ").includes(cacheRuntime.ACTIONS_RUNTIME_TOKEN));
  }
});

test("PRs and non-main runs restore layers without publishing a cache", (t) => {
  const f = fixture(t);
  for (const context of [
    { GITHUB_EVENT_NAME: "pull_request", GITHUB_REF: "refs/pull/12/merge" },
    { GITHUB_EVENT_NAME: "pull_request", GITHUB_REF: "refs/heads/main" },
    { GITHUB_EVENT_NAME: "push", GITHUB_REF: "refs/heads/contributor" },
    { GITHUB_EVENT_NAME: "", GITHUB_REF: "" },
  ]) {
    const result = f.run({ ...cacheRuntime, ...context });
    assert.equal(result.status, 0, result.stderr);
    assert.ok(result.args.includes("--cache-from"));
    assert.ok(!result.args.includes("--cache-to"));
    assert.equal(result.args.at(-1), ".");
  }
});

test("build failures remain fatal with and without the optional cache", (t) => {
  const f = fixture(t);
  for (const env of [{}, cacheRuntime]) {
    const result = f.run({ ...env, SYNVEDA_BUILD_TEST_STATUS: "19" });
    assert.equal(result.status, 19);
  }
});

test("inherited shell tracing cannot disclose cache credentials", (t) => {
  const result = fixture(t).run({ ...cacheRuntime, SHELLOPTS: "xtrace" });
  assert.equal(result.status, 0, result.stderr);
  assert.ok(!(result.stdout + result.stderr).includes(cacheRuntime.ACTIONS_RUNTIME_TOKEN));
});

test("invalid cache settings fail before invoking Docker", (t) => {
  const f = fixture(t);
  for (const env of [
    { SYNVEDA_KIND_GHA_CACHE: "true" },
    { ...cacheRuntime, ACTIONS_RUNTIME_TOKEN: "" },
    { ...cacheRuntime, ACTIONS_RESULTS_URL: "" },
  ]) {
    const result = f.run(env);
    assert.equal(result.status, 64);
    assert.deepEqual(result.args, []);
  }
  for (const args of [[], ["image", "Dockerfile", "unsupported"]]) {
    const result = f.run({}, args);
    assert.equal(result.status, 64);
    assert.deepEqual(result.args, []);
  }
});
