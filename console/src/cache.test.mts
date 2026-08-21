/**
 * The query cache (CPR-8, ADR-0075 decision 4).
 *
 * Four rules, four groups of assertions. The one worth reading is
 * invalidation: a mutation that dropped a cache entry and did not refetch it
 * would leave the screen showing what it had until the reader navigated
 * away and back — which is exactly the state the mutation was supposed to
 * end, and it is invisible in a manual test because navigating away is what
 * a person does next anyway.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import type { Outcome } from "./api.mjs";
import { QueryCache } from "./cache.mjs";

const ok = (body: unknown): Outcome => ({ kind: "ok", body });

test("a key never asked for reads as loading", () => {
  const cache = new QueryCache();
  assert.deepEqual(cache.read("workspaces"), { status: "loading" });
});

test("two askers at once share one request", async () => {
  // Rule 1. Every component on a page asks on mount; without this the
  // People page alone would send its member list three times.
  const cache = new QueryCache();
  let calls = 0;
  const loader = async () => {
    calls += 1;
    return ok({ n: calls });
  };
  await Promise.all([cache.ensure("k", loader), cache.ensure("k", loader), cache.ensure("k", loader)]);
  assert.equal(calls, 1);
  const entry = cache.read("k");
  assert.deepEqual(entry.status === "ready" ? entry.outcome : null, ok({ n: 1 }));
});

test("a cached key is not re-read on a later ensure", async () => {
  const cache = new QueryCache();
  let calls = 0;
  const loader = async () => {
    calls += 1;
    return ok(calls);
  };
  await cache.ensure("k", loader);
  await cache.ensure("k", loader);
  assert.equal(calls, 1);
});

test("a refusal is cached like any other answer", async () => {
  // Rule 4. The console's vocabulary distinguishes "the PDP said no" from
  // "the gateway is down"; a cache that stored only successes would throw
  // that away and re-ask a question already answered.
  const cache = new QueryCache();
  await cache.ensure("k", async () => ({ kind: "forbidden", message: "no" }));
  const entry = cache.read("k");
  assert.equal(entry.status, "ready");
  assert.equal(entry.status === "ready" ? entry.outcome.kind : "", "forbidden");
});

test("a loader that throws becomes an outcome, not a permanent spinner", async () => {
  const cache = new QueryCache();
  await cache.ensure("k", async () => {
    throw new Error("boom");
  });
  const entry = cache.read("k");
  assert.equal(entry.status, "ready");
  assert.equal(entry.status === "ready" ? entry.outcome.kind : "", "unavailable");
});

test("an entry is replaced rather than mutated", async () => {
  // Rule 2. `useSyncExternalStore` compares snapshots by identity, so a
  // mutated entry is a store that reports no change and then renders a
  // different thing.
  const cache = new QueryCache();
  await cache.ensure("k", async () => ok(1));
  const first = cache.read("k");
  await cache.refresh("k");
  const second = cache.read("k");
  assert.notEqual(first, second);
});

test("invalidating a watched key refetches it", async () => {
  // Rule 3, the half that matters.
  const cache = new QueryCache();
  let calls = 0;
  await cache.ensure("workspaces/w-1/members", async () => {
    calls += 1;
    return ok(calls);
  });
  const unwatch = cache.watch("workspaces/w-1/members");
  cache.invalidate("workspaces/w-1");
  // The refetch is started synchronously; let it settle.
  await cache.read("workspaces/w-1/members");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls, 2);
  const entry = cache.read("workspaces/w-1/members");
  assert.deepEqual(entry.status === "ready" ? entry.outcome : null, ok(2));
  unwatch();
});

test("invalidating an unwatched key drops it instead of refetching", async () => {
  // Nobody is looking, so the cheap thing is right: the next reader loads
  // it, and a page that is closed does not get to keep issuing requests.
  const cache = new QueryCache();
  let calls = 0;
  await cache.ensure("audit/events", async () => {
    calls += 1;
    return ok(calls);
  });
  cache.invalidate("audit");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls, 1);
  assert.deepEqual(cache.read("audit/events"), { status: "loading" });
});

test("invalidation is by prefix, and does not reach a neighbouring key", async () => {
  const cache = new QueryCache();
  await cache.ensure("workspaces/w-1/members", async () => ok("members"));
  await cache.ensure("workspaces/w-2/members", async () => ok("other"));
  cache.invalidate("workspaces/w-1");
  const kept = cache.read("workspaces/w-2/members");
  assert.deepEqual(kept.status === "ready" ? kept.outcome : null, ok("other"));
});

test("a subscriber hears about every change", async () => {
  const cache = new QueryCache();
  let heard = 0;
  const stop = cache.subscribe(() => {
    heard += 1;
  });
  await cache.ensure("k", async () => ok(1));
  assert.ok(heard >= 2, "one announcement for the request, one for the answer");
  stop();
  const before = heard;
  await cache.ensure("j", async () => ok(2));
  assert.equal(heard, before, "an unsubscribed listener hears nothing");
});

test("clear forgets everything — sign-out, and nothing else", async () => {
  const cache = new QueryCache();
  await cache.ensure("me", async () => ok({ subject: "robin" }));
  cache.clear();
  assert.deepEqual(cache.read("me"), { status: "loading" });
});
