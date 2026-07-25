/**
 * The bearer seam (ADR-0027 decision 4) against a stand-in CLI.
 *
 * The adapter's contract here is narrow and total: return a bearer when
 * one is available, return `undefined` otherwise, and never throw —
 * because every caller of this is a hook that must exit 0 (decision 3).
 * So each case below is a way the CLI can let it down.
 */

import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

process.env.XDG_STATE_HOME = mkdtempSync(join(tmpdir(), "synveda-cred-state-"));

const { resolveBearer } = await import("./credentials.mjs");
const { resolveGateway, loadConfig } = await import("./config.mjs");

const binDir = mkdtempSync(join(tmpdir(), "synveda-fake-cli-"));

/**
 * Writes an executable stand-in for `synveda` and points the adapter at
 * it. `body` is JavaScript run by this same Node.
 */
function fakeCli(name: string, body: string): string {
  const path = join(binDir, name);
  writeFileSync(path, `#!${process.execPath}\n${body}\n`, "utf8");
  chmodSync(path, 0o755);
  return path;
}

/** Runs `resolveBearer` with a pinned environment and restores it after. */
async function withEnv<T>(
  environment: Record<string, string | undefined>,
  body: () => Promise<T>,
): Promise<T> {
  const previous = new Map<string, string | undefined>();
  for (const [key, value] of Object.entries(environment)) {
    previous.set(key, process.env[key]);
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
  try {
    return await body();
  } finally {
    for (const [key, value] of previous) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

const missing = join(binDir, "definitely-not-installed");

test("an explicit SYNVEDA_TOKEN is used without consulting the CLI", async () => {
  // The CLI would fail loudly if it were consulted at all.
  const cli = fakeCli("never-called.mjs", "process.exit(3)");
  const bearer = await withEnv(
    { SYNVEDA_TOKEN: "dev-bearer", SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
    resolveBearer,
  );
  assert.deepEqual(bearer, { token: "dev-bearer", source: "env" });
});

test("the CLI's JSON becomes the bearer, and names its gateway", async () => {
  const cli = fakeCli(
    "ok.mjs",
    `console.log(JSON.stringify({
       profile: "default",
       access_token: "at-from-cli",
       token_type: "Bearer",
       gateway_url: "https://synveda.corp.test",
       tenant_id: "0198f000-0000-7000-8000-000000000000",
       subject: "alice@example.test",
     }))`,
  );
  const bearer = await withEnv(
    { SYNVEDA_TOKEN: undefined, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
    resolveBearer,
  );
  assert.deepEqual(bearer, {
    token: "at-from-cli",
    gatewayUrl: "https://synveda.corp.test",
    source: "cli",
  });
});

test("SYNVEDA_PROFILE selects the profile the CLI resolves", async () => {
  const cli = fakeCli(
    "profile.mjs",
    `console.log(JSON.stringify({ access_token: process.argv.slice(2).join(" ") }))`,
  );
  const bearer = await withEnv(
    { SYNVEDA_TOKEN: undefined, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: "work" },
    resolveBearer,
  );
  assert.equal(bearer?.token, "auth token --json --profile work");
});

test("a CLI that refuses costs the session nothing", async () => {
  // What "not logged in" actually looks like: a non-zero exit and an
  // instruction on stderr.
  const cli = fakeCli(
    "refuses.mjs",
    `console.error("synveda: no credentials for profile \`default\`; run \`synveda login\` first");
     process.exit(1)`,
  );
  const bearer = await withEnv(
    { SYNVEDA_TOKEN: undefined, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
    resolveBearer,
  );
  assert.equal(bearer, undefined);
});

test("an uninstalled CLI is not an error, only an absence", async () => {
  const bearer = await withEnv(
    { SYNVEDA_TOKEN: undefined, SYNVEDA_CLI: missing, SYNVEDA_PROFILE: undefined },
    resolveBearer,
  );
  assert.equal(bearer, undefined);
});

test("output that is not the documented JSON resolves nothing", async () => {
  for (const [name, body] of [
    ["garbage.mjs", `console.log("not json at all")`],
    ["empty.mjs", `console.log(JSON.stringify({ access_token: "" }))`],
    ["wrong-shape.mjs", `console.log(JSON.stringify({ access_token: 42 }))`],
    ["silent.mjs", ""],
  ]) {
    const cli = fakeCli(name, body);
    const bearer = await withEnv(
      { SYNVEDA_TOKEN: undefined, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
      resolveBearer,
    );
    assert.equal(bearer, undefined, `${name} must resolve nothing`);
  }
});

test("a CLI that hangs is abandoned, not waited on", async () => {
  const cli = fakeCli("hangs.mjs", "setTimeout(() => {}, 60_000)");
  const started = Date.now();
  const bearer = await withEnv(
    { SYNVEDA_TOKEN: undefined, SYNVEDA_CLI: cli, SYNVEDA_PROFILE: undefined },
    resolveBearer,
  );
  assert.equal(bearer, undefined);
  assert.ok(Date.now() - started < 10_000, "the CLI deadline must bound the hook");
});

test("a credential's own gateway wins over a project file's", () => {
  // The threat this closes: `.synveda/config.json` lives inside a
  // checked-out repository, so a `gateway_url` there must not be able to
  // redirect someone's bearer to a host of the repository's choosing.
  const project = { ...loadConfig(undefined), gatewayUrl: "http://attacker.test" };

  const fromCli = resolveGateway(project, {
    token: "t",
    gatewayUrl: "http://127.0.0.1:8120",
    source: "cli",
  });
  assert.equal(fromCli.gatewayUrl, "http://127.0.0.1:8120");

  // A trailing slash is not a different gateway.
  assert.equal(
    resolveGateway(project, { token: "t", gatewayUrl: "http://127.0.0.1:8120/", source: "cli" })
      .gatewayUrl,
    "http://127.0.0.1:8120",
  );

  // An explicit SYNVEDA_TOKEN keeps the configured gateway: an operator
  // who set both meant both.
  assert.equal(
    resolveGateway(project, { token: "t", source: "env" }).gatewayUrl,
    "http://attacker.test",
  );
});
