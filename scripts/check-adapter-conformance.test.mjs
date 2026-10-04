import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { resolve } from "node:path";

import {
  readmeSupportFindings,
  readmeSupportStatement,
  renderTypescript,
  validateRegistry,
} from "./check-adapter-conformance.mjs";

const root = resolve(import.meta.dirname, "..");
const source = JSON.parse(readFileSync(resolve(root, "adapters/registry.json"), "utf8"));
const copy = () => structuredClone(source);

test("the shipped registry is internally truthful", () => {
  assert.deepEqual(validateRegistry(copy(), root), []);
  assert.deepEqual(
    readmeSupportFindings(readFileSync(resolve(root, "README.md"), "utf8"), copy()),
    [],
  );
  assert.match(readmeSupportStatement(copy()), /Claude Code 2\.1\.241/u);
});

test("a competing README lifecycle summary is rejected", () => {
  const original = readFileSync(resolve(root, "README.md"), "utf8");
  const readme = original.replace(
    readmeSupportStatement(source),
    `${readmeSupportStatement(source)} A different client is also verified.`,
  );
  assert.notEqual(readme, original, "the fixture must change the support claim");
  assert.match(readmeSupportFindings(readme, copy()).join("\n"), /must match/u);
});

test("a configured recipe cannot be promoted to verified without a real lifecycle", () => {
  const registry = copy();
  registry.clients.find((client) => client.id === "cursor").support_level = "verified";
  const failures = validateRegistry(registry, root).join("\n");
  assert.match(failures, /verified requires live-client evidence/);
  assert.match(failures, /verified client is missing session_creation/);
});

test("a verified client cannot lose a required criterion", () => {
  for (const client of source.clients.filter((entry) => entry.support_level === "verified")) {
    const registry = copy();
    delete registry.clients.find((entry) => entry.id === client.id).conformance.checks.capture;
    assert.match(validateRegistry(registry, root).join("\n"), /missing capture/);
  }
});

test("captured evidence is content addressed", () => {
  const registry = copy();
  registry.clients.find((client) => client.id === "zed").authentic_fixtures[0].sha256 = "0".repeat(64);
  assert.match(validateRegistry(registry, root).join("\n"), /fixture digest drift/);
});

test("a capture without an installer is not offered as an installable client", () => {
  const registry = copy();
  const client = registry.clients.find((client) => client.id === "zed");
  client.configuration = null;
  delete client.onboarding;
  assert.deepEqual(validateRegistry(registry, root), []);
  assert.ok(!renderTypescript(registry).includes('"id": "zed"'));
  client.support_level = "configured";
  assert.match(validateRegistry(registry, root).join("\n"), /MCP configuration needs/);
  client.support_level = "verified";
  assert.match(validateRegistry(registry, root).join("\n"), /verified requires live-client evidence/);
});

test("a manual native registration remains discoverable without an automatic writer", () => {
  for (const id of ["codex", "copilot-cli"]) {
    const client = source.clients.find((entry) => entry.id === id);
    assert.equal(client.configuration, null);
    assert.equal(client.onboarding.registration, "manual");
    assert.ok(renderTypescript(source).includes(`"id": "${id}"`));
    const registry = copy();
    registry.clients.find((entry) => entry.id === id).onboarding.registration = "automatic";
    assert.match(validateRegistry(registry, root).join("\n"), /automatic onboarding requires a configuration writer/);
  }
});
