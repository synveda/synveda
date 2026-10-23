/**
 * The invitation continuation: the token leaves the address bar,
 * survives only this tab's sign-in for a bounded time, is never reused by
 * another account, and nothing here accepts anything.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  INVITE_PAGE,
  PENDING_INVITE_KEY,
  PENDING_INVITE_TTL_MS,
  accountCheck,
  accountKey,
  captureFromLocation,
  classifyAcceptance,
  clearPending,
  readPending,
  survivesSignIn,
  terminalHeading,
  tokenFromFragment,
  writePending,
  type InviteStore,
} from "./invite.mjs";

const TOKEN =
  "synveda_invite_v1.0199c000-0000-7000-8000-000000000000.AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCdE";

function memoryStore(): InviteStore & { values: Map<string, string> } {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
    removeItem: (key) => {
      values.delete(key);
    },
  };
}

function history() {
  const replaced: string[] = [];
  return {
    replaced,
    replaceState: (_data: unknown, _unused: string, url: string) => {
      replaced.push(url);
    },
  };
}

test("the shared link's fragment yields the token and nothing else", () => {
  assert.equal(tokenFromFragment(`#token=${TOKEN}`), TOKEN);
  assert.equal(tokenFromFragment(`token=${TOKEN}`), TOKEN);
  assert.equal(tokenFromFragment("#token=not-a-token"), null);
  assert.equal(tokenFromFragment(`#token=${TOKEN}<script>`), null);
  assert.equal(tokenFromFragment(`#other=${TOKEN}`), null);
  assert.equal(tokenFromFragment(""), null);
});

test("opening the link moves the token out of the address bar and into this tab", () => {
  const store = memoryStore();
  const moved = history();
  const captured = captureFromLocation(
    { pathname: INVITE_PAGE, search: "", hash: `#token=${TOKEN}` },
    moved,
    store,
    1_000,
  );
  assert.equal(captured, true);
  assert.deepEqual(moved.replaced, [INVITE_PAGE], "the fragment is gone from the entry");
  assert.deepEqual(readPending(store, 1_000), { token: TOKEN, capturedAt: 1_000, account: null });
  assert.equal(survivesSignIn(store), true);
  clearPending(store);
});

test("only the invitation page captures, and junk is stripped without being kept", () => {
  const store = memoryStore();
  const elsewhere = history();
  assert.equal(
    captureFromLocation({ pathname: "/console/", search: "", hash: `#token=${TOKEN}` }, elsewhere, store, 1),
    false,
  );
  assert.deepEqual(elsewhere.replaced, []);
  const junk = history();
  assert.equal(
    captureFromLocation({ pathname: `${INVITE_PAGE}/`, search: "", hash: "#token=nope" }, junk, store, 1),
    false,
  );
  assert.deepEqual(junk.replaced, [INVITE_PAGE]);
  assert.equal(readPending(store, 1), null);
});

test("a pending invitation lapses on its own and is removed when it does", () => {
  const store = memoryStore();
  writePending(store, { token: TOKEN, capturedAt: 0, account: null });
  assert.notEqual(readPending(store, PENDING_INVITE_TTL_MS), null);
  assert.equal(readPending(store, PENDING_INVITE_TTL_MS + 1), null);
  assert.equal(store.values.has(PENDING_INVITE_KEY), false, "expiry clears the stored copy");
  // A clock that moved backwards is not trusted to extend it either.
  writePending(store, { token: TOKEN, capturedAt: 10, account: null });
  assert.equal(readPending(store, 5), null);
});

test("a tampered stored value is ignored rather than presented", () => {
  const store = memoryStore();
  clearPending(store);
  store.values.set(PENDING_INVITE_KEY, JSON.stringify({ token: "x", capturedAt: 1, account: null }));
  assert.equal(readPending(store, 1), null);
  store.values.set(PENDING_INVITE_KEY, "not json");
  assert.equal(readPending(store, 1), null);
});

test("a browser that keeps nothing can still accept while signed in, and says it cannot carry sign-in", () => {
  writePending(null, { token: TOKEN, capturedAt: 1, account: null });
  assert.equal(readPending(null, 1)?.token, TOKEN);
  assert.equal(survivesSignIn(null), false);
  clearPending(null);
  assert.equal(readPending(null, 1), null);
});

test("the first signed-in account owns the decision; another account must reopen the link", () => {
  const mine = accountKey("tenant-1", "robin");
  const pending = { token: TOKEN, capturedAt: 1, account: null };
  assert.equal(accountCheck(pending, mine), "bind");
  assert.equal(accountCheck({ ...pending, account: mine }, mine), "match");
  assert.equal(accountCheck({ ...pending, account: mine }, accountKey("tenant-1", "sam")), "mismatch");
  assert.equal(accountCheck({ ...pending, account: mine }, accountKey("tenant-2", "robin")), "mismatch");
});

test("an acceptance answer separates success, retryable failure and a terminal invitation", () => {
  assert.deepEqual(classifyAcceptance({ kind: "ok", body: { scope_id: "scope-ws", grant: {} } }), {
    kind: "accepted",
    scopeId: "scope-ws",
  });
  // A body that cannot be read is not reported as success.
  assert.equal(classifyAcceptance({ kind: "ok", body: null }).kind, "retry");
  assert.deepEqual(classifyAcceptance({ kind: "unauthenticated" }), {
    kind: "retry",
    reason: "session",
    message: "your session has ended",
  });
  assert.deepEqual(classifyAcceptance({ kind: "unavailable", message: "502" }), {
    kind: "retry",
    reason: "network",
    message: "502",
  });
  const reasons = [
    ["this invitation expired at 2026-10-01T00:00:00+00:00; ask for another", "expired"],
    ["this invitation was withdrawn", "withdrawn"],
    ["this invitation has already been accepted by somebody else", "used"],
    ["something new", "unavailable"],
  ] as const;
  for (const [message, reason] of reasons) {
    const result = classifyAcceptance({ kind: "conflict", message });
    assert.deepEqual(result, { kind: "terminal", reason, message });
    assert.ok(terminalHeading(reason).length > 0);
  }
  assert.equal(
    (classifyAcceptance({ kind: "invalid", message: "invitation not found" }) as { reason: string })
      .reason,
    "not_found",
  );
  assert.equal(
    (classifyAcceptance({ kind: "forbidden", message: "invite.accept denied" }) as { reason: string })
      .reason,
    "refused",
  );
});
