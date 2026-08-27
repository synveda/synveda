/**
 * This installation's stable id (CPR-12, ADR-0078 decision 6).
 *
 * A run carries `client_installation_id` — what tells two machines running the
 * same client apart, so "which laptop was that agent on" is answerable without
 * anything identifying the laptop. It is a random value minted once and kept;
 * it is deliberately not a hostname, a MAC address or a username, because a
 * value that identifies a *machine* would be user data riding into every run.
 *
 * It lives beside the credentials rather than in the spool: a spool is
 * per-conversation and this is per-installation, and a value re-minted every
 * conversation would answer the question it exists for with a different answer
 * every time.
 */

import { randomUUID } from "node:crypto";
import { closeSync, openSync, readFileSync, writeSync } from "node:fs";
import { join } from "node:path";

import { configDir, ensureDir } from "./paths.mjs";
import { diagnostic, log } from "./log.mjs";

/** The file holding it. */
function installationFile(): string {
  return join(configDir(), "installation-id");
}

/**
 * This installation's id, minted on first use.
 *
 * Never throws and always answers: a read-only home directory yields an
 * ephemeral id rather than a failed hook. That degrades the question "which
 * machine" to "some machine", which is the right trade against costing
 * somebody their session.
 */
export function installationId(): string {
  const path = installationFile();
  try {
    const existing = readFileSync(path, "utf8").trim();
    if (existing.length > 0) return existing.slice(0, 200);
  } catch {
    // Not minted yet, or unreadable. Fall through and mint one.
  }
  const minted = randomUUID();
  try {
    ensureDir(configDir());
    // `wx` so two hooks starting at once cannot each mint one and have the
    // loser's id silently win for its own session.
    const handle = openSync(path, "wx");
    try {
      writeSync(handle, minted);
    } finally {
      closeSync(handle);
    }
    return minted;
  } catch {
    // Lost the race, or cannot write. Re-read before giving up: the common
    // case here is the other hook having just created it.
    try {
      const existing = readFileSync(path, "utf8").trim();
      if (existing.length > 0) return existing.slice(0, 200);
    } catch (error) {
      log("installation.unwritable", { error: diagnostic(error) });
    }
    return minted;
  }
}
