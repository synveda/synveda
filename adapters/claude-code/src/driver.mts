#!/usr/bin/env node
/**
 * The recorded-payload driver (ADR-0027 decision 14).
 *
 * It feeds recorded Claude Code hook JSON to the built entry point as a
 * child process — the very `node dist/hook.mjs <mode>` line `hooks.json`
 * registers — and checks what the harness would have seen. No harness is
 * involved and none is needed: a hook is a process that reads JSON on
 * stdin and writes JSON on stdout, so a recording of one is a complete
 * test input.
 *
 * The invariant every case shares is decision 3: whatever the gateway
 * does — refuse, hang up, degrade, 401 — the hook exits 0 and the session
 * continues. A case that fails that fails outright; a case may then
 * assert whatever else it is about.
 *
 * Two modes, one set of cases:
 *
 *     node dist/driver.mjs                                   # mock gateway
 *     node dist/driver.mjs --gateway URL --token BEARER      # live gateway
 *
 * The mock proves the contract; the live run proves the contract is the
 * one the product implements. `demos/adpt-1-claude-code.sh` runs the live
 * half after its timed section, and `driver.test.mts` runs the mock half
 * in the unit suite. A handful of cases can only be posed to a mock (a
 * degradation header is a server-side condition, not a client request);
 * they say so and are reported as skipped rather than quietly dropped.
 */

import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  copyFileSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { startGateway, type RecordedRequest, type Reply, type Responder } from "./mock-gateway.mjs";
import { ensureDir } from "./paths.mjs";
import { qualifiedSessionId } from "./session-start.mjs";
import type { SessionState } from "./spool.mjs";

const HOOK = fileURLToPath(new URL("./hook.mjs", import.meta.url));
const FIXTURES = new URL("../fixtures/", import.meta.url);

/** Nothing listens on port 1, and the connection is refused rather than hanging. */
const DEAD_GATEWAY = "http://127.0.0.1:1";

/** Comfortably over the 64 KiB per-event cap, so truncation has to happen. */
const OVERSIZED_CHARS = 200_000;

type Mode = "session-start" | "observe" | "flush";

interface HookRun {
  code: number | null;
  stdout: string;
  stderr: string;
}

/** What a hook prints on stdout, once parsed. */
interface HookOutput {
  systemMessage?: string;
  hookSpecificOutput?: { hookEventName?: string; additionalContext?: string };
}

export interface DriverOptions {
  /** A live gateway base URL; omitted for mock mode. */
  gateway?: string;
  /** The bearer to present. Required in live mode. */
  token?: string;
  /**
   * Require the inject cases to come back with context. The demo sets it
   * because it seeded memory the caller can read; a bare gateway may
   * legitimately have nothing to say.
   */
  expectContext?: boolean;
  /** Where progress goes. Defaults to stderr, so stdout stays parseable. */
  report?: (line: string) => void;
}

export interface DriverReport {
  passed: number;
  failed: number;
  skipped: number;
  failures: { name: string; reason: string }[];
}

/** Everything a case is handed: its scratch machine, and its gateway. */
interface Case {
  readonly live: boolean;
  readonly url: string;
  readonly token: string;
  /** What the mock gateway received. Empty in live mode. */
  readonly requests: RecordedRequest[];
  readonly stateHome: string;
  readonly project: string;
  readonly expectContext: boolean;
  hook(mode: Mode, payload: unknown, overrides?: Record<string, string>): Promise<HookRun>;
  /** A recorded hook payload, repointed at this case's scratch machine. */
  recorded(name: string, patch?: Record<string, unknown>): Record<string, unknown>;
  /** A recorded transcript, copied into the scratch. */
  transcript(fixture: string): string;
  /** A synthesised transcript, for shapes no recording sensibly holds. */
  synthesise(name: string, entries: unknown[]): string;
  spool(sessionId: string): SessionState | undefined;
  forgetCursor(sessionId: string): void;
  logged(event: string): Record<string, unknown>[];
}

interface Scenario {
  readonly name: string;
  /** Set when the case can only be posed to a mock, and why. */
  readonly mockOnly?: string;
  /** How the mock answers. Unused in live mode. */
  readonly respond?: Responder;
  run(subject: Case): Promise<void>;
}

// ── Canned gateway replies ───────────────────────────────────────────────────

