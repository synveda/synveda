/**
 * The console's `/v1` transport (CNSL-1, ADR-0056), and the calls that are
 * **not on the OpenAPI contract yet**.
 *
 * Since CPR-8 this file is two things and the split matters. The transport —
 * `call`, `classify`, the session — is what everything uses, generated or
 * not. The named calls below it are the routes the contract does not cover:
 * `docs/api/openapi.json` declares the foundation, session and Knowledge
 * planes (62 operations); the rest of `/v1` joins it in the public-contract
 * convergence package. Everything the document *does* declare is
 * reached through `client.mts`, whose types are generated from it; nothing
 * here duplicates one of those routes.
 *
 * A hand-written call is therefore a **marker**: it says "this route has no
 * contract yet". When Prompt 19 lands, these disappear rather than being
 * kept in step.
 *
 * Two things live here, and only the second one is interesting.
 *
 * The first is that **no credential is handled in this file**, or anywhere
 * else in the bundle. The session is an `HttpOnly` cookie the browser
 * attaches on its own (ADR-0056 decision 2); there is no token to read, no
 * header to set, and nothing for an XSS to steal out of JavaScript. A
 * `credentials: "same-origin"` is the whole of the authentication code.
 *
 * The second is the **classification**, which decides what a reviewer is
 * shown and is therefore the part worth testing. It is a pure function of
 * a status code precisely so that it can be.
 */

/** Where the gateway serves the console, and the prefix its API shares. */
export const API_BASE = "/v1";

/**
 * What a response means to the console, as opposed to what it says.
 *
 * The load-bearing distinction is **`unauthenticated` versus `forbidden`**.
 * A 401 means there is no usable session and the answer is to sign in; a
 * 403 means the session is fine and the PDP said no. Collapsing them — the
 * easy mistake, since both are "you cannot have this" — puts a Sign in
 * button in front of somebody who is already signed in, and clicking it
 * returns them to the same 403. That is an infinite loop rendered as a
 * helpful suggestion, and it is the kind of thing that survives a demo and
 * fails in front of a customer whose reviewer holds one role short.
 */
export type Outcome =
  | { kind: "ok"; body: unknown }
  | { kind: "unauthenticated" }
  | { kind: "forbidden"; message: string }
  | { kind: "invalid"; message: string }
  | { kind: "conflict"; message: string }
  | { kind: "unavailable"; message: string };

/**
 * Maps a status and a parsed body onto the console's vocabulary.
 *
 * Unknown statuses land on `unavailable` rather than on `ok`: a surface
 * that treats a status it has never seen as success will render an error
 * body as if it were data, which is worse than saying nothing.
 */
export function classify(status: number, body: unknown): Outcome {
  if (status >= 200 && status < 300) {
    return { kind: "ok", body };
  }
  const message = messageOf(body);
  switch (status) {
    case 401:
      return { kind: "unauthenticated" };
    case 403:
      return { kind: "forbidden", message };
    case 400:
    case 404:
    case 422:
      // A 404 is deliberately `invalid` rather than a kind of its own. The
      // gateway returns a uniform 404 for a resource the caller may not
      // see (AUTHZ-3), so "not found" and "not yours" are the same answer
      // by design, and a console that rendered them differently would be
      // inventing a distinction the product refuses to make.
      return { kind: "invalid", message };
    case 409:
      return { kind: "conflict", message };
    default:
      return { kind: "unavailable", message };
  }
}

/**
 * Pulls the human-facing sentence out of the gateway's error taxonomy,
 * falling back to something honest rather than to `[object Object]`.
 *
 * The gateway owns the wording (`crate::error::caller_facing`), and the
 * console displays it rather than composing its own — the same rule
 * ADR-0056 decision 6 applies to quality shortfalls, for the same reason:
 * two authors of one sentence is two sentences.
 */
function messageOf(body: unknown): string {
  if (typeof body === "object" && body !== null) {
    const message = (body as { message?: unknown }).message;
    if (typeof message === "string" && message.length > 0) {
      return message;
    }
  }
  return "the gateway did not say why";
}

