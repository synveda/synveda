/**
 * The bearer seam (ADR-0027 decision 4).
 *
 * ADPT-1 lands in two steps. Step 1 — this one — reads `SYNVEDA_TOKEN`,
 * the dev bearer that `synveda token issue --tenant <id> --subject <sub>`
 * prints (ADR-0008), so the hook contract can be exercised against a
 * live gateway before the login flow exists. Step 2 replaces the body of
 * `resolveBearer` with an invocation of `synveda auth token --json`,
 * which reads the credentials file and refreshes through the gateway
 * when the access token has expired.
 *
 * The adapter holds no OAuth code in either case: one implementation of
 * PKCE, expiry, and refresh, in Rust, next to the `synveda-identity`
 * code that already does this.
 */

import { log } from "./log.mjs";

/**
 * A currently-valid bearer, or `undefined` when the user must log in.
 * Asynchronous because step 2 resolves it by spawning the CLI.
 */
export async function resolveBearer(): Promise<string | undefined> {
  const token = process.env.SYNVEDA_TOKEN;
  if (token !== undefined && token.length > 0) return token;
  log("credentials.missing", { seam: "SYNVEDA_TOKEN" });
  return undefined;
}

/** What the user is told when no credential resolves (ADR-0027 decision 3). */
export const SIGN_IN_MESSAGE =
  "Synveda: no credentials found — run `synveda login` to receive governed context in this session.";
