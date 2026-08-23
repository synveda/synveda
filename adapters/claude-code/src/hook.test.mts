/**
 * The two hook handlers against a mock gateway, and the entry point against a
 * child process.
 *
 * The invariant every case here shares is ADR-0027 decision 3: whatever the
 * gateway does, the hook exits 0 and the session continues. This is the
 * harness-free driver decision 14 names, at handler scope.
 *
 * Since CPR-12 there is a second invariant, and it is the point of the
 * feature: **whatever the gateway does, the events are on disk first**. Every
 * failure case below therefore asserts what the spool holds, not only that
 * nothing crashed.
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
process.env.XDG_CONFIG_HOME = stateHome;
process.env.SYNVEDA_TOKEN = "dev-bearer";
// Pin the credential seam away from whatever `synveda` this machine has
// installed: these cases are about the hook contract, not about whether the
// person running them happens to be logged in. The CLI seam's own cases live
// in credentials.test.mts.
process.env.SYNVEDA_CLI = join(stateHome, "no-cli-here");

const { turn: turnHook } = await import("./turn.mjs");
const { sessionStart } = await import("./session-start.mjs");
const { loadSpool, pending } = await import("./spool.mjs");
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
    workspaceId: "11111111-1111-1111-1111-111111111111",
    ...overrides,
  };
}

/** The run `POST /v1/sessions` answers with. */
function session(id = "22222222-2222-2222-2222-222222222222"): Record<string, unknown> {
  return {
    id,
    workspace_id: "11111111-1111-1111-1111-111111111111",
    status: "active",
  };
}

/** A composed block, as `POST …/context-runs` answers. */
function contextRun(rendered: string, entries = 1): Record<string, unknown> {
  return {
    id: "33333333-3333-3333-3333-333333333333",
    rendered,
    block_hash: "b3-deadbeef",
    tokens: 120,
    budget_tokens: 2000,
    entry_count: entries,
    degraded: [],
    created_at: "2026-08-25T10:00:00.000Z",
  };
}

/** An append answer that resolves every event it was sent. */
function appended(request: RecordedRequest): Record<string, unknown> {
  const events = (request.body.events ?? []) as { client_event_id: string }[];
  return {
    events: events.map((event) => ({
      outcome: "appended",
      client_event_id: event.client_event_id,
    })),
    appended: events.length,
    duplicates: 0,
    quarantined: 0,
    denied: 0,
  };
}

/**
 * The default script: open a run, compose a block, accept every append.
 * Cases override one leg at a time so the thing under test is the only thing
 * that differs.
 */
function script(overrides: (request: RecordedRequest, index: number) => Reply | undefined) {
  return (request: RecordedRequest, index: number): Reply => {
    const override = overrides(request, index);
    if (override !== undefined) return override;
    if (request.path === "/v1/sessions") return { status: 201, body: session() };
    if (request.path.endsWith("/context-runs")) {
      return { status: 200, body: contextRun("# memory") };
    }
    if (request.path.endsWith("/events")) return { status: 200, body: appended(request) };
    if (request.path.endsWith("/end")) return { status: 200, body: session() };
    if (request.path === "/v1/me") {
      return { status: 200, body: { workspaces: [{ id: "11111111-1111-1111-1111-111111111111" }] } };
    }
    return { status: 404, body: {} };
  };
}

const ok = script(() => undefined);

function transcript(entries: Record<string, unknown>[]): string {
  const file = join(mkdtempSync(join(tmpdir(), "synveda-transcript-")), "session.jsonl");
  writeFileSync(file, entries.map((entry) => JSON.stringify(entry)).join("\n"));
  return file;
}

function entry(
  uuid: string,
  text: string,
  type: "user" | "assistant" = "user",
): Record<string, unknown> {
  return {
    type,
    uuid,
    timestamp: "2026-08-25T10:00:00.000Z",
    message: { role: type, content: text },
  };
}

