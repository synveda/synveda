/**
 * The page a shared invitation link opens (CPR-5, ADR-0072).
 *
 * It is reached before any workspace or project is selected and before the
 * ordinary onboarding redirect, signed in or out. It never accepts on its own:
 * the recipient sees which account would accept, then presses Accept, which
 * sends the authenticated `POST /v1/invites/{token}/accept` through the
 * generated client. What happens next comes from a fresh `/v1/me` — never
 * from the acceptance response — so membership the gateway does not show is
 * never presented as granted.
 */

import { useEffect, useState } from "react";

import { SIGN_IN_URL, signOut } from "./api.mjs";
import { Brand } from "./Brand.js";
import { cache } from "./cache.mjs";
import { request } from "./client.mjs";
import {
  INVITE_PAGE,
  accountCheck,
  accountKey,
  classifyAcceptance,
  clearPending,
  inviteStore,
  readPending,
  survivesSignIn,
  terminalHeading,
  writePending,
  type PendingInvite,
  type TerminalReason,
} from "./invite.mjs";
import { navigate } from "./Router.js";
import { hrefOf } from "./routes.mjs";
import { refreshAccess } from "./Shell.js";
import {
  browserSelectionStore,
  selectionAt,
  selectionKey,
  writeStored,
} from "./selection.mjs";
import type { MeView } from "./generated/api.js";

/** The key `/v1/me` is cached under (`App.ME_KEY`). */
const ME_KEY = "me";

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <main className="centred sign-in invite">
      <Brand />
      <section aria-labelledby="invite-heading">{children}</section>
    </main>
  );
}

function Continue({ label = "Continue to Synveda" }: { label?: string }) {
  return (
    <button type="button" onClick={() => navigate(hrefOf("home"), { replace: true })}>
      {label}
    </button>
  );
}

function NoInvitation() {
  return (
    <Frame>
      <h1 id="invite-heading">No invitation is waiting in this tab</h1>
      <p className="muted">
        An invitation link is kept in the tab that opened it for 20 minutes, until it is
        accepted or cancelled. Open the link you were sent again, or ask the person who invited
        you for a new one.
      </p>
      <p>
        <Continue />
      </p>
    </Frame>
  );
}

/** A signed-out recipient: sign in first; nothing is accepted on the way. */
export function InviteSignIn({ error }: { error: string | null }) {
  const store = inviteStore();
  const [pending, setPending] = useState(() => readPending(store, Date.now()));
  if (pending === null) return <NoInvitation />;
  const carried = survivesSignIn(store);
  return (
    <Frame>
      {error ? (
        <div className="banner error" role="alert">
          Sign-in did not complete ({error}). The invitation is still waiting in this tab: sign
          in again, or cancel it.
        </div>
      ) : null}
      <p className="eyebrow">Invitation</p>
      <h1 id="invite-heading">You have been invited to Synveda</h1>
      <p className="muted">
        Sign in with your own account. You will see which account is accepting before anything
        is accepted.
      </p>
      {carried ? null : (
        <div className="banner warning" role="status">
          This browser is not keeping data for this site, so the invitation cannot be carried
          through sign-in. Sign in first, then open the invitation link again.
        </div>
      )}
      <p>
        <a className="button primary" href={SIGN_IN_URL}>
          {error ? "Try signing in again" : "Sign in to continue"}
        </a>{" "}
        <button
          type="button"
          onClick={() => {
            clearPending(store);
            setPending(null);
          }}
        >
          Cancel invitation
        </button>
      </p>
      <p className="muted sign-in-note">
        Your administrator provides your sign-in account; an invitation adds access to it but
        does not create one.
      </p>
    </Frame>
  );
}

type Phase =
  | { kind: "review" }
  | { kind: "accepting" }
  | { kind: "retry"; reason: "session" | "network"; message: string }
  | { kind: "terminal"; reason: TerminalReason; message: string }
  | { kind: "not_visible"; scopeId: string };

