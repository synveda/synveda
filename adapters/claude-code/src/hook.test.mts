/**
 * The two hook handlers against a mock gateway, and the entry point
 * against a child process.
 *
 * The invariant every case here shares is ADR-0027 decision 3: whatever
 * the gateway does, the hook exits 0 and the session continues. This is
 * the harness-free driver decision 14 names, at handler scope; the
 * recorded-payload suite over a live gateway lands with the demo.
 */

import assert from "node:assert/strict";
import { once } from "node:events";
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { startGateway, type RecordedRequest, type Reply } from "./mock-gateway.mjs";

const stateHome = mkdtempSync(join(tmpdir(), "synveda-state-"));
process.env.XDG_STATE_HOME = stateHome;
process.env.SYNVEDA_TOKEN = "dev-bearer";
// Pin the credential seam away from whatever `synveda` this machine has
// installed: these cases are about the hook contract, not about whether
// the person running them happens to be logged in. The CLI seam's own
// cases live in credentials.test.mts.
process.env.SYNVEDA_CLI = join(stateHome, "no-cli-here");

const { flush } = await import("./flush.mjs");
const { sessionStart } = await import("./session-start.mjs");
const { loadSession } = await import("./spool.mjs");
type AdapterConfig = (typeof import("./config.mjs"))["loadConfig"] extends (
  cwd: string | undefined,
) => infer T
  ? T
  : never;

/** The scriptable gateway lives in `mock-gateway.mts`, shared with the driver. */
const gateway = (respond: (recorded: RecordedRequest, index: number) => Reply) =>
  startGateway(respond);

function config(gatewayUrl: string, overrides: Partial<AdapterConfig> = {}): AdapterConfig {
  return {
    disabled: false,
    inject: true,
    observe: true,
    skills: true,
    gatewayUrl,
    timeoutMs: 2000,
    ...overrides,
  };
}

function block(text: string, records: string[] = ["r1"]): Record<string, unknown> {
  return {
    text,
    block_hash: "b3-deadbeef",
    record_ids: records,
    tokens: 120,
    budget_tokens: 2000,
    as_of: "2026-07-24T10:00:00.000Z",
    degraded: [],
  };
}

function accepted(count: number, duplicates = 0): Record<string, unknown> {
  return {
    session_id: "claude-code:s1",
    accepted: count,
    duplicates,
    quarantined: 0,
    denied: 0,
    events: [],
  };
}

function transcript(entries: Record<string, unknown>[]): string {
  const file = join(mkdtempSync(join(tmpdir(), "synveda-transcript-")), "session.jsonl");
  writeFileSync(file, entries.map((entry) => JSON.stringify(entry)).join("\n"));
  return file;
}

function turn(uuid: string, text: string, type: "user" | "assistant" = "user"): Record<string, unknown> {
  return {
    type,
    uuid,
    timestamp: "2026-07-24T10:00:00.000Z",
    message: { role: type, content: text },
  };
}

test("a session start injects the block as additionalContext", async () => {
  const mock = await gateway(() => ({ status: 200, body: block("# Your team's memory\n- ship Fridays") }));
  try {
    const output = await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s1", source: "startup" },
      config(mock.url),
    );
    assert.equal(output.hookSpecificOutput?.hookEventName, "SessionStart");
    assert.match(output.hookSpecificOutput?.additionalContext ?? "", /ship Fridays/);
    assert.equal(mock.requests[0]?.path, "/v1/inject");
    // The audit correlation of decision 10, and a cold start has no task.
    assert.equal(mock.requests[0]?.body.session_id, "claude-code:s1");
    assert.equal(mock.requests[0]?.body.task, undefined);
  } finally {
    await mock.close();
  }
});

test("a compacted session start carries the last prompt as its task", async () => {
  const mock = await gateway(() => ({ status: 200, body: block("context") }));
  const path = transcript([
    turn("u1", "an older ask"),
    turn("a1", "working", "assistant"),
    turn("u2", "wire the retry budget"),
  ]);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s2", source: "compact", transcript_path: path },
      config(mock.url),
    );
    assert.equal(mock.requests[0]?.body.task, "wire the retry budget");
  } finally {
    await mock.close();
  }
});

test("a budget narrows only when the project configured one", async () => {
  const mock = await gateway(() => ({ status: 200, body: block("context") }));
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s3", source: "startup" },
      config(mock.url, { budgetTokens: 4000, compactBudgetTokens: 1000 }),
    );
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s3", source: "compact" },
      config(mock.url, { budgetTokens: 4000, compactBudgetTokens: 1000 }),
    );
    assert.equal(mock.requests[0]?.body.budget_tokens, 4000);
    assert.equal(mock.requests[1]?.body.budget_tokens, 1000);
  } finally {
    await mock.close();
  }
});

