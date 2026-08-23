#!/usr/bin/env node
/**
 * The recorded-payload driver (ADR-0027 decision 14).
 *
 * It feeds recorded Claude Code hook JSON to the built entry point as a child
 * process — the very `node dist/hook.mjs <mode>` line `hooks.json` registers —
 * and checks what the harness would have seen. No harness is involved and none
 * is needed: a hook is a process that reads JSON on stdin and writes JSON on
 * stdout, so a recording of one is a complete test input.
 *
 * The invariant every case shares is decision 3: whatever the gateway does —
 * refuse, hang up, degrade, 401 — the hook exits 0 and the session continues.
 * A case that fails that fails outright; a case may then assert whatever else
 * it is about.
 *
 * Since CPR-12 there is a second invariant with equal standing: **whatever the
 * gateway does, the events are on disk first**. So the failure cases here
 * assert what the spool holds, not only that nothing crashed — that is the
 * difference between the old design and this one, and a driver that only
 * checked exit codes would pass on both.
 *
 * Two modes, one set of cases:
 *
 *     node dist/driver.mjs                                   # mock gateway
 *     node dist/driver.mjs --gateway URL --token BEARER      # live gateway
 *
 * The mock proves the contract; the live run proves the contract is the one
 * the product implements. `demos/adpt-1-claude-code.sh` runs the live half
 * after its timed section, and `driver.test.mts` runs the mock half in the
 * unit suite. A handful of cases can only be posed to a mock (a degradation
 * header is a server-side condition, not a client request); they say so and
 * are reported as skipped rather than quietly dropped.
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
import type { Spool } from "./spool.mjs";

const HOOK = fileURLToPath(new URL("./hook.mjs", import.meta.url));
const FIXTURES = new URL("../fixtures/", import.meta.url);

/** Nothing listens on port 1, and the connection is refused rather than hanging. */
const DEAD_GATEWAY = "http://127.0.0.1:1";

/** A workspace id the mock accepts and a live run is given by the demo. */
const WORKSPACE = "0198e4c1-0000-7000-8000-0000000000w1".replace("w", "a");

type Mode = "session-start" | "turn" | "skills";

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
  /** The workspace a live run opens sessions in. */
  workspace?: string;
  /** The project the live run is anchored to, when one is selected. */
  project?: string;
  /**
   * Require the composition cases to come back with context. The demo sets it
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
  readonly workspace: string;
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
  /** The spool for one harness session id. */
  spool(externalSessionId: string): Spool | undefined;
  logged(event: string): Record<string, unknown>[];
}

/** The case, plus the one mutation a replay scenario needs. */
type DriverCase = Case & {
  rewriteSpool(externalSessionId: string, mutate: (spool: Spool) => void): void;
};

interface Scenario {
  readonly name: string;
  /** Set when the case can only be posed to a mock, and why. */
  readonly mockOnly?: string;
  /** How the mock answers. Unused in live mode. */
  readonly respond?: Responder;
  run(subject: DriverCase): Promise<void>;
}

// ── Canned gateway replies ───────────────────────────────────────────────────

const RUN_ID = "0198e4c1-0000-7000-8000-00000000ru01".replace("ru", "aa");

function session(status = "active"): unknown {
  return { id: RUN_ID, workspace_id: WORKSPACE, status };
}

function block(rendered: string, entries = 1): unknown {
  return {
    id: "0198e4c1-0000-7000-8000-00000000cc01".replace("cc", "bb"),
    rendered,
    block_hash: "b3-2f1c9d0e",
    tokens: 128,
    budget_tokens: 1500,
    entry_count: entries,
    degraded: [],
    created_at: "2026-08-25T09:00:00.000Z",
  };
}

/**
 * The append route's idempotency, mocked: an event id seen before comes back
 * `duplicate` rather than being appended twice.
 *
 * This is the property the whole redelivery design rests on, so the mock has
 * to have it or the replay case proves nothing.
 */
function appendLeg(): (request: RecordedRequest) => unknown {
  const seen = new Set<string>();
  return (request) => {
    const events = Array.isArray(request.body.events) ? request.body.events : [];
    const outcomes = events.map((event) => {
      const id = String((event as { client_event_id?: unknown }).client_event_id);
      const duplicate = seen.has(id);
      seen.add(id);
      return { outcome: duplicate ? "duplicate" : "appended", client_event_id: id };
    });
    return {
      events: outcomes,
      appended: outcomes.filter((outcome) => outcome.outcome === "appended").length,
      duplicates: outcomes.filter((outcome) => outcome.outcome === "duplicate").length,
      quarantined: 0,
      denied: 0,
    };
  };
}

