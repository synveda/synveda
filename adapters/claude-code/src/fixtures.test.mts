/** CPR-14: a replay is evidence only when its source and bytes are pinned. */

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { isDeepStrictEqual } from "node:util";

interface FileRecord {
  path: string;
  kind: "hook-frame" | "transcript" | "damaged-transcript";
  sha256: string;
}

interface FixtureSet {
  id: string;
  client: { name: string; version: string };
  provenance: {
    kind: string;
    captured_at: string;
    source: string;
    sanitised: boolean;
  };
  files: FileRecord[];
}

const root = fileURLToPath(new URL("../fixtures/", import.meta.url));
const schema = JSON.parse(readFileSync(join(root, "schema.json"), "utf8")) as Record<string, unknown>;
const manifest = JSON.parse(readFileSync(join(root, "manifest.json"), "utf8")) as {
  schema_version?: unknown;
  fixture_sets?: unknown;
};

function sha256(raw: string): string {
  return createHash("sha256").update(raw).digest("hex");
}

function fixtureFiles(): string[] {
  return ["hooks", "transcripts"]
    .flatMap((dir) => readdirSync(join(root, dir)).map((name) => `${dir}/${name}`))
    .sort();
}

/** The small draft-2020-12 subset used by this repository-owned schema. */
function schemaErrors(value: unknown, rawSchema: unknown, path = "$"): string[] {
  if (rawSchema === null || typeof rawSchema !== "object" || Array.isArray(rawSchema)) {
    return [`${path}: schema is not an object`];
  }
  const rule = rawSchema as Record<string, unknown>;
  const errors: string[] = [];
  if ("const" in rule && !isDeepStrictEqual(value, rule.const)) {
    errors.push(`${path}: does not equal const`);
  }
  if (Array.isArray(rule.enum) && !rule.enum.some((item) => isDeepStrictEqual(item, value))) {
    errors.push(`${path}: is outside enum`);
  }

  if (rule.type === "object") {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      return [...errors, `${path}: is not an object`];
    }
    const object = value as Record<string, unknown>;
    const properties =
      rule.properties !== null && typeof rule.properties === "object" && !Array.isArray(rule.properties)
        ? (rule.properties as Record<string, unknown>)
        : {};
    if (Array.isArray(rule.required)) {
      for (const required of rule.required) {
        if (typeof required === "string" && !(required in object)) {
          errors.push(`${path}.${required}: is required`);
        }
      }
    }
    for (const [name, child] of Object.entries(object)) {
      if (name in properties) errors.push(...schemaErrors(child, properties[name], `${path}.${name}`));
      else if (rule.additionalProperties === false) errors.push(`${path}.${name}: is not allowed`);
    }
  } else if (rule.type === "array") {
    if (!Array.isArray(value)) return [...errors, `${path}: is not an array`];
    if (typeof rule.minItems === "number" && value.length < rule.minItems) {
      errors.push(`${path}: has fewer than ${String(rule.minItems)} items`);
    }
    if (rule.items !== undefined) {
      value.forEach((item, index) => errors.push(...schemaErrors(item, rule.items, `${path}[${String(index)}]`)));
    }
  } else if (rule.type === "string") {
    if (typeof value !== "string") return [...errors, `${path}: is not a string`];
    if (typeof rule.minLength === "number" && value.length < rule.minLength) {
      errors.push(`${path}: is shorter than ${String(rule.minLength)}`);
    }
    if (typeof rule.pattern === "string" && !new RegExp(rule.pattern).test(value)) {
      errors.push(`${path}: does not match its pattern`);
    }
    if (rule.format === "date-time" && Number.isNaN(Date.parse(value))) {
      errors.push(`${path}: is not a date-time`);
    }
  }
  return errors;
}

test("the capture manifest validates its schema contract and every byte", () => {
  assert.equal(schema.$id, "https://synveda.example/schemas/claude-code-fixture-manifest-v1.json");
  assert.deepEqual(schemaErrors(manifest, schema), [], "manifest must validate against schema.json");
  assert.match(
    schemaErrors({ ...manifest, invented: true }, schema).join("\n"),
    /\$\.invented: is not allowed/,
    "the committed schema is load-bearing rather than documentary",
  );
  assert.equal(manifest.schema_version, 1);
  assert.ok(Array.isArray(manifest.fixture_sets) && manifest.fixture_sets.length > 0);
  const sets = manifest.fixture_sets as FixtureSet[];
  const recorded: string[] = [];
  for (const set of sets) {
    assert.match(set.id, /^claude-code-[0-9]+\.[0-9]+\.[0-9]+$/);
    assert.equal(set.client.name, "Claude Code");
    assert.match(set.client.version, /^[0-9]+\.[0-9]+\.[0-9]+$/);
    assert.equal(set.provenance.kind, "captured-real-client");
    assert.equal(set.provenance.sanitised, true);
    assert.ok(!Number.isNaN(Date.parse(set.provenance.captured_at)));
    assert.ok(set.provenance.source.length > 20);
    for (const file of set.files) {
      assert.match(file.path, /^(hooks\/.*\.json|transcripts\/.*\.jsonl)$/);
      assert.match(file.sha256, /^[a-f0-9]{64}$/);
      const raw = readFileSync(join(root, file.path), "utf8");
      assert.equal(sha256(raw), file.sha256, `${file.path} changed without a capture record`);
      if (file.kind === "hook-frame") assert.doesNotThrow(() => JSON.parse(raw));
      if (file.kind === "transcript") {
        const frames = raw.trim().split("\n").map((line) => JSON.parse(line) as Record<string, unknown>);
        const versions = frames
          .map((frame) => frame.version)
          .filter((version): version is string => typeof version === "string");
        assert.ok(versions.length > 0, `${file.path} carries no client version`);
        assert.ok(
          versions.every((version) => version === set.client.version),
          `${file.path} does not belong to ${set.client.version}`,
        );
      }
      recorded.push(file.path);
    }
  }
  assert.deepEqual(recorded.sort(), fixtureFiles(), "every replay fixture must have provenance");
});

test("committed fixtures contain synthetic paths and no credential material", () => {
  const forbidden = [
    /\/Users\/(?!dev(?:\/|\b))[^/\s]+/,
    /Authorization\s*:/i,
    /(?:api[_-]?key|access[_-]?token|refresh[_-]?token)\s*["'=:\s]+[A-Za-z0-9_-]{12,}/i,
    /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/,
    /sk-ant-[A-Za-z0-9_-]+/,
  ];
  for (const path of fixtureFiles()) {
    const raw = readFileSync(join(root, path), "utf8");
    for (const pattern of forbidden) {
      assert.doesNotMatch(raw, pattern, `${basename(path)} contains credential or personal data`);
    }
  }
});
