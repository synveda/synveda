#!/usr/bin/env node
// CPR-44: product SQL belongs in synveda-store. Freeze the older audit and
// VedaFlow call sites until they move behind store APIs, preserving their
// transaction and RLS behavior during that migration. Apalis owns only its
// separate transport database; the CLI deployment fixture is test-only.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative } from "node:path";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const QUERY = /\bsqlx\s*::\s*(query(?:_as|_scalar|_file|_file_as)?|raw_sql|QueryBuilder)\b\s*(!)?/g;
const IMPORT = /\buse\s+sqlx\s*::\s*(?:\{([^}]*)\}|(query(?:_as|_scalar|_file|_file_as)?|raw_sql|QueryBuilder)\b)/g;
const QUERY_NAME = /\b(?:query(?:_as|_scalar|_file|_file_as)?|raw_sql|QueryBuilder)\b/;
const RUST_STRING = /^(?:r(#+)"([\s\S]*?)"\1|r"([\s\S]*?)"|"((?:\\.|[^"\\])*)")/;

// SHA-256 of kind + exact Rust SQL literal. Paths and multiplicity are pinned;
// a moved or changed legacy query needs an explicit, reviewable inventory edit.
const LEGACY = new Map([
  ["crates/synveda-audit/src/chain.rs", [
    "47f29829f8e275db2994061d399c921b8439e3cb6012724c667567772bdfa43a",
    "6c5bf6687bd676bafc59f7b53b76b0dd99433ff0e015d9f5a160841261f6b1a6",
    "7773037258dd350b77035bf796cefefb4fea877c02c11bdd1dd4b8555e1634dd",
    "a44cdf7f618f7486d67a02869bd9a7988cd16a69c9b63b1afd9b4c262dcb18f2",
    "b71a540d3be7a9954236b6b87bb8cb2d8011c1329c38b6174ef739b4754293a4",
    "c3887012bbc2b4b71653f45d04915157389353f0c6c8e2d39bfc6739cca0c01e",
    "d14632499e4730c93ea63b564656e76e8ccc9446f1f42c0b0491190ee67d667b",
    "eb87840d0d9f3bedd9d644a301b4fa9010881757ecd264c522f0c43499d601ee",
    "f68c913abaf4611a8975c1cd06e3f957600b0516a23b5fce555fd03cf68267d9",
  ]],
  ["crates/synveda-audit/src/query.rs", [
    "4b0c384732dc5bd1288da34ed86379960305f41ede3f8a23a820d78348845b11",
    "82e5e4e9c5e3b3d8e5cec6ec0c67f3c2ec5483050c86268c13e21e8f6a14ed69",
    "861d835247a9a53006ffbba1804083952c97723fba84b1d63bb2e1181685ba85",
    "a869961e8b2e9981c78607cc71494beec9998dad1b064de0c37f9da841395741",
    "eb87840d0d9f3bedd9d644a301b4fa9010881757ecd264c522f0c43499d601ee",
    "ef3c04b247720837b9eea8070be040b1b38a11bfc10ba4aa76569a857a47effa",
  ]],
  ["crates/synveda-vedaflow/src/channels.rs", [
    "7af65b008b0d442e6a40177b4003dee9013a4dd50a6188e048ea41846c387572",
    "7bd4052bfbe88aad9ce4b8e9fb960246e31fb49f420f2d2e6a17509321840d3b",
    "ad0d64b8e132379873fe6ac2d4bdcd75c017857ff49fec562e86319b76e92324",
    "bdd49de405d9255d34e573fb3e0c5fcb7e4609de33ca8885289a77bc50d02d8c",
    "bff1867c41b43afff8d9f518d1b5056267ef58940dc6b9c795f004f782da4fbb",
    "f07a58f642deddf97758bd8d1a1c9055a36ae3b670a0206c2d0b82e92dc9bedb",
    "f114e164f8d020d73f5fab850c43e15fda84268258fb677ca94dd550d157a57a",
    "f1e129b671134bc3125d706ba5bd247a93f0338f16517407dc39f6353abdc85c",
  ]],
  ["crates/synveda-vedaflow/src/commits.rs", [
    "40e23c07807ad4eb7e87cb736cef5cd82b8e82057b97be87418071346576b055",
    "57c614002482ef98ad6c3304ac28d0b11037d7515ccc41697fc0aa14e54ce529",
    "8145c350a4413e61cb399b22e9f368d6fd1883a8f93edd1d3971bdab71472fb1",
    "bb8c868b5dfdc26ca301bb994b2ac9f1d0bad193f8e151cd57c00aa3426822a1",
    "d8ee4048d331614181d7bf3a76f24bf0f96ec9e45ee15b2e3cf9cd530d606783",
    "ef7fec6d5578d2fafff9ab1aa5e68bd175ed366e6c54a174fd41372b8869dae6",
  ]],
  ["crates/synveda-vedaflow/src/curators.rs", [
    "a24117a8a1554fda7acea53da8b71f30151dd0a79ca6dfc00b29110dc291c215",
  ]],
  ["crates/synveda-vedaflow/src/objects.rs", [
    "69b340c6ee15abde7bc09e15e4f0afedd0a0db6a1b45759547a9fdf5c5759364",
    "ce5b9832b7939dbae7fc04e46be5d29eb0b9a2d82efb9feefd0144276e9c2453",
    "ee2d34f72c9ec642aee5c5fbffb0c9dcfbdc5c68049365e9933b0b8122b2793f",
  ]],
  ["crates/synveda-vedaflow/src/proposals.rs", [
    "1a565fee1e8ceb7138e2ab6f062de196af2da0ae99f6fe6ab333ec174dfc8ed8",
    "34774efc8d979e142d5c2b7189573c0c75b6bece15cac52ab1711fb344e347b7",
    "4e8baedcfad70449687d996d3919f3040bc2ff710c881f883e0fbdd3ba8311e5",
    "64cd0fe7f7dbf65708c2c969fcefc18d8616eb134e2f1d9b02341e70e5c18243",
    "8557c4f45b320d9acac1b8dcc691873e5fe346c4a708c9d2dadeee28b9c7eb5f",
    "9899d2e6eef9bbb602dbcf3069eaa643299afb4c857e3883c5cb800f1c57a47a",
    "e0d9cf470743bbce358e792d0697c7125e3de7f4da7bfa5862de0ae001a48530",
    "f07a58f642deddf97758bd8d1a1c9055a36ae3b670a0206c2d0b82e92dc9bedb",
    "fd11375b6491fb783ff406ee118e04b08267bac82da665e38231c00bffc31729",
  ]],
  ["crates/synveda-vedaflow/src/refs.rs", [
    "34e2c472f45b8c1438a1dd9951e879745761326b1401140192ebefa0bba841f4",
    "7817f6202f4ea2048646c2fa565379a0cd70ec8a3cdad5d182e280084b605236",
    "7af65b008b0d442e6a40177b4003dee9013a4dd50a6188e048ea41846c387572",
    "96f45ee0bae48ea4871644676762f970b4186dd5afccda3f94e0c8b38a8eefaf",
  ]],
  ["crates/synveda-vedaflow/src/trees.rs", [
    "3e3bec069e5f844ae1cc6923a23b4c430776e76236dd6d7226e35d8173e24aff",
    "9fbf10978a8b9acdc6ccb753f4ec9a5ad81556017038a786d7074cdbcf267d4e",
    "d5affb0c03ce29e65a8aa87a3451c265296f14c5daf2595ac2fe907df622d5da",
    "fa2cf2fbf0d754b950d8234b364b14e05d6f21d5fa3b37d4e084549b628bdfc0",
  ]],
  ["crates/synveda-vedaflow/src/verify.rs", [
    "21d8fde40155a10f1df721ef3b1dfd171e0a632edf1ec3a1a4ebd1996322ffbc",
    "706b444df49775bc49344342e25e8d6c204988f1e0ad8f4fe51040640476c9f9",
    "79a9898df72dea91c17953c652892e6c4084cde8ef1f679749e2f261db56fea2",
  ]],
].flatMap(([path, digests]) => digests.map((digest) => [`${path}:${digest}`, 1])));

function rustSources(dir) {
  const pending = [dir];
  const files = [];
  while (pending.length) {
    const current = pending.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile() && entry.name.endsWith(".rs")) files.push(path);
    }
  }
  return files;
}

