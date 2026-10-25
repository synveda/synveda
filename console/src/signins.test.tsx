/**
 * The Signed-in sessions panel (Settings).
 *
 * Rendering follows the console's convention (`renderToStaticMarkup` over a
 * primed cache, asserted as text); the requests the buttons make are
 * asserted at the seam below them, in `signins.mts`, with an injected fetch.
 */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { cache } from "./cache.mjs";
import type { ConsoleSessionPage, ConsoleSessionView } from "./generated/api.js";
import { SignIns } from "./SignIns.js";
import {
  REVOKE_ALL_CALL_LIMIT,
  REVOKE_ALL_LEDGER_FULL,
  LEDGER_FULL,
  revokeAll,
  revokeOne,
} from "./signins.mjs";
import { toText } from "./text.mjs";

beforeEach(() => cache.clear());

function view(overrides: Partial<ConsoleSessionView> = {}): ConsoleSessionView {
  return {
    id: "secret-session-id-1",
    issuer: "https://idp.example.test/realms/acme",
    created_at: "2026-10-20T08:00:00Z",
    last_seen_at: "2026-10-21T09:30:00Z",
    expires_at: "2026-10-20T20:00:00Z",
    ...overrides,
  };
}

async function render(page: ConsoleSessionPage): Promise<string> {
  await cache.ensure("me/console-sessions", async () => ({ kind: "ok", body: page }));
  return renderToStaticMarkup(<SignIns />);
}

interface Recorded {
  method: string;
  url: string;
}

/** A fetch answering from a script, recording what was asked. */
function scripted(answers: Array<{ status: number; body?: unknown }>): {
  fetchImpl: typeof fetch;
  calls: Recorded[];
} {
  const calls: Recorded[] = [];
  const fetchImpl = (async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ method: init?.method ?? "GET", url: String(input) });
    const answer = answers[Math.min(calls.length - 1, answers.length - 1)];
    assert.ok(answer);
    return new Response(answer.body === undefined ? null : JSON.stringify(answer.body), {
      status: answer.status,
    });
  }) as typeof fetch;
  return { fetchImpl, calls };
}

test("rows show the issuer and the three times, and never the session id", async () => {
  const markup = await render({ sessions: [view()], next_after: null });
  const text = toText(markup);
  assert.match(text, /Signed-in sessions/);
  assert.match(text, /https:\/\/idp\.example\.test\/realms\/acme/);
  assert.match(text, /2026-10-20 08:00 UTC/);
  assert.match(text, /2026-10-21 09:30 UTC/);
  assert.match(text, /2026-10-20 20:00 UTC/);
  assert.doesNotMatch(markup, /secret-session-id-1/);
  assert.match(text, /Sessions created before this list existed are not shown/);
  assert.match(text, /does not sign you out of your identity provider/);
  assert.doesNotMatch(text, /Show more/);
});

test("Show more appears only when the page names a next cursor", async () => {
  const markup = await render({ sessions: [view()], next_after: "cursor-1" });
  assert.match(toText(markup), /Show more/);
});

test("revoke calls DELETE with the id, and succeeds on 204", async () => {
  const { fetchImpl, calls } = scripted([{ status: 204 }]);
  const result = await revokeOne("a/b c", fetchImpl);
  assert.deepEqual(result, { kind: "ok" });
  assert.deepEqual(calls, [{ method: "DELETE", url: "/v1/me/console-sessions/a%2Fb%20c" }]);
});

test("a 429 says nothing was revoked", async () => {
  const { fetchImpl } = scripted([{ status: 429, body: { message: "ledger full" } }]);
  const result = await revokeOne("id-1", fetchImpl);
  assert.deepEqual(result, { kind: "ledger-full" });
  assert.match(LEDGER_FULL, /nothing was revoked/);
  const all = await revokeAll(scripted([{ status: 429 }]).fetchImpl);
  assert.deepEqual(all, { kind: "ledger-full", revoked: 0 });
});

test("a revoke-all 429 keeps what earlier batches revoked", async () => {
  const all = await revokeAll(
    scripted([{ status: 200, body: { revoked: 100, complete: false } }, { status: 429 }]).fetchImpl,
  );
  assert.deepEqual(all, { kind: "ledger-full", revoked: 100 });
  assert.doesNotMatch(REVOKE_ALL_LEDGER_FULL, /nothing was revoked/);
  assert.match(REVOKE_ALL_LEDGER_FULL, /stay revoked/);
});

test("revoke-all repeats until complete and totals the revoked sessions", async () => {
  const { fetchImpl, calls } = scripted([
    { status: 200, body: { revoked: 100, complete: false } },
    { status: 200, body: { revoked: 100, complete: false } },
    { status: 200, body: { revoked: 7, complete: true } },
  ]);
  const result = await revokeAll(fetchImpl);
  assert.deepEqual(result, { kind: "done", revoked: 207 });
  assert.equal(calls.length, 3);
  assert.ok(calls.every((c) => c.method === "POST" && c.url === "/v1/me/console-sessions/revoke-all"));
});

test("revoke-all is bounded and reports an incomplete outcome", async () => {
  const { fetchImpl, calls } = scripted([{ status: 200, body: { revoked: 1, complete: false } }]);
  const result = await revokeAll(fetchImpl);
  assert.deepEqual(result, { kind: "incomplete", revoked: REVOKE_ALL_CALL_LIMIT });
  assert.equal(calls.length, REVOKE_ALL_CALL_LIMIT);
});
