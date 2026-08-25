/**
 * The console's entry point (CPR-8, ADR-0075).
 *
 * One call, then one of four things. `GET /v1/me` answers who is calling,
 * which tenant, what exists, what is missing and what this caller may do
 * (ADR-0071 decision 2) — which is exactly the set of facts that decides
 * whether somebody sees a sign-in page, an onboarding wizard, the product,
 * or an error. Before CPR-8 this component asked `whoami`, learned four
 * fewer things, and mounted the proposals inbox and the scope explorer one
 * after the other with no navigation at all.
 *
 * # The onboarding gate is the server's answer, not an inference
 *
 * `me.onboarding.state` is computed by the gateway from the same rows it
 * would refuse a project creation against. The console branches on that
 * word and never on `workspaces.length === 0`, because the two differ
 * exactly when it matters: a caller who can read no workspaces is not the
 * same as a deployment that has none, and only one of them should be shown
 * a "create your first workspace" wizard.
 */

import { useCallback, useEffect, useState } from "react";

import { SIGN_IN_URL, type Outcome } from "./api.mjs";
import { request } from "./client.mjs";
import { Failure, Loading, useQuery, useRefresh } from "./Query.js";
import { navigate, useHistoryEvents, useRoute } from "./Router.js";
import { AppProvider, NotFound, NotOffered, Shell, appContext } from "./Shell.js";
import {
  readStored,
  reconcile,
  writeStored,
  type Selection,
  type SelectionStore,
} from "./selection.mjs";
import { hrefOf, offersRoute, routeOf, type RouteMatch } from "./routes.mjs";
import type { MeView } from "./generated/api.js";

import { Home } from "./Home.js";
import { Knowledge, KnowledgeItem } from "./Knowledge.js";
import { ContextInspector } from "./Context.js";
import { Learnings } from "./Learnings.js";
import { Onboarding } from "./Onboarding.js";
import { OkfExchange } from "./Okf.js";
import { People } from "./People.js";
import { Reviews } from "./Reviews.js";
import { Session } from "./Session.js";
import { Sessions } from "./Sessions.js";
import { Scopes } from "./Scopes.js";
import { Settings } from "./Settings.js";
import { SkillItem, Skills } from "./Skills.js";
import { ToolServerItem, Tools } from "./Tools.js";
import { Audit } from "./Audit.js";
import { Configuration } from "./Configuration.js";
import { ServiceIdentities } from "./ServiceIdentities.js";

/** The key `/v1/me` is cached under. Invalidated by anything that changes it. */
export const ME_KEY = "me";

/** `localStorage`, or nothing at all where it is unavailable. */
function selectionStore(): SelectionStore | null {
  try {
    return window.localStorage;
  } catch {
    // Some browsers throw on the *accessor* when site data is blocked, not
    // just on the read. The console works without a remembered selection;
    // it does not work if the first thing it does is throw.
    return null;
  }
}

export function App() {
  useHistoryEvents();
  const route = useRoute();
  const entry = useQuery(ME_KEY, () => request("get_me", {}));
  const retry = useRefresh(ME_KEY);

  // The gateway sends a failed console login back here with a
  // classification and nothing else (auth::console_error_redirect). Read
  // once, then stripped from the URL: an error that survives a refresh
  // outlives the thing it described.
  const [loginError, setLoginError] = useState<string | null>(null);
  useEffect(() => {
    const error = new URLSearchParams(window.location.search).get("error");
    if (error) {
      setLoginError(error);
      window.history.replaceState({}, "", window.location.pathname);
    }
  }, []);

  if (entry.status === "loading") {
    return (
      <main className="centred">
        <Loading what="your session" />
      </main>
    );
  }
  const outcome: Outcome = entry.outcome;
  if (outcome.kind === "unauthenticated") {
    return <SignIn error={loginError} />;
  }
  if (outcome.kind !== "ok") {
    return (
      <main className="centred">
        {loginError ? <Banner>Sign-in failed: {loginError}</Banner> : null}
        <Failure state={outcome} onRetry={retry} />
      </main>
    );
  }
  return <SignedIn me={outcome.body as MeView} route={route} />;
}

/**
 * Everything behind a resolved session.
 *
 * Split from {@link App} so the selection hooks are only mounted once there
 * is a `MeView` to reconcile against — a hook that has to cope with "there
 * is no answer yet" is a hook that carries a null branch through every
 * line of it.
 */