/** A signed-in recipient: review the account, then accept explicitly. */
export function InviteAcceptance({ me }: { me: MeView }) {
  const store = inviteStore();
  const account = accountKey(me.tenant.id, me.principal.subject);
  const [pending] = useState<PendingInvite | null>(() => readPending(store, Date.now()));
  const check = pending ? accountCheck(pending, account) : null;
  const [phase, setPhase] = useState<Phase>({ kind: "review" });

  useEffect(() => {
    if (pending === null) return;
    if (check === "bind") writePending(store, { ...pending, account });
    // Another account opened it in this tab: it is not carried over.
    if (check === "mismatch") clearPending(store);
    // Once, for the invitation this page was opened with.
  }, []);

  if (pending === null) return <NoInvitation />;
  const who = me.principal.display_name ?? me.principal.subject;
  if (check === "mismatch") {
    return (
      <Frame>
        <h1 id="invite-heading">Open the invitation link again</h1>
        <p className="muted">
          This invitation was opened while a different account was signed in, so it was not
          carried over to {who}. Open the link again to review it as this account.
        </p>
        <p>
          <Continue />
        </p>
      </Frame>
    );
  }

  const land = async (scopeId: string) => {
    await refreshAccess();
    const entry = cache.read(ME_KEY);
    const fresh = entry.status === "ready" && entry.outcome.kind === "ok"
      ? (entry.outcome.body as MeView)
      : null;
    const target = fresh ? selectionAt(fresh, scopeId) : null;
    if (fresh && target) {
      writeStored(browserSelectionStore(), target, selectionKey(fresh));
      navigate(hrefOf("home"), { replace: true });
      return;
    }
    setPhase({ kind: "not_visible", scopeId });
  };

  const accept = async () => {
    setPhase({ kind: "accepting" });
    const result = classifyAcceptance(
      await request("accept_invite", { path: { invite_token: pending.token } }),
    );
    if (result.kind === "retry") {
      setPhase(result);
      return;
    }
    clearPending(store);
    if (result.kind === "terminal") {
      setPhase(result);
      return;
    }
    await land(result.scopeId);
  };

  const otherAccount = async () => {
    // Deliberately carried: the next signed-in account binds it afresh.
    writePending(store, { ...pending, account: null });
    await signOut();
    window.location.assign(INVITE_PAGE);
  };

  switch (phase.kind) {
    case "terminal":
      return (
        <Frame>
          <h1 id="invite-heading">{terminalHeading(phase.reason)}</h1>
          <p>{phase.message}</p>
          <p className="muted">
            Nothing was added to your access. Ask the person who invited you for a new
            invitation if you still need it.
          </p>
          <p>
            <Continue />
          </p>
        </Frame>
      );
    case "not_visible":
      return (
        <Frame>
          <h1 id="invite-heading">Invitation accepted</h1>
          <p className="muted">
            The gateway accepted the invitation, but the access it added is not visible to {who}{" "}
            yet — the policy in force may not let this role see that workspace. Refresh your
            access, or ask the person who invited you.
          </p>
          <p>
            <button type="button" className="primary" onClick={() => void land(phase.scopeId)}>
              Refresh access
            </button>{" "}
            <Continue />
          </p>
        </Frame>
      );
    case "review":
    case "accepting":
    case "retry":
      return (
        <Frame>
          <p className="eyebrow">Invitation</p>
          <h1 id="invite-heading">Join a workspace in {me.tenant.name}</h1>
          {phase.kind === "retry" ? (
            <div className="banner error" role="alert">
              {phase.reason === "session"
                ? "Your session ended before the invitation was accepted. Sign in again; the invitation is still waiting in this tab."
                : `The invitation was not confirmed: ${phase.message}. Try again — accepting twice as the same account is safe.`}
            </div>
          ) : null}
          <p>
            You will accept as <strong>{who}</strong>
            {who !== me.principal.subject ? (
              <>
                {" "}
                <span className="muted">({me.principal.subject})</span>
              </>
            ) : null}
            . The invitation adds the role it was issued with to this account.
          </p>
          <p>
            {phase.kind === "retry" && phase.reason === "session" ? (
              <a className="button primary" href={SIGN_IN_URL}>
                Sign in again
              </a>
            ) : (
              <button
                type="button"
                className="primary"
                disabled={phase.kind === "accepting"}
                onClick={() => void accept()}
              >
                {phase.kind === "accepting" ? "Accepting…" : "Accept invitation"}
              </button>
            )}{" "}
            <button
              type="button"
              disabled={phase.kind === "accepting"}
              onClick={() => {
                clearPending(store);
                navigate(hrefOf("home"), { replace: true });
              }}
            >
              Not now
            </button>{" "}
            <button
              type="button"
              disabled={phase.kind === "accepting"}
              onClick={() => void otherAccount()}
            >
              Sign out to use another account
            </button>
          </p>
          <p className="muted sign-in-note">
            Not now discards the invitation from this tab; the link itself stays valid until it
            expires or is used. Your sign-in provider may still sign you back in as {who}; sign
            out there too, or open the link in a private window, to choose another account.
          </p>
        </Frame>
      );
  }
}
