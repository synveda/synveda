/** CPR-29: the Claude adapter remains a public application client. */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const root = new URL("../../../", import.meta.url);
const document = JSON.parse(
  readFileSync(new URL("docs/api/openapi.json", root), "utf8"),
) as { paths: Record<string, unknown> };
const source = readFileSync(new URL("../src/client.mts", import.meta.url), "utf8");

function executableSource(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");
}

test("every Claude adapter application path exists in generated OpenAPI", () => {
  const paths = new Set(Object.keys(document.paths));
  const literals = [
    ...executableSource(source).matchAll(/(["'`])(\/v1\/[^"'`]+)\1/g),
  ].map((match) => match[2]?.replace(/\$\{[^}]+\}/g, "{session_id}"));

  assert.deepEqual(
    [...new Set(literals)].sort(),
    [
      "/v1/me",
      "/v1/sessions",
      "/v1/sessions/{session_id}/context-runs",
      "/v1/sessions/{session_id}/end",
      "/v1/sessions/{session_id}/events",
    ],
  );
  for (const path of literals) {
    assert.ok(path !== undefined && paths.has(path), `${String(path)} is absent from OpenAPI`);
  }
});

test("the Claude adapter has no storage or retired global-runtime authority", () => {
  const executable = executableSource(source);
  for (const forbidden of [
    "DATABASE_URL",
    "synveda_store",
    "sqlx",
    "/v1/observe",
    "/v1/inject",
    "/v1/recall",
  ]) {
    assert.ok(!executable.includes(forbidden), `client contains forbidden ${forbidden}`);
  }
});
