/** Bounded, local-only checkout observation (CPR-12, ADR-0078). */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";

export interface CheckoutObservation {
  /** Opaque within this client installation; never a cross-machine repo id. */
  ref: string;
  observed_at: string;
  branch?: string;
  commit?: string;
  dirty?: boolean;
}

const MAX_OUTPUT = 64 * 1024;
const DEADLINE_MS = 250;

function git(cwd: string, args: string[]): string | undefined {
  // The harness's cwd is the only discovery origin. Remove inherited Git
  // redirects so a parent process cannot make a different checkout appear.
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")),
  );
  try {
    return execFileSync("git", ["-c", "core.fsmonitor=false", "-C", cwd, ...args], {
      encoding: "utf8",
      env: { ...env, GIT_OPTIONAL_LOCKS: "0" },
      maxBuffer: MAX_OUTPUT,
      stdio: ["ignore", "pipe", "ignore"],
      timeout: DEADLINE_MS,
    }).trimEnd();
  } catch {
    return undefined;
  }
}

/** Returns only safe labels and an opaque checkout ref; remotes are never read. */
export function observeGit(cwd: string | undefined, installationId: string): CheckoutObservation | undefined {
  if (cwd === undefined || cwd.length === 0) return undefined;
  const root = git(cwd, ["rev-parse", "--show-toplevel"]);
  if (root === undefined || root.length === 0) return undefined;
  const ref = createHash("sha256").update(installationId).update("\0").update(root).digest("hex").slice(0, 24);
  const branch = git(cwd, ["symbolic-ref", "-q", "--short", "HEAD"]);
  const commit = git(cwd, ["rev-parse", "--verify", "HEAD"]);
  const status = git(cwd, ["status", "--porcelain=v1", "--untracked-files=normal"]);
  return {
    ref,
    observed_at: new Date().toISOString(),
    ...(branch !== undefined && branch.length <= 255 ? { branch } : {}),
    ...(commit !== undefined && /^[0-9a-f]{40,64}$/u.test(commit) ? { commit } : {}),
    ...(status !== undefined ? { dirty: status.length > 0 } : {}),
  };
}