test("a degraded inject still delivers context and says nothing to the user", async () => {
  const mock = await gateway(() => ({
    status: 200,
    body: { ...block("still ranked"), degraded: ["embedder"] },
    headers: { "x-synveda-degraded": "embedder" },
  }));
  try {
    const output = await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s4", source: "startup" },
      config(mock.url),
    );
    assert.match(output.hookSpecificOutput?.additionalContext ?? "", /still ranked/);
    assert.equal(output.systemMessage, undefined);
  } finally {
    await mock.close();
  }
});

test("an empty block contributes no context and is not an error", async () => {
  const mock = await gateway(() => ({ status: 200, body: block("", []) }));
  try {
    const output = await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s5", source: "startup" },
      config(mock.url),
    );
    assert.deepEqual(output, {});
  } finally {
    await mock.close();
  }
});

test("an unreachable gateway costs the session nothing", async () => {
  const output = await sessionStart(
    { hook_event_name: "SessionStart", session_id: "s6", source: "startup" },
    // Port 1 on loopback: nothing listens, and the connection is refused
    // rather than hanging.
    config("http://127.0.0.1:1", { timeoutMs: 1000 }),
  );
  assert.deepEqual(output, {});
});

test("a rejected credential is the one failure the user is told about", async () => {
  const mock = await gateway(() => ({ status: 401, body: { message: "unauthenticated" } }));
  try {
    const output = await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s7", source: "startup" },
      config(mock.url),
    );
    assert.match(output.systemMessage ?? "", /synveda login/);
    assert.equal(output.hookSpecificOutput, undefined);
  } finally {
    await mock.close();
  }
});

test("a gateway error is silent: no context, no noise", async () => {
  const mock = await gateway(() => ({ status: 500, body: { message: "boom" } }));
  try {
    const output = await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s8", source: "startup" },
      config(mock.url),
    );
    assert.deepEqual(output, {});
  } finally {
    await mock.close();
  }
});

test("a stop flush posts the turn and advances the cursor", async () => {
  const mock = await gateway(() => ({ status: 200, body: accepted(2) }));
  const path = transcript([turn("u1", "the ask"), turn("a1", "the answer", "assistant")]);
  try {
    await flush(
      { hook_event_name: "Stop", session_id: "f1", transcript_path: path },
      config(mock.url),
    );
    const body = mock.requests[0]?.body;
    assert.equal(mock.requests[0]?.path, "/v1/observe");
    assert.equal(body?.session_id, "claude-code:f1");
    assert.equal((body?.events as unknown[]).length, 2);
    assert.equal(loadSession("claude-code:f1")?.cursor, "a1");
  } finally {
    await mock.close();
  }
});

test("a second flush with nothing new sends nothing", async () => {
  const mock = await gateway(() => ({ status: 200, body: accepted(1) }));
  const path = transcript([turn("u1", "the ask")]);
  try {
    await flush({ hook_event_name: "Stop", session_id: "f2", transcript_path: path }, config(mock.url));
    await flush({ hook_event_name: "Stop", session_id: "f2", transcript_path: path }, config(mock.url));
    assert.equal(mock.requests.length, 1);
  } finally {
    await mock.close();
  }
});

test("a failed flush leaves the cursor, and the next one resends", async () => {
  const failing = await gateway(() => ({ status: 503, body: { message: "unavailable" } }));
  const path = transcript([turn("u1", "the ask")]);
  try {
    await flush({ hook_event_name: "Stop", session_id: "f3", transcript_path: path }, config(failing.url));
    assert.equal(loadSession("claude-code:f3")?.cursor, undefined);
  } finally {
    await failing.close();
  }

  const recovered = await gateway(() => ({ status: 200, body: accepted(1) }));
  try {
    await flush({ hook_event_name: "Stop", session_id: "f3", transcript_path: path }, config(recovered.url));
    const events = recovered.requests[0]?.body.events as { idempotency_key: string }[];
    assert.equal(events.length, 1);
    assert.equal(events[0]?.idempotency_key, "u1");
    assert.equal(loadSession("claude-code:f3")?.cursor, "u1");
  } finally {
    await recovered.close();
  }
});

test("a flush whose payload carries no transcript path falls back to the spool", async () => {
  const mock = await gateway(() => ({ status: 200, body: accepted(1) }));
  const path = transcript([turn("u1", "before the compaction")]);
  try {
    // A session start is what puts the path in the spool.
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "f4", source: "startup", transcript_path: path },
      config(mock.url, { inject: false }),
    );
    await flush({ hook_event_name: "PreCompact", session_id: "f4" }, config(mock.url));
    assert.equal(mock.requests.length, 1);
    assert.equal(mock.requests[0]?.path, "/v1/observe");
    assert.equal(loadSession("claude-code:f4")?.cursor, "u1");
  } finally {
    await mock.close();
  }
});

