/**
 * The console's query cache (CPR-8, ADR-0075 decision 4).
 *
 * One place where "read something from the gateway" is expressed, so that
 * every page loads, fails and refreshes the same way. Before this, each
 * screen carried its own `useState` + `useEffect` + `Outcome | {loading}`
 * triple, and each one got to invent its own answer to *what happens after
 * a mutation* — which is how two surfaces come to disagree about whether
 * they are showing the current state.
 *
 * # Why it is written here rather than installed
 *
 * A cache is not a small thing to write and this one is deliberately small
 * — a map, a listener set, and one honest invalidation rule — because the
 * alternative is a runtime dependency in a bundle governed by
 * `scripts/check-npm-licences.mjs` and served under a
 * `default-src 'none'` Content-Security-Policy. What the well-known
 * libraries add over this (retries, windowing, suspense, devtools) is not
 * what an admin console needs, and every one of them is a shipped
 * dependency whose licence and supply chain somebody then owns.
 *
 * # The rules, all of them
 *
 * 1. **A key is the identity of a read.** Same key, same answer; two
 *    components asking at once share one request rather than racing.
 * 2. **An entry is immutable.** Every change replaces it, so a React
 *    subscriber can compare by identity and `useSyncExternalStore` is
 *    stable — a mutated entry would be a store that reports no change and
 *    then renders a different thing.
 * 3. **Invalidation refetches.** `invalidate("workspaces")` drops every
 *    entry under that prefix *and re-runs the loader of the ones somebody
 *    is watching*. Marking stale without refetching would leave a screen
 *    showing what it had until the reader navigated away and back, which is
 *    exactly the state a mutation was supposed to end.
 * 4. **A failed read is a cached answer, not an absent one.** The console's
 *    `Outcome` distinguishes "the PDP said no" from "the gateway is down",
 *    and a cache that only stored successes would throw that away and
 *    re-ask a question already answered.
 */

import type { Outcome } from "./api.mjs";

/** What a caller sees for one key. */
export type Entry =
  | { status: "loading" }
  | { status: "ready"; outcome: Outcome; loadedAt: number };

/** A read: whatever produces the outcome for a key. */
export type Loader = () => Promise<Outcome>;

type Listener = () => void;

interface Slot {
  entry: Entry;
  loader: Loader;
  /** Identifies the request allowed to settle this key. */
  generation: number;
  /** The in-flight request, so two askers share one. */
  inflight: Promise<void> | null;
  /** How many mounted readers are watching. Drives rule 3. */
  watchers: number;
}

/** The loading entry, shared so identity comparison is cheap and stable. */
const LOADING: Entry = { status: "loading" };

export class QueryCache {
  private readonly slots = new Map<string, Slot>();
  private readonly listeners = new Set<Listener>();
  private generation = 0;
  /** Injected so a test can assert `loadedAt` without waiting on a clock. */
  constructor(private readonly now: () => number = () => Date.now()) {}

  /** Subscribes to every change. Returns the unsubscribe. */
  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** The current entry for a key — `loading` for one never asked. */
  read = (key: string): Entry => this.slots.get(key)?.entry ?? LOADING;

  /**
   * Ensures a key is being loaded, and returns when it has been.
   *
   * Idempotent while a request is in flight (rule 1): the second caller
   * awaits the first's promise rather than sending a second request, which
   * is what makes it safe for every component on a page to ask for the
   * same thing on mount.
   */
  ensure = async (key: string, loader: Loader): Promise<void> => {
    const existing = this.slots.get(key);
    if (existing) {
      // The loader is refreshed even on a hit: a page that re-renders with
      // a new closure (a new selected project, say) must invalidate to a
      // *current* loader, not the one captured when the key first appeared.
      existing.loader = loader;
      if (existing.inflight) return existing.inflight;
      if (existing.entry.status === "ready") return;
    }
    return this.run(key, loader);
  };

  /** Re-runs a key's loader now, keeping what is on screen until it lands. */
  refresh = async (key: string): Promise<void> => {
    const slot = this.slots.get(key);
    if (!slot) return;
    if (slot.inflight) return slot.inflight;
    return this.run(key, slot.loader);
  };

  /**
   * Drops everything under a key prefix, and refetches what is watched.
   *
   * The prefix is the unit of invalidation because keys are built as
   * `"workspaces"`, `"workspaces/<id>/members"` and so on: a mutation knows
   * the noun it changed and should not have to enumerate every read of it.
   */
  invalidate = (prefix: string): void => {
    const affected = [...this.slots.entries()].filter(([key]) => key.startsWith(prefix));
    if (affected.length === 0) return;
    for (const [key, slot] of affected) {
      if (slot.watchers > 0) {
        // Watched: re-ask, keeping the last answer visible meanwhile. The
        // alternative — clear, then load — flashes every open panel back to
        // "loading" on every mutation.
        void this.run(key, slot.loader);
      } else {
        this.slots.delete(key);
      }
    }
    this.announce();
  };

  /** Forgets everything. Sign-out, and nothing else. */
  clear = (): void => {
    this.slots.clear();
    this.announce();
  };

  /** Registers a mounted reader, so rule 3 knows what to refetch. */
  watch = (key: string): (() => void) => {
    const slot = this.slots.get(key);
    if (slot) slot.watchers += 1;
    return () => {
      const current = this.slots.get(key);
      if (current && current.watchers > 0) current.watchers -= 1;
    };
  };

  private run(key: string, loader: Loader): Promise<void> {
    const previous = this.slots.get(key);
    const generation = ++this.generation;
    const slot: Slot = {
      entry: previous?.entry ?? LOADING,
      loader,
      generation,
      inflight: null,
      watchers: previous?.watchers ?? 0,
    };
    this.slots.set(key, slot);
    const inflight = loader()
      .then((outcome) => {
        this.settle(key, generation, { status: "ready", outcome, loadedAt: this.now() });
      })
      .catch((cause: unknown) => {
        // A loader that throws is a bug in a page, not a gateway failure —
        // but a cache entry stuck on `loading` forever is worse than an
        // honest error, so it is recorded as one the console can render.
        this.settle(key, generation, {
          status: "ready",
          outcome: {
            kind: "unavailable",
            message: cause instanceof Error ? cause.message : "the console could not read this",
          },
          loadedAt: this.now(),
        });
      });
    slot.inflight = inflight;
    this.announce();
    return inflight;
  }

  private settle(key: string, generation: number, entry: Entry): void {
    const slot = this.slots.get(key);
    // Invalidation may have started a newer request while this one was in
    // flight. Only the newest generation may replace the visible answer.
    if (!slot || slot.generation !== generation) return;
    // Replaced rather than mutated (rule 2).
    this.slots.set(key, { ...slot, entry, inflight: null });
    this.announce();
  }

  private announce(): void {
    for (const listener of [...this.listeners]) listener();
  }
}

/** The cache the application uses. One per document. */
export const cache = new QueryCache();
