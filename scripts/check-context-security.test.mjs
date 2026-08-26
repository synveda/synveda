import assert from "node:assert/strict";
import test from "node:test";

import {
  executionFindings,
  markerFindings,
  rawDiagnosticFindings,
  readOnlyMethodFindings,
  spoolContractFindings,
} from "./check-context-security.mjs";

test("deleting a named adversarial boundary fails the inventory", () => {
  const requirements = [["tenant isolation", "tests/rls.rs", "forced_rls_is_complete"]];
  assert.deepEqual(markerFindings({ "tests/rls.rs": "fn something_else() {}" }, requirements), [
    "tenant isolation: tests/rls.rs lost forced_rls_is_complete",
  ]);
});

test("raw exception text in adapter diagnostics is rejected", () => {
  const findings = rawDiagnosticFindings({
    "adapter.mts": 'log("failed", { error: String(error) });',
  });
  assert.equal(findings.length, 1);
  assert.match(findings[0], /raw exception/);
  assert.deepEqual(
    rawDiagnosticFindings({ "adapter.mts": 'log("failed", { error: diagnostic(error) });' }),
    [],
  );
});

test("gateway metadata handlers cannot grow a process execution seam", () => {
  assert.equal(executionFindings({ "tools.rs": "Command::new(command).spawn();" }).length, 1);
  assert.deepEqual(executionFindings({ "tools.rs": "validate_read_only(methods)?;" }), []);
});

test("the MCP test method set is exact and excludes execution", () => {
  const safe = `const READ_ONLY_METHODS: [&str; 4] = [
    "server/discover", "tools/list", "resources/list", "prompts/list",
  ];`;
  assert.deepEqual(readOnlyMethodFindings(safe), []);
  assert.equal(readOnlyMethodFindings(safe.replace('"prompts/list"', '"tools/call"')).length, 1);
});

test("removing payload verification or either gateway pin fails the spool gate", () => {
  const sources = {
    "adapters/claude-code/src/spool.mts":
      'entryIntact(entry); loadOrCreateSpool(); return { status: "held" };',
    "adapters/claude-code/src/session-start.mts": "bindGateway(spool, config.gatewayUrl);",
    "adapters/claude-code/src/turn.mts": "bindGateway(spool, config.gatewayUrl);",
    "adapters/claude-code/src/log.mts": 'safeFields(fields); "[redacted]";',
    "crates/synveda-cli/src/session.rs": "pin_gateway(&mut spool.gateway_url, api.gateway());",
  };
  assert.deepEqual(spoolContractFindings(sources), []);
  sources["adapters/claude-code/src/spool.mts"] = 'loadOrCreateSpool(); return { status: "held" };';
  assert.equal(spoolContractFindings(sources).length, 1);
});
