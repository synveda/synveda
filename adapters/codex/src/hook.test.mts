import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { startGateway } from "../../claude-code/dist/mock-gateway.mjs";

const lifecycle = JSON.parse(readFileSync(new URL("../fixtures/lifecycle.json", import.meta.url), "utf8"));
const transcript = readFileSync(new URL("../fixtures/transcript.jsonl", import.meta.url), "utf8");
const compaction = JSON.parse(readFileSync(new URL("../fixtures/compaction.json", import.meta.url), "utf8"));
const compactedTranscript = readFileSync(new URL("../fixtures/transcript-compaction.jsonl", import.meta.url), "utf8");
const autoCompaction = JSON.parse(readFileSync(new URL("../fixtures/auto-compaction.json", import.meta.url), "utf8"));
const autoCompactedTranscript = readFileSync(new URL("../fixtures/transcript-auto-compaction.jsonl", import.meta.url), "utf8");
const nativeId = lifecycle.invocations[0].frames[0].session_id;
const workspace = "11111111-1111-1111-1111-111111111111";
const session = "22222222-2222-2222-2222-222222222222";

async function hook(root: string, gateway: string, payload: unknown, extra: Record<string, string> = {}) {
  const child = spawn(process.execPath, [new URL("./hook.mjs", import.meta.url).pathname], {
    env: { ...process.env, XDG_STATE_HOME: root, XDG_CONFIG_HOME: root,
      SYNVEDA_CLI: join(root, "no-cli"), SYNVEDA_TOKEN: "synthetic-codex-bearer",
      SYNVEDA_GATEWAY: gateway, SYNVEDA_WORKSPACE: workspace, SYNVEDA_PROJECT: "",
      SYNVEDA_DISABLED: "0", ...extra },
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
  child.stdin.end(JSON.stringify(payload));
  assert.equal(await done, 0, stderr);
  assert.ok(!stderr.includes("synthetic-codex-bearer"));
  return stdout;
}

test("native start, durable Stop, outage, runtime exit, resume and duplicate hooks keep one application Session", async () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-codex-hooks-"));
  const path = join(root, "transcript.jsonl");
  let outage = true;
  const accepted = new Set<string>();
  const gateway = await startGateway((request) => {
    assert.equal(request.authorization, "Bearer synthetic-codex-bearer");
    if (request.path === "/v1/sessions") return { status: 201, body: { id: session, workspace_id: workspace, status: "active" } };
    if (request.path.endsWith("/context-runs")) return { status: 201, body: { rendered: "permitted context", tokens: 10, entry_count: 1 } };
    if (request.path.endsWith("/events")) {
      if (outage) return { status: 503 };
      const events = request.body.events as { client_event_id: string }[];
      return { status: 200, body: { events: events.map((event) => {
        assert.ok(!accepted.has(event.client_event_id), "already acknowledged events must not be sent again");
        accepted.add(event.client_event_id);
        return { ...event, outcome: "appended" };
      }), denied: 0, quarantined: 0 } };
    }
    assert.fail(`unexpected API operation ${request.path}`);
  });
  const invoke = (frame: Record<string, unknown>) => hook(root, gateway.url, { ...frame, cwd: root, transcript_path: path });
  const frames = lifecycle.invocations[0].frames as Record<string, unknown>[];
  const resumed = lifecycle.invocations[1].frames as Record<string, unknown>[];
  const saved = () => {
    const directory = join(root, "synveda", "spool");
    const files = readdirSync(directory).filter((name) => name.endsWith(".json"));
    assert.equal(files.length, 1);
    return JSON.parse(readFileSync(join(directory, files[0]), "utf8"));
  };
  try {
    writeFileSync(path, transcript.split("\n")[0] + "\n");
    assert.ok((await invoke(frames[0])).includes(session));
    writeFileSync(path, transcript.split("\n").filter((line) => line && JSON.parse(line).ordinal <= 13).join("\n") + "\n");
    const calls = gateway.requests.length;
    await invoke(frames[4]);
    assert.equal(gateway.requests.length, calls, "Stop must persist without networking");
    assert.equal(saved().entries.length, 4);
    await invoke(frames[5]);
    assert.equal(saved().entries.filter((entry: { acknowledged: boolean }) => !entry.acknowledged).length, 4);
    assert.equal(saved().close_requested, false, "native exit is not task completion");
    outage = false;
    assert.ok((await invoke(resumed[0])).includes(session));
    writeFileSync(path, transcript);
    await invoke(resumed[2]);
    await invoke(resumed[3]);
    await invoke(resumed[2]);
    await invoke(resumed[3]);
    assert.equal(accepted.size, 6);
    assert.equal(saved().session_id, session);
    assert.equal(saved().external_session_id, `codex:${nativeId}`);
    assert.equal(saved().client_name, "codex");
    assert.equal(saved().close_requested, false);
    assert.equal(gateway.requests.filter((request) => request.path === "/v1/sessions").length, 1);
    assert.ok(!gateway.requests.some((request) => request.path.endsWith("/end")));
  } finally { await gateway.close(); rmSync(root, { recursive: true, force: true }); }
});

for (const scenario of [
  { trigger: "manual", frames: compaction.frames, transcript: compactedTranscript,
    start: 2, before: 0, after: 1, compact: 3, stop: 5, pending: 2, query: "gateway recovery" },
  { trigger: "auto", frames: autoCompaction.frames, transcript: autoCompactedTranscript,
    start: 0, before: 4, after: 5, compact: 6, stop: 7, pending: 3, query: "ingestion retry" },
]) test(`captured ${scenario.trigger} compaction persists locally then refreshes bounded context on the same task`, async () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-codex-compact-"));
  const path = join(root, "transcript.jsonl");
  const records = scenario.transcript.trim().split("\n");
  const compactIndex = records.findIndex((line) => JSON.parse(line).type === "compacted");
  const accepted = new Set<string>();
  const gateway = await startGateway((request) => {
    if (request.path === "/v1/sessions") return { status: 201, body: { id: session, workspace_id: workspace } };
    if (request.path.endsWith("/context-runs")) return { status: 201, body: { rendered: "fresh permitted context", tokens: 5, entry_count: 1 } };
    if (request.path.endsWith("/events")) return { status: 200, body: {
      events: (request.body.events as { client_event_id: string }[]).map((entry) => {
        assert.ok(!accepted.has(entry.client_event_id), "compaction must not replay acknowledged observations");
        accepted.add(entry.client_event_id);
        return { ...entry, outcome: "appended" };
      }), denied: 0, quarantined: 0,
    } };
    assert.fail(`unexpected API operation ${request.path}`);
  });
  const invoke = (frame: unknown) => hook(root, gateway.url,
    { ...(frame as Record<string, unknown>), cwd: root, transcript_path: path });
  try {
    mkdirSync(join(root, ".synveda"));
    writeFileSync(join(root, ".synveda", "config.json"), JSON.stringify({ compact_budget_tokens: 512 }));
    writeFileSync(path, records[0] + "\n");
    await invoke(scenario.frames[scenario.start]);
    writeFileSync(path, records.slice(0, compactIndex).join("\n") + "\n");
    const calls = gateway.requests.length;
    await invoke(scenario.frames[scenario.before]);
    await invoke(scenario.frames[scenario.before]);
    const directory = join(root, "synveda", "spool");
    const files = readdirSync(directory).filter((name) => name.endsWith(".json"));
    assert.equal(files.length, 1);
    const saved = JSON.parse(readFileSync(join(directory, files[0]), "utf8"));
    assert.equal(saved.entries.filter((entry: { acknowledged: boolean }) => !entry.acknowledged).length, scenario.pending);
    assert.equal(gateway.requests.length, calls, "PreCompact records without a network request");
    writeFileSync(path, records.slice(0, compactIndex + 1).join("\n") + "\n");
    assert.equal(await invoke(scenario.frames[scenario.after]), "", "PostCompact has no separate delivery path");
    assert.ok((await invoke(scenario.frames[scenario.compact])).includes("fresh permitted context"));
    const context = gateway.requests.filter((request) => request.path.endsWith("/context-runs"));
    assert.equal(context.length, 2);
    assert.equal(context[1].body.budget_tokens, 512);
    assert.ok(String(context[1].body.query).includes(scenario.query));
    writeFileSync(path, scenario.transcript);
    await invoke(scenario.frames[scenario.stop]);
    await invoke(scenario.frames[scenario.compact]);
    await invoke(scenario.frames[scenario.stop]);
    await invoke(scenario.frames[scenario.compact]);
    assert.equal(accepted.size, 4);
    assert.equal(gateway.requests.filter((request) => request.path === "/v1/sessions").length, 1);
    assert.ok(!gateway.requests.some((request) => request.path.endsWith("/end")));
    const final = JSON.parse(readFileSync(join(directory, files[0]), "utf8"));
    assert.equal(final.session_id, session);
    assert.equal(final.close_requested, false);
    assert.ok(final.entries.every((entry: { acknowledged: boolean }) => entry.acknowledged));
  } finally { await gateway.close(); rmSync(root, { recursive: true, force: true }); }
});