function block(text: string, records: string[] = ["0198e4c1-0000-7000-8000-00000000ab01"]): unknown {
  return {
    text,
    block_hash: "b3-2f1c9d0e",
    record_ids: records,
    tokens: 128,
    budget_tokens: 1500,
    as_of: "2026-07-25T09:00:00.000Z",
    degraded: [],
  };
}

/** The mock's inject leg, with an observe leg that answers nothing useful. */
function injects(reply: (index: number) => Reply): Responder {
  return (request, index) =>
    request.path === "/v1/inject"
      ? reply(index)
      : { status: 200, body: buffered(request, new Set()) };
}

/**
 * MEM-1's idempotency, mocked: a key seen before is a duplicate and is not
 * re-enqueued (ADR-0020 decision 2). This is the property the whole cursor
 * design rests on, so the mock has to have it or the replay case proves
 * nothing.
 */
function buffer(): Responder {
  const seen = new Set<string>();
  return (request) => ({ status: 200, body: buffered(request, seen) });
}

function buffered(request: RecordedRequest, seen: Set<string>): unknown {
  const events = Array.isArray(request.body.events) ? request.body.events : [];
  let accepted = 0;
  let duplicates = 0;
  for (const event of events) {
    const key = String((event as { idempotency_key?: unknown }).idempotency_key);
    if (seen.has(key)) {
      duplicates += 1;
    } else {
      seen.add(key);
      accepted += 1;
    }
  }
  return {
    session_id: request.body.session_id,
    accepted,
    duplicates,
    quarantined: 0,
    denied: 0,
    events: [],
  };
}

// ── The cases ────────────────────────────────────────────────────────────────

