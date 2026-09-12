import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { startGateway } from "../../claude-code/dist/mock-gateway.mjs";

const lifecycle = JSON.parse(readFileSync(new URL("../fixtures/lifecycle.json", import.meta.url), "utf8"));
const transcript = readFileSync(new URL("../fixtures/transcript.jsonl", import.meta.url), "utf8");
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

test("missing identity, unsupported events and project opt-out cause no API calls or spool", async () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-codex-optout-"));
  const gateway = await startGateway(() => assert.fail("invalid or disabled hook reached the API"));
  try {
    const start = { ...lifecycle.invocations[0].frames[0], cwd: root, transcript_path: join(root, "missing") };
    for (const input of [{}, { ...start, session_id: undefined }, { ...start, session_id: "" },
      { ...start, hook_event_name: "PreCompact" }, { ...start, source: "compact" },
      { ...start, model: "x".repeat(70_000) }]) {
      assert.equal(await hook(root, gateway.url, input), "");
    }
    assert.equal(await hook(root, gateway.url, start, { SYNVEDA_DISABLED: "1" }), "");
    assert.ok(!existsSync(join(root, "synveda", "spool")));
  } finally { await gateway.close(); rmSync(root, { recursive: true, force: true }); }
});
