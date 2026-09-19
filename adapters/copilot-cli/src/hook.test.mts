/** Authored vendor-contract tests, not captured Copilot frames (ADPT-9). */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { once } from "node:events";
import { createServer } from "node:http";
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
const capture = JSON.parse(readFileSync(new URL("../fixtures/lifecycle.json", import.meta.url), "utf8"));
const resume = JSON.parse(readFileSync(new URL("../fixtures/resume-lifecycle.json", import.meta.url), "utf8"));
const transcript = readFileSync(new URL("../fixtures/transcript.jsonl", import.meta.url), "utf8");
const resumeTranscript = readFileSync(new URL("../fixtures/resume-transcript.jsonl", import.meta.url), "utf8");
const governedTranscript = readFileSync(new URL("../fixtures/governed-transcript.jsonl", import.meta.url), "utf8");
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
    const output = JSON.parse(result.stdout);
    assert.deepEqual(Object.keys(output), ["additionalContext"]);
    const context = `Synveda Session ID: ${session}. Pass this as session_id to Synveda MCP tools for this task.\n\npermitted context`;
    if (source === "startup") {
      assert.ok(output.additionalContext.startsWith(context));
      assert.ok(output.additionalContext.includes("Synveda is active in this project"));
    } else assert.equal(output.additionalContext, context);
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
  assert.ok(output.additionalContext.startsWith(
    `Synveda Session ID: ${session}. Pass this as session_id to Synveda MCP tools for this task.`));
});

function native(root: string, event: string, resumed = false): Record<string, unknown> {
  const value = (resumed ? resume : capture).invocations[0].frames
    .find((frame: { event: string }) => frame.event === event).input;
  return { ...value, cwd: root, ...(value.transcriptPath === undefined ? {} : { transcriptPath: join(root, "native.jsonl") }) };
}

function appended(request: Parameters<Responder>[0]) {
  return { status: 200, body: {
    events: (request.body.events as { client_event_id: string }[]).map((event) => ({ ...event, outcome: "appended" })),
    denied: 0, quarantined: 0,
  } };
}

test("captured MCP denial drains once through Stop and exit without ending the task", async (t) => {
  const accepted: { client_event_id: string; payload: unknown }[] = [];
  const { root, gateway } = await fixture(t, (request) => {
    if (!request.path.endsWith("/events")) return allowed(request);
    accepted.push(...request.body.events as typeof accepted);
    return appended(request);
  });
  const records = governedTranscript.trim().split("\n").map((line) => JSON.parse(line));
  const sessionId = records[0].data.sessionId;
  const input = frame(root, { sessionId, source: "new", transcriptPath: join(root, "native.jsonl") });
  writeFileSync(join(root, "native.jsonl"), governedTranscript);
  await hook(root, gateway.url, input);
  const requests = gateway.requests.length;
  await hook(root, gateway.url, input, {}, "agentStop");
  assert.equal(gateway.requests.length, requests, "Stop must remain local even when a tool failed");
  assert.equal(saved(root)[0].entries.length, 8);
  assert.ok(saved(root)[0].entries.every((entry) => !entry.acknowledged));
  for (let repeat = 0; repeat < 2; repeat += 1) {
    await hook(root, gateway.url, input, {}, "sessionEnd");
    await hook(root, gateway.url, input, {}, "agentStop");
  }
  assert.equal(accepted.length, 8);
  assert.equal(new Set(accepted.map((entry) => entry.client_event_id)).size, 8);
  const denied = records.find((record) => record.data.success === false);
  const result = accepted.find((entry) => entry.client_event_id === denied.id)?.payload as {
    tools: { is_error: boolean; text: string }[];
  };
  assert.equal(result.tools[0].is_error, true);
  assert.equal(result.tools[0].text, denied.data.error.message);
  const [spool] = saved(root);
  assert.equal(spool.session_id, session);
  assert.equal(spool.close_requested, false);
  assert.ok(spool.entries.every((entry) => entry.acknowledged));
});