const SCENARIOS: Scenario[] = [
  {
    name: "a recorded session start injects the block as additionalContext",
    respond: injects(() => ({ status: 200, body: block("# Core team\n- deploys go through make deploy") })),
    async run(subject) {
      const payload = subject.recorded("session-start-startup");
      const run = await subject.hook("session-start", payload);
      exits(run);
      const output = parse(run);
      if (subject.live && !subject.expectContext) return;
      expect(
        (output.hookSpecificOutput?.additionalContext ?? "").length > 0,
        "the session received no context",
      );
      expect(
        output.hookSpecificOutput?.hookEventName === "SessionStart",
        "additionalContext must be tagged with its hook event",
      );
      if (subject.live) return;
      const request = subject.requests[0];
      expect(request?.path === "/v1/inject", `posted to ${String(request?.path)}`);
      expect(
        request?.body.session_id === qualifiedSessionId(String(payload.session_id)),
        "the audit correlation must ride the request (decision 10)",
      );
      expect(request?.body.task === undefined, "a cold start has no task to send");
      expect(
        request?.authorization === `Bearer ${subject.token}`,
        "the caller's own bearer must be presented, and nothing else",
      );
    },
  },
  {
    name: "a recorded compact start carries the last prompt as its task",
    respond: injects(() => ({ status: 200, body: block("# Core team\n- deploys go through make deploy") })),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const run = await subject.hook(
        "session-start",
        subject.recorded("session-start-compact", { transcript_path: transcript }),
      );
      exits(run);
      if (subject.live) return;
      expect(
        String(subject.requests[0]?.body.task ?? "").startsWith("Give the payments client"),
        "post-compaction inject must carry the last real prompt (decision 11)",
      );
    },
  },
  {
    name: "an unreachable gateway costs the session nothing",
    async run(subject) {
      const run = await subject.hook("session-start", subject.recorded("session-start-startup"), {
        SYNVEDA_GATEWAY: DEAD_GATEWAY,
      });
      exits(run);
      expect(run.stdout === "", `a dead gateway must contribute nothing, got: ${run.stdout}`);
    },
  },
  {
    name: "a rejected credential is the one failure the user is told about",
    respond: () => ({ status: 401, body: { message: "unauthenticated" } }),
    async run(subject) {
      // Live: a token the gateway will refuse. Mock: a gateway that
      // refuses whatever it is given. Same 401, same expectation.
      const run = await subject.hook("session-start", subject.recorded("session-start-startup"), {
        SYNVEDA_TOKEN: "not-a-token-this-gateway-will-accept",
      });
      exits(run);
      const output = parse(run);
      expect(
        /synveda login/.test(output.systemMessage ?? ""),
        "an expired login is the one thing the user can act on",
      );
      expect(
        output.hookSpecificOutput === undefined,
        "a refused inject must contribute no context",
      );
    },
  },
  {
    name: "a degraded inject still delivers context and stays silent",
    mockOnly: "degradation is a server-side condition; CTX-3's demo stops TEI to produce it live",
    respond: injects(() => ({
      status: 200,
      body: { ...(block("# Core team\n- deploys go through make deploy") as object), degraded: ["embedder"] },
      headers: { "x-synveda-degraded": "embedder" },
    })),
    async run(subject) {
      const run = await subject.hook("session-start", subject.recorded("session-start-startup"));
      exits(run);
      const output = parse(run);
      expect(
        (output.hookSpecificOutput?.additionalContext ?? "").length > 0,
        "a degraded inject still delivers context",
      );
      expect(
        !/synveda login/.test(output.systemMessage ?? ""),
        "a degradation is recorded server-side; the user is not asked to do anything",
      );
    },
  },
  {
    name: "the first session in a project discloses what is sent, exactly once",
    respond: injects(() => ({ status: 200, body: block("# Core team\n- deploys go through make deploy") })),
    async run(subject) {
      const first = parse(exits(await subject.hook("session-start", subject.recorded("session-start-startup"))));
      const second = parse(exits(await subject.hook("session-start", subject.recorded("session-start-startup"))));
      expect(
        /transcripts are sent to/.test(first.systemMessage ?? ""),
        "capture must be disclosed on the first session in a project (decision 13)",
      );
      expect(
        second.systemMessage === undefined,
        "the disclosure is once per project, not once per session",
      );
    },
  },
  {
    name: "a recorded stop flush posts the turn and advances the cursor",
    respond: buffer(),
    async run(subject) {
      const payload = subject.recorded("stop", { transcript_path: subject.transcript("turn.jsonl") });
      exits(await subject.hook("observe", payload));
      const session = qualifiedSessionId(String(payload.session_id));
      expect(
        subject.spool(session)?.cursor === "11111111-0000-4000-8000-000000000004",
        "the cursor must sit on the last accepted entry",
      );
      const done = subject.logged("observe.done").at(-1);
      expect(done?.events === 3, `meta and sidechain entries must be skipped, sent ${String(done?.events)}`);
      if (subject.live) return;
      const kinds = (subject.requests[0]?.body.events as { kind: string }[]).map((event) => event.kind);
      expect(
        kinds.join(",") === "transcript_delta,transcript_delta,tool_result",
        `unexpected event kinds: ${kinds.join(",")}`,
      );
    },
  },
  {
    name: "an oversized tool result is truncated, never dropped",
    respond: buffer(),
    async run(subject) {
      const transcript = subject.synthesise("oversized.jsonl", [
        {
          type: "user",
          uuid: "33333333-0000-4000-8000-000000000001",
          timestamp: "2026-07-25T09:30:00.000Z",
          isSidechain: false,
          cwd: subject.project,
          gitBranch: "feat/retry-budget",
          message: {
            role: "user",
            content: [
              {
                tool_use_id: "toolu_01Oversized",
                type: "tool_result",
                content: `cargo test --workspace\n${"a passing test line that nobody will read\n".repeat(OVERSIZED_CHARS / 42)}`,
                is_error: false,
              },
            ],
          },
        },
      ]);
      const payload = subject.recorded("stop", { transcript_path: transcript });
      exits(await subject.hook("observe", payload));
      const session = qualifiedSessionId(String(payload.session_id));
      expect(
        subject.spool(session)?.cursor === "33333333-0000-4000-8000-000000000001",
        "an oversized event must still be accepted, not silently dropped",
      );
      if (subject.live) return;
      const events = subject.requests[0]?.body.events as { payload: { truncated?: boolean } }[];
      expect(events.length === 1, `expected one event, got ${String(events.length)}`);
      const bytes = Buffer.byteLength(JSON.stringify(events[0]?.payload), "utf8");
      expect(bytes <= 64 * 1024, `payload is ${String(bytes)} bytes, over the 64 KiB cap`);
      expect(events[0]?.payload.truncated === true, "a truncated payload must say so (decision 8)");
    },
  },
  {
    name: "a replayed batch is reported as duplicates and re-enqueued nowhere",
    respond: buffer(),
    async run(subject) {
      const payload = subject.recorded("stop", { transcript_path: subject.transcript("turn.jsonl") });
      const session = qualifiedSessionId(String(payload.session_id));
      exits(await subject.hook("observe", payload));
      // A machine that lost its spool — or any of the half-dozen ways a
      // cursor can be behind what the gateway already holds.
      subject.forgetCursor(session);
      exits(await subject.hook("observe", payload));
      const replay = subject.logged("observe.done").at(-1);
      expect(replay?.events === 3, "the whole delta must be resent when the cursor is gone");
      expect(
        replay?.duplicates === 3 && replay?.accepted === 0,
        `the replay must be all duplicates, got accepted=${String(replay?.accepted)} duplicates=${String(replay?.duplicates)}`,
      );
      expect(
        subject.spool(session)?.cursor === "11111111-0000-4000-8000-000000000004",
        "a duplicate batch still advances the cursor: the gateway holds it",
      );
    },
  },
  {
    name: "a failed flush leaves the cursor, and the next hook resends",
    respond: buffer(),
    async run(subject) {
      const payload = subject.recorded("stop", { transcript_path: subject.transcript("turn.jsonl") });
      const session = qualifiedSessionId(String(payload.session_id));
      exits(await subject.hook("observe", payload, { SYNVEDA_GATEWAY: DEAD_GATEWAY }));
      expect(
        subject.spool(session)?.cursor === undefined,
        "a failed flush must not advance the cursor",
      );
      exits(await subject.hook("observe", payload));
      expect(
        subject.spool(session)?.cursor === "11111111-0000-4000-8000-000000000004",
        "the recovery flush must resend and then advance",
      );
      const done = subject.logged("observe.done").at(-1);
      expect(done?.events === 3, "the recovery flush must resend the whole delta");
    },
  },
  {
    name: "a damaged transcript line is skipped, never fatal to the flush",
    respond: buffer(),
    async run(subject) {
      const payload = subject.recorded("stop", { transcript_path: subject.transcript("damaged.jsonl") });
      exits(await subject.hook("observe", payload));
      const done = subject.logged("observe.done").at(-1);
      expect(done?.events === 2, `expected the two readable entries, sent ${String(done?.events)}`);
      expect(
        subject.spool(qualifiedSessionId(String(payload.session_id)))?.cursor ===
          "22222222-0000-4000-8000-000000000003",
        "the readable entries either side of the damage must still land",
      );
    },
  },
  {
    name: "a recorded PreCompact flushes before the transcript is rewritten",
    respond: buffer(),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const payload = subject.recorded("pre-compact", { transcript_path: transcript });
      exits(await subject.hook("flush", payload));
      expect(
        subject.spool(qualifiedSessionId(String(payload.session_id)))?.cursor ===
          "11111111-0000-4000-8000-000000000004",
        "PreCompact must flush the turn the compaction is about to rewrite",
      );
    },
  },
  {
    name: "a PreCompact carrying no transcript path falls back to the spool",
    respond: buffer(),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      // The session start is what puts the path in the spool. A payload
      // without one is not what this harness build sends, and is exactly
      // what the fallback exists for: the payload is another program's
      // internal format, and the adapter must not need any given field.
      exits(
        await subject.hook(
          "session-start",
          subject.recorded("session-start-startup", { transcript_path: transcript }),
          { SYNVEDA_GATEWAY: DEAD_GATEWAY },
        ),
      );
      const payload = subject.recorded("pre-compact");
      delete payload.transcript_path;
      exits(await subject.hook("flush", payload));
      expect(
        subject.spool(qualifiedSessionId(String(payload.session_id)))?.cursor ===
          "11111111-0000-4000-8000-000000000004",
        "the spooled transcript path must carry a pathless flush",
      );
    },
  },
  {
    name: "a recorded SessionEnd flush closes the session out",
    respond: buffer(),
    async run(subject) {
      const payload = subject.recorded("session-end", {
        transcript_path: subject.transcript("turn.jsonl"),
      });
      exits(await subject.hook("flush", payload));
      expect(
        subject.spool(qualifiedSessionId(String(payload.session_id)))?.cursor ===
          "11111111-0000-4000-8000-000000000004",
        "the last flush of a session is where the final turn lands",
      );
    },
  },
  {
    name: "a payload the adapter cannot parse asks the gateway for nothing",
    // A reachable gateway is the point: the mode argument alone would be
    // enough to inject, and injecting for a payload this process cannot
    // read would mean composing for a session it cannot name, in a project
    // whose opt-out it cannot see.
    respond: injects(() => ({ status: 200, body: block("# should never be asked for") })),
    async run(subject) {
      const run = await subject.hook("session-start", "this is not the json you are looking for");
      exits(run);
      expect(run.stdout === "", `an unparseable payload must print nothing, got: ${run.stdout}`);
      if (subject.live) return;
      expect(
        subject.requests.length === 0,
        `the gateway was called ${String(subject.requests.length)} times for an unreadable payload`,
      );
    },
  },
  {
    name: "every recorded payload survives a gateway that answers an error",
    respond: () => ({ status: 500, body: { message: "boom" } }),
    async run(subject) {
      // Live: a path the gateway does not serve, which is a real non-2xx
      // from a real gateway rather than a mock's opinion of one.
      const overrides: Record<string, string> = subject.live
        ? { SYNVEDA_GATEWAY: `${subject.url}/no-such-prefix` }
        : {};
      const transcript = subject.transcript("turn.jsonl");
      for (const [fixture, mode] of [
        ["session-start-startup", "session-start"],
        ["session-start-compact", "session-start"],
        ["stop", "observe"],
        ["pre-compact", "flush"],
        ["session-end", "flush"],
      ] as [string, Mode][]) {
        const run = await subject.hook(
          mode,
          subject.recorded(fixture, { transcript_path: transcript }),
          overrides,
        );
        exits(run, fixture);
        expect(run.stdout === "", `${fixture} must contribute nothing on an error: ${run.stdout}`);
      }
    },
  },
];

