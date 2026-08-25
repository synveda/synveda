#!/usr/bin/env node
// Generates console/src/generated/api.ts from docs/api/openapi.json
// (CPR-4, ADR-0071 decision 7).
//
// The direction is the point. At the base commit of the context-platform
// programme there were two hand-written descriptions of one contract — a DTO
// per handler in Rust, and a second copy in console/src/api.mts — and nothing
// made them agree (ADR-0068's context; deletion map row 15). Now there is one:
// the Rust types are the source, `docs/api/openapi.json` is derived from them
// by the gateway, and this derives the TypeScript from that. Nobody edits the
// middle or the end.
//
// It is written here rather than taken from `openapi-typescript` for two
// reasons, neither of them "not invented here". The document is *ours* — it
// comes out of one generator, so it uses a narrow, known subset of JSON Schema
// rather than the whole of it — and the console's dependency list is a thing
// CLAUDE.md's licence rule and scripts/check-npm-licences.mjs both police, so a
// build-time dependency is a reviewed diff either way. If the document ever
// grows shapes this cannot express, it says so and exits non-zero rather than
// emitting something plausible: an unreadable failure beats a silently wrong
// type.
//
// Usage:
//   node scripts/generate-api-types.mjs           write the file
//   node scripts/generate-api-types.mjs --check   fail if it is out of date

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

const DOCUMENT = "docs/api/openapi.json";
const OUTPUT = "console/src/generated/api.ts";

const check = process.argv.includes("--check");

const document = JSON.parse(readFileSync(DOCUMENT, "utf8"));

// ── Schema → TypeScript ──────────────────────────────────────────────────────

const unsupported = [];

/** Renders one JSON Schema node as a TypeScript type expression. */
function typeOf(schema, path) {
  if (schema.$ref) {
    const name = schema.$ref.replace("#/components/schemas/", "");
    if (name.includes("/")) {
      unsupported.push(`${path}: unresolvable $ref ${schema.$ref}`);
      return "unknown";
    }
    return name;
  }
  // Utoipa emits `allOf: [{$ref}]` when a field carries a description beside a
  // referenced schema, and multiple members for Rust response structs that
  // use `serde(flatten)`. TypeScript intersections preserve both shapes.
  if (Array.isArray(schema.allOf)) {
    return schema.allOf
      .map((member, index) => typeOf(member, `${path}.allOf[${index}]`))
      .join(" & ");
  }
  if (Array.isArray(schema.oneOf) || Array.isArray(schema.anyOf)) {
    const members = schema.oneOf ?? schema.anyOf;
    return members.map((member, index) => typeOf(member, `${path}[${index}]`)).join(" | ");
  }

  // `type` may be a string or, for a nullable field, an array such as
  // ["string", "null"].
  const types = Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : [];
  const nullable = types.includes("null");
  const concrete = types.filter((type) => type !== "null");

  let rendered;
  if (concrete.length === 0) {
    if (nullable) return "null";
    // No `type` at all: an untyped object (utoipa's `Object` value type) or a
    // free-form value.
    rendered = schema.enum ? enumOf(schema.enum) : "unknown";
  } else if (concrete.length > 1) {
    rendered = concrete.map((type) => scalarOf(type, schema, path)).join(" | ");
  } else {
    rendered = scalarOf(concrete[0], schema, path);
  }
  return nullable ? `${rendered} | null` : rendered;
}

function scalarOf(type, schema, path) {
  switch (type) {
    case "string":
      return schema.enum ? enumOf(schema.enum) : "string";
    case "integer":
    case "number":
      return "number";
    case "boolean":
      return "boolean";
    case "array":
      return `${wrap(typeOf(schema.items ?? {}, `${path}[]`))}[]`;
    case "object":
      return objectOf(schema, path);
    default:
      unsupported.push(`${path}: unknown type ${JSON.stringify(type)}`);
      return "unknown";
  }
}

function enumOf(values) {
  return values.map((value) => JSON.stringify(value)).join(" | ");
}

