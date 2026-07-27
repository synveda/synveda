/**
 * The MCP server, frame by frame (CTX-5, ADR-0042 decision 15).
 *
 * The protocol is written out rather than taken as a dependency
 * (ADR-0042 option 8), so it is this suite's job to prove the handshake
 * is real: a client that sends `initialize`, `tools/list` and
 * `tools/call` gets back what the spec says it should, and the odd frames
 * a client can legitimately send — a notification, a ping, an unknown
 * method, malformed JSON — are answered without the process dying.
 *
 * The other half is the posture inversion this feature decided
 * deliberately: a hook degrades in silence, but an agent that *asked* is
 * told what went wrong (decision 12). Every failure below reaches the
 * model as `isError` text it can read and act on, never as a dropped
 * connection.
 */

import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { startGateway, type RecordedRequest, type Reply } from "./mock-gateway.mjs";

const stateHome = mkdtempSync(join(tmpdir(), "synveda-mcp-"));
process.env.XDG_STATE_HOME = stateHome;
process.env.SYNVEDA_TOKEN = "dev-bearer";
// As in hook.test.mts: pin the credential seam away from whatever
// `synveda` this machine has, so these cases are about the protocol.
process.env.SYNVEDA_CLI = join(stateHome, "no-cli-here");

const { handleLine } = await import("./mcp.mjs");

interface Rpc {
  jsonrpc?: string;
  id?: unknown;
  result?: {
    content?: { type: string; text: string }[];
    isError?: boolean;
    tools?: { name: string; inputSchema: unknown }[];
    protocolVersion?: string;
    serverInfo?: { name: string };
    capabilities?: Record<string, unknown>;
  };
  error?: { code: number; message: string };
}

async function frame(request: unknown): Promise<Rpc | undefined> {
  return (await handleLine(JSON.stringify(request))) as Rpc | undefined;
}

/** The text an agent actually receives from a tool call. */
function text(response: Rpc | undefined): string {
  return response?.result?.content?.[0]?.text ?? "";
}

async function withGateway(
  respond: (recorded: RecordedRequest, index: number) => Reply,
  body: (requests: RecordedRequest[]) => Promise<void>,
): Promise<void> {
  const gateway = await startGateway(respond);
  const previous = process.env.SYNVEDA_GATEWAY;
  process.env.SYNVEDA_GATEWAY = gateway.url;
  try {
    await body(gateway.requests);
  } finally {
    if (previous === undefined) delete process.env.SYNVEDA_GATEWAY;
    else process.env.SYNVEDA_GATEWAY = previous;
    await gateway.close();
  }
}

function served(entries: Record<string, unknown>[], overrides: Record<string, unknown> = {}) {
  return {
    entries,
    mode: "query",
    requested: 32,
    as_of: "2026-07-27T10:00:00Z",
    valid_at: "2026-07-27T10:00:00Z",
    scopes_considered: 4,
    scopes_decided: 4,
    truncated: false,
    degraded: [],
    ...overrides,
  };
}

function entry(overrides: Record<string, unknown> = {}) {
  return {
    record_id: "0198f0a0-0000-7000-8000-000000000001",
    scope_id: "0198f0a0-0000-7000-8000-0000000000aa",
    channel: "published",
    kind: "derived",
    class: "procedure",
    sensitivity: "internal",
    content: "Rotate the payments key through the HSM runbook.",
    valid_from: "2026-01-01T00:00:00Z",
    valid_to: null,
    object_hash: "abc123def456",
    staleness_permille: 120,
    ...overrides,
  };
}