// ── SessionStart ─────────────────────────────────────────────────────────

test("a session start opens a run and injects the block as additionalContext", async () => {
  const mock = await gateway(ok);
  try {
    const output = await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s1", source: "startup" },
      config(mock.url),
    );
    assert.match(output.hookSpecificOutput?.additionalContext ?? "", /memory/);
    assert.equal(output.hookSpecificOutput?.hookEventName, "SessionStart");
    const open = mock.requests.find((request) => request.path === "/v1/sessions");
    assert.ok(open, "the run must be opened");
    // Idempotent by the harness's own id: a SessionStart that fires twice for
    // one conversation must land on one run.
    assert.equal(open.idempotencyKey, "cc-open-s1");
    assert.equal(open.body.external_session_id, "s1");
    assert.equal(open.body.client_name, "claude-code");
    assert.ok(mock.requests.some((request) => request.path.endsWith("/context-runs")));
  } finally {
    await mock.close();
  }
});

test("the run is reused on the next start rather than reopened", async () => {
  const mock = await gateway(ok);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s-reuse", source: "startup" },
      config(mock.url),
    );
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s-reuse", source: "resume" },
      config(mock.url),
    );
    const opens = mock.requests.filter((request) => request.path === "/v1/sessions");
    assert.equal(opens.length, 1, "the second start must resume, not reopen");
  } finally {
    await mock.close();
  }
});

test("a compacted session start carries the last prompt as its query", async () => {
  const mock = await gateway(ok);
  const path = transcript([
    entry("u1", "an older ask"),
    entry("a1", "working", "assistant"),
    entry("u2", "wire the retry budget"),
  ]);
  try {
    await sessionStart(
      {
        hook_event_name: "SessionStart",
        session_id: "s2",
        source: "compact",
        transcript_path: path,
      },
      config(mock.url),
    );
    const compose = mock.requests.find((request) => request.path.endsWith("/context-runs"));
    assert.equal(compose?.body.query, "wire the retry budget");
  } finally {
    await mock.close();
  }
});

test("a cold start has no query — the block is the recency branch", async () => {
  const mock = await gateway(ok);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s2b", source: "startup" },
      config(mock.url),
    );
    const compose = mock.requests.find((request) => request.path.endsWith("/context-runs"));
    assert.equal(compose?.body.query, undefined);
  } finally {
    await mock.close();
  }
});

test("a budget narrows only when the project configured one", async () => {
  const mock = await gateway(ok);
  const settings = { budgetTokens: 4000, compactBudgetTokens: 1000 };
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s3", source: "startup" },
      config(mock.url, settings),
    );
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s3", source: "compact" },
      config(mock.url, settings),
    );
    const composes = mock.requests.filter((request) => request.path.endsWith("/context-runs"));
    assert.equal(composes[0]?.body.budget_tokens, 4000);
    assert.equal(composes[1]?.body.budget_tokens, 1000);
  } finally {
    await mock.close();
  }
});

test("a degraded composition still delivers context and says nothing to the user", async () => {
  const mock = await gateway(
    script((request) =>
      request.path.endsWith("/context-runs")
        ? {
            status: 200,
            body: { ...contextRun("still ranked"), degraded: ["embedder"] },
            headers: { "x-synveda-degraded": "embedder" },
          }
        : undefined,
    ),
  );
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
  const mock = await gateway(
    script((request) =>
      request.path.endsWith("/context-runs")
        ? { status: 200, body: contextRun("", 0) }
        : undefined,
    ),
  );
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
  // Port 1 on loopback: nothing listens, and the connection is refused rather
  // than hanging.
  const output = await sessionStart(
    { hook_event_name: "SessionStart", session_id: "s6", source: "startup" },
    config("http://127.0.0.1:1", { timeoutMs: 1000 }),
  );
  assert.deepEqual(output, {});
});

