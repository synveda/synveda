/**
 * Where the adapter keeps its files (ADR-0027 decisions 6, 7 and 13):
 * credentials under the XDG config directory, the session spool and the
 * log under the XDG state directory. Nothing is ever written inside the
 * user's project.
 */

import { mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join, parse, resolve, sep } from "node:path";

function xdg(variable: string, fallback: string[]): string {
  const configured = process.env[variable];
  // A relative XDG path is undefined behaviour per the spec; ignore it
  // rather than scatter directories through the user's project.
  if (configured !== undefined && configured.startsWith("/")) {
    return join(configured, "synveda");
  }
  return join(homedir(), ...fallback, "synveda");
}

/** `$XDG_CONFIG_HOME/synveda`, else `~/.config/synveda`. */
export function configDir(): string {
  return xdg("XDG_CONFIG_HOME", [".config"]);
}

/** `$XDG_STATE_HOME/synveda`, else `~/.local/state/synveda`. */
export function stateDir(): string {
  return xdg("XDG_STATE_HOME", [".local", "state"]);
}

/**
 * The durable spool (CPR-12, ADR-0078 decision 6): one file per harness
 * session, holding recorded events and their delivery state.
 *
 * `synveda session flush` resolves the same directory the same way. Two
 * programs that disagree about where the spool lives is a spool that silently
 * never drains, so this resolution and `crates/synveda-cli/src/spool.rs`'s
 * must stay identical.
 */
export function spoolDir(): string {
  return join(stateDir(), "spool");
}

/**
 * Where the pre-cut per-session cursors lived (ADR-0027 decision 7).
 *
 * Named only so it can be **removed**. Nothing reads it: the old format held a
 * cursor into somebody else's transcript file and no events, so there is
 * nothing in one to migrate.
 */
export function legacySessionDir(): string {
  return join(stateDir(), "sessions");
}

/**
 * Where `synveda login` will write the credentials file (ADR-0027
 * decision 6). ADPT-1 step 2 fills this in; step 1 resolves its bearer
 * through the seam in `credentials.mts`.
 */
export function credentialsFile(): string {
  return join(configDir(), "credentials.json");
}

/**
 * Create `dir` and any missing parent, and RETURN — whatever the
 * filesystem answers.
 *
 * `mkdirSync(dir, { recursive: true })` does not. It reads ENOENT as "the
 * parent is missing, create it and try the child again", and procfs
 * answers ENOENT for a name it will never let anybody create. So
 * `mkdir("/proc/x")` is ENOENT, `mkdir("/proc")` is EEXIST, and Node
 * alternates between those two for the life of the process — measured at
 * ~500,000 syscalls a second, on Node 20, 22 and 24 alike. It is a spin
 * and not a block: no timeout ends it, and there is nothing to catch.
 *
 * Every directory this module names is rooted in `$XDG_STATE_HOME` or
 * `$XDG_CONFIG_HOME` — environment variables, so user input — and every
 * caller is a hook that swallows its errors, because a diagnostic that
 * cannot be written must never fail a session. A swallowing `catch` is
 * only worth having if the call inside it comes back.
 *
 * So the walk here goes downwards: one mkdir per component, no retry of
 * anything. It terminates by construction, and the first component that
 * refuses is thrown to the caller who was always ready to catch it.
 */
export function ensureDir(dir: string): void {
  try {
    mkdirSync(dir);
    return;
  } catch (error) {
    const { code } = error as NodeJS.ErrnoException;
    // Already a directory is the common case by far, and a missing parent
    // is the only answer worth walking for.
    if (code === "EEXIST") return;
    if (code !== "ENOENT") throw error;
  }

  const absolute = resolve(dir);
  const { root } = parse(absolute);
  let built = root;
  for (const component of absolute.slice(root.length).split(sep)) {
    if (component.length === 0) continue;
    built = join(built, component);
    try {
      mkdirSync(built);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
  }
}
