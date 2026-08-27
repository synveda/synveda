/** Pure acceptance for browser-side OKF envelopes and progress wording (CPR-28). */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  classificationCounts,
  importBody,
  importProgress,
  normaliseLogicalPath,
  type UploadFile,
} from "./okf.mjs";
import { describe as describeRequest } from "./client.mjs";
import type { OkfImportJobView, OkfMappingView } from "./generated/api.js";

function upload(name: string, content: string, relative = ""): UploadFile {
  const bytes = new TextEncoder().encode(content);
  return {
    name,
    size: bytes.length,
    webkitRelativePath: relative,
    arrayBuffer: async () => bytes.buffer,
  };
}

function mapping(classification: string): OkfMappingView {
  return {
    id: `mapping-${classification}`,
    artifact_id: `artifact-${classification}`,
    ordinal: 1,
    okf_type: "pulseboard-extension",
    knowledge_type: "reference",
    content: {
      title: classification,
      body_markdown: classification,
      summary: classification,
      tags: [],
      sensitivity: "internal",
      confidence_permille: 800,
      metadata: { okf: { extensions: { "x-owner": "platform" } } },
    },
    content_hash: classification,
    classification,
    proposed_relations: {},
    materializable: classification !== "duplicate",
    content_erased: false,
  };
}

test("classification counts keep additions updates duplicates and conflicts separate", () => {
  const counts = classificationCounts([
    mapping("addition"),
    mapping("addition"),
    mapping("update"),
    mapping("duplicate"),
    mapping("conflict"),
  ]);
  assert.deepEqual(counts, { addition: 2, update: 1, duplicate: 1, conflict: 1 });
});

test("directory files become inert Git entries and omit administration data", async () => {
  const body = await importBody(
    [
      upload(
        "decision.md",
        "---\ntype: pulseboard-extension\nx-owner: platform\n---\nUse traceparent.\n",
        "bundle/decision.md",
      ),
      upload("config", "credential=never-send", "bundle/.git/config"),
    ],
    "pulseboard-okf",
    "abc123",
  );
  assert.equal(body.source_kind, "git");
  assert.equal(body.source_revision, "abc123");
  assert.equal(body.encoding, "entries");
  assert.equal(body.entries?.length, 1);
  assert.equal(body.entries?.[0]?.logical_path, "bundle/decision.md");
  const decoded = atob(body.entries?.[0]?.content_base64 ?? "");
  assert.match(decoded, /pulseboard-extension/);
  assert.match(decoded, /x-owner/);
  assert.doesNotMatch(JSON.stringify(body), /never-send/);
});

test("one archive is identified without pretending it is a directory", async () => {
  const body = await importBody([upload("knowledge.tar.gz", "archive-bytes")], "release-42", "");
  assert.equal(body.source_kind, "tar");
  assert.equal(body.encoding, "tar_gzip");
  assert.deepEqual(body.entries, []);
  assert.equal(atob(body.archive_base64 ?? ""), "archive-bytes");
});

test("browser paths fail closed before they become filesystem-shaped server input", async () => {
  assert.throws(() => normaliseLogicalPath("../secret.md"), /Unsafe OKF path/);
  assert.throws(() => normaliseLogicalPath("a\\b.md"), /Unsafe OKF path/);
  await assert.rejects(
    importBody([upload("secret.md", "x", "../secret.md")], "source", ""),
    /Unsafe OKF path/,
  );
  await assert.rejects(
    importBody([upload("bundle.zip", "x")], "source", "revision"),
    /applies only to checked-out directory/,
  );
  const oversized = upload("large.md", "not-read");
  oversized.size = 262_145;
  await assert.rejects(importBody([oversized], "source", ""), /exceeds 262144 bytes/);
  const archive = upload("large.zip", "not-read");
  archive.size = 1_500_001;
  await assert.rejects(importBody([archive], "source", ""), /exceeds 1500000 bytes/);
});

test("progress never describes a dry-run as publication", () => {
  const base = {
    id: "job-1",
    project_id: "project-1",
    format: "okf",
    format_version: "0.2",
    specification_commit: "ad30107",
    source_kind: "directory",
    source_locator: "pulseboard",
    bundle_digest: "digest",
    state: "planned",
    artifact_count: 2,
    mapping_count: 2,
    candidate_count: 0,
    notices: [],
    created_at: "2026-08-25T12:00:00Z",
  } as OkfImportJobView;
  assert.match(importProgress({ ...base, state: "planned" }), /no candidates created/);
  assert.match(
    importProgress({ ...base, state: "materialized", candidate_count: 2 }),
    /2 reviewable/,
  );
});

test("all exchange acts use the generated project contract and exact retry headers", () => {
  const plan = describeRequest("plan_okf_import", {
    path: { project_id: "project-1" },
    body: {
      source_kind: "directory",
      source_locator: "pulseboard",
      encoding: "entries",
      entries: [],
    },
    idempotencyKey: "plan-key",
  });
  const materialize = describeRequest("materialize_okf_import", {
    path: { id: "job-1" },
    idempotencyKey: "materialize-key",
  });
  const exportBundle = describeRequest("export_okf", {
    path: { project_id: "project-1" },
    body: { item_ids: [] },
  });

  assert.deepEqual(
    [plan, materialize, exportBundle].map((operation) => [operation.path, operation.init.method]),
    [
      ["/projects/project-1/okf/imports", "POST"],
      ["/okf/imports/job-1/materialize", "POST"],
      ["/projects/project-1/okf/exports", "POST"],
    ],
  );
  assert.equal(new Headers(plan.init.headers).get("idempotency-key"), "plan-key");
  assert.equal(
    new Headers(materialize.init.headers).get("idempotency-key"),
    "materialize-key",
  );
  assert.equal(new Headers(exportBundle.init.headers).has("idempotency-key"), false);
});