/** Parenthesises a union so that `A | B` inside an array reads as `(A | B)[]`. */
function wrap(rendered) {
  return rendered.includes("|") ? `(${rendered})` : rendered;
}

function objectOf(schema, path) {
  const properties = schema.properties ?? {};
  const names = Object.keys(properties);
  if (names.length === 0) {
    const additional = schema.additionalProperties;
    if (additional && additional !== true) {
      return `Record<string, ${typeOf(additional, `${path}{}`)}>`;
    }
    return "Record<string, unknown>";
  }
  const required = new Set(schema.required ?? []);
  const fields = names.map((name) => {
    const property = properties[name];
    const optional = required.has(name) ? "" : "?";
    return `${docComment(property, "    ")}    ${key(name)}${optional}: ${typeOf(
      property,
      `${path}.${name}`,
    )};`;
  });
  return `{\n${fields.join("\n")}\n  }`;
}

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
function key(name) {
  return IDENTIFIER.test(name) ? name : JSON.stringify(name);
}

function docComment(schema, indent) {
  const text = schema.description;
  if (!text) return "";
  const lines = text.split("\n").map((line) => `${indent} * ${line}`.trimEnd());
  return `${indent}/**\n${lines.join("\n")}\n${indent} */\n`;
}

// ── Emit ─────────────────────────────────────────────────────────────────────

const schemas = document.components?.schemas ?? {};
const parts = [];

parts.push(`// GENERATED FILE — DO NOT EDIT.
//
// Written by scripts/generate-api-types.mjs from ${DOCUMENT}, which the
// gateway derives from its own request and response types (CPR-4, ADR-0071
// decision 7). Editing this file is editing the wrong end of the chain: change
// the Rust, run \`cargo test -p synveda-gateway --test openapi\` with
// SYNVEDA_WRITE_OPENAPI=1 to refresh the document, then
// \`node scripts/generate-api-types.mjs\`.
//
// \`make check-api-types\` fails when this file and the document disagree.
//
// Source document: ${document.info?.title ?? "Synveda"} ${document.info?.version ?? ""}
`);

for (const [name, schema] of Object.entries(schemas)) {
  if (!IDENTIFIER.test(name)) {
    unsupported.push(`components.schemas.${name}: not a TypeScript identifier`);
    continue;
  }
  const body = typeOf(schema, name);
  parts.push(`${docComment(schema, "")}export type ${name} = ${body};\n`);
}

// The operation map: one entry per (path, method), keyed by operationId, so a
// client names an operation rather than re-typing a path string that the
// contract already knows.
const METHODS = ["get", "put", "post", "patch", "delete", "head", "options", "trace"];
const operations = [];
const routes = [];
for (const [path, item] of Object.entries(document.paths ?? {})) {
  for (const method of METHODS) {
    const operation = item[method];
    if (!operation) continue;
    const id = operation.operationId;
    if (!id || !IDENTIFIER.test(id)) {
      unsupported.push(`${method.toUpperCase()} ${path}: missing or unusable operationId`);
      continue;
    }
    const body = requestBodyType(operation, `${id}.requestBody`);
    const success = successResponseType(operation, `${id}.responses`);
    const fields = [
      `    readonly path: ${JSON.stringify(path)};`,
      `    readonly method: ${JSON.stringify(method.toUpperCase())};`,
    ];
    if (body) fields.push(`    readonly body: ${body};`);
    // A required `Idempotency-Key` is part of the contract, so it is part of
    // the generated type: the client makes the key a required argument on
    // exactly these operations (CPR-8). A flag rather than the parameter
    // list, because the header's *name* is fixed by the document and the only
    // thing a caller supplies is the value.
    const idempotent = requiresIdempotencyKey(operation);
    if (idempotent) {
      fields.push(`    readonly idempotent: true;`);
    }
    fields.push(`    readonly response: ${success};`);
    operations.push(
      `${docComment(operation.summary ? { description: operation.summary } : operation, "  ")}  readonly ${id}: {\n${fields.join(
        "\n",
      )}\n  };`,
    );
    // The same rows as values. `Operations` is a type and erases, so a
    // client that has to build a URL needs the path and the method at
    // runtime — and taking them from anywhere but here would be the second
    // hand-written copy of one contract this generator exists to remove.
    routes.push(
      `  ${id}: { path: ${JSON.stringify(path)}, method: ${JSON.stringify(
        method.toUpperCase(),
      )}${idempotent ? ", idempotent: true" : ""} },`,
    );
  }
}

