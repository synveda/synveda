/** Authored vendor-contract tests, not captured Copilot frames (ADPT-9). */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import { startGateway, type Responder } from "../../claude-code/dist/mock-gateway.mjs";
import { newSpool, record, type Spool } from "../../claude-code/dist/spool.mjs";

const nativeId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const anotherId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const workspace = "11111111-1111-4111-8111-111111111111";
const session = "22222222-2222-4222-8222-222222222222";
const bearer = "synthetic-copilot-bearer";
const frame = (cwd: string, extra: Record<string, unknown> = {}) => ({
  sessionId: nativeId, timestamp: 1_789_200_000_000, cwd, source: "startup", ...extra,
});

async function hook(root: string, gateway: string, input: unknown,
  extra: Record<string, string> = {}, event = "sessionStart", raw?: string) {
  const child = spawn(process.execPath, [new URL("./hook.mjs", import.meta.url).pathname, event], {
    env: { ...process.env, XDG_STATE_HOME: root, XDG_CONFIG_HOME: root,
      SYNVEDA_CLI: join(root, "no-cli"), SYNVEDA_TOKEN: bearer,
      SYNVEDA_GATEWAY: gateway, SYNVEDA_WORKSPACE: workspace, SYNVEDA_PROJECT: "",
      SYNVEDA_DISABLED: "0", SYNVEDA_TIMEOUT_MS: "300", ...extra },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (piece) => { stdout += String(piece); });
  child.stderr.on("data", (piece) => { stderr += String(piece); });
  const done = new Promise<number | null>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
  child.stdin.end(raw ?? JSON.stringify(input));
  assert.equal(await done, 0, stderr);
  const logPath = join(root, "synveda", "adapter.log");
  const diagnosticLog = existsSync(logPath) ? readFileSync(logPath, "utf8") : "";
  assert.ok(!stderr.includes(bearer));
  assert.ok(!stderr.includes("private-prompt-marker"));
  assert.ok(!diagnosticLog.includes(bearer));
  assert.ok(!diagnosticLog.includes("private-prompt-marker"));
  return { stdout, stderr };
}

async function fixture(t: TestContext, respond: Responder) {
  const root = mkdtempSync(join(tmpdir(), "synveda-copilot-hooks-"));
  const gateway = await startGateway(respond);
  t.after(async () => { await gateway.close(); rmSync(root, { recursive: true, force: true }); });
  return { root, gateway };
}

function allowed(request: Parameters<Responder>[0]) {
  assert.equal(request.authorization, `Bearer ${bearer}`);
  if (request.path === "/v1/sessions") return { status: 201, body: { id: session, workspace_id: workspace } };
  if (request.path === `/v1/sessions/${session}/context-runs`) {
    return { status: 201, body: { rendered: "permitted context", tokens: 10, entry_count: 1 } };
  }
  assert.fail(`unexpected API operation ${request.path}`);
}

function saved(root: string): Spool[] {
  const directory = join(root, "synveda", "spool");
  return readdirSync(directory).filter((name) => name.endsWith(".json"))
    .map((name) => JSON.parse(readFileSync(join(directory, name), "utf8")) as Spool);
}

test("start/resume reuse one authenticated task and Copilot's native context output", async (t) => {
  const { root, gateway } = await fixture(t, allowed);
  for (const source of ["startup", "resume", "new"]) {
    const result = await hook(root, gateway.url, frame(root, { source,
      initialPrompt: "private-prompt-marker", transcript_path: "/unqualified/transcript" }));
    assert.deepEqual(JSON.parse(result.stdout), { additionalContext:
      `Synveda Session ID: ${session}. Pass this as session_id to Synveda MCP tools for this task.\n\npermitted context` });
  }
  const opens = gateway.requests.filter((request) => request.path === "/v1/sessions");
  assert.equal(opens.length, 1);
  assert.equal(opens[0].body.client_name, "copilot-cli");
  assert.equal(opens[0].body.external_session_id, `copilot-cli:${nativeId}`);
  assert.equal(opens[0].idempotencyKey, `copilot-cli-open-copilot-cli:${nativeId}`);
  assert.equal(new Set(gateway.requests.slice(1).map((request) => request.idempotencyKey)).size, 3);
  const [spool] = saved(root);
  assert.equal(spool.session_id, session);
  assert.equal(spool.close_requested, false);
  assert.equal(spool.transcript_path, undefined);
  assert.deepEqual(spool.entries, [], "context-only hooks must not fabricate observation evidence");
});

test("another conversation's start retains the previous active task binding", async (t) => {
  const { root, gateway } = await fixture(t, allowed);
  for (const id of [nativeId, anotherId, nativeId]) {
    await hook(root, gateway.url, frame(root, { sessionId: id }));
  }
  assert.equal(gateway.requests.filter((request) => request.path === "/v1/sessions").length, 2);
  assert.equal(saved(root).length, 2, "empty active Copilot spools still carry task identity");
  const calls = gateway.requests.length;
  for (const event of ["agentStop", "preCompact", "sessionEnd", "unknown"]) {
    assert.equal((await hook(root, gateway.url, frame(root), {}, event)).stdout, "");
  }
  assert.equal(gateway.requests.length, calls, "native exits must not implicitly end the task");
});

test("gateway outage retries the same idempotent open and preserves the task after recovery", async (t) => {
  let outage = true;
  const { root, gateway } = await fixture(t, (request) => outage ? { status: 503 } : allowed(request));
  assert.equal((await hook(root, gateway.url, frame(root))).stdout, "");
  const first = gateway.requests[0].idempotencyKey;
  assert.equal(saved(root)[0].session_id, undefined);
  outage = false;
  assert.ok((await hook(root, gateway.url, frame(root, { source: "resume" }))).stdout.includes(session));
  const opens = gateway.requests.filter((request) => request.path === "/v1/sessions");
  assert.ok(opens.every((request) => request.idempotencyKey === first));
  assert.equal(saved(root)[0].session_id, session);
});

test("401 and foreign-workspace 403 responses disclose no context or task identity", async (t) => {
  let denied = 401;
  const { root, gateway } = await fixture(t, (request) => request.path === "/v1/sessions"
    ? allowed(request) : { status: denied, body: { message: "private-prompt-marker" } });
  const unauthenticated = await hook(root, gateway.url, frame(root));
  assert.match(unauthenticated.stdout, /synveda login/);
  assert.ok(!unauthenticated.stdout.includes(session));
  denied = 403;
  assert.equal((await hook(root, gateway.url, frame(root, { source: "resume" }))).stdout, "");
  assert.ok(!JSON.stringify(saved(root)).includes("private-prompt-marker"));
});

test("a profile-origin change holds the saved task and never sends it to another gateway", async (t) => {
  const { root, gateway } = await fixture(t, allowed);
  const other = await startGateway(() => assert.fail("foreign origin received a saved task"));
  t.after(other.close);
  await hook(root, gateway.url, frame(root));
  const path = join(root, "synveda", "spool", readdirSync(join(root, "synveda", "spool"))[0]);
  const before = readFileSync(path);
  const result = await hook(root, other.url, frame(root, { source: "resume" }));
  assert.equal(result.stdout, "");
  assert.match(readFileSync(join(root, "synveda", "adapter.log"), "utf8"), /gateway_mismatch/);
  assert.equal(saved(root)[0].gateway_url, gateway.url);
  assert.deepEqual(readFileSync(path), before);
});

test("Copilot starts do not deliver or retire another harness's pending spool", async (t) => {
  const { root, gateway } = await fixture(t, allowed);
  const directory = join(root, "synveda", "spool");
  mkdirSync(directory, { recursive: true });
  const foreign = newSpool(nativeId, "claude-code", "synthetic-installation");
  foreign.gateway_url = gateway.url;
  foreign.session_id = session;
  record(foreign, [{ event_type: "message.user", client_event_id: "foreign-event",
    occurred_at: new Date().toISOString(), payload: { text: "private-prompt-marker" } }]);
  const path = join(directory, `${createHash("sha256").update(nativeId).digest("hex")}.json`);
  const before = JSON.stringify(foreign);
  writeFileSync(path, before, { mode: 0o600 });
  await hook(root, gateway.url, frame(root));
  assert.equal(readFileSync(path, "utf8"), before);
  assert.equal(saved(root).length, 2);
});

test("invalid/oversized input and opt-out make no API calls or spool", async (t) => {
  const { root, gateway } = await fixture(t, () => assert.fail("invalid/disabled input reached the API"));
  for (const input of [null, [], {}, frame(root, { sessionId: "" }), frame(root, { sessionId: "bad" }),
    frame("relative"), frame(root, { source: "compact" }), frame(root, { timestamp: "now" }),
    frame(root, { initialPrompt: "private-prompt-marker".repeat(4000) })]) {
    assert.equal((await hook(root, gateway.url, input)).stdout, "");
  }
  assert.equal((await hook(root, gateway.url, {}, {}, "sessionStart", "{broken")).stdout, "");
  assert.equal((await hook(root, gateway.url, frame(root), { SYNVEDA_DISABLED: "1" })).stdout, "");
  mkdirSync(join(root, ".synveda"));
  writeFileSync(join(root, ".synveda", "config.json"), '{"disabled":true}');
  assert.equal((await hook(root, gateway.url, frame(root))).stdout, "");
  assert.ok(!existsSync(join(root, "synveda", "spool")));
});

test("missing login never opens a task and empty allowed context still identifies the task", async (t) => {
  const { root, gateway } = await fixture(t, (request) => request.path === "/v1/sessions"
    ? allowed(request) : { status: 201, body: { rendered: "", tokens: 0, entry_count: 0 } });
  assert.match((await hook(root, gateway.url, frame(root), { SYNVEDA_TOKEN: "" })).stdout, /synveda login/);
  assert.equal(gateway.requests.length, 0);
  const output = JSON.parse((await hook(root, gateway.url, frame(root))).stdout);
  assert.equal(output.additionalContext,
    `Synveda Session ID: ${session}. Pass this as session_id to Synveda MCP tools for this task.`);
});

test("captured 1.0.83 new/resume frames reuse one task across runtime end without inventing observations", async (t) => {
  const { root, gateway } = await fixture(t, allowed);
  for (const name of ["lifecycle.json", "resume-lifecycle.json"]) {
    const capture = JSON.parse(readFileSync(new URL(`../fixtures/${name}`, import.meta.url), "utf8"));
    for (const frame of capture.invocations[0].frames) {
      const output = await hook(root, gateway.url, { ...frame.input, cwd: root }, {}, frame.event);
      if (frame.event === "sessionStart") {
        assert.ok(JSON.parse(output.stdout).additionalContext.includes(session));
      } else {
        assert.equal(output.stdout, "");
      }
    }
    const spools = saved(root);
    assert.equal(spools.length, 1);
    assert.equal(spools[0].session_id, session);
    assert.equal(spools[0].close_requested, false);
    assert.deepEqual(spools[0].entries, []);
  }
  assert.deepEqual(gateway.requests.map((request) => request.path),
    ["/v1/sessions", `/v1/sessions/${session}/context-runs`, `/v1/sessions/${session}/context-runs`]);
  assert.equal(new Set(gateway.requests.slice(1).map((request) => request.idempotencyKey)).size, 2);
});
