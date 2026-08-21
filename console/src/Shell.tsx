/**
 * The product shell (CPR-8, ADR-0075 decision 1): the frame every page is
 * rendered inside.
 *
 * What replaced what is worth stating, because this is a hard cut. Until
 * this feature the console's entry point was *governance-first*: signing in
 * put you in front of the proposals inbox and the scope explorer, mounted
 * directly by `App.tsx` one after the other, with no navigation at all. That
 * is the right first screen for the person who reviews other people's
 * publications and the wrong one for everybody else, and for a product
 * whose smallest unit is now one person it is the wrong one for the person
 * who just installed it. Both surfaces still exist and are unchanged in
 * substance; they moved to Advanced ▸ Reviews and Advanced ▸ Scopes, behind
 * their own capability, where somebody goes to govern rather than lands to
 * work.
 *
 * The shell carries four things: the two switchers, the two menus, the
 * caller's identity, and the outlet. Nothing else — a shell that reached
 * into a page's data would be a second place a page's state lives.
 */

import { createContext, useCallback, useContext } from "react";

import { signOut } from "./api.mjs";
import { invalidate } from "./Query.js";
import { Link } from "./Router.js";
import { advancedNav, hrefOf, primaryNav, routeOf, type RouteDef, type RouteId } from "./routes.mjs";
import {
  orderedWorkspaces,
  projectsOf,
  selectedProject,
  selectedWorkspace,
  type Selection,
} from "./selection.mjs";
import type { MeView, ProjectView, WorkspaceView } from "./generated/api.js";

/**
 * What every page needs and no page should fetch for itself: who is
 * calling, what they can see, and what they have currently selected.
 *
 * A context rather than props threaded through the shell, because the
 * alternative is every page taking five arguments it forwards unchanged.
 * It holds `/v1/me`'s answer — one call, made once, at the top — which is
 * the shape that call was designed for (ADR-0071 decision 2).
 */
export interface AppContextValue {
  me: MeView;
  selection: Selection;
  workspace: WorkspaceView | null;
  project: ProjectView | null;
  chooseWorkspace: (id: string) => void;
  chooseProject: (id: string | null) => void;
  /** Re-reads `/v1/me` — after creating a workspace or a project. */
  reload: () => void;
}

const AppContext = createContext<AppContextValue | null>(null);

export const AppProvider = AppContext.Provider;

/** The context, or a loud failure — a page outside the shell is a bug. */
export function useApp(): AppContextValue {
  const value = useContext(AppContext);
  if (!value) {
    throw new Error("useApp outside the shell");
  }
  return value;
}

/** Builds the context value from `/v1/me` and the reconciled selection. */
export function appContext(
  me: MeView,
  selection: Selection,
  choose: (next: Selection) => void,
): AppContextValue {
  return {
    me,
    selection,
    workspace: selectedWorkspace(me, selection),
    project: selectedProject(me, selection),
    chooseWorkspace: (id: string) => {
      // The project is dropped rather than carried: an id is only
      // meaningful inside its own workspace (`selection.mts` rule 3), and
      // `reconcile` picks the new workspace's first project.
      choose({ workspaceId: id, projectId: null });
    },
    chooseProject: (id: string | null) => {
      choose({ workspaceId: selection.workspaceId, projectId: id });
    },
    reload: () => invalidate("me"),
  };
}

