import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { readFileSync } from "node:fs";
import { test, type TestContext } from "node:test";
import { ApiError, Client, TransportError } from "./client.mjs";
import type { OperationId } from "./generated/api.js";

const parent = "00-11111111111111111111111111111111-2222222222222222-01";
async function server(t: TestContext, handle: (req: IncomingMessage, res: ServerResponse) => void) {
  const instance = createServer(handle);
  instance.listen(0, "127.0.0.1"); await once(instance, "listening");
  t.after(() => { instance.closeAllConnections(); instance.close(); });
  const address = instance.address();
  assert.ok(address && typeof address === "object");
  return `http://127.0.0.1:${address.port}`;
}
function reply(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(body));
}

test("shared Python/TypeScript wire fixtures: encoding, auth, idempotency and correlation", async (t) => {
  const fixtures = JSON.parse(readFileSync(new URL("../../fixtures/wire.json", import.meta.url), "utf8"));
  const observed: unknown[] = [];
  const base = await server(t, (req, res) => {
    let body = ""; req.on("data", (chunk) => body += chunk);
    req.on("end", () => {
      const fixture = fixtures[observed.length];
      observed.push({ method: req.method, url: req.url, body: body ? JSON.parse(body) : null,
        token: req.headers.authorization, key: req.headers["idempotency-key"], trace: req.headers.traceparent });
      reply(res, 200, fixture.response);
    });
  });
  const client = new Client(base, async () => "synthetic-token");
  for (const fixture of fixtures) {
    const result = await client.request(fixture.operation as OperationId, { ...fixture.options, traceparent: parent });
    assert.deepEqual(result.data, fixture.response);
    assert.equal(result.traceId, parent.slice(3, 35));
    assert.deepEqual(observed.at(-1), { method: fixture.method, url: fixture.url, body: fixture.body,
      token: "Bearer synthetic-token", key: fixture.options.idempotencyKey, trace: parent });
  }
});

test("refresh once for replay-safe requests; never retry an event append automatically", async (t) => {
  let calls = 0;
  const base = await server(t, (req, res) => {
    calls += 1;
    reply(res, req.headers.authorization === "Bearer fresh" ? 200 : 401, { kind: "unauthenticated" });
  });
  const refreshes: boolean[] = [];
  const client = new Client(base, async (refresh) => { refreshes.push(refresh); return refresh ? "fresh" : "expired"; });
  await client.request("get_me", {});
  assert.deepEqual(refreshes, [false, true]); assert.equal(calls, 2);
  await assert.rejects(client.request("append_session_events", { path: { session_id: "s1" }, body: { events: [] } }), ApiError);
  assert.deepEqual(refreshes, [false, true, false]); assert.equal(calls, 3);
});

test("unchanged refresh and rate limits surface typed errors without retry loops", async (t) => {
  let calls = 0;
  const base = await server(t, (_req, res) => { calls += 1; res.setHeader("retry-after", "7"); reply(res, 429, { kind: "rate_limited" }); });
  const client = new Client(base, async () => "token");
  await assert.rejects(client.request("get_me", {}), (error) => error instanceof ApiError && error.status === 429 && error.retryAfter === "7");
  assert.equal(calls, 1);
});

test("refuse redirects, oversized replies and invalid JSON", async (t) => {
  let visits = 0;
  const base = await server(t, (_req, res) => {
    visits += 1;
    if (visits === 1) { res.writeHead(302, { location: "/credential-target" }); res.end(); }
    else if (visits === 2) res.end("x".repeat(2048));
    else res.end("not JSON");
  });
  const client = new Client(base, async () => "secret-token", { maxResponseBytes: 1024 });
  for (const kind of ["transport", "response_too_large", "invalid_response"]) {
    await assert.rejects(client.request("get_me", {}), (e) => e instanceof TransportError && e.kind === kind && !e.message.includes("secret-token"));
  }
  assert.equal(visits, 3, "no redirect target was visited");
});

test("deadlines include bearer resolution and caller cancellation", async () => {
  const client = new Client("http://127.0.0.1:1", async () => new Promise(() => {}), { timeoutMs: 20 });
  // Keep the test process alive while an unref'ed AbortSignal timer expires.
  const keepalive = setInterval(() => {}, 1000);
  try {
    await assert.rejects(client.request("get_me", {}), (e) => e instanceof TransportError && e.kind === "timeout");
    await assert.rejects(client.request("get_me", { signal: AbortSignal.abort() }), (e) => e instanceof TransportError && e.kind === "cancelled");
  } finally { clearInterval(keepalive); }
});

test("invalid parameters and missing idempotency keys fail before authentication", async () => {
  const client = new Client("http://127.0.0.1:1", async () => { assert.fail("must not resolve credentials"); });
  await assert.rejects(client.request("get_session", {}), TypeError);
  await assert.rejects(client.request("get_session", { path: { session_id: ".." } }), TypeError);
  await assert.rejects(client.request("get_me", { query: { token: "wrong" } }), TypeError);
  // Deliberately exercise a JavaScript caller that bypasses TypeScript's required key.
  await assert.rejects(client.request("open_session", { body: { workspace_id: "w1", client_name: "test" } } as never), TypeError);
});

test("idempotent refresh preserves the request; transient errors stay explicit", async (t) => {
  const calls: { body: string; key: unknown; trace: unknown }[] = [];
  const base = await server(t, (req, res) => {
    let body = ""; req.on("data", (chunk) => body += chunk);
    req.on("end", () => {
      calls.push({ body, key: req.headers["idempotency-key"], trace: req.headers.traceparent });
      reply(res, calls.length === 1 ? 401 : 503, { kind: "unavailable" });
    });
  });
  const client = new Client(base, async (refresh) => refresh ? "fresh" : "expired");
  await assert.rejects(client.request("open_session", { body: { workspace_id: "w1", client_name: "test" }, idempotencyKey: "stable" }),
    (error) => error instanceof ApiError && error.status === 503);
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[0], calls[1]);
});

test("provider errors never include credentials", async () => {
  const client = new Client("http://127.0.0.1:1", async () => { throw new TypeError("Bearer private-credential"); });
  await assert.rejects(client.request("get_me", {}),
    (error) => error instanceof TransportError && !error.message.includes("private-credential"));
});
