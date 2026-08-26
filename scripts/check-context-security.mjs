#!/usr/bin/env node
// CPR-42: security guarantees are spread across database, gateway, adapter,
// console and format-boundary suites. `make test` executes those suites; this
// checker makes the inventory explicit and prevents a refactor from silently
// deleting an entire adversarial boundary while leaving the rest green.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));

export const REQUIRED_EVIDENCE = [
  ["forced-RLS completeness", "crates/synveda-store/tests/rls.rs", "every_tenant_scoped_table_is_covered_and_forced"],
  ["cross-tenant identifier oracle", "crates/synveda-gateway/tests/foundation_audit.rs", "a_valid_identifier_from_another_tenant_is_indistinguishable_from_a_fictional_one"],
  ["principal-scope privacy", "crates/synveda-gateway/tests/foundation_audit.rs", "a_tenant_administrator_cannot_reach_somebody_elses_own_scope"],
  ["one-time invitation replay", "crates/synveda-gateway/tests/access_api.rs", "an_invitation_link_works_once_and_a_retry_is_not_a_second_redemption"],
  ["invitation audit secrecy", "crates/synveda-gateway/tests/access_api.rs", "the_invitation_token_never_reaches_the_audit_chain"],
  ["session actor/scope spoofing", "crates/synveda-gateway/tests/sessions_api.rs", "a_client_cannot_submit_its_own_tenant_or_acting_principal"],
  ["cross-run event oracle", "crates/synveda-gateway/tests/sessions_api.rs", "An event id from another run, another tenant, or nowhere at all"],
  ["capture source forgery", "crates/synveda-gateway/tests/capture_api.rs", "capture_candidate_events_frozen_event_fk"],
  ["governed content erasure", "crates/synveda-gateway/tests/knowledge_lifecycle.rs", "review_is_live_and_forget_leaves_only_content_free_evidence"],
  ["knowledge-source disclosure", "crates/synveda-gateway/tests/knowledge_lifecycle.rs", "public_knowledge_api_is_current_governed_paginated_and_tenant_safe"],
  ["context side-channel resistance", "crates/synveda-gateway/tests/context_runs.rs", "denied_private_knowledge_leaks_no_address_content_count_or_block_fingerprint"],
  ["graph-path authorisation", "crates/synveda-gateway/tests/context_runs.rs", "bounded_graph_improves_two_hop_recall_and_denied_endpoints_leave_no_trace"],
  ["trace-retention disclosure", "crates/synveda-gateway/tests/context_runs.rs", "retention_modes_and_diagnostic_query_have_distinct_disclosure"],
  ["skill bundle path safety", "crates/synveda-types/src/skill.rs", "a_bundled_path_is_validated_against_filesystems"],
  ["skill sandbox and declared-tool separation", "crates/synveda-gateway/tests/skills.rs", "declared_tools_are_authorization"],
  ["MCP execution/read-only boundary", "crates/synveda-gateway/src/tool_registry.rs", "tools/call is never accepted"],
  ["MCP schema-change quarantine", "crates/synveda-gateway/tests/tools.rs", "versions_discovery_bindings_config_and_tests_share_one_governed_path"],
  ["MCP secret lifecycle", "crates/synveda-gateway/tests/tools.rs", "stable_tool_secret_references_fail_closed_rotate_without_rewriting_versions_and_can_be_removed"],
  ["OKF traversal/expansion/credential boundary", "crates/synveda-okf/tests/okf_v02.rs", "source_and_entry_boundaries_refuse_credentials_links_binary_and_escape"],
  ["audit content minimisation", "crates/synveda-gateway/tests/audit_query.rs", "no_knowledge_content_reaches_any_audit_answer"],
  ["directory credential fail-closed", "crates/synveda-gateway/tests/directory_sync.rs", "an_unusable_stable_credential_never_falls_back_to_deployment_configuration"],
  ["personal auto-apply stays in VedaFlow", "crates/synveda-gateway/tests/relaxations.rs", "personal_auto_apply_uses_vedaflow_and_immutable_versions"],
  ["UI denied-content rendering", "console/src/context.test.tsx", "denied page leaked"],
  ["UI capability failure closes", "console/src/review.test.tsx", "an unreadable capability forecast fails closed"],
  ["adapter spool tamper hold", "adapters/claude-code/src/hook.test.mts", "a tampered spool is held and the automatic retry sends nothing"],
  ["adapter cross-gateway isolation", "adapters/claude-code/src/hook.test.mts", "a spool never crosses from one gateway deployment to another"],
  ["adapter diagnostic redaction", "adapters/claude-code/src/log.test.mts", "secret fields and exception messages never reach the diagnostic file"],
];

const READ_ONLY_METHODS = ["server/discover", "tools/list", "resources/list", "prompts/list"];

function read(relative) {
  return readFileSync(join(ROOT, relative), "utf8");
}

function filesUnder(relative, accept) {
  const root = join(ROOT, relative);
  const output = [];
  const visit = (path) => {
    for (const entry of readdirSync(path)) {
      const full = join(path, entry);
      if (statSync(full).isDirectory()) visit(full);
      else if (accept(full)) output.push(relativePath(full));
    }
  };
  visit(root);
  return output.sort();
}

function relativePath(path) {
  return relative(ROOT, path).replaceAll("\\", "/");
}

