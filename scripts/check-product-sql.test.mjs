import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { checkProductSql, findSqlCalls } from "./check-product-sql.mjs";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "synveda-product-sql-"));
  for (const crate of ["synveda-audit", "synveda-store", "synveda-apalis"]) {
    mkdirSync(join(root, "crates", crate, "src"), { recursive: true });
  }
  return root;
}

test("pins legacy SQL text, path and multiplicity while allowing store SQL", () => {
  const root = fixture();
  const path = "crates/synveda-audit/src/chain.rs";
  const original = 'sqlx::query!("select seq from audit_log")';
  const call = findSqlCalls(original, path)[0];
  const legacy = new Map([[`${path}:${call.digest}`, 1]]);
  try {
    writeFileSync(join(root, path), original);
    writeFileSync(join(root, "crates/synveda-store/src/new.rs"), 'sqlx::query!("select id from scopes")');
    writeFileSync(join(root, "crates/synveda-apalis/src/queue.rs"), 'sqlx::query("select id from apalis_jobs")');
    assert.deepEqual(checkProductSql(root, legacy).errors, []);

    writeFileSync(join(root, path), `${original}\n${original}`);
    assert.match(checkProductSql(root, legacy).errors.join("\n"), /2 SQLx call\(s\), 1 grandfathered/);

    writeFileSync(join(root, path), 'sqlx::query!("select payload from audit_log")');
    const changed = checkProductSql(root, legacy).errors.join("\n");
    assert.match(changed, /0 grandfathered/);
    assert.match(changed, /legacy count fell from 1 to 0/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects dynamic SQL and query imports outside store", () => {
  const root = fixture();
  try {
    writeFileSync(
      join(root, "crates/synveda-audit/src/chain.rs"),
      'use sqlx::{query};\nlet _ = sqlx::query("select 1");',
    );
    const errors = checkProductSql(root, new Map()).errors.join("\n");
    assert.match(errors, /import SQLx outside synveda-store/);
    assert.match(errors, /dynamic SQLx outside synveda-store/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("reads a raw SQL literal after comments and rejects a nonliteral macro", () => {
  const path = "crates/synveda-vedaflow/src/refs.rs";
  const source = 'sqlx::query!(\n // nullable join\n r#"select id as "id?" from refs"#\n)';
  assert.equal(findSqlCalls(source, path)[0].kind, "macro");
  assert.equal(findSqlCalls("sqlx::query!(SQL_CONSTANT)", path)[0].kind, "unparsed");
});

test("CLI deployment fixture remains excluded only while test-gated", () => {
  const root = fixture();
  const cli = join(root, "crates/synveda-cli/src");
  mkdirSync(cli, { recursive: true });
  writeFileSync(join(cli, "deployment_database.rs"), 'sqlx::query!("select 1")');
  try {
    writeFileSync(join(cli, "main.rs"), "#[cfg(test)]\nmod deployment_database;");
    assert.deepEqual(checkProductSql(root, new Map()).errors, []);
    writeFileSync(join(cli, "main.rs"), "mod deployment_database;");
    assert.match(checkProductSql(root, new Map()).errors.join("\n"), /test-only module/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
