import { useCallback, useEffect, useState } from "react";

import { SIGN_IN_URL, signOut, whoami, type Outcome, type WhoAmI } from "./api.mjs";
import { Inbox } from "./Inbox.js";

/**
 * The console shell (CNSL-1, ADR-0056).
 *
 * The session underneath it is the part worth naming: a login that leaves
 * no credential in the browser, `/v1` calls authenticated by a cookie the
 * bundle cannot read, and a sign-out that ends the session server-side.
 * The inbox sits on top of that and was written against the parity corpus
 * (decision 7) rather than the other way round — a renderer built first is
 * a renderer the corpus then has to be written around.
 */
export function App() {
  const [state, setState] = useState<Outcome | { kind: "loading" }>({ kind: "loading" });

  const load = useCallback(async () => {
    setState(await whoami());
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

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

  const onSignOut = useCallback(async () => {
    await signOut();
    // Reload rather than clear local state: the cookie is gone, so every
    // subsequent call is a 401 anyway, and a fresh load is the one path
    // that cannot leave a stale view of a session that no longer exists.
    window.location.assign("/console/");
  }, []);

  return (
    <main>
      <header>
        <h1>Synveda</h1>
        {state.kind === "ok" ? (
          <button type="button" onClick={() => void onSignOut()}>
            Sign out
          </button>
        ) : null}
      </header>

      {loginError ? <Banner kind="error">Sign-in failed: {loginError}</Banner> : null}

      <Body state={state} onRetry={() => void load()} />
    </main>
  );
}

function Body({
  state,
  onRetry,
}: {
  state: Outcome | { kind: "loading" };
  onRetry: () => void;
}) {
  switch (state.kind) {
    case "loading":
      return <p className="muted">Checking your session…</p>;

    case "ok": {
      const me = state.body as WhoAmI;
      return (
        <>
          <section>
            <h2>Signed in</h2>
            <dl>
              <dt>Subject</dt>
              <dd>{me.subject}</dd>
              <dt>Organisation</dt>
              <dd>
                {me.tenant.name} <span className="muted">({me.tenant.slug})</span>
              </dd>
            </dl>
          </section>
          <Inbox />
        </>
      );
    }

    case "unauthenticated":
      return (
        <section>
          <h2>Sign in</h2>
          <p className="muted">
            You will be sent to your identity provider and back. The console keeps no token in
            your browser.
          </p>
          <a className="button" href={SIGN_IN_URL}>
            Sign in
          </a>
        </section>
      );

    // Signed in, and told no. Offering a login here would send somebody
    // who is already signed in round a loop that cannot end (see api.mts).
    case "forbidden":
      return (
        <Banner kind="error">
          Your roles do not allow this: {state.message}
          <p className="muted">
            Ask an administrator for the role this scope requires — signing in again will not
            change the answer.
          </p>
        </Banner>
      );

    case "invalid":
    case "conflict":
      return <Banner kind="error">{state.message}</Banner>;

    case "unavailable":
      return (
        <Banner kind="error">
          The gateway is not answering: {state.message}
          <p>
            <button type="button" onClick={onRetry}>
              Try again
            </button>
          </p>
        </Banner>
      );
  }
}

function Banner({ kind, children }: { kind: "error"; children: React.ReactNode }) {
  return (
    <div className={`banner ${kind}`} role="alert">
      {children}
    </div>
  );
}
