/**
 * Same-origin transport for the generated public application client.
 *
 * CPR-29 removed the named route wrappers that used to live here: every
 * production `/v1` operation is now generated from OpenAPI and called through
 * `client.mts`. This module owns only HTTP/session mechanics and the console's
 * status taxonomy. It contains no application path strings.
 */

/** Where the gateway serves the application API. */
export const API_BASE = "/v1";

/** What an HTTP response means to the console. */
export type Outcome =
  | { kind: "ok"; body: unknown }
  | { kind: "unauthenticated" }
  | { kind: "forbidden"; message: string }
  | { kind: "invalid"; message: string }
  | { kind: "conflict"; message: string }
  | { kind: "unavailable"; message: string };

/** Maps a status and parsed body onto the console's stable vocabulary. */
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
      // Uniform 404 deliberately does not distinguish absent from denied.
      return { kind: "invalid", message };
    case 409:
      return { kind: "conflict", message };
    default:
      return { kind: "unavailable", message };
  }
}

function messageOf(body: unknown): string {
  if (typeof body === "object" && body !== null) {
    const message = (body as { message?: unknown }).message;
    if (typeof message === "string" && message.length > 0) {
      return message;
    }
  }
  return "the gateway did not say why";
}

/** A same-origin fetch that returns the console's status vocabulary. */
export async function call(
  path: string,
  init: RequestInit = {},
  fetchImpl: typeof fetch = fetch,
): Promise<Outcome> {
  let response: Response;
  try {
    response = await fetchImpl(`${API_BASE}${path}`, {
      ...init,
      credentials: "same-origin",
      headers: {
        accept: "application/json",
        ...(init.headers ?? {}),
      },
    });
  } catch (cause) {
    return {
      kind: "unavailable",
      message: cause instanceof Error ? cause.message : "the gateway is unreachable",
    };
  }

  let body: unknown = null;
  try {
    const text = await response.text();
    if (text.length > 0) {
      body = JSON.parse(text);
    }
  } catch {
    // Status is still authoritative when a proxy returns an empty/non-JSON
    // body or the body stream itself cannot be read.
    body = null;
  }
  return classify(response.status, body);
}

/** Ends the cookie session on the unauthenticated auth plane. */
export async function signOut(fetchImpl: typeof fetch = fetch): Promise<void> {
  try {
    await fetchImpl("/auth/console/logout", {
      method: "POST",
      credentials: "same-origin",
    });
  } catch {
    // Reloading onto the signed-out page remains the safe outcome.
  }
}

/** Where a browser sign-in starts. */
export const SIGN_IN_URL = "/auth/login?console=true";
