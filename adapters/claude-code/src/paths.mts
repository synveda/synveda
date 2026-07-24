/**
 * Where the adapter keeps its files (ADR-0027 decisions 6, 7 and 13):
 * credentials under the XDG config directory, the session spool and the
 * log under the XDG state directory. Nothing is ever written inside the
 * user's project.
 */

import { homedir } from "node:os";
import { join } from "node:path";

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

/** Per-session cursors and transcript paths (ADR-0027 decision 7). */
export function sessionDir(): string {
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