test("the model a session start reported rides every observed event", async () => {
  const mock = await gateway(() => ({ status: 200, body: accepted(1) }));
  const path = transcript([turn("u1", "the ask")]);
  try {
    // Only `SessionStart` carries a model; `Stop` does not, which is why
    // the spool has to bring it across (ADR-0027 decision 8).
    await sessionStart(
      {
        hook_event_name: "SessionStart",
        session_id: "f6",
        source: "startup",
        transcript_path: path,
        model: "claude-opus-5",
      },
      config(mock.url, { inject: false }),
    );
    await flush({ hook_event_name: "Stop", session_id: "f6", transcript_path: path }, config(mock.url));
    const events = mock.requests[0]?.body.events as { payload: { context?: { model?: string } } }[];
    assert.equal(events[0]?.payload.context?.model, "claude-opus-5");
  } finally {
    await mock.close();
  }
});

test("observe turned off in a project posts nothing", async () => {
  const mock = await gateway(() => ({ status: 200, body: accepted(1) }));
  const path = transcript([turn("u1", "the ask")]);
  try {
    await flush(
      { hook_event_name: "Stop", session_id: "f5", transcript_path: path },
      config(mock.url, { observe: false }),
    );
    assert.equal(mock.requests.length, 0);
  } finally {
    await mock.close();
  }
});

const hookPath = fileURLToPath(new URL("./hook.mjs", import.meta.url));

async function runHook(
  mode: string,
  stdin: string,
  environment: Record<string, string>,
): Promise<{ code: number | null; stdout: string }> {
  const child = spawn(process.execPath, [hookPath, mode], {
    env: { ...process.env, ...environment },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stdout = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (piece: unknown) => {
    stdout += String(piece);
  });
  child.stdin.end(stdin);
  const [code] = (await once(child, "exit")) as [number | null];
  return { code, stdout };
}

test("the entry point exits 0 and emits context on the happy path", async () => {
  const mock = await gateway(() => ({ status: 200, body: block("# remembered") }));
  try {
    const result = await runHook(
      "session-start",
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "e1", source: "startup" }),
      { SYNVEDA_GATEWAY: mock.url, XDG_STATE_HOME: stateHome, SYNVEDA_TOKEN: "dev-bearer" },
    );
    assert.equal(result.code, 0);
    const parsed: unknown = JSON.parse(result.stdout);
    assert.match(
      (parsed as { hookSpecificOutput?: { additionalContext?: string } }).hookSpecificOutput
        ?.additionalContext ?? "",
      /remembered/,
    );
  } finally {
    await mock.close();
  }
});

test("the entry point exits 0 and stays silent when the gateway fails", async () => {
  const mock = await gateway(() => ({ status: 500, body: { message: "boom" } }));
  try {
    const result = await runHook(
      "session-start",
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "e2", source: "startup" }),
      { SYNVEDA_GATEWAY: mock.url, XDG_STATE_HOME: stateHome, SYNVEDA_TOKEN: "dev-bearer" },
    );
    assert.equal(result.code, 0);
    assert.equal(result.stdout, "");
  } finally {
    await mock.close();
  }
});

test("a payload it cannot parse asks the gateway for nothing", async () => {
  const mock = await gateway(() => ({ status: 200, body: block("should never be asked for") }));
  try {
    // The mode argument would be enough to dispatch on, and that is the
    // trap: with no payload there is no session to name and no `cwd` to
    // read the project's opt-out from (ADR-0027 decision 13).
    const result = await runHook("session-start", "this is not json", {
      XDG_STATE_HOME: stateHome,
      SYNVEDA_GATEWAY: mock.url,
      SYNVEDA_TOKEN: "dev-bearer",
    });
    assert.equal(result.code, 0);
    assert.equal(result.stdout, "");
    assert.equal(mock.requests.length, 0);
  } finally {
    await mock.close();
  }
});

test("the entry point exits 0 with no credentials and asks for a login", async () => {
  const result = await runHook(
    "session-start",
    JSON.stringify({ hook_event_name: "SessionStart", session_id: "e4", source: "startup" }),
    { XDG_STATE_HOME: stateHome, SYNVEDA_TOKEN: "" },
  );
  assert.equal(result.code, 0);
  assert.match(result.stdout, /synveda login/);
});

test("SYNVEDA_DISABLED turns the adapter off entirely", async () => {
  const mock = await gateway(() => ({ status: 200, body: block("should never be asked for") }));
  try {
    const result = await runHook(
      "session-start",
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "e5", source: "startup" }),
      { SYNVEDA_GATEWAY: mock.url, XDG_STATE_HOME: stateHome, SYNVEDA_DISABLED: "1" },
    );
    assert.equal(result.code, 0);
    assert.equal(result.stdout, "");
    assert.equal(mock.requests.length, 0);
  } finally {
    await mock.close();
  }
});
