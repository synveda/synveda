/**
 * The generated client (CPR-8, ADR-0075 decision 3).
 *
 * What is asserted here is the wire shape: the URL a call builds, the
 * headers it sets, and the two things it refuses to send. The refusals are
 * the interesting half — a creation without an idempotency key and a path
 * with an unfilled placeholder are both requests that *look* fine and are
 * not, and both are cheap to catch here and expensive to diagnose from a
 * gateway log.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { describe, fillPath, queryString, request } from "./client.mjs";
import { OPERATIONS } from "./generated/api.js";

test("a path template is filled and its values are encoded", () => {
  assert.equal(fillPath("/v1/workspaces/{workspace_id}", { workspace_id: "w-1" }), "/v1/workspaces/w-1");
  // An id is a path segment, so anything in it that could end the segment
  // has to stop being a path character.
  assert.equal(
    fillPath("/v1/projects/{project_id}/members/{principal_id}", {
      project_id: "p 1",
      principal_id: "robin@example.test",
    }),
    "/v1/projects/p%201/members/robin%40example.test",
  );
});

test("an unfilled placeholder throws rather than reaching the gateway", () => {
  // Otherwise the gateway receives a literal `{workspace_id}` and answers a
  // 404, which reads like a missing row rather than like a bug here.
  assert.throws(
    () => fillPath("/v1/workspaces/{workspace_id}", {}),
    /no value for path parameter \{workspace_id\}/,
  );
});

test("a query parameter nobody set is not sent at all", () => {
  // "Filter by nothing" and "do not filter" are different requests, and the
  // second is the one an omitted field means.
  assert.equal(queryString({ scope_id: undefined, principal_id: "robin" }), "?principal_id=robin");
  assert.equal(queryString({}), "");
  assert.equal(queryString({ a: undefined }), "");
});

test("a GET is built from the document's own path and method", () => {
  const call = describe("list_workspace_members", { path: { workspace_id: "w-1" } });
  // The `/v1` prefix is stripped because the transport adds it.
  assert.equal(call.path, "/workspaces/w-1/members");
  assert.equal(call.init.method, "GET");
  assert.deepEqual(call.init.headers, {});
  assert.equal(call.init.body, undefined);
});

test("a body is JSON, and says so", () => {
  const call = describe("create_workspace", {
    idempotencyKey: "key-1",
    body: { display_name: "Payments", slug: "payments" },
  });
  assert.equal(call.path, "/workspaces");
  assert.equal(call.init.method, "POST");
  assert.deepEqual(call.init.headers, {
    "content-type": "application/json",
    "idempotency-key": "key-1",
  });
  assert.equal(call.init.body, JSON.stringify({ display_name: "Payments", slug: "payments" }));
});

test("an operation the contract calls idempotent will not be sent without a key", () => {
  // The type already forbids it; this is what catches an untyped caller,
  // and the document is the authority either way. Sending the creation
  // without the header is how one timed-out request becomes two workspaces.
  assert.throws(
    () => describe("create_workspace", { body: { display_name: "P", slug: "p" } }),
    /requires an Idempotency-Key/,
  );
  assert.throws(
    () => describe("create_project", { path: { workspace_id: "w" }, body: { display_name: "P", slug: "p" } }),
    /requires an Idempotency-Key/,
  );
});

test("an operation the contract does not call idempotent will not carry a key either", () => {
  // Not pedantry: a key on a route that ignores it is a client believing it
  // has a guarantee the server never made.
  assert.throws(
    () => describe("list_workspaces", { idempotencyKey: "key-1" }),
    /declares no Idempotency-Key/,
  );
});

test("every operation the document declares is callable, and none is invented", () => {
  // The runtime table is generated beside the type table from one document,
  // so this is really asserting the generator did not skip a row — the
  // failure that would make an operation typecheck and then throw.
  const ids = Object.keys(OPERATIONS);
  assert.equal(ids.length, 40, "the contract's operation count moved; update the count here");
  for (const id of ids) {
    const declared = OPERATIONS[id as keyof typeof OPERATIONS];
    assert.ok(declared.path.startsWith("/v1/"), `${id} is not a /v1 path`);
    assert.ok(
      ["GET", "POST", "PATCH", "PUT", "DELETE"].includes(declared.method),
      `${id} has method ${declared.method}`,
    );
  }
});

test("the idempotent creations are exactly the ones the document marks", () => {
  const idempotent = Object.entries(OPERATIONS)
    .filter(([, declared]) => "idempotent" in declared)
    .map(([id]) => id)
    .sort();
  assert.deepEqual(idempotent, [
    "add_project_member",
    "attach_repository",
    // CPR-10: opening a run and composing context for one are both creations
    // whose retry after a timeout would otherwise make a second row.
    // Appending events is deliberately **not** here — its idempotency unit is
    // the event, keyed by the client's own `client_event_id`.
    "create_context_run",
    "create_grant",
    "create_group",
    "create_project",
    "create_scope",
    "create_workspace",
    "create_workspace_invite",
    "open_session",
  ]);
});

test("a call goes through the transport and comes back classified", async () => {
  const seen: { url: string; init: RequestInit }[] = [];
  const fake: typeof fetch = async (input, init) => {
    seen.push({ url: String(input), init: init ?? {} });
    return new Response(JSON.stringify({ workspaces: [] }), { status: 200 });
  };
  const answer = await request("list_workspaces", {}, fake);
  assert.equal(answer.kind, "ok");
  assert.deepEqual(answer.kind === "ok" ? answer.body : null, { workspaces: [] });
  assert.equal(seen[0]?.url, "/v1/workspaces");
  // The session is a cookie the browser attaches; the bundle handles no
  // credential (ADR-0056 decision 2), and this is where that is visible.
  assert.equal(seen[0]?.init.credentials, "same-origin");
});

test("a refusal keeps its kind rather than becoming a thrown error", async () => {
  const forbidden: typeof fetch = async () =>
    new Response(JSON.stringify({ message: "the pack in force refuses this" }), { status: 403 });
  const answer = await request("list_workspaces", {}, forbidden);
  assert.equal(answer.kind, "forbidden");
  assert.equal(
    answer.kind === "forbidden" ? answer.message : "",
    "the pack in force refuses this",
  );
});