function SignedIn({ me, route }: { me: MeView; route: RouteMatch | null }) {
  const [preference, setPreference] = useState<Selection>(() => readStored(selectionStore()));
  const selection = reconcile(preference, me);

  const choose = useCallback((next: Selection) => {
    const store = selectionStore();
    writeStored(store, next);
    setPreference(next);
  }, []);

  // The reconciled answer is written back, so a preference that named a
  // workspace this caller lost is replaced rather than re-resolved on every
  // load. Guarded on inequality: an unconditional write in an effect that
  // sets state is a render loop.
  useEffect(() => {
    if (
      selection.workspaceId !== preference.workspaceId ||
      selection.projectId !== preference.projectId
    ) {
      writeStored(selectionStore(), selection);
      setPreference(selection);
    }
    // The primitive ids rather than the objects: `reconcile` returns a fresh
    // object every render, so depending on it would re-run this on every
    // render for no reason.
  }, [selection.workspaceId, selection.projectId, preference.workspaceId, preference.projectId]);

  const context = appContext(me, selection, choose);

  // First run. `blocked` is the fourth state — a caller who may not create
  // anything and has nothing to see — and it is *not* sent to the wizard,
  // because the wizard's first step is a creation they would be refused.
  //
  // `replace` rather than `push`, so Back does not return somebody to a
  // wizard they have just finished. In an effect rather than in render,
  // because navigating is a side effect and a render that performs one runs
  // twice under StrictMode.
  const needsOnboarding =
    me.onboarding.state === "needs_workspace" || me.onboarding.state === "needs_project";
  useEffect(() => {
    if (needsOnboarding && route?.id !== "welcome") {
      navigate(hrefOf("welcome"), { replace: true });
    }
  }, [needsOnboarding, route?.id]);

  return (
    <AppProvider value={context}>
      <Shell route={route?.id ?? null} context={context}>
        <Page route={route} me={me} />
      </Shell>
    </AppProvider>
  );
}

/**
 * The route table's other half: which component a route renders.
 *
 * A `switch` over a closed union, so a route added to `routes.mts` without
 * a page here is a compile error rather than a blank screen.
 */
function Page({ route, me }: { route: RouteMatch | null; me: MeView }) {
  if (route === null) {
    return <NotFound />;
  }
  // The capability guard. A forecast, never a grant (`routes.mts`): it
  // decides what to render, the page's own calls still meet the gateway's
  // decision, and a reader who gets past a stale forecast sees a refusal
  // from the act rather than a blank page.
  if (!offersRoute(routeOf(route.id), me.capabilities.actions)) {
    return <NotOffered route={route.id} />;
  }
  switch (route.id) {
    case "home":
      return <Home />;
    case "welcome":
      return <Onboarding />;
    case "people":
      return <People />;
    case "settings":
      return <Settings />;
    case "skills":
      return <Skills />;
    case "skill-item":
      return <SkillItem skillId={route.params.skill_id as string} />;
    case "sessions":
      return <Sessions />;
    case "session":
      // The id comes from the URL, so a refresh and a pasted link land on
      // the same run. `matchRoute` cannot produce this route without it.
      return <Session sessionId={route.params.session_id as string} />;
    case "context-run":
      return <ContextInspector contextRunId={route.params.context_run_id as string} />;
    case "knowledge":
      return <Knowledge />;
    case "knowledge-item":
      return <KnowledgeItem knowledgeId={route.params.knowledge_id as string} />;
    case "learnings":
      return <Learnings />;
    case "okf":
      return <OkfExchange />;
    case "tools":
      return <Tools />;
    case "tool-server":
      return <ToolServerItem serverId={route.params.server_id as string} />;
    case "reviews":
      return <Reviews />;
    case "scopes":
      return <Scopes />;
    case "configuration":
      return <Configuration />;
    case "audit":
      return <Audit />;
    case "service-identities":
      return <ServiceIdentities />;
  }
}

function SignIn({ error }: { error: string | null }) {
  return (
    <main className="centred">
      <h1>Synveda</h1>
      {error ? <Banner>Sign-in failed: {error}</Banner> : null}
      <section>
        <h2>Sign in</h2>
        <p className="muted">
          You will be sent to your identity provider and back. The console keeps no token in your
          browser.
        </p>
        <a className="button" href={SIGN_IN_URL}>
          Sign in
        </a>
      </section>
    </main>
  );
}

function Banner({ children }: { children: React.ReactNode }) {
  return (
    <div className="banner error" role="alert">
      {children}
    </div>
  );
}