/** A `fetch` that returns the console's vocabulary instead of a Response. */
export async function call(
  path: string,
  init: RequestInit = {},
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  let response: Response;
  try {
    response = await fetchImpl(`${API_BASE}${path}`, {
      ...init,
      // Same-origin by construction (ADR-0056 decision 1). Stated rather
      // than left to the default so that moving the console off this
      // origin fails here, visibly, instead of silently dropping the
      // cookie and looking like a session that expired.
      credentials: "same-origin",
      headers: {
        accept: "application/json",
        ...(init.headers ?? {}),
      },
    });
  } catch (cause) {
    // A dead gateway is `unavailable`, not `unauthenticated`. Told apart
    // because the answers differ: wait and retry, versus sign in again.
    return {
      kind: "unavailable",
      message: cause instanceof Error ? cause.message : "the gateway is unreachable",
    };
  }
  let body: unknown = null;
  const text = await response.text();
  if (text.length > 0) {
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
  }
  return classify(response.status, body);
}

// `whoami` and its `WhoAmI` type were deleted by CPR-8. The console resolves
// its session with `GET /v1/me`, through the generated client — one call that
// answers who, which tenant, what exists, what is missing and what this caller
// may do (ADR-0071 decision 2), where `whoami` answered the first two. Keeping
// a second, weaker session call would be keeping a second answer to the
// question every page starts with.

/**
 * Ends the session. Not under {@link API_BASE}: sign-out is part of the
 * unauthenticated auth plane, because a session too expired to authenticate
 * is exactly the one that most needs clearing.
 */
export async function signOut(fetchImpl: typeof fetch = fetch): Promise<void> {
  try {
    await fetchImpl("/auth/console/logout", {
      method: "POST",
      credentials: "same-origin",
    });
  } catch {
    // The gateway clears the cookie; if it could not be reached, reloading
    // onto a signed-out page is still the right next step.
  }
}

/** Where a sign-in starts. `console=true` is what asks for a cookie. */
export const SIGN_IN_URL = "/auth/login?console=true";

// ── Proposals ───────────────────────────────────────────────────────────
//
// Four calls, none of them new: FLOW-6 built a review flow and gave it a
// JSON API, and the CLI is a client of that API rather than the owner of
// it. CNSL-1 adds no governed route (ADR-0056 decision 9) — if this screen
// needed something the API could not answer, the API would gain it and the
// CLI would gain it too.
//
// Nothing here sets an `Origin` header, and nothing may: it is forbidden to
// scripts, which is exactly what makes it worth checking. The browser
// attaches it to these mutations on its own, and the gateway refuses a
// cookie-authenticated non-GET without one (decision 4).

/**
 * The queue, newest first.
 *
 * `state=open` is the **stored** state, and that is wider than it looks: a
 * proposal whose requirement is satisfied is stored `open` and rendered
 * `approved` (ADR-0032 decision 11), because "has enough approvals" is
 * computed live against a requirement a pack switch can move. Filtering on
 * the stored state is therefore what an inbox wants — everything still
 * actionable, including the ones waiting to be published — and filtering on
 * the rendered one would drop exactly the rows somebody is coming here to
 * finish.
 */
export async function listProposals(fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call("/proposals?state=open", { method: "GET" }, fetchImpl);
}

/** One proposal in full: members, approvals, scan and quality. */
export async function readProposal(id: string, fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call(`/proposals/${encodeURIComponent(id)}`, { method: "GET" }, fetchImpl);
}

/**
 * Approve. The comment is optional, so an empty box sends no comment at all
 * rather than an empty string somebody has to read as if it meant something.
 */
export async function approve(
  id: string,
  comment: string,
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  const trimmed = comment.trim();
  return call(
    `/proposals/${encodeURIComponent(id)}/approve`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(trimmed.length > 0 ? { comment: trimmed } : {}),
    },
    fetchImpl,
  );
}

/**
 * Reject. The reason is mandatory at the gateway, and the button that sends
 * this is disabled without one — two checks for one rule, because the
 * server's is the one that counts and the client's is the one that stops a
 * reviewer losing what they typed to a 400.
 */
export async function reject(
  id: string,
  reason: string,
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  return call(
    `/proposals/${encodeURIComponent(id)}/reject`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ reason: reason.trim() }),
    },
    fetchImpl,
  );
}

// ── The explorer (CNSL-2, ADR-0058; the scope re-cut CPR-7, ADR-0074) ──
//
// Four reads, and the same rule as above: every one of them is a route the
// CLI also has (`synveda scope list|show`, `synveda whoami
// --capabilities`, `synveda lapse list`). ADR-0056 decision 9 is a
// standing decision — no console-only route.
//
// The tree is **lazy**: children on expand, never `descendants` from the
// root (ADR-0058 decision 5). A screen that fetched a subtree to draw a
// sidebar would pull all of it and then probe every scope in it.
//
// The roles panel is gone with role bindings (CPR-7): "who holds what
// here" is the access plane's own surface, and grants are listed by
// `/v1/admin/grants` — a later console prompt's screen, not this one's.