test("a rejected credential is the one failure the user is told about", async () => {
  const mock = await gateway(
    script((request) =>
      request.path.endsWith("/context-runs")
        ? { status: 401, body: { message: "unauthenticated" } }
        : undefined,
    ),
  );
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
  const mock = await gateway(
    script((request) =>
      request.path.endsWith("/context-runs") ? { status: 500, body: { message: "boom" } } : undefined,
    ),
  );
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

/**
 * More than one workspace is a question, not a guess: composing in whichever
 * sorted first would answer from the wrong team's memory.
 */
test("an ambiguous workspace opens no run rather than guessing", async () => {
  const mock = await gateway(
    script((request) =>
      request.path === "/v1/me"
        ? { status: 200, body: { workspaces: [{ id: "a" }, { id: "b" }] } }
        : undefined,
    ),
  );
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "s9", source: "startup" },
      config(mock.url, { workspaceId: undefined }),
    );
    assert.equal(
      mock.requests.filter((request) => request.path === "/v1/sessions").length,
      0,
    );
  } finally {
    await mock.close();
  }
});

// ── Stop, PreCompact, SessionEnd ─────────────────────────────────────────

test("a stop records the turn and delivers it", async () => {
  const mock = await gateway(ok);
  const path = transcript([entry("u1", "the ask"), entry("a1", "the answer", "assistant")]);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "f1", source: "startup", transcript_path: path },
      config(mock.url, { inject: false }),
    );
    await turnHook(
      { hook_event_name: "Stop", session_id: "f1", transcript_path: path },
      config(mock.url),
    );
    const append = mock.requests.find((request) => request.path.endsWith("/events"));
    assert.ok(append);
    assert.equal((append.body.events as unknown[]).length, 2);
    // An append takes no Idempotency-Key: its unit is the event.
    assert.equal(append.idempotencyKey, undefined);
    const spool = loadSpool("f1");
    assert.ok(spool);
    assert.equal(pending(spool).length, 0, "everything delivered is acknowledged");
  } finally {
    await mock.close();
  }
});

test("a second stop with nothing new sends nothing", async () => {
  const mock = await gateway(ok);
  const path = transcript([entry("u1", "the ask")]);
  try {
    await turnHook({ hook_event_name: "Stop", session_id: "f2", transcript_path: path }, config(mock.url));
    const after = mock.requests.length;
    await turnHook({ hook_event_name: "Stop", session_id: "f2", transcript_path: path }, config(mock.url));
    assert.equal(mock.requests.length, after, "nothing new means no request");
  } finally {
    await mock.close();
  }
});

/**
 * **The property this whole feature exists for.** The gateway is down, the
 * events are recorded anyway, and the next start delivers them.
 */
test("a failed delivery keeps the events and the next start sends them", async () => {
  const path = transcript([entry("u1", "the ask")]);
  // A run first, so the failure under test is the *delivery* failing rather
  // than there being nowhere to deliver to.
  const opening = await gateway(ok);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "f3", source: "startup" },
      config(opening.url, { inject: false }),
    );
  } finally {
    await opening.close();
  }

  const failing = await gateway(
    script((request) =>
      request.path.endsWith("/events") ? { status: 503, body: { message: "unavailable" } } : undefined,
    ),
  );
  try {
    await turnHook(
      { hook_event_name: "Stop", session_id: "f3", transcript_path: path },
      config(failing.url),
    );
    const held = loadSpool("f3");
    assert.ok(held, "the spool exists even though nothing was delivered");
    assert.equal(pending(held).length, 1, "the event is held, not lost");
    assert.equal(held.entries[0]?.delivery_attempts, 1, "the attempt is counted");
  } finally {
    await failing.close();
  }

  const recovered = await gateway(ok);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "f3", source: "resume", transcript_path: path },
      config(recovered.url, { inject: false }),
    );
    const append = recovered.requests.find((request) => request.path.endsWith("/events"));
    assert.ok(append, "the backlog is delivered at the next start");
    const events = append.body.events as { client_event_id: string }[];
    assert.equal(events[0]?.client_event_id, "u1");
    const drained = loadSpool("f3");
    assert.ok(drained);
    assert.equal(pending(drained).length, 0, "the backlog is gone once it lands");
  } finally {
    await recovered.close();
  }
});