/**
 * The default script. Cases override one leg at a time, so the thing under
 * test is the only thing that differs from a working deployment.
 */
function gatewayScript(
  overrides: (request: RecordedRequest, index: number) => Reply | undefined = () => undefined,
): Responder {
  const append = appendLeg();
  return (request, index) => {
    const override = overrides(request, index);
    if (override !== undefined) return override;
    if (request.path === "/v1/me") {
      return { status: 200, body: { workspaces: [{ id: WORKSPACE, name: "driver" }] } };
    }
    if (request.path === "/v1/sessions") return { status: 201, body: session() };
    if (request.path.endsWith("/context-runs")) return { status: 200, body: block("# memory") };
    if (request.path.endsWith("/events")) return { status: 200, body: append(request) };
    if (request.path.endsWith("/end")) return { status: 200, body: session("ended") };
    return { status: 404, body: { message: "no such route" } };
  };
}

// ── The cases ────────────────────────────────────────────────────────────────

const SCENARIOS: Scenario[] = [
  {
    name: "a recorded session start opens a run and injects the block",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const payload = subject.recorded("session-start-startup");
      const run = exits(await subject.hook("session-start", payload));
      const output = parse(run);
      if (subject.expectContext) {
        expect(
          (output.hookSpecificOutput?.additionalContext ?? "").length > 0,
          "a session start with memory to serve must contribute context",
        );
        expect(
          output.hookSpecificOutput?.hookEventName === "SessionStart",
          "the context must be labelled for the hook that emitted it",
        );
      }
      if (subject.live) return;
      const open = subject.requests.find((request) => request.path === "/v1/sessions");
      expect(open !== undefined, "the run must be opened before anything else");
      expect(
        open?.body.external_session_id === String(payload.session_id),
        "the harness session id is what makes opening idempotent",
      );
      expect(
        open?.idempotencyKey === `cc-open-${String(payload.session_id)}`,
        `open carried ${String(open?.idempotencyKey)} rather than a key derived from the harness id`,
      );
      expect(
        subject.requests.some((request) => request.path.endsWith("/context-runs")),
        "the block is composed through the session's own context-run endpoint",
      );
    },
  },
  {
    name: "a recorded post-compaction start carries the last prompt as its query",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const payload = subject.recorded("session-start-compact", {
        transcript_path: subject.transcript("turn.jsonl"),
      });
      exits(await subject.hook("session-start", payload));
      if (subject.live) return;
      const compose = subject.requests.find((request) => request.path.endsWith("/context-runs"));
      expect(
        typeof compose?.body.query === "string" && (compose.body.query as string).length > 0,
        "post-compaction composition must carry the last real prompt (decision 11)",
      );
    },
  },
  {
    name: "a recorded stop records the turn and delivers it",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const start = subject.recorded("session-start-startup", { transcript_path: transcript });
      exits(await subject.hook("session-start", start));
      const payload = subject.recorded("stop", { transcript_path: transcript });
      exits(await subject.hook("turn", payload));
      const spool = subject.spool(String(payload.session_id));
      expect(spool !== undefined, "a stop must leave a spool behind");
      expect(
        (spool?.entries.length ?? 0) > 0,
        "the turn must be recorded as events, not merely posted",
      );
      expect(
        (spool?.entries ?? []).every((entry) => entry.acknowledged),
        "a reachable gateway must leave nothing pending",
      );
    },
  },
  {
    name: "a dead gateway keeps the turn on disk rather than losing it",
    // The whole point of the feature, and the case the previous design could
    // not have passed: the events existed only as a byte range of the
    // harness's transcript file, which nothing had copied.
    respond: gatewayScript(() => undefined),
    async run(subject) {
      // A run first, against the reachable gateway, so what fails below is the
      // *delivery* rather than there being nowhere to deliver to. The
      // transcript starts empty so the start has nothing to deliver and the
      // turn below is genuinely new work.
      const transcript = subject.synthesise("growing.jsonl", []);
      const start = subject.recorded("session-start-startup", { transcript_path: transcript });
      exits(await subject.hook("session-start", start));

      // The turn the client just had, arriving after the run was opened.
      subject.synthesise("growing.jsonl", [
        {
          type: "user",
          uuid: "b9f0e1a2-0000-4000-8000-00000000d001",
          timestamp: "2026-08-25T09:05:00.000Z",
          message: { role: "user", content: "the ask nobody delivered" },
        },
      ]);
      const payload = subject.recorded("stop", {
        session_id: start.session_id,
        transcript_path: transcript,
      });
      exits(
        await subject.hook("turn", payload, { SYNVEDA_GATEWAY: DEAD_GATEWAY }),
        "a stop against a dead gateway",
      );
      const spool = subject.spool(String(payload.session_id));
      expect(spool !== undefined, "a failed delivery must still leave a spool");
      expect(
        (spool?.entries.length ?? 0) > 0,
        "the events must be recorded even though nothing was delivered",
      );
      expect(
        (spool?.entries ?? []).some((entry) => !entry.acknowledged),
        "nothing may be marked delivered when the gateway was never reached",
      );
      expect(
        (spool?.entries ?? []).some((entry) => entry.delivery_attempts >= 1),
        "the attempt must be counted so `spool status` can report it",
      );
    },
  },
  {
    name: "the next session start delivers what a dead gateway left behind",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const stop = subject.recorded("stop", { transcript_path: transcript });
      exits(await subject.hook("turn", stop, { SYNVEDA_GATEWAY: DEAD_GATEWAY }));
      const held = subject.spool(String(stop.session_id));
      expect(
        (held?.entries ?? []).some((entry) => !entry.acknowledged),
        "precondition: the events are held",
      );

      const start = subject.recorded("session-start-startup", {
        session_id: stop.session_id,
        transcript_path: transcript,
      });
      exits(await subject.hook("session-start", start));
      const after = subject.spool(String(stop.session_id));
      expect(
        (after?.entries ?? []).every((entry) => entry.acknowledged),
        "the backlog must be delivered once a gateway is reachable again",
      );
    },
  },
  {
    name: "a redelivered event is answered duplicate and is not appended twice",
    mockOnly: "asserting the duplicate count needs a gateway whose whole history is visible",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const start = subject.recorded("session-start-startup", { transcript_path: transcript });
      exits(await subject.hook("session-start", start));
      const stop = subject.recorded("stop", { transcript_path: transcript });
      exits(await subject.hook("turn", stop));

      // Forget the delivery state, keeping the recording: exactly what a
      // spool restored from a backup, or a crash between the append and the
      // spool write, produces.
      const spool = subject.spool(String(stop.session_id));
      expect(spool !== undefined, "precondition: a spool exists");
      subject.rewriteSpool(String(stop.session_id), (held) => {
        for (const entry of held.entries) {
          entry.acknowledged = false;
          delete entry.outcome;
        }
      });

      exits(await subject.hook("turn", subject.recorded("stop", { transcript_path: transcript })));
      const appends = subject.requests.filter((request) => request.path.endsWith("/events"));
      const last = appends.at(-1);
      const outcomes = ((last?.body.events ?? []) as unknown[]).length;
      expect(outcomes > 0, "the redelivery must actually resend the events");
      const replayed = subject.spool(String(stop.session_id));
      expect(
        (replayed?.entries ?? []).every((entry) => entry.outcome === "duplicate"),
        "a redelivered event must come back as a duplicate rather than appending twice",
      );
    },
  },
  {
    name: "a precompact records before the transcript can be rewritten",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const start = subject.recorded("session-start-startup", { transcript_path: transcript });
      exits(await subject.hook("session-start", start, { SYNVEDA_GATEWAY: DEAD_GATEWAY }));
      const payload = subject.recorded("pre-compact", { transcript_path: transcript });
      exits(await subject.hook("turn", payload, { SYNVEDA_GATEWAY: DEAD_GATEWAY }));
      // Compaction now rewrites the transcript out from under the adapter.
      writeFileSync(transcript, "");
      const spool = subject.spool(String(payload.session_id));
      expect(
        (spool?.entries.length ?? 0) > 0,
        "a compaction must not be able to eat a turn the adapter was handed",
      );
    },
  },
  {
    name: "a recorded SessionEnd flushes and closes the run",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const start = subject.recorded("session-start-startup", { transcript_path: transcript });
      exits(await subject.hook("session-start", start));
      const payload = subject.recorded("session-end", { transcript_path: transcript });
      exits(await subject.hook("turn", payload));
      if (subject.live) return;
      const close = subject.requests.find((request) => request.path.endsWith("/end"));
      expect(close !== undefined, "the last hook of a run must close it");
      expect(
        close?.body.status === "ended",
        `a drained close must say ended, said ${String(close?.body.status)}`,
      );
    },
  },
  {
    name: "a SessionEnd that cannot drain says ending and owes the close",
    mockOnly: "a gateway that accepts a close and refuses an append is a scripted condition",
    respond: gatewayScript((request) =>
      request.path.endsWith("/events") ? { status: 503, body: { message: "unavailable" } } : undefined,
    ),
    async run(subject) {
      const transcript = subject.transcript("turn.jsonl");
      const start = subject.recorded("session-start-startup", { transcript_path: transcript });
      exits(await subject.hook("session-start", start));
      const payload = subject.recorded("session-end", { transcript_path: transcript });
      exits(await subject.hook("turn", payload));
      const close = subject.requests.find((request) => request.path.endsWith("/end"));
      expect(
        close?.body.status === "ending",
        `a close over a backlog must say ending, said ${String(close?.body.status)}`,
      );
      const spool = subject.spool(String(payload.session_id));
      expect(
        spool?.close_requested === true,
        "the close must be recorded as owed so a later flush can finish it",
      );
    },
  },
  {
    name: "a degraded composition still delivers context and stays silent",
    mockOnly: "a degradation header is a server-side condition, not a client request",
    respond: gatewayScript((request) =>
      request.path.endsWith("/context-runs")
        ? {
            status: 200,
            body: { ...(block("# still ranked") as Record<string, unknown>), degraded: ["embedder"] },
            headers: { "x-synveda-degraded": "embedder" },
          }
        : undefined,
    ),
    async run(subject) {
      // Twice, and the second run is the one that asserts silence. The first
      // start in a project also carries the capture disclosure (ADR-0027
      // decision 13), which is a different message for a different reason —
      // asserting `systemMessage === undefined` on the first run would be
      // testing the disclosure, not the degradation.
      const payload = subject.recorded("session-start-startup");
      const first = parse(exits(await subject.hook("session-start", payload)));
      expect(
        (first.systemMessage ?? "").includes("Synveda is active"),
        "the first start in a project discloses that it is capturing",
      );

      const run = exits(await subject.hook("session-start", payload));
      const output = parse(run);
      expect(
        (output.hookSpecificOutput?.additionalContext ?? "").includes("still ranked"),
        "a degraded composition still delivers context",
      );
      expect(
        output.systemMessage === undefined,
        "a degradation is recorded in the audit event and the metrics, not said to the user",
      );
    },
  },
  {
    name: "a refused composition contributes no context and never fails the session",
    respond: gatewayScript((request) =>
      request.path.endsWith("/context-runs") ? { status: 403, body: { message: "denied" } } : undefined,
    ),
    async run(subject) {
      const overrides: Record<string, string> = subject.live
        ? { SYNVEDA_GATEWAY: `${subject.url}/no-such-prefix` }
        : {};
      const run = exits(
        await subject.hook("session-start", subject.recorded("session-start-startup"), overrides),
      );
      const output = parse(run);
      expect(
        output.hookSpecificOutput === undefined,
        "a refused composition must contribute no context",
      );
    },
  },
  {
    name: "a payload the adapter cannot parse asks the gateway for nothing",
    // A reachable gateway is the point: the mode argument alone would be
    // enough to dispatch, and dispatching for a payload this process cannot
    // read would mean composing for a session it cannot name, in a project
    // whose opt-out it cannot see.
    respond: gatewayScript(() => undefined),
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
    name: "a pre-cut hook argument is not honoured",
    // `observe` and `flush` named a plane that no longer exists. A stale
    // hooks.json still passing one must do nothing rather than guess.
    respond: gatewayScript(() => undefined),
    async run(subject) {
      for (const stale of ["observe", "flush", "inject"]) {
        const run = await subject.hook(
          stale as Mode,
          subject.recorded("stop", { transcript_path: subject.transcript("turn.jsonl") }),
        );
        exits(run, `the stale argument ${stale}`);
      }
      if (subject.live) return;
      expect(
        subject.requests.length === 0,
        "no stale hook argument may reach the gateway",
      );
    },
  },
  {
    name: "every recorded payload survives a gateway that answers an error",
    respond: () => ({ status: 500, body: { message: "boom" } }),
    async run(subject) {
      // Live: a path the gateway does not serve, which is a real non-2xx from
      // a real gateway rather than a mock's opinion of one.
      const overrides: Record<string, string> = subject.live
        ? { SYNVEDA_GATEWAY: `${subject.url}/no-such-prefix` }
        : {};
      const transcript = subject.transcript("turn.jsonl");
      for (const [fixture, mode] of [
        ["session-start-startup", "session-start"],
        ["session-start-compact", "session-start"],
        ["stop", "turn"],
        ["pre-compact", "turn"],
        ["session-end", "turn"],
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
  {
    name: "a damaged transcript costs the session nothing",
    respond: gatewayScript(() => undefined),
    async run(subject) {
      const payload = subject.recorded("stop", {
        transcript_path: subject.transcript("damaged.jsonl"),
      });
      exits(await subject.hook("turn", payload));
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
  if (live && (options.workspace ?? "").length === 0) {
    throw new Error("a live run needs --workspace: a run happens in one");
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
        live || scenario.respond === undefined ? undefined : await startGateway(scenario.respond);
      const subject = makeCase(root, result.passed + result.failed + result.skipped, {
        live,
        url: options.gateway ?? mock?.url ?? DEAD_GATEWAY,
        token: options.token ?? "driver-bearer",
        workspace: options.workspace ?? WORKSPACE,
        projectId: options.project,
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
  workspace: string;
  projectId?: string;
  requests: RecordedRequest[];
  expectContext: boolean;
}

function makeCase(root: string, index: number, setup: CaseSetup): DriverCase {
  const scratch = join(root, `case-${String(index).padStart(2, "0")}`);
  const stateHome = join(scratch, "state");
  const configHome = join(scratch, "config");
  const project = join(scratch, "project");
  const transcripts = join(scratch, "transcripts");
  // These are all under a scratch root this driver made itself, so none of
  // them can reach the hazard in `ensureDir`'s docstring. It is used anyway so
  // the adapter has exactly one way to make a directory.
  for (const dir of [stateHome, configHome, project, transcripts]) ensureDir(dir);

  const spoolRoot = join(stateHome, "synveda", "spool");

  function spoolFiles(): { file: string; spool: Spool }[] {
    let names: string[];
    try {
      names = readdirSync(spoolRoot);
    } catch {
      return [];
    }
    const found: { file: string; spool: Spool }[] = [];
    for (const name of names) {
      if (!name.endsWith(".json")) continue;
      const file = join(spoolRoot, name);
      try {
        found.push({ file, spool: JSON.parse(readFileSync(file, "utf8")) as Spool });
      } catch {
        // A file mid-write, or something that is not ours.
      }
    }
    return found;
  }

  return {
    ...setup,
    stateHome,
    project,

    async hook(mode, payload, overrides = {}) {
      const environment: Record<string, string> = {};
      for (const [key, value] of Object.entries(process.env)) {
        // Whatever this machine has configured is not what is under test: an
        // ambient SYNVEDA_DISABLED or a real login would quietly change the
        // answer.
        if (key.startsWith("SYNVEDA_") || key.startsWith("XDG_") || value === undefined) continue;
        environment[key] = value;
      }
      Object.assign(environment, {
        XDG_STATE_HOME: stateHome,
        XDG_CONFIG_HOME: configHome,
        SYNVEDA_GATEWAY: setup.url,
        SYNVEDA_TOKEN: setup.token,
        SYNVEDA_WORKSPACE: setup.workspace,
        ...(setup.projectId === undefined ? {} : { SYNVEDA_PROJECT: setup.projectId }),
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
      // A recording points at the machine it was taken from; the replay points
      // at this case's scratch and nothing else.
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

    spool(externalSessionId) {
      return spoolFiles().find(({ spool }) => spool.external_session_id === externalSessionId)
        ?.spool;
    },

    rewriteSpool(externalSessionId, mutate) {
      for (const { file, spool } of spoolFiles()) {
        if (spool.external_session_id !== externalSessionId) continue;
        mutate(spool);
        writeFileSync(file, JSON.stringify(spool));
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
    else if (flag === "--workspace") options.workspace = argv[(index += 1)];
    else if (flag === "--project") options.project = argv[(index += 1)];
    else if (flag === "--expect-context") options.expectContext = true;
    else {
      process.stderr.write(
        "usage: driver.mjs [--gateway URL] [--token BEARER] [--workspace ID] [--project ID] [--expect-context]\n",
      );
      process.exit(2);
    }
  }
  if (options.gateway !== undefined && options.token === undefined) {
    options.token = process.env.SYNVEDA_TOKEN;
  }
  if (options.gateway !== undefined && options.workspace === undefined) {
    options.workspace = process.env.SYNVEDA_WORKSPACE;
  }
  if (options.gateway !== undefined && options.project === undefined) {
    options.project = process.env.SYNVEDA_PROJECT;
  }
  const report = await runDriver(options);
  process.exit(report.failed > 0 ? 1 : 0);
}