// ── The runner ───────────────────────────────────────────────────────────────

export async function runDriver(options: DriverOptions = {}): Promise<DriverReport> {
  const report = options.report ?? ((line: string) => process.stderr.write(`${line}\n`));
  const live = options.gateway !== undefined;
  if (live && (options.token ?? "").length === 0) {
    throw new Error("a live run needs --token (or SYNVEDA_TOKEN): the adapter never mints one");
  }
  const root = mkdtempSync(join(tmpdir(), "synveda-driver-"));
  const result: DriverReport = { passed: 0, failed: 0, skipped: 0, failures: [] };

  report(
    live
      ? `recorded-payload driver: ${String(SCENARIOS.length)} cases against ${String(options.gateway)}`
      : `recorded-payload driver: ${String(SCENARIOS.length)} cases against a mock gateway`,
  );
  try {
    for (const scenario of SCENARIOS) {
      if (live && scenario.mockOnly !== undefined) {
        result.skipped += 1;
        report(`  – ${scenario.name}\n      skipped live: ${scenario.mockOnly}`);
        continue;
      }
      const mock =
        live || scenario.respond === undefined
          ? undefined
          : await startGateway(scenario.respond);
      const subject = makeCase(root, result.passed + result.failed, {
        live,
        url: options.gateway ?? mock?.url ?? DEAD_GATEWAY,
        token: options.token ?? "driver-bearer",
        requests: mock?.requests ?? [],
        expectContext: options.expectContext === true,
      });
      try {
        await scenario.run(subject);
        result.passed += 1;
        report(`  ✓ ${scenario.name}`);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        result.failed += 1;
        result.failures.push({ name: scenario.name, reason });
        report(`  ✗ ${scenario.name}\n      ${reason}`);
      } finally {
        await mock?.close();
      }
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }

  report(
    `${String(result.passed)} passed, ${String(result.failed)} failed, ${String(result.skipped)} skipped` +
      " — every case above ran the built hook as a child process, and every one exited 0",
  );
  return result;
}

interface CaseSetup {
  live: boolean;
  url: string;
  token: string;
  requests: RecordedRequest[];
  expectContext: boolean;
}

function makeCase(root: string, index: number, setup: CaseSetup): Case {
  const scratch = join(root, `case-${String(index).padStart(2, "0")}`);
  const stateHome = join(scratch, "state");
  const project = join(scratch, "project");
  const transcripts = join(scratch, "transcripts");
  // These are all under a scratch root this driver made itself, so none of
  // them can reach the hazard in `ensureDir`'s docstring. It is used anyway
  // so the adapter has exactly one way to make a directory.
  for (const dir of [stateHome, project, transcripts]) ensureDir(dir);

  const sessions = join(stateHome, "synveda", "sessions");

  return {
    ...setup,
    stateHome,
    project,

    async hook(mode, payload, overrides = {}) {
      const environment: Record<string, string> = {};
      for (const [key, value] of Object.entries(process.env)) {
        // Whatever this machine has configured is not what is under test:
        // an ambient SYNVEDA_DISABLED or a real login would quietly change
        // the answer.
        if (key.startsWith("SYNVEDA_") || key.startsWith("XDG_") || value === undefined) continue;
        environment[key] = value;
      }
      Object.assign(environment, {
        XDG_STATE_HOME: stateHome,
        SYNVEDA_GATEWAY: setup.url,
        SYNVEDA_TOKEN: setup.token,
        // The credential seam has its own suite; pin it away from any
        // `synveda` this machine happens to have installed.
        SYNVEDA_CLI: join(scratch, "no-cli-here"),
        ...overrides,
      });
      return run(mode, typeof payload === "string" ? payload : JSON.stringify(payload), environment);
    },

    recorded(name, patch = {}) {
      const raw = readFileSync(new URL(`hooks/${name}.json`, FIXTURES), "utf8");
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      // A recording points at the machine it was taken from; the replay
      // points at this case's scratch and nothing else.
      return { ...parsed, cwd: project, ...patch };
    },

    transcript(fixture) {
      const target = join(transcripts, fixture);
      copyFileSync(new URL(`transcripts/${fixture}`, FIXTURES), target);
      return target;
    },

    synthesise(name, entries) {
      const target = join(transcripts, name);
      writeFileSync(target, `${entries.map((entry) => JSON.stringify(entry)).join("\n")}\n`);
      return target;
    },

    spool(sessionId) {
      let names: string[];
      try {
        names = readdirSync(sessions);
      } catch {
        return undefined;
      }
      for (const name of names) {
        try {
          const state = JSON.parse(readFileSync(join(sessions, name), "utf8")) as SessionState;
          if (state.session_id === sessionId) return state;
        } catch {
          // A file mid-write, or something that is not ours.
        }
      }
      return undefined;
    },

    forgetCursor(sessionId) {
      for (const name of readdirSync(sessions)) {
        const file = join(sessions, name);
        const state = JSON.parse(readFileSync(file, "utf8")) as SessionState;
        if (state.session_id !== sessionId) continue;
        delete state.cursor;
        writeFileSync(file, JSON.stringify(state));
      }
    },

    logged(event) {
      let raw: string;
      try {
        raw = readFileSync(join(stateHome, "synveda", "adapter.log"), "utf8");
      } catch {
        return [];
      }
      const lines: Record<string, unknown>[] = [];
      for (const line of raw.split("\n")) {
        if (line.trim().length === 0) continue;
        try {
          const parsed = JSON.parse(line) as Record<string, unknown>;
          if (parsed.event === event) lines.push(parsed);
        } catch {
          // The log is diagnostics; a torn line is not a failure.
        }
      }
      return lines;
    },
  };
}

function run(mode: Mode, stdin: string, environment: Record<string, string>): Promise<HookRun> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [HOOK, mode], {
      env: environment,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (piece: unknown) => {
      stdout += String(piece);
    });
    child.stderr.on("data", (piece: unknown) => {
      stderr += String(piece);
    });
    child.on("error", reject);
    child.stdin.end(stdin);
    once(child, "exit")
      .then(([code]: unknown[]) => {
        resolve({ code: code as number | null, stdout, stderr });
      })
      .catch(reject);
  });
}