/**
 * The tenant's root scope and its children — where the tree starts.
 *
 * The response carries the level's parent under `parent` and its children
 * under `scopes`, which is also the shape [`childrenOf`] returns, so the
 * tree renders one component for both.
 */
export async function scopeLevel(
  parentId?: string,
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  const query = parentId ? `?parent_id=${encodeURIComponent(parentId)}` : "";
  return call(`/admin/scopes${query}`, { method: "GET" }, fetchImpl);
}

/** One scope's direct children, slug order. The tree's only expansion call. */
export async function childrenOf(
  id: string,
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  return scopeLevel(id, fetchImpl);
}

/** The pack in force at a scope, and where it came from. */
export async function scopePolicy(id: string, fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call(`/admin/scopes/${encodeURIComponent(id)}/policy`, { method: "GET" }, fetchImpl);
}

/**
 * What *this reader* may do at one scope.
 *
 * A forecast, never a grant (ADR-0058 decision 2). Nothing in this bundle
 * may use the answer to decide whether an act is allowed — only whether to
 * offer it. The gateway decides again, at the act's own seam, under the
 * pack effective then; if the two disagree the act's answer is the one that
 * counts and the reader sees the refusal.
 */
export async function scopeCapabilities(
  ids: string[],
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  const query = ids.map((id) => encodeURIComponent(id)).join(",");
  return call(`/capabilities?scopes=${query}`, { method: "GET" }, fetchImpl);
}

/**
 * Every standing grant this reader may see, anywhere in the tenant.
 *
 * Scope-free on purpose: an explorer whose lapse view required you to
 * already know which scope to ask about is answering the question you would
 * have asked if you already had the answer.
 */
export async function standingLapses(fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call("/lapses", { method: "GET" }, fetchImpl);
}

// ── The planes the contract does not cover yet ──────────────────────────
//
// Policy packs, lapses, audit, service identities and skills. Every one of
// them is a route the CLI also has (ADR-0056 decision 9 is standing: no
// console-only route), and every one of them is hand-written here because
// the OpenAPI document does not declare it. That is the whole reason they
// are grouped and labelled rather than scattered — the group is a to-do
// list with a prompt number on it.

/** The packs this tenant can assign: the embedded ones and its stored ones. */
export async function policyPacks(fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call("/policy/packs", { method: "GET" }, fetchImpl);
}

/** The tenant-wide default pack, and where it came from. */
export async function defaultPolicy(fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call("/policy/default", { method: "GET" }, fetchImpl);
}

/**
 * Assigns a pack at one scope, governing its subtree.
 *
 * Used by first-run onboarding to seed the shape somebody chose, and by
 * Advanced ▸ Policies to change it afterwards. The same route both times:
 * onboarding is a caller of the product's own surface, never a second way
 * in (seed §2.2).
 */
export async function assignScopePolicy(
  scopeId: string,
  name: string,
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  return call(
    `/admin/scopes/${encodeURIComponent(scopeId)}/policy`,
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name }),
    },
    fetchImpl,
  );
}

/** The most recent audit events, newest first. */
export async function auditEvents(
  limit: number,
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  return call(`/audit/events?limit=${encodeURIComponent(String(limit))}`, { method: "GET" }, fetchImpl);
}

/**
 * Whether the hash chain still verifies.
 *
 * A broken chain is a 200 with `valid: false` rather than an error — the
 * verification succeeded; it is the chain that did not — so the page reads
 * the verdict rather than the status.
 */
export async function auditVerify(fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call("/audit/verify", { method: "GET" }, fetchImpl);
}

/** The agents registered to act in this tenant. */
export async function serviceIdentities(fetchImpl: typeof fetch = fetch): Promise<Outcome> {
  return call("/service-identities", { method: "GET" }, fetchImpl);
}

/**
 * The skills available at a scope.
 *
 * Skills the caller may not read at their tier are omitted by the gateway
 * rather than refused, so an empty list here means "nothing you may load",
 * which is what the page says.
 */
export async function skillsAt(
  scopeId: string | null,
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  const query = scopeId ? `?scope_id=${encodeURIComponent(scopeId)}` : "";
  return call(`/skills${query}`, { method: "GET" }, fetchImpl);
}
