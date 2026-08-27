/**
 * Adapter configuration (ADR-0027 decision 13). Precedence: the
 * environment, then the project's optional `.synveda/config.json`, then
 * defaults. Every field is optional and a malformed file is ignored —
 * a configuration mistake must not cost the user their session.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import type { Bearer } from "./credentials.mjs";
import { diagnostic, log } from "./log.mjs";

/** The gateway's own default listen address (`SYNVEDA_LISTEN_ADDR`). */
const DEFAULT_GATEWAY = "http://127.0.0.1:8120";

/**
 * The per-call deadline (ADR-0027 decision 3): two decimal orders above
 * the 150ms context engineering budget, sized to absorb a cold cache—not to wait out a
 * broken dependency.
 */
const DEFAULT_TIMEOUT_MS = 3000;

export interface AdapterConfig {
  /** `SYNVEDA_DISABLED=1`, or `disabled` in the project config. */
  disabled: boolean;
  inject: boolean;
  observe: boolean;
  /**
   * Whether this plugin's own `skills/` directory is reconciled with what
   * the registry serves this identity (SKIL-4, ADR-0054). On by default,
   * like the other two — a project that wants governed context but not
   * governed skills sets `"skills": false`.
   */
  skills: boolean;
  gatewayUrl: string;
  timeoutMs: number;
  budgetTokens?: number;
  compactBudgetTokens?: number;
  /**
   * The workspace runs in this project belong to (CPR-12, ADR-0078).
   *
   * Optional: with one workspace the adapter asks `/v1/me` and takes the
   * answer. It is worth setting in a checked-out repository that belongs to a
   * particular workspace — and it is safe to, unlike `gateway_url`, because
   * naming a workspace inside a tenant the caller is already authenticated to
   * cannot redirect a credential anywhere.
   */
  workspaceId?: string;
  /**
   * The project inside `workspaceId` that owns this checkout's runs.
   *
   * A project is optional in the session API, but it must not be guessed: a
   * workspace can contain many projects and choosing one by order would write
   * the transcript into the wrong governed subtree. CPR-14's real-client gate
   * found that the spool and API already carried `project_id`, while no
   * supported adapter setting could put it there.
   */
  projectId?: string;
}

/** The project file's shape — every field unknown until proven. */
interface ProjectConfig {
  disabled?: unknown;
  inject?: unknown;
  observe?: unknown;
  skills?: unknown;
  gateway_url?: unknown;
  workspace_id?: unknown;
  project_id?: unknown;
  timeout_ms?: unknown;
  budget_tokens?: unknown;
  compact_budget_tokens?: unknown;
}

export function loadConfig(cwd: string | undefined): AdapterConfig {
  const project = readProjectConfig(cwd);
  return {
    disabled: truthy(process.env.SYNVEDA_DISABLED) || bool(project.disabled) === true,
    inject: bool(project.inject) !== false,
    observe: bool(project.observe) !== false,
    skills: bool(project.skills) !== false,
    gatewayUrl: trimSlash(
      str(process.env.SYNVEDA_GATEWAY) ?? str(project.gateway_url) ?? DEFAULT_GATEWAY,
    ),
    timeoutMs:
      positive(process.env.SYNVEDA_TIMEOUT_MS) ??
      positive(project.timeout_ms) ??
      DEFAULT_TIMEOUT_MS,
    budgetTokens: positive(project.budget_tokens),
    compactBudgetTokens: positive(project.compact_budget_tokens),
    workspaceId: str(process.env.SYNVEDA_WORKSPACE) ?? str(project.workspace_id),
    projectId: str(process.env.SYNVEDA_PROJECT) ?? str(project.project_id),
  };
}

/**
 * The gateway a call actually goes to, once a credential is in hand.
 *
 * A bearer the CLI resolved names the gateway it was issued for, and that
 * one wins: `synveda login` is what binds a machine to a gateway, and
 * `.synveda/config.json` lives inside a checked-out repository — a
 * `gateway_url` there must not be able to send someone's bearer to a host
 * of the repository's choosing. An explicit `SYNVEDA_TOKEN` keeps the
 * configured gateway: an operator who set both meant both.
 */
export function resolveGateway(config: AdapterConfig, bearer: Bearer): AdapterConfig {
  const credentialed = bearer.gatewayUrl;
  if (bearer.source !== "cli" || credentialed === undefined) return config;
  const gatewayUrl = trimSlash(credentialed);
  if (gatewayUrl === config.gatewayUrl) return config;
  log("gateway.from_credential", { source: "stored_profile" });
  return { ...config, gatewayUrl };
}

function readProjectConfig(cwd: string | undefined): ProjectConfig {
  if (cwd === undefined || cwd.length === 0) return {};
  let raw: string;
  try {
    raw = readFileSync(join(cwd, ".synveda", "config.json"), "utf8");
  } catch (error) {
    if (!missing(error)) log("config.unreadable", { error: diagnostic(error) });
    return {};
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as ProjectConfig;
    }
    log("config.invalid", { reason: "not a JSON object" });
  } catch {
    log("config.invalid", { reason: "invalid_json" });
  }
  return {};
}

function missing(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    (error as { code?: unknown }).code === "ENOENT"
  );
}

function truthy(value: string | undefined): boolean {
  if (value === undefined) return false;
  const normalised = value.toLowerCase();
  return normalised === "1" || normalised === "true" || normalised === "yes";
}

function bool(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function str(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function positive(value: unknown): number | undefined {
  const parsed = typeof value === "string" ? Number(value) : value;
  if (typeof parsed !== "number" || !Number.isFinite(parsed) || parsed <= 0) {
    return undefined;
  }
  return Math.floor(parsed);
}

function trimSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}
