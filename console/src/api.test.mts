/**
 * The response classification (CNSL-1, ADR-0056). What is asserted here is
 * the rule that decides what a reviewer is shown — most of all that a 403
 * is not a 401, because collapsing them puts a Sign in button in front of
 * somebody who is already signed in.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { call, classify } from "./api.mjs";

test("a 2xx is ok and carries its body", () => {
  const outcome = classify(200, { subject: "reviewer@example.test" });
  assert.equal(outcome.kind, "ok");
  assert.deepEqual(outcome.kind === "ok" ? outcome.body : null, {
    subject: "reviewer@example.test",
  });
  assert.equal(classify(204, null).kind, "ok");
});

test("401 and 403 are different answers, because the next step differs", () => {
  // The whole point of the type. A reviewer holding one role short of
  // ProposalRead gets a 403 forever, and offering them a login is a loop.
  assert.equal(classify(401, { message: "no" }).kind, "unauthenticated");

  const forbidden = classify(403, { message: "the pack in force refuses this" });
  assert.equal(forbidden.kind, "forbidden");
  assert.equal(
    forbidden.kind === "forbidden" ? forbidden.message : "",
    "the pack in force refuses this",
  );
});

test("an unauthenticated outcome carries no message to render", () => {
  // There is nothing useful to say and the gateway deliberately says
  // nothing: 401 is uniform across unknown, suspended and missing
  // (ADR-0008), so a console quoting it would be quoting a non-answer.
  const outcome = classify(401, { message: "token does not resolve" });
  assert.deepEqual(outcome, { kind: "unauthenticated" });
});

test("a 404 is invalid, not a kind of its own", () => {
  // AUTHZ-3's uniform 404: "not found" and "not yours" are one answer by
  // design, so rendering them differently would invent a distinction the
  // product refuses to make.
  assert.equal(classify(404, { message: "no such proposal" }).kind, "invalid");
  assert.equal(classify(400, { message: "bad" }).kind, "invalid");
  assert.equal(classify(409, { message: "already reviewed" }).kind, "conflict");
});

test("a status the console has never seen is not treated as success", () => {
  // The failure mode this prevents: rendering an error body as data.
  for (const status of [418, 500, 502, 503, 599]) {
    assert.equal(classify(status, { message: "x" }).kind, "unavailable", `status ${status}`);
  }
});

test("a body with no message still yields something honest to show", () => {
  const outcome = classify(500, null);
  assert.equal(outcome.kind, "unavailable");
  assert.match(outcome.kind === "unavailable" ? outcome.message : "", /did not say why/);
});

test("the gateway's own sentence is displayed rather than recomposed", () => {
  // ADR-0056 decision 6's rule, applied to errors: the gateway owns the
  // wording, and a second author of one sentence is a second sentence.
  const outcome = classify(403, { message: "curator and reviewer must be two people" });
  assert.equal(
    outcome.kind === "forbidden" ? outcome.message : "",
    "curator and reviewer must be two people",
  );
});

// ── The call wrapper ─────────────────────────────────────────────────────────

test("a dead gateway is unavailable rather than signed out", () => {
  // Told apart because the answers differ: wait and retry, versus sign in
  // again. A network failure rendered as "your session ended" sends the
  // operator round a login that cannot succeed either.
  const dead: typeof fetch = () => Promise.reject(new Error("connection refused"));
  return call("/whoami", {}, dead).then((outcome) => {
    assert.equal(outcome.kind, "unavailable");
    assert.match(outcome.kind === "unavailable" ? outcome.message : "", /connection refused/);
  });
});

test("the request carries the cookie and no credential of its own", async () => {
  // The bundle holds no token, so there must be no authorization header
  // to hold one in — this is the assertion that would fail if somebody
  // "helpfully" added one.
  let seen: RequestInit | undefined;
  const spy: typeof fetch = (_url, init) => {
    seen = init;
    return Promise.resolve(new Response("{}", { status: 200 }));
  };
  await call("/whoami", { method: "GET" }, spy);
  assert.equal(seen?.credentials, "same-origin");
  const headers = (seen?.headers ?? {}) as Record<string, string>;
  assert.ok(!("authorization" in headers), "the console must send no bearer");
  assert.ok(!("Authorization" in headers), "the console must send no bearer");
});

test("an empty body is not a parse failure", async () => {
  // `JSON.parse("")` throws, and a console that let that escape would
  // report a success as a crash. Both shapes of empty: a 204, which may
  // carry no body at all, and a 200 that simply has none.
  const noBody: typeof fetch = () => Promise.resolve(new Response(null, { status: 204 }));
  assert.equal((await call("/whoami", {}, noBody)).kind, "ok");

  const emptyBody: typeof fetch = () => Promise.resolve(new Response("", { status: 200 }));
  assert.equal((await call("/whoami", {}, emptyBody)).kind, "ok");
});

test("a body that is not json does not crash the surface", async () => {
  // A proxy's HTML error page reaching a JSON client is the classic case.
  const html: typeof fetch = () =>
    Promise.resolve(new Response("<html>502</html>", { status: 502 }));
  const outcome = await call("/proposals", {}, html);
  assert.equal(outcome.kind, "unavailable");
});
