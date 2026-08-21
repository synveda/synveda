/**
 * The console's route table and its navigation model (CPR-8, ADR-0075).
 *
 * Everything about *where a page lives and who may see it* is here, as
 * data, so that the shell renders a table rather than a hand-written menu
 * — and so that the gating rule can be tested without rendering anything.
 *
 * # Two groups, and the difference between them is the product's shape
 *
 * The **primary** group is the product: what somebody came here to do.
 * It is shown to everybody, unconditionally, because a nav item that
 * appears and disappears with a role turns the shape of the application
 * into a function of who is looking at it — and the first question a new
 * user asks ("what is this thing?") should have the same answer for all
 * of them.
 *
 * The **advanced** group is governance: reviews, scopes, policies, audit,
 * service identities. Those are shown only to a caller whose capability
 * probe says they may read the plane behind them. This is not enforcement
 * and cannot be — see the forecast note below — it is the difference
 * between an administrator's console and everybody else's.
 *
 * # A capability here is a forecast, never a grant
 *
 * ADR-0058 decision 2, restated where somebody editing this file will meet
 * it: `MeView.capabilities.actions` is what the PDP said when `/v1/me` was
 * answered, and it decides what to **offer**. It never decides what to
 * **allow**. A reader who reaches a guarded route anyway — a stale
 * forecast, a typed URL, a pack that moved — gets the gateway's own refusal
 * from the page's own calls, which is the answer that counts. The guard
 * exists so a viewer is not shown five screens that will 403; it does not
 * exist to keep anybody out, and nothing here should ever be the only thing
 * standing between a caller and an act.
 */

/** Where the gateway mounts the bundle (`console::CONSOLE_PREFIX`). */
export const BASE = "/console";

/** Every page the console has. Closed, so a switch over it is exhaustive. */
export type RouteId =
  | "home"
  | "sessions"
  | "knowledge"
  | "learnings"
  | "skills"
  | "tools"
  | "people"
  | "settings"
  | "reviews"
  | "scopes"
  | "policies"
  | "audit"
  | "service-identities"
  | "welcome";

/** Which menu a route belongs to, or neither. */
export type NavGroup = "primary" | "advanced" | "none";

export interface RouteDef {
  id: RouteId;
  /** The path under {@link BASE}; `""` is the index. */
  segment: string;
  label: string;
  group: NavGroup;
  /**
   * The tenant-plane action a caller must be forecast to hold. Absent means
   * the route is offered to everybody — which is every primary route, by
   * the group's own rule above.
   *
   * The strings are `synveda_policy::Action::as_str()`'s, because that is
   * what `/v1/me` keys its map by. A name that drifts is a route that
   * silently disappears from every menu, which is why
   * `routes.test.mts` pins the whole set.
   */
  capability?: string;
  /** One line under the heading: what this page is for. */
  blurb: string;
}

/**
 * The table. Order is menu order.
 *
 * `welcome` is `none`: first-run onboarding is reached by being sent there,
 * not by choosing it, and a menu item for it would still be sitting in the
 * navigation after it had been completed.
 */
export const ROUTES: readonly RouteDef[] = [
  {
    id: "home",
    segment: "",
    label: "Home",
    group: "primary",
    blurb: "Where you are, what you have, and what to do next.",
  },
  {
    id: "sessions",
    segment: "sessions",
    label: "Sessions",
    group: "primary",
    blurb: "Every run of an agent against this project.",
  },
  {
    id: "knowledge",
    segment: "knowledge",
    label: "Knowledge",
    group: "primary",
    blurb: "What has been reviewed and published.",
  },
  {
    id: "learnings",
    segment: "learnings",
    label: "New Learnings",
    group: "primary",
    blurb: "What your sessions produced and nobody has stood behind yet.",
  },
  {
    id: "skills",
    segment: "skills",
    label: "Skills",
    group: "primary",
    blurb: "The governed skills your agents may load here.",
  },
  {
    id: "tools",
    segment: "tools",
    label: "Tools",
    group: "primary",
    blurb: "The governed tool registry.",
  },
  {
    id: "people",
    segment: "people",
    label: "People",
    group: "primary",
    blurb: "Who may act here, why, and what you can change about it.",
  },
  {
    id: "settings",
    segment: "settings",
    label: "Settings",
    group: "primary",
    blurb: "This workspace, this project, and the repositories it is about.",
  },
  {
    id: "reviews",
    segment: "advanced/reviews",
    label: "Reviews",
    group: "advanced",
    capability: "proposal.read",
    blurb: "The proposals waiting on a verdict.",
  },
  {
    id: "scopes",
    segment: "advanced/scopes",
    label: "Scopes",
    group: "advanced",
    capability: "scope.read",
    blurb: "The governed scope tree, the pack in force, and what you may do.",
  },
  {
    id: "policies",
    segment: "advanced/policies",
    label: "Policies",
    group: "advanced",
    capability: "policy.read",
    blurb: "The packs this tenant can assign, and the relaxations standing.",
  },
  {
    id: "audit",
    segment: "advanced/audit",
    label: "Audit",
    group: "advanced",
    capability: "audit.read",
    blurb: "The hash-chained record, and whether it still verifies.",
  },
  {
    id: "service-identities",
    segment: "advanced/service-identities",
    label: "Service identities",
    group: "advanced",
    capability: "service_identity.read",
    blurb: "The agents registered to act in this tenant.",
  },
  {
    id: "welcome",
    segment: "welcome",
    label: "Getting started",
    group: "none",
    blurb: "From nothing to an agent that can reach this deployment.",
  },
] as const;

/** The route with this id. Total, because {@link RouteId} is closed. */
export function routeOf(id: RouteId): RouteDef {
  const found = ROUTES.find((route) => route.id === id);
  if (!found) {
    throw new Error(`no route definition for ${id}`);
  }
  return found;
}

/** The URL for a route, ready for an `href`. */
export function hrefOf(id: RouteId): string {
  const segment = routeOf(id).segment;
  return segment.length === 0 ? `${BASE}/` : `${BASE}/${segment}`;
}

/**
 * The route a pathname names, or `null` for one the console does not have.
 *
 * `null` is rendered as a not-found page rather than redirected to Home:
 * the gateway's SPA fallback answers *every* path under the prefix with the
 * bundle (`console.rs`), so a typo arrives here rather than at a 404, and
 * silently landing on Home would tell somebody their link worked.
 */
export function matchRoute(pathname: string): RouteId | null {
  if (!pathname.startsWith(BASE)) {
    return null;
  }
  // Everything between the prefix and any trailing slash, normalised, so
  // `/console`, `/console/` and `/console/people/` all behave.
  const rest = pathname.slice(BASE.length).replace(/^\/+/, "").replace(/\/+$/, "");
  const found = ROUTES.find((route) => route.segment === rest);
  return found ? found.id : null;
}

/** Whether a caller's forecast offers a route. See the forecast note above. */
export function offersRoute(route: RouteDef, actions: Record<string, boolean>): boolean {
  return route.capability === undefined || actions[route.capability] === true;
}

/** The primary menu — every primary route, for everybody. */
export function primaryNav(): RouteDef[] {
  return ROUTES.filter((route) => route.group === "primary");
}

/** The advanced menu — only the planes this caller is forecast to read. */
export function advancedNav(actions: Record<string, boolean>): RouteDef[] {
  return ROUTES.filter((route) => route.group === "advanced" && offersRoute(route, actions));
}