test("missing identity, unsupported events and project opt-out cause no API calls or spool", async () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-codex-optout-"));
  const gateway = await startGateway(() => assert.fail("invalid or disabled hook reached the API"));
  try {
    const start = { ...lifecycle.invocations[0].frames[0], cwd: root, transcript_path: join(root, "missing") };
    for (const input of [{}, { ...start, session_id: undefined }, { ...start, session_id: "" },
      { ...start, hook_event_name: "PreCompact" }, { ...start, hook_event_name: "PreCompact", trigger: "unknown" },
      { ...start, source: "unknown" }, { ...start, hook_event_name: "PostCompact" },
      { ...start, model: "x".repeat(70_000) }]) {
      assert.equal(await hook(root, gateway.url, input), "");
    }
    assert.equal(await hook(root, gateway.url, start, { SYNVEDA_DISABLED: "1" }), "");
    assert.ok(!existsSync(join(root, "synveda", "spool")));
  } finally { await gateway.close(); rmSync(root, { recursive: true, force: true }); }
});

test("exit shares its deadline across credentials and a stalled append, retaining events for retry", async () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-codex-deadline-"));
  const path = join(root, "transcript.jsonl");
  const cli = join(root, "credential.mjs");
  let appends = 0;
  let stalled = true;
  const server = createServer((request, response) => {
    let body = "";
    request.on("data", (piece) => { body += String(piece); });
    request.on("end", () => {
      let result: unknown;
      if (request.url === "/v1/sessions") result = { id: session, workspace_id: workspace, status: "active" };
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
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert.ok(address !== null && typeof address === "object");
  const gateway = `http://127.0.0.1:${address.port}`;
  const frame = (name: string) => ({ ...lifecycle.invocations[0].frames[0],
    hook_event_name: name, cwd: root, transcript_path: path });
  const saved = () => {
    const directory = join(root, "synveda", "spool");
    const files = readdirSync(directory).filter((name) => name.endsWith(".json"));
    assert.equal(files.length, 1);
    return JSON.parse(readFileSync(join(directory, files[0]), "utf8"));
  };
  try {
    writeFileSync(path, transcript.split("\n")[0] + "\n");
    await hook(root, gateway, frame("SessionStart"));
    writeFileSync(path, transcript);
    await hook(root, gateway, frame("Stop"));
    for (const body of [
      "setTimeout(() => {}, 60_000)",
      `setTimeout(() => console.log(JSON.stringify({ access_token: "synthetic-codex-bearer", gateway_url: ${JSON.stringify(gateway)} })), 700)`,
    ]) {
      writeFileSync(cli, `#!${process.execPath}\n${body}\n`);
      chmodSync(cli, 0o700);
      const started = Date.now();
      await hook(root, gateway, frame("SessionEnd"), { SYNVEDA_TOKEN: "", SYNVEDA_CLI: cli, SYNVEDA_TIMEOUT_MS: "10000" });
      assert.ok(Date.now() - started < 3000, "native exit must finish before the host's three-second cap");
      assert.equal(saved().entries.filter((entry: { acknowledged: boolean }) => !entry.acknowledged).length, 6);
      assert.equal(saved().close_requested, false);
    }
    assert.equal(appends, 1, "a resolved credential must leave only the remaining budget for delivery");
    stalled = false;
    await hook(root, gateway, frame("SessionEnd"));
    assert.equal(saved().entries.filter((entry: { acknowledged: boolean }) => !entry.acknowledged).length, 0);
    assert.equal(saved().session_id, session);
  } finally {
    server.closeAllConnections();
    server.close();
    await once(server, "close");
    rmSync(root, { recursive: true, force: true });
  }
});