/**
 * A turn before any run exists still records. There is nowhere to deliver to,
 * so nothing is attempted — and the events are kept for the start that opens
 * the run.
 */
test("a turn before the run is opened records and attempts nothing", async () => {
  const failing = await gateway(() => ({ status: 503, body: {} }));
  const path = transcript([entry("u1", "the ask")]);
  try {
    await turnHook(
      { hook_event_name: "Stop", session_id: "f3b", transcript_path: path },
      config(failing.url),
    );
    const held = loadSpool("f3b");
    assert.ok(held);
    assert.equal(held.session_id, undefined);
    assert.equal(pending(held).length, 1, "the event is kept");
    assert.equal(
      held.entries[0]?.delivery_attempts,
      0,
      "no attempt is counted when there is no run to attempt against",
    );
  } finally {
    await failing.close();
  }
});

/**
 * The compaction case. `PreCompact` runs while the transcript is rewritten, so
 * recording there is what stops a compaction eating a turn — and the events
 * survive even though the gateway never answered.
 */
test("a precompact records before the transcript can be rewritten", async () => {
  const failing = await gateway(() => ({ status: 503, body: {} }));
  const path = transcript([entry("u1", "before the compaction")]);
  try {
    await turnHook(
      { hook_event_name: "PreCompact", session_id: "f4", transcript_path: path },
      config(failing.url),
    );
    // The transcript is now rewritten out from under us.
    writeFileSync(path, "");
    const spool = loadSpool("f4");
    assert.ok(spool);
    assert.equal(spool.entries.length, 1);
    assert.equal(spool.entries[0]?.client_event_id, "u1");
  } finally {
    await failing.close();
  }
});

test("a session end flushes and closes the run", async () => {
  const mock = await gateway(ok);
  const path = transcript([entry("u1", "the ask")]);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "f5", source: "startup", transcript_path: path },
      config(mock.url, { inject: false }),
    );
    await turnHook(
      { hook_event_name: "SessionEnd", session_id: "f5", transcript_path: path, reason: "user exited" },
      config(mock.url),
    );
    const close = mock.requests.find((request) => request.path.endsWith("/end"));
    assert.ok(close);
    assert.equal(close.body.status, "ended");
    assert.equal(close.body.end_reason, "user exited");
  } finally {
    await mock.close();
  }
});

/**
 * The two-phase close (ADR-0076). A `SessionEnd` that could not drain says
 * `ending` — not `ended` — and leaves the close owed, so the run is not
 * reported as finished over a backlog.
 */
test("a session end that cannot drain says ending and owes a close", async () => {
  const mock = await gateway(
    script((request) =>
      request.path.endsWith("/events") ? { status: 503, body: {} } : undefined,
    ),
  );
  const path = transcript([entry("u1", "the ask")]);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "f6", source: "startup", transcript_path: path },
      config(mock.url, { inject: false }),
    );
    await turnHook(
      { hook_event_name: "SessionEnd", session_id: "f6", transcript_path: path },
      config(mock.url),
    );
    const close = mock.requests.find((request) => request.path.endsWith("/end"));
    assert.equal(close?.body.status, "ending");
    const spool = loadSpool("f6");
    assert.ok(spool);
    assert.equal(spool.close_requested, true);
    assert.equal(pending(spool).length, 1);
  } finally {
    await mock.close();
  }
});

/**
 * Every terminal answer acknowledges. A denied event that stayed pending would
 * be retried forever and the spool would never drain.
 */