/** Whether the operation declares `Idempotency-Key` as a required header. */
function requiresIdempotencyKey(operation) {
  return (operation.parameters ?? []).some(
    (parameter) =>
      parameter.in === "header" &&
      typeof parameter.name === "string" &&
      parameter.name.toLowerCase() === "idempotency-key" &&
      parameter.required === true,
  );
}

function requestBodyType(operation, path) {
  const content = operation.requestBody?.content;
  if (!content) return null;
  const json = content["application/json"];
  if (!json) {
    unsupported.push(`${path}: no application/json content`);
    return null;
  }
  return typeOf(json.schema ?? {}, path);
}

function successResponseType(operation, path) {
  const responses = operation.responses ?? {};
  const codes = Object.keys(responses)
    .filter((code) => /^2\d\d$/.test(code))
    .sort();
  const rendered = codes
    .map((code) => {
      const json = responses[code].content?.["application/json"];
      return json ? typeOf(json.schema ?? {}, `${path}.${code}`) : "void";
    })
    // 200 and 201 usually carry the same body (a replayed creation), so one
    // union member is the honest rendering of both.
    .filter((type, index, all) => all.indexOf(type) === index);
  if (rendered.length === 0) return "void";
  return rendered.join(" | ");
}

parts.push(`/**
 * Every operation the contract declares, keyed by its operation id.
 *
 * \`body\` is present exactly when the operation takes a request body;
 * \`idempotent\` exactly when it requires an \`Idempotency-Key\` header;
 * \`response\` is the union of its 2xx bodies (\`void\` for a 204). Error
 * bodies are {@link ApiErrorBody} on every operation and are not repeated
 * here.
 */
export type Operations = {
${operations.join("\n")}
};

/** An operation id. */
export type OperationId = keyof Operations;

/**
 * Every operation's path template and method, as values.
 *
 * The runtime half of {@link Operations}: a type erases, and a client has to
 * build a URL. Generated from the same document in the same pass, so the two
 * cannot disagree. \`idempotent\` marks the operations whose document requires
 * an \`Idempotency-Key\` header.
 */
export const OPERATIONS = {
${routes.join("\n")}
} as const satisfies Record<
  OperationId,
  { readonly path: string; readonly method: string; readonly idempotent?: true }
>;
`);

const rendered = `${parts.join("\n")}`;

if (unsupported.length > 0) {
  console.error("scripts/generate-api-types.mjs: shapes this generator cannot express:");
  for (const problem of unsupported) console.error(`  - ${problem}`);
  console.error(
    "\nThe document uses JSON Schema this generator does not handle. Either simplify\n" +
      "the Rust type, or teach this script the shape — do not hand-edit the output.",
  );
  process.exit(1);
}

if (check) {
  let current = "";
  try {
    current = readFileSync(OUTPUT, "utf8");
  } catch {
    console.error(`${OUTPUT} is missing. Run: node scripts/generate-api-types.mjs`);
    process.exit(1);
  }
  if (current !== rendered) {
    console.error(
      `${OUTPUT} is out of date with ${DOCUMENT}.\n` +
        "Run: node scripts/generate-api-types.mjs",
    );
    process.exit(1);
  }
  console.log(`${OUTPUT} is current with ${DOCUMENT}`);
} else {
  mkdirSync(dirname(OUTPUT), { recursive: true });
  writeFileSync(OUTPUT, rendered);
  console.log(`wrote ${OUTPUT} (${Object.keys(schemas).length} schemas, ${operations.length} operations)`);
}
