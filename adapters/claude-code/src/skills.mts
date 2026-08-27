/**
 * Skills materialisation (SKIL-4, ADR-0054 decisions 16, 17 and 18) — the
 * `skills/` half of the manifest ADR-0027 decision 1 reserved for this
 * feature.
 *
 * It runs `synveda skill sync` into the **plugin's own** `skills/`
 * directory and does nothing else. Three properties, and each is a
 * decision rather than convenience:
 *
 * 1. **The adapter reimplements nothing.** ADR-0027 decision 4 kept OAuth
 *    in the CLI so there would be one implementation of refresh; path
 *    safety is the stronger case of the same rule. ADR-0051 decision 7's
 *    grammar — no `..`, no absolute form, no reserved device name, no
 *    trailing dot or space, no case-fold collision — is what stops a
 *    governed bundle writing outside its directory, and a second copy of it
 *    in TypeScript is exactly the two-parsers-disagreeing failure ADR-0051
 *    decision 4 refused for YAML, with a filesystem underneath instead of a
 *    frontmatter.
 * 2. **The root is this plugin's own directory, never `~/.claude/skills`.**
 *    A sync removes as well as writes (that is what makes a rollback reach
 *    a laptop), and the only directory this product may prune is one it
 *    created.
 * 3. **It is off the inject hook's critical path.** A second `SessionStart`
 *    entry, `async: true`, so N bundle writes never sit inside a call whose
 *    design budget is 150ms. The consequence is stated rather than hidden: a client
 *    reads its skills folder when it starts, so what this writes is loaded
 *    by the *next* session — which is the gap the block's own skills
 *    section exists to cover (ADR-0054 force 2).
 */

import { execFile } from "node:child_process";
import { join } from "node:path";

import { diagnostic, log } from "./log.mjs";

/**
 * How long the sync gets. Above the 3s per-call deadline the inject path
 * uses, because this is N resolves rather than one — and it is async, so
 * nobody is waiting on it — but far below the hook's own watchdog.
 */
const SYNC_TIMEOUT_MS = 8000;

/** What `synveda skill sync --json` answers. */
interface SyncResult {
  root?: unknown;
  available?: unknown;
  written?: unknown;
  unchanged?: unknown;
  removed?: unknown;
}

/**
 * The governed skills root: this plugin's own `skills/` directory.
 *
 * `CLAUDE_PLUGIN_ROOT` is set by the harness for every hook it runs from a
 * plugin. Without it there is no directory this adapter owns, and the
 * honest thing is to do nothing rather than to guess at one — a wrong guess
 * here is a `remove_dir_all` somewhere a person keeps their own work.
 */
export function governedRoot(): string | undefined {
  const pluginRoot = process.env.CLAUDE_PLUGIN_ROOT;
  if (pluginRoot === undefined || pluginRoot.length === 0) return undefined;
  return join(pluginRoot, "skills");
}

/**
 * Reconciles the governed skills root with what this identity may install.
 *
 * Never throws and never writes to stdout: this runs as a `SessionStart`
 * hook, where stdout is context the model reads (ADR-0027 decision 3, and
 * its secrets note).
 */
export async function syncSkills(): Promise<void> {
  const root = governedRoot();
  if (root === undefined) {
    log("skills.no_plugin_root", {});
    return;
  }
  const binary = process.env.SYNVEDA_CLI ?? "synveda";
  const args = ["skill", "sync", "--client", "claude-code", "--root", root, "--json"];
  const profile = process.env.SYNVEDA_PROFILE;
  if (profile !== undefined && profile.length > 0) args.push("--profile", profile);

  let stdout: string;
  try {
    stdout = await run(binary, args);
  } catch (error) {
    // Not installed, not logged in, gateway down: the same outcome either
    // way — this session's skills are whatever the last successful sync
    // left on the disk, which is the degrade-never-fail posture applied to
    // a directory instead of a block.
    log("skills.unavailable", { reason: diagnostic(error) });
    return;
  }

  let parsed: SyncResult;
  try {
    parsed = JSON.parse(stdout) as SyncResult;
  } catch {
    log("skills.unparsed", { reason: "invalid_json" });
    return;
  }
  log("skills.synced", {
    root: typeof parsed.root === "string" ? parsed.root : root,
    available: count(parsed.available),
    written: count(parsed.written),
    unchanged: count(parsed.unchanged),
    removed: count(parsed.removed),
  });
}

/** A count from either a number or the array the CLI actually sends. */
function count(value: unknown): number {
  if (typeof value === "number") return value;
  return Array.isArray(value) ? value.length : 0;
}

function run(binary: string, args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      binary,
      args,
      { timeout: SYNC_TIMEOUT_MS, encoding: "utf8", windowsHide: true },
      (error, stdout, stderr) => {
        if (error !== null) {
          reject(new Error(stderr.trim().length > 0 ? stderr.trim() : error.message));
          return;
        }
        resolve(stdout);
      },
    );
  });
}
