/**
 * The invitation a recipient opened, held across sign-in (ADR-0072).
 *
 * People shares `/console/invite#token=<token>`. The token is a bearer
 * credential, and the fragment is the one part of a URL no request carries:
 * not to the gateway, not to the reverse proxy's logs, not in a `Referer`.
 * {@link captureFromLocation} moves it out of the address bar before the
 * first render and into this tab's `sessionStorage`, which survives the OIDC
 * round trip in the same tab and nothing longer: it is never written to
 * `localStorage`, it is gone when the tab closes, and it lapses after
 * {@link PENDING_INVITE_TTL_MS} whatever happens.
 *
 * Nothing here accepts anything. Accepting is the authenticated
 * `POST /v1/invites/{token}/accept`, sent only when the recipient presses
 * the button on a page that names the account it will accept as.
 */

import type { Outcome } from "./api.mjs";

/** Where an invitation link opens (`access::INVITE_PAGE`). */
export const INVITE_PAGE = "/console/invite";

/** The token's greppable prefix (`synveda_identity::invite`). */
const TOKEN_PREFIX = "synveda_invite_v1.";

/** Longer than a minted token (prefix, tenant uuid, 43-character secret). */
const MAX_TOKEN_LENGTH = 128;

/** Namespaced, because the origin is shared with every other console key. */
export const PENDING_INVITE_KEY = "synveda.console.pending-invite";

/**
 * How long an opened invitation waits for this tab to sign in and decide.
 * Longer than the gateway's ten-minute pending login, so one retried sign-in
 * fits; short enough that a forgotten tab does not keep a credential.
 */
export const PENDING_INVITE_TTL_MS = 20 * 60 * 1000;

/** An invitation opened in this tab and not yet decided. */
export interface PendingInvite {
  token: string;
  /** When it was captured, in epoch milliseconds. */
  capturedAt: number;
  /**
   * The signed-in account (`tenant:subject`) it was first shown to. A
   * different account later in this tab must open the link again rather than
   * inherit it silently.
   */
  account: string | null;
}

/** The subset of `Storage` this uses. */
export interface InviteStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/**
 * The tab's pending invitation where `sessionStorage` is unavailable. Such a
 * tab can still accept while signed in; it cannot carry the invitation
 * through a sign-in redirect, and the page says so.
 */
let memoryPending: PendingInvite | null = null;

/** This tab's `sessionStorage`, or nothing where the browser refuses it. */
export function inviteStore(): InviteStore | null {
  try {
    return typeof window === "undefined" ? null : window.sessionStorage;
  } catch {
    // Blocked site data throws on the accessor itself.
    return null;
  }
}

/** The account key a pending invitation is bound to. */
export function accountKey(tenantId: string, subject: string): string {
  return `${tenantId}:${subject}`;
}

/** The token in `#token=…`, if it has the shape of one. */
export function tokenFromFragment(hash: string): string | null {
  const raw = hash.startsWith("#") ? hash.slice(1) : hash;
  if (raw.length === 0 || raw.length > MAX_TOKEN_LENGTH * 2) return null;
  const token = new URLSearchParams(raw).get("token");
  return token !== null && isTokenShaped(token) ? token : null;
}

function isTokenShaped(token: string): boolean {
  return (
    token.startsWith(TOKEN_PREFIX) &&
    token.length <= MAX_TOKEN_LENGTH &&
    /^[A-Za-z0-9._-]+$/.test(token)
  );
}

/** Whether a pathname is the invitation page. */
export function isInvitePage(pathname: string): boolean {
  return pathname.replace(/\/+$/, "") === INVITE_PAGE;
}

/**
 * Moves an invitation token out of the address bar.
 *
 * Called once, before the first render. On the invitation page any fragment
 * is removed from the current history entry — a token or not — and a
 * well-shaped token becomes this tab's pending invitation, replacing any
 * earlier one. Returns whether a token was captured.
 */
export function captureFromLocation(
  location: { pathname: string; search: string; hash: string },
  history: { replaceState(data: unknown, unused: string, url: string): void },
  store: InviteStore | null,
  now: number,
): boolean {
  if (!isInvitePage(location.pathname) || location.hash.length === 0) {
    return false;
  }
  const token = tokenFromFragment(location.hash);
  history.replaceState(null, "", `${INVITE_PAGE}${location.search}`);
  if (token === null) return false;
  writePending(store, { token, capturedAt: now, account: null });
  return true;
}

/** Records the pending invitation, in storage where it can be kept. */
export function writePending(store: InviteStore | null, pending: PendingInvite): void {
  memoryPending = pending;
  if (!store) return;
  try {
    store.setItem(PENDING_INVITE_KEY, JSON.stringify(pending));
  } catch {
    // The in-memory copy still serves a signed-in tab; see `memoryPending`.
  }
}

