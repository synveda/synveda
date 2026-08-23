/**
 * Client-side routing (CPR-8, ADR-0075 decision 1).
 *
 * The History API, a subscription to `popstate`, and a `Link` that does not
 * reload the page. That is the whole of it, and it is written here rather
 * than installed for the reason `cache.mts` gives: this bundle's dependency
 * list is governed by `scripts/check-npm-licences.mjs` and served under a
 * `default-src 'none'` policy, and what a routing library adds over a route
 * table (nested layouts, loaders, data revalidation) is machinery this
 * console does not have a use for — {@link routes.mts} is a flat table by
 * design.
 *
 * The gateway already anticipated this: `console.rs` answers **every** path
 * under `/console/` with the bundle rather than a 404, precisely so that a
 * client-side route is a real, linkable, refreshable URL.
 */

import { useCallback, useEffect, useSyncExternalStore } from "react";

import { matchRoute, type RouteMatch } from "./routes.mjs";

type Listener = () => void;

const listeners = new Set<Listener>();

function announce(): void {
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Goes to a path without reloading.
 *
 * Exported as a plain function rather than a hook so that a page can
 * navigate from an event handler — after a creation, say — without holding
 * a router object it otherwise has no use for.
 */
export function navigate(href: string, options: { replace?: boolean } = {}): void {
  if (window.location.pathname + window.location.search === href) {
    return;
  }
  if (options.replace) {
    window.history.replaceState({}, "", href);
  } else {
    window.history.pushState({}, "", href);
  }
  // The History API deliberately does not fire an event for a programmatic
  // navigation, so the store has to be told. This is the only place that
  // knows a navigation happened, which is why it is the only place allowed
  // to push one.
  announce();
  window.scrollTo(0, 0);
}

/** The current pathname, as a value React can subscribe to. */
function currentPath(): string {
  return window.location.pathname;
}

/** The server snapshot: there is no server render, so this is never used. */
function serverPath(): string {
  return "/console/";
}

/**
 * The route the browser is on and the parameters it carried, or `null` for a
 * path the console does not have. See {@link matchRoute} for why `null` is
 * rendered rather than redirected.
 *
 * The parameters come from the address bar and from nowhere else (CPR-11):
 * a detail page reads the id it is showing out of its own URL, so Back,
 * refresh and a pasted link all land on the same screen.
 */
export function useRoute(): RouteMatch | null {
  const path = useSyncExternalStore(subscribe, currentPath, serverPath);
  return matchRoute(path);
}

/** Wires the back and forward buttons into the same store. */
export function useHistoryEvents(): void {
  useEffect(() => {
    const onPop = () => announce();
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);
}

/**
 * An internal link.
 *
 * A real `<a href>`, so middle-click, copy-link and open-in-new-tab all
 * behave — a `<button>` that navigates is a link that has thrown those
 * away. The click handler only intercepts the plain-left-click case and
 * leaves every modified click to the browser.
 */
export function Link({
  href,
  className,
  children,
  onNavigate,
}: {
  href: string;
  className?: string;
  children: React.ReactNode;
  onNavigate?: () => void;
}) {
  const onClick = useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>) => {
      if (
        event.defaultPrevented ||
        event.button !== 0 ||
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey ||
        event.altKey
      ) {
        return;
      }
      event.preventDefault();
      navigate(href);
      onNavigate?.();
    },
    [href, onNavigate],
  );
  return (
    <a href={href} className={className} onClick={onClick}>
      {children}
    </a>
  );
}
