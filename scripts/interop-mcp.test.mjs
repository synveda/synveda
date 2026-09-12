// ADPT-2 / ADR-0106: real CLI and stdio, synthetic HTTP contract replies.
// Gateway policy/isolation is tested separately against an ordinary tenant.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:http";
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { test } from "node:test";

const first = "11111111-1111-4111-8111-111111111111";
const second = "22222222-2222-4222-8222-222222222222";
const denied = "33333333-3333-4333-8333-333333333333";

async function fixture(t) {
  const requests = [];
  const gateway = createServer(async (req, res) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    requests.push({ path: req.url, body: body ? JSON.parse(body) : null,
      key: req.headers["idempotency-key"], authorization: req.headers.authorization });
    res.setHeader("content-type", "application/json");
    if (req.url === "/v1/me") {
      res.end(JSON.stringify({ workspaces: [{ id: "w1", scope_id: "ws1" }, { id: "w2", scope_id: "ws2" }],
        projects: [{ id: "p1", workspace_id: "w1", scope_id: "ps1" }] }));
    } else if (req.url === "/v1/sessions") {
      const task = JSON.parse(body).external_session_id;
      res.end(JSON.stringify({ id: task === "task-b" ? second : first }));
    } else if (req.url === `/v1/sessions/${denied}`) {
      res.statusCode = 404;
      res.end(JSON.stringify({ error: "not_found", message: "not found" }));
    } else if (/\/v1\/sessions\/[^/]+$/.test(req.url)) {
      const id = req.url.split("/").at(-1);
      res.end(JSON.stringify({ id, workspace_id: id === second ? "w2" : "w1",
        project_id: id === second ? "p2" : "p1", scope_id: "ps1" }));
    } else if (req.url.endsWith("/knowledge-query")) {
      res.end(JSON.stringify({ items: [], as_of: "2026-09-12T00:00:00Z", retrieval_mode: "lexical" }));
    } else if (req.url.endsWith("/events")) {
      res.end(JSON.stringify({ appended: 1, duplicates: 0, quarantined: 0, denied: 0 }));
    } else {
      res.end(JSON.stringify({ skills: [], bindings: [] }));
    }
  });
  gateway.listen(0, "127.0.0.1");
  await once(gateway, "listening");
  t.after(() => { gateway.closeAllConnections(); gateway.close(); });
  return { requests, url: `http://127.0.0.1:${gateway.address().port}` };
}

async function client(t, gateway, args = []) {
  const child = spawn(resolve("target/debug/synveda"), ["mcp", ...args], {
    env: { ...process.env, SYNVEDA_TOKEN: "synthetic-interop-token", SYNVEDA_GATEWAY: gateway.url },
    stdio: ["pipe", "pipe", "ignore"],
  });
  const closed = once(child, "close");
  const lines = createInterface({ input: child.stdout });
  const pending = new Map();
  let id = 0;
  lines.on("line", (line) => {
    const frame = JSON.parse(line);
    pending.get(frame.id)?.(frame);
  });
  t.after(async () => {
    child.stdin.end();
    const timer = setTimeout(() => child.kill("SIGKILL"), 3000);
    try { await closed; } finally { clearTimeout(timer); lines.close(); }
  });
  const request = (method, params) => new Promise((resolve, reject) => {
    const key = ++id;
    const timer = setTimeout(() => { pending.delete(key); reject(new Error("MCP response timeout")); }, 8000);
    pending.set(key, (frame) => { clearTimeout(timer); pending.delete(key); resolve(frame); });
    child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id: key, method, params })}\n`);
  });
  const initialized = await request("initialize", { protocolVersion: "2025-11-25", capabilities: {},
    clientInfo: { name: "synthetic-interop-test", version: "1" } });
  assert.ok(initialized.result);
  child.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');
  return (name, arguments_) => request("tools/call", { name, arguments: arguments_ });
}

test("task references survive reconnect; different tasks stay separate", async (t) => {
  const gateway = await fixture(t);
  for (const task of ["tâche-α", "tâche-α", "task-b"]) {
    const call = await client(t, gateway, ["--task", task, "--workspace", task === "task-b" ? "w2" : "w1"]);
    const result = await call("recall", { query: "context" });
    assert.notEqual(result.result?.isError, true, JSON.stringify(result));
  }
  const opens = gateway.requests.filter((req) => req.path === "/v1/sessions");
  assert.deepEqual(opens.map((req) => req.body.external_session_id), ["tâche-α", "tâche-α", "task-b"]);
  assert.equal(opens[0].key, opens[1].key);
  assert.match(opens[0].key, /^mcp-open-[A-Za-z0-9_-]+$/);
  assert.ok(opens.every((request) => request.body.client_version === undefined), "transport upgrades must not change the replayed body");
  assert.notEqual(opens[1].key, opens[2].key);
  assert.deepEqual(gateway.requests.filter((req) => req.path.endsWith("/knowledge-query")).map((req) => req.path),
    [`/v1/sessions/${first}/knowledge-query`, `/v1/sessions/${first}/knowledge-query`, `/v1/sessions/${second}/knowledge-query`]);
});

test("a shared transport uses each explicit Session and forwards no denied query", async (t) => {
  const gateway = await fixture(t);
  const call = await client(t, gateway, ["--writes", "host"]);
  for (const session_id of [first, second, first]) {
    const result = await call("recall", { session_id, query: "context" });
    assert.notEqual(result.result?.isError, true, JSON.stringify(result));
  }
  assert.equal((await call("recall", { session_id: denied, query: "secret" })).result.isError, true);
  assert.equal((await call("recall", { query: "missing identity" })).result.isError, true);
  assert.ok((await call("remember", { session_id: first, text: "duplicate host observation" })).error);
  assert.equal(gateway.requests.filter((req) => req.path.endsWith("/knowledge-query")).length, 3);
  assert.ok(gateway.requests.every((req) => req.path !== "/v1/sessions" && !req.path.endsWith("/events")));
});

test("bound Session and configured target cannot be overridden", async (t) => {
  const gateway = await fixture(t);
  const call = await client(t, gateway, ["--session", first, "--workspace", "w1", "--project", "p1"]);
  assert.equal((await call("recall", { session_id: second, query: "context" })).result.isError, true);
  const mismatched = await client(t, gateway, ["--workspace", "w1"]);
  assert.equal((await mismatched("recall", { session_id: second, query: "context" })).result.isError, true);
  assert.ok(gateway.requests.every((req) => !req.path.endsWith("/knowledge-query")));
  const result = await call("remember", { text: "A useful observation" });
  assert.notEqual(result.result?.isError, true, JSON.stringify(result));
  assert.match(result.result.content[0].text, /Observation recorded/);
  assert.doesNotMatch(result.result.content[0].text, /enters extraction|recallable.*shortly/);
  const event = gateway.requests.find((req) => req.path.endsWith("/events"));
  assert.equal(event.path, `/v1/sessions/${first}/events`);
  assert.equal(event.body.events[0].event_type, "memory.asserted");
});