test("captured Stop persists locally; outage, exit, resume and duplicate hooks deliver six events on one task", async (t) => {
  let outage = true;
  const accepted = new Set<string>();
  const { root, gateway } = await fixture(t, (request) => {
    if (!request.path.endsWith("/events")) return allowed(request);
    assert.equal(request.authorization, `Bearer ${bearer}`);
    if (outage) return { status: 503 };
    for (const event of request.body.events as { client_event_id: string }[]) {
      assert.ok(!accepted.has(event.client_event_id), "acknowledged events must not be replayed");
      accepted.add(event.client_event_id);
    }
    return appended(request);
  });
  const invoke = (event: string, resumed = false) => hook(root, gateway.url, native(root, event, resumed), {}, event);
  writeFileSync(join(root, "native.jsonl"), transcript);
  await invoke("sessionStart");
  const calls = gateway.requests.length;
  for (const event of ["userPromptSubmitted", "preToolUse", "postToolUse", "agentStop"]) await invoke(event);
  assert.equal(gateway.requests.length, calls, "Stop crosses local durability without credentials or network");
  assert.equal(saved(root)[0].entries.length, 4);
  assert.ok(saved(root)[0].entries.every((entry) => !entry.acknowledged));
  await invoke("sessionEnd");
  assert.equal(saved(root)[0].close_requested, false);
  assert.ok(saved(root)[0].entries.every((entry) => !entry.acknowledged));
  outage = false;
  // The authentic resumed user message precedes SessionStart; the assistant
  // response is only present before the later agentStop.
  const records = resumeTranscript.trim().split("\n");
  appendFileSync(join(root, "native.jsonl"), records.slice(0, 2).join("\n") + "\n");
  await invoke("sessionStart", true);
  assert.equal(accepted.size, 5);
  appendFileSync(join(root, "native.jsonl"), records.slice(2).join("\n") + "\n");
  for (let repeat = 0; repeat < 2; repeat += 1) {
    await invoke("agentStop", true);
    await invoke("sessionEnd", true);
  }
  assert.equal(accepted.size, 6);
  const [spool] = saved(root);
  assert.equal(saved(root).length, 1);
  assert.equal(spool.session_id, session);
  assert.equal(spool.external_session_id, `copilot-cli:${capture.invocations[0].result.session_id}`);
  assert.equal(spool.close_requested, false);
  assert.ok(spool.entries.every((entry) => entry.acknowledged));
  assert.equal(gateway.requests.filter((request) => request.path === "/v1/sessions").length, 1);
  const context = gateway.requests.filter((request) => request.path.endsWith("/context-runs"));
  assert.equal(context.length, 2);
  assert.equal(new Set(context.map((request) => request.idempotencyKey)).size, 2);
  assert.equal(context[1].body.query, resume.invocations[0].frames[0].input.prompt);
});

test("observation opt-out and invalid or foreign Stop payloads preserve the saved cursor", async (t) => {
  const { root, gateway } = await fixture(t, allowed);
  const path = join(root, "native.jsonl");
  writeFileSync(path, transcript);
  await hook(root, gateway.url, native(root, "sessionStart"));
  await hook(root, gateway.url, native(root, "agentStop"), {}, "agentStop");
  const before = JSON.stringify(saved(root));
  const calls = gateway.requests.length;
  for (const value of [undefined, "relative", "/nul\0", "/" + "a".repeat(4096)]) {
    await hook(root, gateway.url, { ...native(root, "agentStop"), transcriptPath: value }, {}, "agentStop");
    assert.equal(JSON.stringify(saved(root)), before);
  }
  for (const raw of [transcript + "{private-prompt-marker",
    transcript.replace(capture.invocations[0].result.session_id, anotherId)]) {
    writeFileSync(path, raw);
    await hook(root, gateway.url, native(root, "agentStop"), {}, "agentStop");
    assert.equal(JSON.stringify(saved(root)), before);
  }
  writeFileSync(path, transcript + resumeTranscript);
  mkdirSync(join(root, ".synveda"));
  writeFileSync(join(root, ".synveda", "config.json"), '{"observe":false}');
  await hook(root, gateway.url, native(root, "agentStop"), {}, "agentStop");
  await hook(root, gateway.url, native(root, "sessionEnd"), {}, "sessionEnd");
  assert.equal(JSON.stringify(saved(root)), before);
  assert.equal(gateway.requests.length, calls);
});

