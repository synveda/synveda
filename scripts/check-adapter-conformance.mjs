#!/usr/bin/env node
// CPR-39: one registry is the authority for config generation, onboarding and
// support claims. A fixture can prove captured protocol bytes; only a complete
// named real-client run can prove `verified`.

import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { pathToFileURL } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const DEFAULT_REGISTRY = resolve(ROOT, "adapters/registry.json");
const DEFAULT_MATRIX = resolve(ROOT, "docs/CLIENT_SUPPORT.md");
const DEFAULT_TYPESCRIPT = resolve(ROOT, "console/src/generated/adapter-clients.ts");
const LEVELS = new Set(["configured", "captured", "verified", "experimental", "unsupported"]);
const STATUSES = new Set(["passed", "failed", "not_run", "not_applicable"]);

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function validateRegistry(registry, root = ROOT) {
  const failures = [];
  const fail = (message) => failures.push(message);
  if (registry.schema_version !== 1) fail("schema_version must be 1");
  const required = registry.required_conformance;
  if (!Array.isArray(required) || required.length === 0) fail("required_conformance must be non-empty");
  if (new Set(required ?? []).size !== (required ?? []).length) fail("required_conformance has duplicates");
  for (const level of LEVELS) {
    if (typeof registry.support_levels?.[level] !== "string") fail(`support_levels.${level} is missing`);
  }
  if (!Array.isArray(registry.clients) || registry.clients.length === 0) fail("clients must be non-empty");

  const ids = new Set();
  for (const client of registry.clients ?? []) {
    const at = client.id || "<missing-id>";
    if (!/^[a-z0-9][a-z0-9-]*$/.test(client.id ?? "")) fail(`${at}: invalid id`);
    if (ids.has(client.id)) fail(`${at}: duplicate id`);
    ids.add(client.id);
    if (!client.display_name) fail(`${at}: display_name is missing`);
    if (!LEVELS.has(client.support_level)) fail(`${at}: invalid support_level ${client.support_level}`);
    if (!(["plugin", "mcp"].includes(client.connection))) fail(`${at}: connection must be plugin or mcp`);
    if (!Array.isArray(client.tested_versions)) fail(`${at}: tested_versions must be an array`);
    if (!Array.isArray(client.limitations) || client.limitations.length === 0) fail(`${at}: limitations must be explicit`);
    if (!Array.isArray(client.authentic_fixtures)) fail(`${at}: authentic_fixtures must be an array`);

    if (client.connection === "mcp" && client.support_level !== "unsupported") {
      const config = client.configuration;
      if (!config || !config.key || !["json", "jsonc"].includes(config.syntax) || !config.restart) {
        fail(`${at}: MCP configuration needs key, syntax and restart`);
      }
      if (!config?.path || Object.keys(config.path).length === 0) fail(`${at}: MCP configuration has no documented path`);
    }

    for (const fixture of client.authentic_fixtures ?? []) {
      if (!fixture.kind?.startsWith("captured")) fail(`${at}: ${fixture.path} is not labelled captured`);
      if (!/^[0-9a-f]{64}$/.test(fixture.sha256 ?? "")) fail(`${at}: ${fixture.path} has no SHA-256 digest`);
      const path = resolve(root, fixture.path ?? "");
      if (!existsSync(path)) fail(`${at}: fixture does not exist: ${fixture.path}`);
      else if (sha256(path) !== fixture.sha256) fail(`${at}: fixture digest drift: ${fixture.path}`);
    }

    const checks = client.conformance?.checks ?? {};
    for (const [criterion, result] of Object.entries(checks)) {
      if (!(required ?? []).includes(criterion)) fail(`${at}: unknown conformance criterion ${criterion}`);
      if (!STATUSES.has(result.status)) fail(`${at}: ${criterion} has invalid status ${result.status}`);
      for (const evidence of result.evidence ?? []) {
        if (!existsSync(resolve(root, evidence))) fail(`${at}: ${criterion} evidence does not exist: ${evidence}`);
      }
    }

    if (client.support_level === "captured" && client.authentic_fixtures.length === 0) {
      fail(`${at}: captured requires authentic digest-pinned frames`);
    }
    if (client.support_level === "verified") {
      if (client.conformance?.evidence_level !== "live-client") fail(`${at}: verified requires live-client evidence`);
      if (!client.conformance?.tested_at || !client.conformance?.tested_version) fail(`${at}: verified requires a named version and instant`);
      if (!client.tested_versions.includes(client.conformance?.tested_version)) fail(`${at}: verified version is absent from tested_versions`);
      if (client.authentic_fixtures.length === 0) fail(`${at}: verified requires authentic fixtures`);
      for (const criterion of required ?? []) {
        const result = checks[criterion];
        if (!result) fail(`${at}: verified client is missing ${criterion}`);
        else if (result.status !== "passed" && result.status !== "not_applicable") {
          fail(`${at}: verified client has not passed ${criterion}`);
        } else if (!Array.isArray(result.evidence) || result.evidence.length === 0) {
          fail(`${at}: ${criterion} has no evidence path`);
        }
      }
    }
  }
  return failures;
}