test("the handshake answers what the spec asks for", async () => {
  const initialized = await frame({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
  assert.equal(initialized?.jsonrpc, "2.0");
  assert.equal(initialized?.id, 1);
  assert.equal(initialized?.result?.protocolVersion, "2025-06-18");
  assert.equal(initialized?.result?.serverInfo?.name, "synveda");
  assert.ok(
    initialized?.result?.capabilities?.tools !== undefined,
    "a server offering a tool must declare the tools capability",
  );

  // A notification is answered with silence: the spec forbids a reply,
  // and a client receiving an id-less response may disconnect.
  assert.equal(await frame({ jsonrpc: "2.0", method: "notifications/initialized" }), undefined);
  const pinged = await frame({ jsonrpc: "2.0", id: 2, method: "ping" });
  assert.deepEqual(pinged?.result, {});
});

test("exactly one tool is offered, and it is recall", async () => {
  const listed = await frame({ jsonrpc: "2.0", id: 3, method: "tools/list" });
  const tools = listed?.result?.tools ?? [];
  assert.equal(tools.length, 1, "ONE MCP tool is the feature text, not a preference");
  assert.equal(tools[0]?.name, "recall");
  const schema = tools[0]?.inputSchema as { properties: Record<string, unknown> };
  // The whole surface reachable from one tool: both shapes and both axes.
  for (const field of ["query", "ids", "as_of", "valid_at", "limit"]) {
    assert.ok(field in schema.properties, `the tool must expose ${field}`);
  }
});

test("a query reaches the gateway and comes back as weighable text", async () => {
  await withGateway(
    () => ({ status: 200, body: served([entry()]) }),
    async (requests) => {
      const response = await frame({
        jsonrpc: "2.0",
        id: 4,
        method: "tools/call",
        params: { name: "recall", arguments: { query: "how do we rotate payment keys" } },
      });
      assert.equal(requests.length, 1);
      assert.equal(requests[0]?.path, "/v1/recall");
      assert.equal(requests[0]?.body.query, "how do we rotate payment keys");
      assert.equal(requests[0]?.authorization, "Bearer dev-bearer");

      const answer = text(response);
      assert.equal(response?.result?.isError, undefined);
      assert.match(answer, /Rotate the payments key/);
      // The trust label is the point: a reader cannot weigh derived
      // against published unless the answer says which it is.
      assert.match(answer, /\[published\]/);
      assert.match(answer, /\[procedure\]/);
      // And it stays citable — the same watermark discipline a block has.
      assert.match(answer, /Watermark: 0198f0a0-0000-7000-8000-000000000001/);
      // The handle round-trips, so an agent can go from an answer back to
      // a body without learning a second format.
      assert.match(answer, /\(recall 0198f0a0-0000-7000-8000-000000000001\)/);
    },
  );
});

test("unreviewed material says so, and an earned tier is never silent", async () => {
  await withGateway(
    () => ({
      status: 200,
      body: served([
        entry({ channel: "derived", sensitivity: "confidential", content: "Draft note." }),
      ]),
    }),
    async () => {
      const answer = text(
        await frame({
          jsonrpc: "2.0",
          id: 5,
          method: "tools/call",
          params: { name: "recall", arguments: { query: "draft" } },
        }),
      );
      assert.match(answer, /\[unreviewed\]/);
      assert.match(answer, /\[confidential\]/, "ADR-0038: the harness must be told what it holds");
    },
  );
});

test("ids and query together is refused before the gateway is troubled", async () => {
  await withGateway(
    () => ({ status: 200, body: served([]) }),
    async (requests) => {
      const response = await frame({
        jsonrpc: "2.0",
        id: 6,
        method: "tools/call",
        params: { name: "recall", arguments: { query: "x", ids: ["some-id"] } },
      });
      assert.equal(response?.result?.isError, true);
      assert.match(text(response), /either `query` or `ids`/);
      assert.equal(requests.length, 0, "a shape error costs no round trip");
    },
  );
});

test("an empty ask is refused with a sentence rather than a 400", async () => {
  await withGateway(
    () => ({ status: 200, body: served([]) }),
    async (requests) => {
      const response = await frame({
        jsonrpc: "2.0",
        id: 7,
        method: "tools/call",
        params: { name: "recall", arguments: {} },
      });
      assert.equal(response?.result?.isError, true);
      assert.match(text(response), /Pass a `query`/);
      assert.equal(requests.length, 0);
    },
  );
});

test("a truncated answer never reads as a complete one", async () => {
  await withGateway(
    () => ({
      status: 200,
      body: served([entry()], { truncated: true, scopes_considered: 900, scopes_decided: 512 }),
    }),
    async () => {
      const answer = text(
        await frame({
          jsonrpc: "2.0",
          id: 8,
          method: "tools/call",
          params: { name: "recall", arguments: { query: "anything" } },
        }),
      );
      // ADR-0042 decision 5: a bounded answer presented as complete is
      // the one failure this surface cannot afford.
      assert.match(answer, /Incomplete: 900 scopes could have contributed, 512 were searched/);
    },
  );
});

test("degradation is reported to the agent, not swallowed", async () => {
  await withGateway(
    () => ({ status: 200, body: served([entry()], { degraded: ["embedder"] }) }),
    async () => {
      const answer = text(
        await frame({
          jsonrpc: "2.0",
          id: 9,
          method: "tools/call",
          params: { name: "recall", arguments: { query: "anything" } },
        }),
      );
      assert.match(answer, /Degraded \(embedder\)/);
    },
  );
});

test("the as-of pair is passed through verbatim", async () => {
  await withGateway(
    () => ({ status: 200, body: served([]) }),
    async (requests) => {
      await frame({
        jsonrpc: "2.0",
        id: 10,
        method: "tools/call",
        params: {
          name: "recall",
          arguments: { query: "what did we know", as_of: "2026-03-03T00:00:00Z", limit: 5 },
        },
      });
      assert.equal(requests[0]?.body.as_of, "2026-03-03T00:00:00Z");
      assert.equal(requests[0]?.body.limit, 5);
      // Never invented by the client: `valid_at` defaulting to `as_of` is
      // the gateway's contract, and a second place for it would be a
      // second contract.
      assert.equal("valid_at" in (requests[0]?.body ?? {}), false);
    },
  );
});

test("nothing readable is an answer, not an error", async () => {
  await withGateway(
    () => ({ status: 200, body: served([]) }),
    async () => {
      const response = await frame({
        jsonrpc: "2.0",
        id: 11,
        method: "tools/call",
        params: { name: "recall", arguments: { query: "someone else's secrets" } },
      });
      // A policy outcome on a read is a result (ADR-0026 decision 1), and
      // the wording must not become an existence oracle either.
      assert.equal(response?.result?.isError, undefined);
      assert.match(text(response), /No memory available to you/);
    },
  );
});

test("a gateway failure reaches the model as something it can read", async () => {
  await withGateway(
    () => ({ status: 503, body: { error: "unavailable" } }),
    async () => {
      const response = await frame({
        jsonrpc: "2.0",
        id: 12,
        method: "tools/call",
        params: { name: "recall", arguments: { query: "anything" } },
      });
      assert.equal(response?.result?.isError, true);
      assert.match(text(response), /Recall failed: gateway returned 503/);
    },
  );
});

test("odd frames are answered rather than fatal", async () => {
  const parse = (await handleLine("{not json")) as Rpc;
  assert.equal(parse.error?.code, -32700);

  const noMethod = await frame({ jsonrpc: "2.0", id: 13 });
  assert.equal(noMethod?.error?.code, -32600);

  const unknown = await frame({ jsonrpc: "2.0", id: 14, method: "resources/list" });
  assert.equal(unknown?.error?.code, -32601);

  const unknownTool = await frame({
    jsonrpc: "2.0",
    id: 15,
    method: "tools/call",
    params: { name: "inject", arguments: {} },
  });
  assert.equal(unknownTool?.error?.code, -32602);

  // An unknown *notification* is silence, not an error: clients send
  // notifications this server has never heard of and must not be scolded.
  assert.equal(await frame({ jsonrpc: "2.0", method: "notifications/progress" }), undefined);
});
