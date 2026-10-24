/**
 * Settings: Signed-in sessions, the person's own console sessions.
 *
 * Personal, so it never reads the selected workspace. Rows show only the
 * issuer and three times; the session id is the revoke handle and is never
 * rendered.
 */

import { useCallback, useState } from "react";

import { SIGN_IN_URL, signOut } from "./api.mjs";
import type { ConsoleSessionPage, ConsoleSessionView } from "./generated/api.js";
import { whenOf } from "./people.mjs";
import { Loaded, invalidate, useQuery, useRefresh } from "./Query.js";
import {
  LEDGER_FULL,
  REVOKE_ALL_CALL_LIMIT,
  REVOKE_ALL_LEDGER_FULL,
  listPage,
  revokeAll,
  revokeOne,
  type Failed,
} from "./signins.mjs";

const CACHE_KEY = "me/console-sessions";

function describeFailure(outcome: Failed): string {
  return outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message;
}

/** The sign-in page is where a revoked current session lands. */
async function leave(): Promise<void> {
  await signOut();
  window.location.assign(SIGN_IN_URL);
}

export function SignIns() {
  const entry = useQuery(CACHE_KEY, async () => {
    const result = await listPage();
    return result.kind === "ok" ? { kind: "ok" as const, body: result.page } : result.outcome;
  });
  const retry = useRefresh(CACHE_KEY);
  const [more, setMore] = useState<ConsoleSessionView[]>([]);
  const [after, setAfter] = useState<string | null | undefined>(undefined);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ error: boolean; text: string } | null>(null);

  const showMore = useCallback(async (cursor: string) => {
    setBusy(true);
    const result = await listPage(cursor);
    setBusy(false);
    if (result.kind !== "ok") {
      setMessage({ error: true, text: describeFailure(result.outcome) });
      return;
    }
    setMore((rows) => [...rows, ...result.page.sessions]);
    setAfter(result.page.next_after ?? null);
  }, []);

  const reset = () => {
    setMore([]);
    setAfter(undefined);
    invalidate(CACHE_KEY);
  };

  const revoke = useCallback(async (id: string) => {
    setBusy(true);
    setMessage(null);
    const result = await revokeOne(id);
    if (result.kind !== "ok") {
      setBusy(false);
      setConfirming(null);
      setMessage({
        error: true,
        text: result.kind === "ledger-full" ? LEDGER_FULL : describeFailure(result.outcome),
      });
      return;
    }
    // The list is the only way to learn whether that was this console's own
    // session: if it now answers 401, it was.
    const relist = await listPage();
    setBusy(false);
    setConfirming(null);
    if (relist.kind === "failed" && relist.outcome.kind === "unauthenticated") {
      await leave();
      return;
    }
    reset();
    setMessage({ error: false, text: "Session revoked." });
  }, []);

  const revokeEvery = useCallback(async () => {
    setBusy(true);
    setMessage(null);
    const result = await revokeAll();
    setBusy(false);
    setConfirming(null);
    if (result.kind === "done") {
      await leave();
      return;
    }
    reset();
    const revoked = `${result.revoked} session${result.revoked === 1 ? "" : "s"} revoked.`;
    setMessage({
      error: true,
      text:
        result.kind === "ledger-full"
          ? REVOKE_ALL_LEDGER_FULL
          : result.kind === "incomplete"
            ? `Revoking is still not complete after ${REVOKE_ALL_CALL_LIMIT} requests. ${revoked} Run it again.`
            : `${describeFailure(result.outcome)} ${revoked}`,
    });
  }, []);

  return (
    <section>
      <h2>Signed-in sessions</h2>
      <p className="muted">
        Revoking a session deletes the browser session Synveda stores and, where your identity
        provider has a revocation profile, also refuses other credentials from the same sign-in
        (for example a CLI login done in the same browser). Revoking your current session signs you
        out of this console. Sessions created before this list existed are not shown and end at
        their 12-hour limit. This does not sign you out of your identity provider.
      </p>
      {message ? (
        <div
          className={message.error ? "banner error" : "banner"}
          role={message.error ? "alert" : "status"}
        >
          {message.text}
        </div>
      ) : null}
      <Loaded<ConsoleSessionPage> entry={entry} what="your signed-in sessions" onRetry={retry}>
        {(page) => {
          const rows = [...page.sessions, ...more];
          const cursor = after === undefined ? page.next_after : after;
          return rows.length === 0 ? (
            <p className="muted">No signed-in sessions are listed.</p>
          ) : (
            <>
              <table className="members">
                <thead>
                  <tr>
                    <th>Issuer</th>
                    <th>Signed in</th>
                    <th>Last seen</th>
                    <th>Expires</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={row.id}>
                      <td className="mono breakable">{row.issuer}</td>
                      <td>{whenOf(row.created_at)}</td>
                      <td>{whenOf(row.last_seen_at)}</td>
                      <td>{whenOf(row.expires_at)}</td>
                      <td>
                        {confirming === row.id ? (
                          <span
                            className="inline-confirm"
                            role="group"
                            aria-label="Confirm revocation"
                          >
                            <button
                              type="button"
                              className="danger"
                              disabled={busy}
                              onClick={() => void revoke(row.id)}
                            >
                              Confirm revoke
                            </button>
                            <button
                              type="button"
                              disabled={busy}
                              onClick={() => setConfirming(null)}
                            >
                              Keep session
                            </button>
                          </span>
                        ) : (
                          <button
                            type="button"
                            className="danger"
                            disabled={busy}
                            onClick={() => setConfirming(row.id)}
                          >
                            Revoke
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {cursor ? (
                <p>
                  <button type="button" disabled={busy} onClick={() => void showMore(cursor)}>
                    Show more
                  </button>
                </p>
              ) : null}
              {confirming === "all" ? (
                <div className="inline-confirm" role="group" aria-label="Confirm revoking all">
                  <span>Revoke every listed session, including this one, and sign out?</span>
                  <button
                    type="button"
                    className="danger"
                    disabled={busy}
                    onClick={() => void revokeEvery()}
                  >
                    Confirm revoke all
                  </button>
                  <button type="button" disabled={busy} onClick={() => setConfirming(null)}>
                    Keep sessions
                  </button>
                </div>
              ) : (
                <button
                  type="button"
                  className="danger"
                  disabled={busy}
                  onClick={() => setConfirming("all")}
                >
                  Revoke all
                </button>
              )}
            </>
          );
        }}
      </Loaded>
    </section>
  );
}
