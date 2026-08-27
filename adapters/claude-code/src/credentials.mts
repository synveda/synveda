/**
 * The bearer seam (ADR-0027 decision 4).
 *
 * The adapter holds no OAuth: no PKCE, no client configuration, no
 * refresh logic, no credentials file of its own. It shells out to
 * `synveda auth token --json`, which reads the credentials file and
 * refreshes through the gateway when the access token has expired. One
 * implementation of expiry and refresh — in Rust, next to the
 * `synveda-identity` code that already does this — instead of a second,
 * drifting one in TypeScript. The cost is one process spawn per hook,
 * against a network call two decimal orders larger.
 *
 * `SYNVEDA_TOKEN` stays as an explicit override for CI, for demos, and
 * for the dev bearer `synveda token issue` prints (ADR-0008). It is
 * checked first precisely because it is explicit: an operator who set it
 * meant it.
 */

import { execFile } from "node:child_process";

import { diagnostic, log } from "./log.mjs";

/** How long the CLI gets to answer before the hook gives up on memory. */
const CLI_TIMEOUT_MS = 3000;

/** The resolved credential and where it came from. */
export interface Bearer {
  /** The `/v1` bearer token. */
  token: string;
  /**
   * The gateway the credential names, when the CLI resolved it. This is
   * the gateway the adapter posts to: `synveda login` is what binds a
   * machine to a gateway, and a file inside a checked-out repository must
   * not be able to redirect someone's bearer somewhere else.
   */
  gatewayUrl?: string;
  source: "env" | "cli";
}

/** The `--json` contract of `synveda auth token`. */
interface CliToken {
  access_token?: unknown;
  gateway_url?: unknown;
  subject?: unknown;
  tenant_id?: unknown;
  expires_at?: unknown;
}

/**
 * A currently-valid bearer, or `undefined` when the user must log in.
 * Never throws: every failure here is "no memory this time" (decision 3).
 */
export async function resolveBearer(): Promise<Bearer | undefined> {
  const override = process.env.SYNVEDA_TOKEN;
  if (override !== undefined && override.length > 0) {
    return { token: override, source: "env" };
  }
  return resolveFromCli();
}

async function resolveFromCli(): Promise<Bearer | undefined> {
  const binary = process.env.SYNVEDA_CLI ?? "synveda";
  const args = ["auth", "token", "--json"];
  const profile = process.env.SYNVEDA_PROFILE;
  if (profile !== undefined && profile.length > 0) args.push("--profile", profile);

  let stdout: string;
  try {
    stdout = await run(binary, args);
  } catch (error) {
    // Not installed, not logged in, expired past refresh, gateway down
    // mid-refresh: the same outcome either way, and the reason belongs in
    // the log rather than in the user's session.
    log("credentials.unavailable", { reason: diagnostic(error) });
    return undefined;
  }

  let parsed: CliToken;
  try {
    parsed = JSON.parse(stdout) as CliToken;
  } catch {
    log("credentials.unparsed", { reason: "invalid_json" });
    return undefined;
  }
  const token = typeof parsed.access_token === "string" ? parsed.access_token : "";
  if (token.length === 0) {
    log("credentials.empty", {});
    return undefined;
  }
  return {
    token,
    gatewayUrl: typeof parsed.gateway_url === "string" ? parsed.gateway_url : undefined,
    source: "cli",
  };
}

function run(binary: string, args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      binary,
      args,
      { timeout: CLI_TIMEOUT_MS, encoding: "utf8", windowsHide: true },
      (error, stdout, stderr) => {
        if (error !== null) {
          // The CLI's stderr says what to do ("run `synveda login`"); keep
          // it for the log and never for stdout, which is model-visible
          // context on SessionStart.
          reject(new CliFailure(error.message, stderr.trim()));
          return;
        }
        resolve(stdout);
      },
    );
  });
}

class CliFailure extends Error {
  readonly detail: string;
  constructor(message: string, detail: string) {
    super(message);
    this.name = "CliFailure";
    this.detail = detail;
  }
}

/** What the user is told when no credential resolves (ADR-0027 decision 3). */
export const SIGN_IN_MESSAGE =
  "Synveda: no credentials found — run `synveda login` to receive governed context in this session.";
