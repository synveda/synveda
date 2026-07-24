/**
 * Adapter configuration (ADR-0027 decision 13). Precedence: the
 * environment, then the project's optional `.synveda/config.json`, then
 * defaults. Every field is optional and a malformed file is ignored —
 * a configuration mistake must not cost the user their session.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { log } from "./log.mjs";

/** The gateway's own default listen address (`SYNVEDA_LISTEN_ADDR`). */
const DEFAULT_GATEWAY = "http://127.0.0.1:8120";

/**
 * The per-call deadline (ADR-0027 decision 3): two decimal orders above
 * inject's 150ms SLO, sized to absorb a cold cache — not to wait out a
 * broken dependency.
 */
const DEFAULT_TIMEOUT_MS = 3000;

export interface AdapterConfig {
  /** `SYNVEDA_DISABLED=1`, or `disabled` in the project config. */
  disabled: boolean;
  inject: boolean;
  observe: boolean;
  gatewayUrl: string;
  timeoutMs: number;
  budgetTokens?: number;
  compactBudgetTokens?: number;
}

/** The project file's shape — every field unknown until proven. */
interface ProjectConfig {
  disabled?: unknown;
  inject?: unknown;
  observe?: unknown;
  gateway_url?: unknown;
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
    gatewayUrl: trimSlash(
      str(process.env.SYNVEDA_GATEWAY) ?? str(project.gateway_url) ?? DEFAULT_GATEWAY,
    ),
    timeoutMs:
      positive(process.env.SYNVEDA_TIMEOUT_MS) ??
      positive(project.timeout_ms) ??
      DEFAULT_TIMEOUT_MS,
    budgetTokens: positive(project.budget_tokens),
    compactBudgetTokens: positive(project.compact_budget_tokens),
  };
}

function readProjectConfig(cwd: string | undefined): ProjectConfig {
  if (cwd === undefined || cwd.length === 0) return {};
  let raw: string;
  try {
    raw = readFileSync(join(cwd, ".synveda", "config.json"), "utf8");
  } catch (error) {
    if (!missing(error)) log("config.unreadable", { error: String(error) });
    return {};
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as ProjectConfig;
    }
    log("config.invalid", { reason: "not a JSON object" });
  } catch (error) {
    log("config.invalid", { error: String(error) });
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