/** Forgets the pending invitation: accepted, refused, cancelled or expired. */
export function clearPending(store: InviteStore | null): void {
  memoryPending = null;
  if (!store) return;
  try {
    store.removeItem(PENDING_INVITE_KEY);
  } catch {
    /* Nothing was stored, or nothing can be removed; the TTL still applies. */
  }
}

/**
 * This tab's pending invitation, or `null` when there is none, it is
 * malformed, or it has lapsed — a lapsed one is removed on the way.
 */
export function readPending(store: InviteStore | null, now: number): PendingInvite | null {
  const pending = storedPending(store) ?? memoryPending;
  if (pending === null) return null;
  if (now < pending.capturedAt || now - pending.capturedAt > PENDING_INVITE_TTL_MS) {
    clearPending(store);
    return null;
  }
  return pending;
}

function storedPending(store: InviteStore | null): PendingInvite | null {
  if (!store) return null;
  try {
    const raw = store.getItem(PENDING_INVITE_KEY);
    if (!raw || raw.length > 1024) return null;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return null;
    const record = parsed as Record<string, unknown>;
    if (
      typeof record.token !== "string" ||
      !isTokenShaped(record.token) ||
      typeof record.capturedAt !== "number" ||
      !(record.account === null || typeof record.account === "string")
    ) {
      return null;
    }
    return { token: record.token, capturedAt: record.capturedAt, account: record.account };
  } catch {
    return null;
  }
}

/** Whether this tab can carry an invitation through a sign-in redirect. */
export function survivesSignIn(store: InviteStore | null): boolean {
  if (!store) return false;
  try {
    return store.getItem(PENDING_INVITE_KEY) !== null;
  } catch {
    return false;
  }
}

/**
 * What the signed-in account may do with the pending invitation.
 *
 * `bind` is the first signed-in view: the invitation becomes this account's
 * to decide. `mismatch` means another account opened it in this tab; it is
 * not carried over.
 */
export function accountCheck(
  pending: PendingInvite,
  account: string,
): "bind" | "match" | "mismatch" {
  if (pending.account === null) return "bind";
  return pending.account === account ? "match" : "mismatch";
}

/** Why an invitation can no longer be accepted, from the gateway's refusal. */
export type TerminalReason =
  | "expired"
  | "withdrawn"
  | "used"
  | "not_found"
  | "refused"
  | "unavailable";

/** What one press of Accept produced. */
export type AcceptResult =
  | { kind: "accepted"; scopeId: string }
  | { kind: "retry"; reason: "session" | "network"; message: string }
  | { kind: "terminal"; reason: TerminalReason; message: string };

/**
 * Reads the answer to `accept_invite`.
 *
 * A session or network failure is retryable and keeps the invitation; the
 * same recipient's retry of an acceptance that did land replays rather than
 * failing. Everything else is terminal for this link. The gateway's own
 * sentence is kept: its refusals are already worded not to disclose another
 * tenant's or person's details, and a rewording here would drift from the
 * CLI's.
 */
export function classifyAcceptance(
  answer: Outcome,
): AcceptResult {
  switch (answer.kind) {
    case "ok": {
      const body = answer.body as { scope_id?: unknown } | null;
      const scopeId = typeof body?.scope_id === "string" ? body.scope_id : null;
      if (scopeId === null) {
        return {
          kind: "retry",
          reason: "network",
          message: "the gateway's answer could not be read, so acceptance is not confirmed",
        };
      }
      return { kind: "accepted", scopeId };
    }
    case "unauthenticated":
      return { kind: "retry", reason: "session", message: "your session has ended" };
    case "unavailable":
      return { kind: "retry", reason: "network", message: answer.message };
    case "forbidden":
      return { kind: "terminal", reason: "refused", message: answer.message };
    case "invalid":
      return { kind: "terminal", reason: "not_found", message: answer.message };
    case "conflict":
      return { kind: "terminal", reason: conflictReason(answer.message), message: answer.message };
  }
}

/** The gateway's three conflict sentences for a redemption it refuses. */
function conflictReason(message: string): TerminalReason {
  if (/expired/i.test(message)) return "expired";
  if (/withdrawn/i.test(message)) return "withdrawn";
  if (/already been accepted/i.test(message)) return "used";
  return "unavailable";
}

/** The heading a terminal invitation state shows. */
export function terminalHeading(reason: TerminalReason): string {
  switch (reason) {
    case "expired":
      return "This invitation has expired";
    case "withdrawn":
      return "This invitation was withdrawn";
    case "used":
      return "This invitation has already been used";
    case "not_found":
      return "This invitation is not available here";
    case "refused":
      return "This account cannot accept invitations here";
    case "unavailable":
      return "This invitation can no longer be accepted";
  }
}
