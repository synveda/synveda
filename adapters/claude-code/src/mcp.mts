/**
 * The MCP server: one tool, `recall` (CTX-5, ADR-0042 decision 15).
 *
 * Registered through the `mcpServers` slot ADR-0027 decision 1 reserved
 * for exactly this, so it arrives as configuration rather than as a
 * restructuring of the plugin.
 *
 * # One tool, not three
 *
 * `inject` is a hook and `observe` is a hook — the harness calls them, not
 * the model. The only primitive an agent should *choose* to call is the
 * deep one, so this exposes exactly `recall`, with the route's own
 * `ids` xor `query` shape rather than a tool per shape. A second tool
 * would be a second place for that exclusivity to be got wrong.
 *
 * # Why the protocol is written out
 *
 * ADR-0027 decision 1 fixed this package as `tsc`-built, Node ≥22 stdlib
 * only, with no runtime dependencies and no install step — so enabling
 * the plugin costs nothing. Taking the MCP SDK would mean adopting a
 * bundler to keep that true. What is actually needed is `initialize`,
 * `notifications/initialized`, `tools/list` and `tools/call` over
 * newline-delimited JSON-RPC 2.0, which is small enough to read in one
 * sitting (ADR-0042 option 8 records what would reverse this).
 *
 * # Failure posture, inverted from the hooks
 *
 * A hook must never break a session, so it degrades in silence. This
 * caller is an agent that asked a question and can read an answer, so a
 * failure is *reported*: a gateway that is down, a login that has
 * expired, or a retrieval error come back as tool errors with a usable
 * sentence. The process still never crashes the client — a malformed
 * frame is answered, not thrown.
 */

import { createInterface } from "node:readline";

import { recall } from "./client.mjs";
import { loadConfig, resolveGateway } from "./config.mjs";
import { resolveBearer, SIGN_IN_MESSAGE } from "./credentials.mjs";
import { log } from "./log.mjs";
import type { RecallEntry, RecallRequest, RecallResponse } from "./types.mjs";

/** The revision this server implements. */
const PROTOCOL_VERSION = "2025-06-18";

const SERVER_NAME = "synveda";
const SERVER_VERSION = "0.1.0";

/** JSON-RPC error codes this server uses (the spec's reserved range). */
const INVALID_REQUEST = -32600;
const METHOD_NOT_FOUND = -32601;
const INVALID_PARAMS = -32602;
const PARSE_ERROR = -32700;

interface Request {
  jsonrpc?: unknown;
  id?: unknown;
  method?: unknown;
  params?: unknown;
}

/**
 * The tool as the model sees it. The description is doing real work: an
 * agent that does not know recall reaches *wider* than its session-start
 * block will never reach for it, and one that does not know `as_of`
 * exists cannot ask what was true in March.
 */
const RECALL_TOOL = {
  name: "recall",
  title: "Recall governed memory",
  description:
    "Search or fetch governed organisational memory. Ask a question with `query` " +
    "to search every scope your policy lets you read — which is wider than the " +
    "scopes your session-start context block composes from — or pass `ids` to " +
    "fetch the full body of records a block named as `(recall <id>)`. " +
    "Use `as_of` to ask what was known at a past instant. " +
    "Results carry their channel (published means reviewed, derived means " +
    "unreviewed), provenance, and validity window, so you can weigh them. " +
    "What you may read is decided at call time under your own identity: " +
    "material you have no access to is simply absent from the answer.",
  inputSchema: {
    type: "object",
    properties: {
      query: {
        type: "string",
        description: "The question to answer. Mutually exclusive with `ids`.",
      },
      ids: {
        type: "array",
        items: { type: "string" },
        description:
          "Record ids to fetch in full, as an inject block printed them. " +
          "Mutually exclusive with `query`.",
      },
      as_of: {
        type: "string",
        description:
          "RFC 3339 instant. Serve bodies as the database held them then " +
          "(\"what did we know on 2026-03-03\"). Rewinds the corpus, never your access.",
      },
      valid_at: {
        type: "string",
        description:
          "RFC 3339 instant. Which assertions were true about the world then. " +
          "Defaults to `as_of`.",
      },
      limit: {
        type: "number",
        description: "How many records a query may return (1-32, default 32).",
      },
    },
    additionalProperties: false,
  },
} as const;

export async function main(): Promise<void> {
  const lines = createInterface({ input: process.stdin });
  log("mcp.started", { protocol: PROTOCOL_VERSION });
  for await (const line of lines) {
    if (line.trim().length === 0) continue;
    const response = await handleLine(line);
    // A notification produces nothing: the spec forbids replying to one,
    // and a client that gets an id-less response may well disconnect.
    if (response !== undefined) process.stdout.write(`${JSON.stringify(response)}\n`);
  }
  log("mcp.stopped");
}