// ── Assertions ───────────────────────────────────────────────────────────────

/** The invariant of decision 3, asserted the same way in every case. */
function exits(run: HookRun, what = "the hook"): HookRun {
  expect(
    run.code === 0,
    `${what} exited ${String(run.code)}; a hook that fails a session is the one thing this adapter must never do` +
      (run.stderr.length > 0 ? ` (stderr: ${run.stderr.trim()})` : ""),
  );
  return run;
}

function parse(run: HookRun): HookOutput {
  if (run.stdout.length === 0) return {};
  try {
    const parsed: unknown = JSON.parse(run.stdout);
    if (parsed !== null && typeof parsed === "object") return parsed as HookOutput;
  } catch {
    throw new Error(`the harness would not parse this stdout: ${run.stdout}`);
  }
  throw new Error(`stdout is not a hook output object: ${run.stdout}`);
}

function expect(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

// ── Entry point ──────────────────────────────────────────────────────────────

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const options: DriverOptions = {};
  const argv = process.argv.slice(2);
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--gateway") options.gateway = argv[(index += 1)];
    else if (flag === "--token") options.token = argv[(index += 1)];
    else if (flag === "--expect-context") options.expectContext = true;
    else {
      process.stderr.write(`usage: driver.mjs [--gateway URL] [--token BEARER] [--expect-context]\n`);
      process.exit(2);
    }
  }
  if (options.gateway !== undefined && options.token === undefined) {
    options.token = process.env.SYNVEDA_TOKEN;
  }
  const report = await runDriver(options);
  process.exit(report.failed > 0 ? 1 : 0);
}