test("observation delivery holds denied credentials and a changed gateway, then retries the same task", async (t) => {
  let status = 401;
  const { root, gateway } = await fixture(t, (request) => request.path.endsWith("/events")
    ? status === 200 ? appended(request) : { status, body: { message: "private-prompt-marker" } } : allowed(request));
  const other = await startGateway(() => assert.fail("foreign gateway received saved observations"));
  t.after(other.close);
  writeFileSync(join(root, "native.jsonl"), transcript);
  await hook(root, gateway.url, native(root, "sessionStart"));
  await hook(root, gateway.url, native(root, "agentStop"), {}, "agentStop");
  for (status of [401, 403]) {
    await hook(root, gateway.url, native(root, "sessionEnd"), {}, "sessionEnd");
    assert.ok(saved(root)[0].entries.every((entry) => !entry.acknowledged));
    assert.equal(saved(root)[0].close_requested, false);
  }
  const before = JSON.stringify(saved(root));
  await hook(root, other.url, native(root, "sessionEnd"), {}, "sessionEnd");
  assert.equal(saved(root)[0].gateway_url, gateway.url);
  assert.deepEqual(saved(root)[0].entries, JSON.parse(before)[0].entries);
  status = 200;
  await hook(root, gateway.url, native(root, "sessionEnd"), {}, "sessionEnd");
  assert.ok(saved(root)[0].entries.every((entry) => entry.acknowledged));
  assert.equal(saved(root)[0].session_id, session);
});

test("exit bounds credential resolution and stalled append together, retaining pending observations", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "synveda-copilot-deadline-"));
  let stalled = true;
  let appends = 0;
  const server = createServer((request, response) => {
    let body = "";
    request.on("data", (piece) => { body += String(piece); });
    request.on("end", () => {
      let result: unknown;
      if (request.url === "/v1/sessions") result = { id: session, workspace_id: workspace };
      else if (request.url?.endsWith("/context-runs")) result = { rendered: "allowed", tokens: 1, entry_count: 1 };
      else if (request.url?.endsWith("/events")) {
        appends += 1;
        if (stalled) return;
        result = { events: JSON.parse(body).events.map((entry: { client_event_id: string }) => ({
          client_event_id: entry.client_event_id, outcome: "appended",
        })), denied: 0, quarantined: 0 };
      } else assert.fail(`unexpected request ${request.url}`);
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify(result));
    });
  });
  t.after(async () => {
    server.closeAllConnections();
    server.close();
    await once(server, "close");
    rmSync(root, { recursive: true, force: true });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert.ok(address !== null && typeof address === "object");
  const gateway = `http://127.0.0.1:${address.port}`;
  writeFileSync(join(root, "native.jsonl"), transcript);
  await hook(root, gateway, native(root, "sessionStart"));
  await hook(root, gateway, native(root, "agentStop"), {}, "agentStop");
  const cli = join(root, "credential.mjs");
  for (const body of [
    "setTimeout(() => {}, 60_000)",
    `setTimeout(() => console.log(JSON.stringify({ access_token: "${bearer}", gateway_url: ${JSON.stringify(gateway)} })), 700)`,
  ]) {
    writeFileSync(cli, `#!${process.execPath}\n${body}\n`);
    chmodSync(cli, 0o700);
    const started = Date.now();
    await hook(root, gateway, native(root, "sessionEnd"), {
      SYNVEDA_TOKEN: "", SYNVEDA_CLI: cli, SYNVEDA_TIMEOUT_MS: "10000",
    }, "sessionEnd");
    assert.ok(Date.now() - started < 3000, "exit must honour the two-second delivery budget with teardown headroom");
    assert.equal(saved(root)[0].entries.filter((entry) => !entry.acknowledged).length, 4);
    assert.equal(saved(root)[0].close_requested, false);
  }
  assert.equal(appends, 1, "credentials consume the same deadline as the append");
  stalled = false;
  await hook(root, gateway, native(root, "sessionEnd"), {}, "sessionEnd");
  assert.ok(saved(root)[0].entries.every((entry) => entry.acknowledged));
});
