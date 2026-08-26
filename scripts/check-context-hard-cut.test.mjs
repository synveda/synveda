import assert from "node:assert/strict";
import test from "node:test";

import {
  baselineFindings,
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

test("the baseline admits only the epoch-three extension shape", () => {
  assert.deepEqual(
    baselineFindings(`
CREATE EXTENSION IF NOT EXISTS btree_gin WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;
CREATE TABLE knowledge_items (id uuid primary key);
`),
    [],
  );
  assert.match(
    baselineFindings(`
CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS pgmq WITH SCHEMA public;
CREATE TABLE records (id uuid primary key);
`).join("\n"),
    /retired table|baseline extensions/u,
  );
});