export function markerFindings(sources, requirements = REQUIRED_EVIDENCE) {
  const findings = [];
  for (const [boundary, file, marker] of requirements) {
    const source = sources[file];
    if (source === undefined) findings.push(`${boundary}: missing ${file}`);
    else if (!source.includes(marker)) findings.push(`${boundary}: ${file} lost ${marker}`);
  }
  return findings;
}

export function rawDiagnosticFindings(sources) {
  const findings = [];
  const raw = /\bString\s*\(\s*(?:error|reason)\s*\)/g;
  for (const [file, source] of Object.entries(sources)) {
    for (const match of source.matchAll(raw)) {
      findings.push(`${file}:${lineOf(source, match.index)} logs or propagates a raw exception`);
    }
  }
  return findings;
}

export function executionFindings(sources) {
  const findings = [];
  const process = /\b(?:std|tokio)::process\b|\bprocess::Command\b|\bCommand::new\s*\(/g;
  for (const [file, source] of Object.entries(sources)) {
    for (const match of source.matchAll(process)) {
      findings.push(`${file}:${lineOf(source, match.index)} can execute a process inside a metadata boundary`);
    }
  }
  return findings;
}

export function clientStorageFindings(sources) {
  const findings = [];
  const storage = /\bsynveda_store\b|\bsqlx(?:::|\s)|\bDATABASE_URL\b|postgres(?:ql)?:\/\//g;
  for (const [file, source] of Object.entries(sources)) {
    for (const match of source.matchAll(storage)) {
      findings.push(`${file}:${lineOf(source, match.index)} couples an ordinary client to storage`);
    }
  }
  return findings;
}

export function spoolContractFindings(sources) {
  const required = [
    ["adapters/claude-code/src/spool.mts", "entryIntact(entry)"],
    ["adapters/claude-code/src/spool.mts", "loadOrCreateSpool"],
    ["adapters/claude-code/src/spool.mts", "status: \"held\""],
    ["adapters/claude-code/src/session-start.mts", "bindGateway(spool, config.gatewayUrl)"],
    ["adapters/claude-code/src/turn.mts", "bindGateway(spool, config.gatewayUrl)"],
    ["crates/synveda-cli/src/session.rs", "pin_gateway(&mut spool.gateway_url, api.gateway())"],
    ["adapters/claude-code/src/log.mts", "safeFields(fields)"],
    ["adapters/claude-code/src/log.mts", "[redacted]"],
  ];
  return markerFindings(
    sources,
    required.map(([file, marker]) => [`runtime guard ${marker}`, file, marker]),
  );
}

export function readOnlyMethodFindings(source) {
  const match = source.match(/const READ_ONLY_METHODS[^=]*=\s*\[([\s\S]*?)\];/);
  if (!match) return ["tool registry lost its closed read-only method set"];
  const methods = [...match[1].matchAll(/"([^"]+)"/g)].map((item) => item[1]);
  return JSON.stringify(methods) === JSON.stringify(READ_ONLY_METHODS)
    ? []
    : [`tool registry read-only methods are ${JSON.stringify(methods)}`];
}

function lineOf(source, index = 0) {
  return source.slice(0, index).split("\n").length;
}

function sourceMap(files) {
  return Object.fromEntries(files.map((file) => [file, read(file)]));
}

export function main() {
  const findings = [];
  const evidenceFiles = [...new Set(REQUIRED_EVIDENCE.map(([, file]) => file))];
  findings.push(...markerFindings(sourceMap(evidenceFiles)));

  const adapterFiles = filesUnder(
    "adapters/claude-code/src",
    (file) =>
      file.endsWith(".mts") &&
      !file.endsWith(".test.mts") &&
      !file.endsWith("/driver.mts") &&
      !file.endsWith("/mock-gateway.mts"),
  );
  findings.push(...rawDiagnosticFindings(sourceMap(adapterFiles)));

  const executionFiles = [
    "crates/synveda-gateway/src/skills.rs",
    "crates/synveda-gateway/src/tool_registry.rs",
    ...filesUnder("crates/synveda-okf/src", (file) => file.endsWith(".rs")),
  ];
  findings.push(...executionFindings(sourceMap(executionFiles)));

  const clientFiles = [
    ...adapterFiles,
    ...filesUnder(
      "console/src",
      (file) => /\.(?:ts|tsx|mts)$/.test(file) && !/\.test\./.test(file) && !file.includes("/generated/"),
    ),
  ];
  findings.push(...clientStorageFindings(sourceMap(clientFiles)));

  const guardFiles = [
    "adapters/claude-code/src/spool.mts",
    "adapters/claude-code/src/session-start.mts",
    "adapters/claude-code/src/turn.mts",
    "adapters/claude-code/src/log.mts",
    "crates/synveda-cli/src/session.rs",
  ];
  findings.push(...spoolContractFindings(sourceMap(guardFiles)));
  findings.push(...readOnlyMethodFindings(read("crates/synveda-gateway/src/tool_registry.rs")));

  if (findings.length > 0) {
    throw new Error(`context security gate:\n${findings.map((item) => `- ${item}`).join("\n")}`);
  }
  console.log(
    `context security gate holds: ${REQUIRED_EVIDENCE.length} adversarial boundaries, ` +
      `${adapterFiles.length} adapter files, ${clientFiles.length} public-client files, ` +
      "closed MCP discovery methods and tamper-held deployment-bound spools",
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
