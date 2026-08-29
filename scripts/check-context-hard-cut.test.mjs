import assert from "node:assert/strict";
import test from "node:test";

import {
  baselineFindings,
  databaseBootstrapFindings,
  retiredProductionFindings,
} from "./check-context-hard-cut.mjs";

test("a deliberately reintroduced route DTO table and sidecar fail the gate", () => {
  const findings = retiredProductionFindings(`
const path = "/v1/observe";
struct RecallSweepRequest;
select * from record_embeddings;
let index = SearchIndex::open("tantivy");
`);
  assert.equal(findings.length, 5, findings.join("\n"));
  assert.match(findings.join("\n"), /global runtime route/u);
  assert.match(findings.join("\n"), /retired aggregate or adapter DTO/u);
  assert.match(findings.join("\n"), /retired table/u);
  assert.match(findings.join("\n"), /retired runtime dependency/u);
});

test("new-contract defaults and revision comparisons are not compatibility residue", () => {
  assert.deepEqual(
    retiredProductionFindings(`
const fallback = "hash_embedder";
let old_revision = current_revision;
select * from idempotency_records;
const format = "okf-v0.2";
`),
    [],
  );
});

test("the baseline refuses deployment-owned role and extension DDL", () => {
  assert.deepEqual(
    baselineFindings(`
CREATE TABLE knowledge_items (id uuid primary key);
`),
    [],
  );
  assert.match(
    baselineFindings(`
CREATE ROLE synveda_app NOLOGIN;
CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;
CREATE TABLE records (id uuid primary key);
`).join("\n"),
    /retired table|deployment-owned/u,
  );
  for (const ddl of [
    "ALTER EXTENSION vector UPDATE;",
    "DROP EXTENSION vector;",
    "ALTER ROLE synveda_app SUPERUSER;",
    "DROP USER synveda_app;",
  ]) {
    assert.match(baselineFindings(ddl).join("\n"), /deployment-owned/u);
  }
});

test("deployment owns one exact versioned and identifier-safe extension loop", () => {
  const bootstrap = `
select format('create role synveda_app nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1') where true \\gexec
select format('create role synveda_migrator nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1') where true \\gexec
select format('create role synveda_gateway nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1') where true \\gexec
select format('create role synveda_worker nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1') where true \\gexec
select format('create role keycloak nologin inherit nosuperuser nocreatedb nocreaterole noreplication nobypassrls connection limit -1') where true \\gexec
select format(
  'create extension %I with schema public version %L',
  required.name,
  required.version
)
from (values ('btree_gin', '1.3'), ('vector', '0.8.6')) as required(name, version)
where not exists (
  select 1 from pg_catalog.pg_extension extension
  where extension.extname = required.name
)
\\gexec
`;
  assert.deepEqual(databaseBootstrapFindings(bootstrap), []);
  for (const mutation of [
    bootstrap.replace("%I", "%s"),
    bootstrap.replace("version %L", ""),
    bootstrap.replace("('vector', '0.8.6')", "('pgmq', '1.5.0')"),
    `${bootstrap}\nselect 'create extension vector';`,
  ]) {
    assert.match(databaseBootstrapFindings(mutation).join("\n"), /version-pinned/u);
  }
  for (const mutation of [
    bootstrap.replace("synveda_app nologin", "synveda_app superuser"),
    `${bootstrap}\ncreate role extra superuser;`,
    `${bootstrap}\nalter role synveda_gateway bypassrls;`,
    `${bootstrap}\ndrop role synveda_worker;`,
    `${bootstrap}\nalter extension vector update;`,
  ]) {
    assert.notDeepEqual(databaseBootstrapFindings(mutation), []);
  }
});
