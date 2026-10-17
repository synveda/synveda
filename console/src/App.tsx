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

import { canExplore } from "./onboarding.mjs";
import { useCallback, useEffect, useState } from "react";
import { accountKey, inviteStore, readPending } from "./invite.mjs";
import { InviteAcceptance, InviteSignIn } from "./Invite.js";

import { SIGN_IN_URL, type Outcome } from "./api.mjs";
import { Brand } from "./Brand.js";
import { request } from "./client.mjs";
import { Failure, Loading, useQuery, useRefresh } from "./Query.js";
import { navigate, useHistoryEvents, useRoute } from "./Router.js";
import {
  AppProvider,
  NotFound,
  NotOffered,
  Shell,
  appContext,
} from "./Shell.js";
import {
  browserSelectionStore,
  readStored,
  reconcile,
  selectionKey,
  writeStored,
  type Selection,
} from "./selection.mjs";
import { hrefOf, offersRoute, routeOf, type RouteMatch } from "./routes.mjs";
import type { Entry } from "./cache.mjs";
import type { MeView } from "./generated/api.js";

import { Home } from "./Home.js";
import { Knowledge, KnowledgeItem } from "./Knowledge.js";
import { Context, ContextInspector } from "./Context.js";
import { Learnings } from "./Learnings.js";
import { Onboarding } from "./Onboarding.js";
import { OkfExchange } from "./Okf.js";
import { Operations } from "./Operations.js";
import { People } from "./People.js";
import { ProposalReview, Reviews } from "./Reviews.js";
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

/** The account (`tenant:subject`) this document first rendered for. */
let documentAccount: string | null = null;

/**
 * Starts a fresh document when a refreshed `/v1/me` names another account —
 * another tab signed out and in as somebody else. Every cached read in this
 * document was decided for the first account, and a reload is the one path
 * that cannot show any of it to the second.
 */
function useAccountGuard(entry: Entry): void {
  const me =
    entry.status === "ready" && entry.outcome.kind === "ok" ? (entry.outcome.body as MeView) : null;
  const account = me ? accountKey(me.tenant.id, me.principal.subject) : null;
  useEffect(() => {
    if (account === null) return;
    if (documentAccount === null) {
      documentAccount = account;
    } else if (documentAccount !== account) {
      window.location.assign(hrefOf("home"));
    }
  }, [account]);
}

export function App() {
  useHistoryEvents();
  const route = useRoute();
  const entry = useQuery(ME_KEY, () => request("get_me", {}));
  const retry = useRefresh(ME_KEY);
  useAccountGuard(entry);

  // An invitation opened in this tab is reachable before any selection and
  // before the onboarding redirect: a completed sign-in lands on Home, and
  // Home becomes the invitation page while one is waiting (`invite.mts`).
  const inviting =
    route?.id === "invite" ||
    (route?.id === "home" && readPending(inviteStore(), Date.now()) !== null);
  useEffect(() => {
    if (inviting && route?.id !== "invite") {
      navigate(hrefOf("invite"), { replace: true });
    }
  }, [inviting, route?.id]);

  // The gateway sends a failed console login back here with a
  // classification and nothing else (auth::console_error_redirect). Read
  // once, then stripped from the URL: an error that survives a refresh
  // outlives the thing it described. Read during the first render rather than
  // in an effect, so the invitation redirect above cannot replace the URL
  // before the classification is seen.
  const [loginError] = useState<string | null>(() =>
    typeof window === "undefined"
      ? null
      : new URLSearchParams(window.location.search).get("error"),
  );
  useEffect(() => {
    if (new URLSearchParams(window.location.search).has("error")) {
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
    return inviting ? <InviteSignIn error={loginError} /> : <SignIn error={loginError} />;
  }
  if (outcome.kind !== "ok") {
    return (
      <main className="centred">
        {loginError ? <Banner>Sign-in failed: {loginError}</Banner> : null}
        <Failure state={outcome} onRetry={retry} />
      </main>
    );
  }
  if (inviting) {
    return <InviteAcceptance me={outcome.body as MeView} />;
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
  const storageKey = selectionKey(me);
  const [preference, setPreference] = useState<Selection>(() =>
    readStored(browserSelectionStore(), storageKey),
  );
  const selection = reconcile(preference, me);

  const choose = useCallback(
    (next: Selection) => {
      writeStored(browserSelectionStore(), next, storageKey);
      setPreference(next);
    },
    [storageKey],
  );

  // The reconciled answer is written back, so a preference that named a
  // workspace this caller lost is replaced rather than re-resolved on every
  // load. Guarded on inequality: an unconditional write in an effect that
  // sets state is a render loop.
  useEffect(() => {
    if (
      selection.workspaceId !== preference.workspaceId ||
      selection.projectId !== preference.projectId
    ) {
      writeStored(browserSelectionStore(), selection, storageKey);
      setPreference(selection);
    }
    // The primitive ids rather than the objects: `reconcile` returns a fresh
    // object every render, so depending on it would re-run this on every
    // render for no reason.
  }, [
    selection.workspaceId,
    selection.projectId,
    preference.workspaceId,
    preference.projectId,
    storageKey,
  ]);

  const context = appContext(me, selection, choose);

  // First run. `blocked` and `access_required` are *not* sent to the wizard:
  // its first step would be a creation the gateway refuses. They land on Home,
  // which explains what is missing and how to get it. The two `needs_*`
  // states are only reported where that creation is forecast at its target.
  //
  // `replace` rather than `push`, so Back does not return somebody to a
  // wizard they have just finished. In an effect rather than in render,
  // because navigating is a side effect and a render that performs one runs
  // twice under StrictMode.
  const needsOnboarding =
    me.onboarding.state === "needs_workspace" ||
    me.onboarding.state === "needs_project";
  useEffect(() => {
    const explore = canExplore(me.tenant.id, me.principal.subject);
    if (needsOnboarding && !explore && route?.id !== "welcome") {
      navigate(hrefOf("welcome"), { replace: true });
    }
  }, [needsOnboarding, route?.id, me.tenant.id, me.principal.subject]);

  return (
    <AppProvider value={context}>
      <Shell
        route={route?.id ?? null}
        pageKey={route ? hrefOf(route.id, route.params) : null}
        context={context}
      >
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
    case "operations":
      return <Operations />;
    case "welcome":
      return <Onboarding />;
    case "invite":
      // `App` renders the invitation page before the shell; listed so the
      // switch stays exhaustive.
      return <InviteAcceptance me={me} />;
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
    case "context":
      return <Context />;
    case "context-run":
      return (
        <ContextInspector
          contextRunId={route.params.context_run_id as string}
        />
      );
    case "knowledge":
      return <Knowledge />;
    case "knowledge-item":
      return (
        <KnowledgeItem knowledgeId={route.params.knowledge_id as string} />
      );
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
    case "review":
      return <ProposalReview proposalId={route.params.proposal_id as string} />;
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
    <main className="centred sign-in">
      <Brand />
      {error ? <Banner>Sign-in failed: {error}</Banner> : null}
      <section>
        <p className="eyebrow">Your agent workspace</p>
        <h1>Welcome to Synveda</h1>
        <p className="muted">
          Sign in to find shared knowledge, review new learnings and manage the
          context your agents use.
        </p>
        <a className="button primary" href={SIGN_IN_URL}>
          Sign in
        </a>
        <p className="sign-in-note muted">
          Continue with the sign-in provider configured for this Synveda
          installation.
        </p>
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