function escapeCell(value) {
  return String(value).replaceAll("|", "\\|").replaceAll("\n", " ");
}

export function renderMatrix(registry) {
  const rows = registry.clients.map((client) => {
    const versions = client.tested_versions.length ? client.tested_versions.join(", ") : "none";
    return `| ${escapeCell(client.display_name)} | \`${client.support_level}\` | ${escapeCell(versions)} | ${escapeCell(client.lifecycle.mechanism)} | ${escapeCell(client.limitations[0])} |`;
  });
  const details = registry.clients.map((client) => {
    const fixtureLines = client.authentic_fixtures.length
      ? client.authentic_fixtures.map((fixture) => `- \`${fixture.path}\` — ${fixture.kind}, SHA-256 \`${fixture.sha256}\``).join("\n")
      : "- None. Configuration or an inspected vendor contract is not a captured client frame.";
    const checks = registry.required_conformance.map((name) => {
      const result = client.conformance.checks[name];
      return `- \`${name}\`: ${result?.status ?? "not_run"}${result?.evidence?.length ? ` — ${result.evidence.map((path) => `\`${path}\``).join(", ")}` : ""}`;
    }).join("\n");
    return `### ${client.display_name} — \`${client.support_level}\`\n\nContract: ${client.lifecycle.contract_version ?? "not established"}. Evidence level: \`${client.conformance.evidence_level}\`.\n\nAuthentic fixtures:\n\n${fixtureLines}\n\nConformance:\n\n${checks}\n\nKnown limits:\n\n${client.limitations.map((item) => `- ${item}`).join("\n")}`;
  }).join("\n\n");
  return `# Client adapter support\n\nThis file is generated from \`adapters/registry.json\` by \`make check-adapters\`. Do not edit it by hand.\n\nA connection recipe is not a support claim. \`captured\` means authentic frames replay; only \`verified\` means a named real client version completed the full public-API lifecycle and left persisted, audited evidence.\n\n| Client | Level | Tested versions | Lifecycle | Principal limit |\n| --- | --- | --- | --- | --- |\n${rows.join("\n")}\n\n## Evidence\n\n${details}\n`;
}

export function renderTypescript(registry) {
  const clients = registry.clients.map((client) => ({
    id: client.id,
    label: client.display_name,
    via: client.connection,
    supportLevel: client.support_level,
    note: `${client.support_level}: ${client.limitations[0]}`,
  }));
  return `// Generated from adapters/registry.json by scripts/check-adapter-conformance.mjs.\n// Do not edit by hand: support claims and connection choices share one authority.\n\nexport const GENERATED_AGENT_CLIENTS = ${JSON.stringify(clients, null, 2)} as const;\n\nexport type GeneratedAgentClient = (typeof GENERATED_AGENT_CLIENTS)[number];\n`;
}

function checkOrWrite(path, expected, write, failures) {
  if (write) {
    writeFileSync(path, expected);
    return;
  }
  const actual = existsSync(path) ? readFileSync(path, "utf8") : "";
  if (actual !== expected) failures.push(`${path}: generated output drift; run node scripts/check-adapter-conformance.mjs --write`);
}

export function run({ registryPath = DEFAULT_REGISTRY, root = ROOT, write = false, matrixPath = DEFAULT_MATRIX, typescriptPath = DEFAULT_TYPESCRIPT } = {}) {
  const registry = JSON.parse(readFileSync(registryPath, "utf8"));
  const failures = validateRegistry(registry, root);
  if (failures.length === 0) {
    checkOrWrite(matrixPath, renderMatrix(registry), write, failures);
    checkOrWrite(typescriptPath, renderTypescript(registry), write, failures);
  }
  return failures;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const failures = run({ write: process.argv.includes("--write") });
  if (failures.length) {
    console.error("adapter conformance gate failed:");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exitCode = 1;
  } else {
    console.log("adapter conformance: registry, evidence digests and generated support surfaces agree");
  }
}