export function Shell({
  route,
  context,
  children,
}: {
  route: RouteId | null;
  context: AppContextValue;
  children: React.ReactNode;
}) {
  const onSignOut = useCallback(async () => {
    await signOut();
    // Reload rather than clear local state: the cookie is gone, so every
    // subsequent call is a 401 anyway, and a fresh load is the one path
    // that cannot leave a stale view of a session that no longer exists.
    window.location.assign("/console/");
  }, []);

  const advanced = advancedNav(context.me.capabilities.actions);

  return (
    <div className="shell">
      <header className="shell-header">
        <div className="brand">
          <Link href={hrefOf("home")} className="brand-link">
            Synveda
          </Link>
          <span className="muted tenant">{context.me.tenant.name}</span>
        </div>
        <div className="switchers">
          <WorkspaceSwitcher context={context} />
          <ProjectSwitcher context={context} />
        </div>
        <div className="identity">
          <span className="muted">
            {context.me.principal.display_name ?? context.me.principal.subject}
          </span>
          <button type="button" onClick={() => void onSignOut()}>
            Sign out
          </button>
        </div>
      </header>

      <div className="shell-body">
        <nav className="sidebar" aria-label="Primary">
          <ul className="nav">
            {primaryNav().map((item) => (
              <NavItem key={item.id} item={item} current={route} />
            ))}
          </ul>
          {/* Rendered only when there is something in it. An empty
              "Advanced" heading tells a viewer there is a part of the
              product they cannot see, which is both true and useless. */}
          {advanced.length > 0 ? (
            <>
              <h2 className="nav-heading">Advanced</h2>
              <ul className="nav">
                {advanced.map((item) => (
                  <NavItem key={item.id} item={item} current={route} />
                ))}
              </ul>
            </>
          ) : null}
        </nav>
        <main className="page">{children}</main>
      </div>
    </div>
  );
}

function NavItem({ item, current }: { item: RouteDef; current: RouteId | null }) {
  const selected = item.id === current;
  return (
    <li>
      <Link
        href={hrefOf(item.id)}
        className={selected ? "nav-link selected" : "nav-link"}
        // The accessible name of "which page am I on" — a class alone says
        // it to a sighted reader and to nobody else.
      >
        <span aria-current={selected ? "page" : undefined}>{item.label}</span>
      </Link>
    </li>
  );
}

function WorkspaceSwitcher({ context }: { context: AppContextValue }) {
  const workspaces = orderedWorkspaces(context.me);
  if (workspaces.length === 0) {
    return <span className="muted">no workspace yet</span>;
  }
  return (
    <label className="switcher">
      <span className="switcher-label">Workspace</span>
      <select
        value={context.selection.workspaceId ?? ""}
        onChange={(event) => context.chooseWorkspace(event.target.value)}
      >
        {workspaces.map((workspace) => (
          <option key={workspace.id} value={workspace.id}>
            {workspace.display_name}
            {workspace.status === "archived" ? " (archived)" : ""}
          </option>
        ))}
      </select>
    </label>
  );
}

function ProjectSwitcher({ context }: { context: AppContextValue }) {
  const projects = projectsOf(context.me, context.selection.workspaceId);
  if (projects.length === 0) {
    // Not hidden: "this workspace has no project" is a fact somebody needs,
    // and a switcher that vanished would read as a broken header.
    return <span className="muted">no project yet</span>;
  }
  return (
    <label className="switcher">
      <span className="switcher-label">Project</span>
      <select
        value={context.selection.projectId ?? ""}
        onChange={(event) => context.chooseProject(event.target.value || null)}
      >
        {projects.map((project) => (
          <option key={project.id} value={project.id}>
            {project.display_name}
            {project.status === "archived" ? " (archived)" : ""}
          </option>
        ))}
      </select>
    </label>
  );
}

/** The heading every page opens with, from the route table. */
export function PageHeading({ route }: { route: RouteId }) {
  const def = routeOf(route);
  return (
    <header className="page-heading">
      <h1>{def.label}</h1>
      <p className="muted">{def.blurb}</p>
    </header>
  );
}

/**
 * What a page shows when the caller's forecast does not offer its plane.
 *
 * Reached by typing the URL, or by a forecast that aged. It says what was
 * missing rather than redirecting, because a redirect to Home tells
 * somebody their link was wrong when in fact their role was.
 */
export function NotOffered({ route }: { route: RouteId }) {
  const def = routeOf(route);
  return (
    <>
      <PageHeading route={route} />
      <div className="banner error" role="alert">
        You do not hold {def.capability ?? "the role"} in this tenant, so this page has nothing
        to show you.
        <p className="muted">
          This is what the policy decision point said when this page loaded. Ask an administrator
          for the role, and reload — signing in again will not change the answer.
        </p>
      </div>
    </>
  );
}

/** A path the console does not have. */
export function NotFound() {
  return (
    <>
      <header className="page-heading">
        <h1>No such page</h1>
        <p className="muted">This console has no page at that address.</p>
      </header>
      <p>
        <Link href={hrefOf("home")}>Go to Home</Link>
      </p>
    </>
  );
}