test("a denied event is acknowledged and not retried", async () => {
  const mock = await gateway(
    script((request) =>
      request.path.endsWith("/events")
        ? {
            status: 200,
            body: {
              events: [{ outcome: "denied", client_event_id: "u1" }],
              appended: 0,
              duplicates: 0,
              quarantined: 0,
              denied: 1,
            },
          }
        : undefined,
    ),
  );
  const path = transcript([entry("u1", "a secret slipped in")]);
  try {
    await sessionStart(
      { hook_event_name: "SessionStart", session_id: "f7", source: "startup", transcript_path: path },
      config(mock.url, { inject: false }),
    );
    await turnHook(
      { hook_event_name: "Stop", session_id: "f7", transcript_path: path },
      config(mock.url),
    );
    const spool = loadSpool("f7");
    assert.ok(spool);
    assert.equal(pending(spool).length, 0);
    assert.equal(spool.entries[0]?.outcome, "denied");
  } finally {
    await mock.close();
  }
});

test("the model a session start reported rides every recorded event", async () => {
  const mock = await gateway(ok);
  const path = transcript([entry("u1", "the ask")]);
  try {
    // Only `SessionStart` carries a model; `Stop` does not, which is why the
    // spool has to bring it across (ADR-0027 decision 8).
    await sessionStart(
      {
        hook_event_name: "SessionStart",
        session_id: "f8",
        source: "startup",
        transcript_path: path,
        model: "claude-opus-5",
      },
      config(mock.url, { inject: false }),
    );
    const append = mock.requests.find((request) => request.path.endsWith("/events"));
    const events = append?.body.events as { payload: { context?: { model?: string } } }[];
    assert.equal(events[0]?.payload.context?.model, "claude-opus-5");
  } finally {
    await mock.close();
  }
});

test("observe turned off in a project records and posts nothing", async () => {
  const mock = await gateway(ok);
  const path = transcript([entry("u1", "the ask")]);
  try {
    await turnHook(
      { hook_event_name: "Stop", session_id: "f9", transcript_path: path },
      config(mock.url, { observe: false }),
    );
    assert.equal(mock.requests.length, 0);
    assert.equal(loadSpool("f9"), undefined);
  } finally {
    await mock.close();
  }
});

// ── The entry point ──────────────────────────────────────────────────────

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
  const mock = await gateway(ok);
  try {
    const result = await runHook(
      "session-start",
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "e1", source: "startup" }),
      {
        SYNVEDA_GATEWAY: mock.url,
        XDG_STATE_HOME: stateHome,
        SYNVEDA_TOKEN: "dev-bearer",
        SYNVEDA_WORKSPACE: "11111111-1111-1111-1111-111111111111",
      },
    );
    assert.equal(result.code, 0);
    const parsed: unknown = JSON.parse(result.stdout);
    assert.match(
      (parsed as { hookSpecificOutput?: { additionalContext?: string } }).hookSpecificOutput
        ?.additionalContext ?? "",
      /memory/,
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
  const mock = await gateway(ok);
  try {
    // The mode argument would be enough to dispatch on, and that is the trap:
    // with no payload there is no session to name and no `cwd` to read the
    // project's opt-out from (ADR-0027 decision 13).
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
  const mock = await gateway(ok);
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

/**
 * The pre-cut hook arguments named a plane that no longer exists. A stale
 * `hooks.json` still passing one must do nothing rather than guess (ADR-0078
 * decision 7) — a guess would be a hook writing a turn under the wrong
 * dispatch.
 */
test("a pre-cut hook argument is not honoured", async () => {
  const mock = await gateway(ok);
  try {
    for (const stale of ["observe", "flush", "inject"]) {
      const result = await runHook(stale, JSON.stringify({ session_id: "e6" }), {
        SYNVEDA_GATEWAY: mock.url,
        XDG_STATE_HOME: stateHome,
        SYNVEDA_TOKEN: "dev-bearer",
      });
      assert.equal(result.code, 0, `${stale} must still exit 0`);
    }
    assert.equal(mock.requests.length, 0, "no stale argument may reach the gateway");
  } finally {
    await mock.close();
  }
});