export async function handleLine(line: string): Promise<unknown> {
  let request: Request;
  try {
    request = JSON.parse(line) as Request;
  } catch (error) {
    log("mcp.parse_error", { error: String(error) });
    return failure(null, PARSE_ERROR, "invalid JSON");
  }
  if (typeof request !== "object" || request === null) {
    return failure(null, INVALID_REQUEST, "request must be an object");
  }
  const id = request.id ?? null;
  const method = typeof request.method === "string" ? request.method : undefined;
  if (method === undefined) {
    return failure(id, INVALID_REQUEST, "request has no method");
  }
  // Notifications carry no id and are answered with silence.
  const notification = request.id === undefined || request.id === null;

  switch (method) {
    case "initialize":
      return {
        jsonrpc: "2.0",
        id,
        result: {
          protocolVersion: PROTOCOL_VERSION,
          capabilities: { tools: {} },
          serverInfo: { name: SERVER_NAME, version: SERVER_VERSION },
        },
      };
    case "notifications/initialized":
    case "notifications/cancelled":
      return undefined;
    case "ping":
      return { jsonrpc: "2.0", id, result: {} };
    case "tools/list":
      return { jsonrpc: "2.0", id, result: { tools: [RECALL_TOOL] } };
    case "tools/call":
      return await callTool(id, request.params);
    default:
      if (notification) return undefined;
      return failure(id, METHOD_NOT_FOUND, `unknown method ${method}`);
  }
}

async function callTool(id: unknown, params: unknown): Promise<unknown> {
  const call = (params ?? {}) as { name?: unknown; arguments?: unknown };
  if (call.name !== RECALL_TOOL.name) {
    return failure(id, INVALID_PARAMS, `unknown tool ${String(call.name)}`);
  }
  const args = (call.arguments ?? {}) as Record<string, unknown>;

  const query = typeof args.query === "string" ? args.query : undefined;
  const ids = Array.isArray(args.ids)
    ? args.ids.filter((value): value is string => typeof value === "string")
    : undefined;
  // Checked here as well as at the gateway, so an agent that gets it
  // wrong reads a sentence rather than a 400.
  if (query !== undefined && ids !== undefined && ids.length > 0) {
    return toolError(id, "Pass either `query` or `ids`, not both.");
  }
  if (query === undefined && (ids === undefined || ids.length === 0)) {
    return toolError(id, "Pass a `query` to search, or `ids` to fetch records by name.");
  }

  const request: RecallRequest = {};
  if (query !== undefined) request.query = query;
  if (ids !== undefined && ids.length > 0) request.ids = ids;
  if (typeof args.as_of === "string") request.as_of = args.as_of;
  if (typeof args.valid_at === "string") request.valid_at = args.valid_at;
  if (typeof args.limit === "number") request.limit = args.limit;

  const bearer = await resolveBearer();
  if (bearer === undefined) return toolError(id, SIGN_IN_MESSAGE);
  const config = resolveGateway(loadConfig(process.cwd()), bearer);
  if (config.disabled) return toolError(id, "Synveda is disabled for this project.");

  const result = await recall(config, bearer.token, request);
  if (!result.ok) {
    log("mcp.recall_failed", { status: result.status, reason: result.reason });
    return toolError(id, `Recall failed: ${result.reason}`);
  }
  log("mcp.recall", {
    mode: result.value.mode,
    served: result.value.entries.length,
    truncated: result.value.truncated,
  });
  return {
    jsonrpc: "2.0",
    id,
    result: {
      content: [{ type: "text", text: render(result.value) }],
    },
  };
}

/**
 * The answer as text, in the shape an inject block already uses — trust
 * markers first, then the body — so an agent that has read a block does
 * not have to learn a second format (ADR-0042 decision 15).
 */
function render(response: RecallResponse): string {
  if (response.entries.length === 0) {
    return `No memory available to you at ${response.as_of}.`;
  }
  const lines = response.entries.map(entryLine);
  const notes: string[] = [];
  if (response.truncated) {
    notes.push(
      `Incomplete: ${String(response.scopes_considered)} scopes could have contributed, ` +
        `${String(response.scopes_decided)} were searched.`,
    );
  }
  if (response.degraded.length > 0) {
    notes.push(`Degraded (${response.degraded.join(", ")}): ranked on the lexical leg only.`);
  }
  // The watermark, so a recall is as citable as a block: the reader can
  // say which versions of which records it was answering from.
  notes.push(
    `Watermark: ${response.entries.map((entry) => entry.record_id).join(" ")} ` +
      `as of ${response.as_of} (valid ${response.valid_at}).`,
  );
  return `${lines.join("\n\n")}\n\n${notes.join("\n")}`;
}

function entryLine(entry: RecallEntry): string {
  const markers = [entry.class, entry.channel === "published" ? "published" : "unreviewed"];
  // A reader cannot know what they are holding unless they are told, and
  // that does not change because they asked for it by name (ADR-0038).
  if (entry.sensitivity === "confidential" || entry.sensitivity === "restricted") {
    markers.push(entry.sensitivity);
  }
  const validity =
    entry.valid_to === null
      ? `valid from ${entry.valid_from}`
      : `valid ${entry.valid_from}..${entry.valid_to}`;
  return (
    `[${markers.join("] [")}] ${entry.content}\n` +
    `  (recall ${entry.record_id}) scope ${entry.scope_id} · ${validity} · ` +
    `freshness ${String(entry.staleness_permille)}‰`
  );
}

/**
 * A tool-level failure: `isError` on a successful JSON-RPC result, which
 * is how MCP puts a problem in front of the *model* rather than the
 * client's error handler. An agent can read this and try something else.
 */
function toolError(id: unknown, message: string): unknown {
  return {
    jsonrpc: "2.0",
    id,
    result: { content: [{ type: "text", text: message }], isError: true },
  };
}

/** A protocol-level failure, for the client rather than the model. */
function failure(id: unknown, code: number, message: string): unknown {
  return { jsonrpc: "2.0", id, error: { code, message } };
}