function firstSqlLiteral(after, kind) {
  const open = after.match(/^\s*\(\s*/);
  if (!open) return null;
  let rest = after.slice(open[0].length);
  if (kind === "query_as") {
    const rowType = rest.match(/^[A-Za-z_][A-Za-z0-9_:<>]*\s*,/);
    if (!rowType) return null;
    rest = rest.slice(rowType[0].length);
  }
  for (;;) {
    rest = rest.trimStart();
    if (rest.startsWith("//")) {
      const end = rest.indexOf("\n");
      if (end < 0) return null;
      rest = rest.slice(end + 1);
    } else if (rest.startsWith("/*")) {
      const end = rest.indexOf("*/");
      if (end < 0) return null;
      rest = rest.slice(end + 2);
    } else {
      return rest.match(RUST_STRING);
    }
  }
}

export function findSqlCalls(source, path) {
  const calls = [];
  for (const match of source.matchAll(IMPORT)) {
    if (match[2] || QUERY_NAME.test(match[1])) {
      calls.push({ path, line: source.slice(0, match.index).split("\n").length, kind: "import" });
    }
  }
  for (const match of source.matchAll(QUERY)) {
    const line = source.slice(0, match.index).split("\n").length;
    if (!match[2] || match[1] === "raw_sql" || match[1] === "QueryBuilder") {
      calls.push({ path, line, kind: "dynamic" });
      continue;
    }
    const after = source.slice(match.index + match[0].length, match.index + match[0].length + 16_384);
    const literal = firstSqlLiteral(after, match[1]);
    if (!literal) {
      calls.push({ path, line, kind: "unparsed" });
      continue;
    }
    const sql = literal[2] ?? literal[3] ?? literal[4];
    const digest = createHash("sha256").update(`${match[1]}:${sql}`).digest("hex");
    calls.push({ path, line, kind: "macro", digest });
  }
  return calls;
}

export function checkProductSql(root, legacy = LEGACY) {
  const found = new Map();
  const errors = [];
  const cliMain = join(root, "crates", "synveda-cli", "src", "main.rs");
  if (readdirSync(join(root, "crates")).includes("synveda-cli") &&
      !/#\[cfg\(test\)\]\s*mod deployment_database;/.test(readFileSync(cliMain, "utf8"))) {
    errors.push("CLI deployment_database exemption requires a test-only module");
  }
  for (const crate of readdirSync(join(root, "crates"), { withFileTypes: true })) {
    if (!crate.isDirectory() || crate.name === "synveda-store" || crate.name === "synveda-apalis") continue;
    const src = join(root, "crates", crate.name, "src");
    for (const absolute of rustSources(src)) {
      const path = relative(root, absolute).replaceAll("\\", "/");
      if (path === "crates/synveda-cli/src/deployment_database.rs") continue;
      const source = readFileSync(absolute, "utf8");
      const testModule = source.search(/#\[cfg\(test\)\]\s*mod tests\s*\{/);
      const production = testModule < 0 ? source : source.slice(0, testModule);
      for (const call of findSqlCalls(production, path)) {
        if (call.kind !== "macro") {
          errors.push(`${path}:${call.line}: ${call.kind} SQLx outside synveda-store`);
          continue;
        }
        const key = `${path}:${call.digest}`;
        found.set(key, (found.get(key) ?? 0) + 1);
      }
    }
  }
  for (const [key, count] of found) {
    const allowed = legacy.get(key) ?? 0;
    if (count > allowed) errors.push(`${key}: ${count} SQLx call(s), ${allowed} grandfathered`);
  }
  for (const [key, allowed] of legacy) {
    const count = found.get(key) ?? 0;
    if (count < allowed) errors.push(`${key}: legacy count fell from ${allowed} to ${count}; reduce the inventory`);
  }
  return { errors, calls: [...found.values()].reduce((sum, count) => sum + count, 0) };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = checkProductSql(ROOT);
  if (result.errors.length) {
    for (const error of result.errors) console.error(`FAIL: ${error}`);
    process.exitCode = 1;
  } else {
    console.log(`product SQL placement holds: ${result.calls} grandfathered SQLx macros; new product SQL stays in synveda-store`);
  }
}
