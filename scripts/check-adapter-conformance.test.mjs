import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { resolve } from "node:path";

import { validateRegistry } from "./check-adapter-conformance.mjs";

const root = resolve(import.meta.dirname, "..");
const source = JSON.parse(readFileSync(resolve(root, "adapters/registry.json"), "utf8"));
const copy = () => structuredClone(source);

test("the shipped registry is internally truthful", () => {
  assert.deepEqual(validateRegistry(copy(), root), []);
});

test("a configured recipe cannot be promoted to verified without a real lifecycle", () => {
  const registry = copy();
  registry.clients.find((client) => client.id === "cursor").support_level = "verified";
  const failures = validateRegistry(registry, root).join("\n");
  assert.match(failures, /verified requires live-client evidence/);
  assert.match(failures, /verified client is missing session_creation/);
});

test("a verified client cannot lose a required criterion", () => {
  const registry = copy();
  delete registry.clients.find((client) => client.id === "claude-code").conformance.checks.capture;
  assert.match(validateRegistry(registry, root).join("\n"), /missing capture/);
});

test("captured evidence is content addressed", () => {
  const registry = copy();
  registry.clients.find((client) => client.id === "zed").authentic_fixtures[0].sha256 = "0".repeat(64);
  assert.match(validateRegistry(registry, root).join("\n"), /fixture digest drift/);
});
