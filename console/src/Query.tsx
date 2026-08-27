/**
 * React's view of the query cache (CPR-8, ADR-0075 decision 4).
 *
 * `cache.mts` holds the state and the rules; this is the twenty lines that
 * connect them to a component, plus the two pieces of shared furniture
 * every page then renders identically: {@link Loading} and {@link Failure}.
 *
 * The furniture is the point. Route-level loading and error states are a
 * requirement of this feature, and the way they stay consistent is that no
 * page writes one — a page says what it is reading and hands the outcome to
 * `Loaded`, and the difference between "the PDP said no" and "the gateway
 * is down" is rendered in one place, once, the way `api.mts` classified it.
 */

import { useCallback, useEffect, useSyncExternalStore } from "react";

import type { Outcome } from "./api.mjs";
import { cache, type Entry, type Loader } from "./cache.mjs";

/**
 * Reads a key, loading it if nobody has.
 *
 * The loader is intentionally **not** a dependency of the effect: it is a
 * closure that changes on every render, and depending on it would refetch
 * on every render. The key is the identity of the read (`cache.mts` rule 1)
 * — if what you are reading changed, the key must change, and if the key
 * did not change, neither did the read.
 */
export function useQuery(key: string, loader: Loader): Entry {
  const entry = useSyncExternalStore(
    cache.subscribe,
    useCallback(() => cache.read(key), [key]),
    useCallback(() => cache.read(key), [key]),
  );
  useEffect(() => {
    void cache.ensure(key, loader);
    return cache.watch(key);
    // `key` only: the loader is a fresh closure every render, and depending
    // on it would refetch every render. See the note above.
  }, [key]);
  return entry;
}

/** Re-reads a key now. What a Try again button calls. */
export function useRefresh(key: string): () => void {
  return useCallback(() => void cache.refresh(key), [key]);
}

/** What a mutation does when it lands: drop these prefixes and re-read. */
export function invalidate(...prefixes: string[]): void {
  for (const prefix of prefixes) cache.invalidate(prefix);
}

/** The one loading state. */
export function Loading({ what }: { what: string }) {
  return <p className="muted">Reading {what}…</p>;
}

/**
 * The one failure state.
 *
 * The 401/403 distinction is the reason this exists in one place: a session
 * that expired needs a reload, and a refusal needs a role — and a surface
 * that collapsed them would offer somebody who is already signed in a
 * button that returns them to the same refusal (`api.mts`).
 */
export function Failure({
  state,
  onRetry,
}: {
  state: Exclude<Outcome, { kind: "ok" }>;
  onRetry?: () => void;
}) {
  switch (state.kind) {
    case "unauthenticated":
      return (
        <div className="banner error" role="alert">
          Your session has expired. Reload the page to sign in again.
        </div>
      );
    case "forbidden":
      return (
        <div className="banner error" role="alert">
          Your roles do not allow this: {state.message}
          <p className="muted">
            Ask an administrator for the role this scope requires — signing in again will not
            change the answer.
          </p>
        </div>
      );
    case "unavailable":
      return (
        <div className="banner error" role="alert">
          The gateway is not answering: {state.message}
          {onRetry ? (
            <p>
              <button type="button" onClick={onRetry}>
                Try again
              </button>
            </p>
          ) : null}
        </div>
      );
    case "invalid":
    case "conflict":
      return (
        <div className="banner error" role="alert">
          {state.message}
        </div>
      );
  }
}

/**
 * Renders a cache entry: loading, the failure, or the body.
 *
 * The cast on the success body is the same one `client.mts` makes and for
 * the same reason — the transport parsed JSON and the contract is what says
 * which shape it is.
 */
export function Loaded<T>({
  entry,
  what,
  onRetry,
  children,
}: {
  entry: Entry;
  what: string;
  onRetry?: () => void;
  children: (body: T) => React.ReactNode;
}) {
  if (entry.status === "loading") {
    return <Loading what={what} />;
  }
  if (entry.outcome.kind !== "ok") {
    return <Failure state={entry.outcome} onRetry={onRetry} />;
  }
  return <>{children(entry.outcome.body as T)}</>;
}
