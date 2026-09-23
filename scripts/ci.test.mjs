import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { changedPaths, selectChanges, selectForEvent, stages } from "./ci-select.mjs";
import { ciResult, requiredJobs } from "./ci-result.mjs";

const all = () => selectChanges(undefined);
function evidence(selected = all()) {
  return {
    changes: {
      result: "success",
      outputs: Object.fromEntries(
        Object.entries(selected).map(([key, value]) => [key, String(value)]),
      ),
    },
    check: { result: "success" },
    licences: { result: "success" },
    ...Object.fromEntries(
      Object.entries(requiredJobs).map(([job, stage]) => [
        job,
        { result: selected[stage] ? "success" : "skipped" },
      ]),
    ),
  };
}
test("conservative selection covers shared dependencies, unknowns and explicit full runs", () => {
  for (const path of [
    "Cargo.lock",
    "pnpm-lock.yaml",
    "crates/synveda-types/src/lib.rs",
    ".sqlx/query-old.json",
    "policies/base.cedar",
    ".github/workflows/ci.yml",
    "scripts/install.sh",
    "sdks/python/requirements-dev.lock",
    "adapters/registry.json",
    "deploy/helm/synveda/Chart.lock",
    "demos/fixtures/new.json",
    "new-directory/file",
  ])
    assert.deepEqual(selectChanges([path]), all(), path);
  assert.ok(
    Object.values(
      selectChanges(["docs/backlog/OPS-12.md", "docs/adr/adr-0108.md"]),
    ).every((value) => !value),
  );
  assert.equal(selectChanges(["website/styles.css"]).typescript, true);
  assert.equal(selectChanges(["website/styles.css"]).docker, false);
  for (const stage of ["rust", "typescript", "deploy", "docker", "helm"])
    assert.equal(selectChanges(["console/src/app.tsx"])[stage], true);
  assert.deepEqual(
    selectChanges(["docs/backlog/OPS-12.md", "scripts/install.sh"]),
    all(),
  );
  assert.deepEqual(selectChanges([]), all());
  assert.deepEqual(selectChanges(["docs/CI.md"], true), all());
});
test("PRs defer native Docker qualification while main, merge queue and dispatch require it", () => {
  for (const paths of [
    undefined,
    [],
    ["Cargo.lock"],
    ["console/src/app.tsx"],
    ["new-directory/file"],
  ]) {
    const selected = selectForEvent(paths, "pull_request");
    assert.equal(selected.docker, false);
    assert.equal(selected.deploy, true);
    assert.equal(selected.helm, true);
    assert.deepEqual(ciResult(evidence(selected)), []);
    const unexpectedlySkipped = evidence(selected);
    unexpectedlySkipped.helm.result = "skipped";
    assert.ok(ciResult(unexpectedlySkipped).length);
  }
  for (const event of ["push", "merge_group", "workflow_dispatch"])
    assert.deepEqual(selectForEvent(["docs/CI.md"], event), all());
});
test("diff covers deletions, both rename paths and more than GitHub's 300-path limit", () => {
  const directory = mkdtempSync(join(tmpdir(), "synveda-ci-diff-"));
  const run = (command, args, options) => execFileSync(command, args, { ...options, cwd: directory });
  const git = (...args) => run("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  try {
    git("init", "--quiet");
    git("config", "user.name", "CI fixture");
    git("config", "user.email", "ci@example.invalid");
    git("config", "commit.gpgsign", "false");
    mkdirSync(join(directory, "empty-hooks"));
    git("config", "core.hooksPath", join(directory, "empty-hooks"));
    writeFileSync(join(directory, "Cargo.lock"), "deleted shared input\n");
    writeFileSync(join(directory, "shared-config"), "renamed shared input\n");
    git("add", "."); git("commit", "--quiet", "-m", "fixture base");
    const base = git("rev-parse", "HEAD").trim();
    mkdirSync(join(directory, "docs/backlog"), { recursive: true });
    for (let i = 0; i < 350; i++) writeFileSync(join(directory, `docs/backlog/test-${i}.md`), "prose\n");
    renameSync(join(directory, "shared-config"), join(directory, "docs/backlog/renamed file.md"));
    rmSync(join(directory, "Cargo.lock"));
    git("add", "."); git("commit", "--quiet", "-m", "fixture change");
    const paths = changedPaths(base, git("rev-parse", "HEAD").trim(), run);
    assert.equal(paths.length, 353);
    for (const path of ["Cargo.lock", "shared-config", "docs/backlog/renamed file.md"]) assert.ok(paths.includes(path));
    assert.deepEqual(selectChanges(paths), all());
    assert.throws(() => changedPaths("", "b".repeat(40)));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
test("only declared skips pass; failed, cancelled, missing and unknown results block", () => {
  assert.deepEqual(ciResult(evidence()), []);
  assert.deepEqual(ciResult(evidence(selectChanges(["docs/CI.md"]))), []);
  for (const job of Object.keys(evidence()))
    for (const result of ["failure", "cancelled", "skipped", undefined]) {
      const needs = evidence();
      needs[job].result = result;
      assert.ok(ciResult(needs).length, `${job}: ${result}`);
    }
  for (const stage of stages) {
    const needs = evidence();
    delete needs.changes.outputs[stage];
    assert.ok(ciResult(needs).length);
  }
  const missing = evidence();
  delete missing.cli;
  assert.ok(ciResult(missing).length);
});
test("an intentionally failing required test produces a nonzero CI Result", () => {
  const dir = mkdtempSync(join(tmpdir(), "synveda-ci-failure-"));
  try {
    const fixture = join(dir, "required.test.mjs");
    writeFileSync(
      fixture,
      'import test from "node:test"; test("intentional required failure", () => { throw new Error("injected failure"); });\n',
    );
    const env = { ...process.env };
    delete env.NODE_TEST_CONTEXT;
    const failed = spawnSync(process.execPath, ["--test", fixture], { env });
    assert.equal(failed.status, 1);
    const needs = evidence();
    needs["build-test"].result = failed.status === 0 ? "success" : "failure";
    const gate = spawnSync(process.execPath, ["scripts/ci-result.mjs"], {
      encoding: "utf8",
      env: { ...process.env, CI_NEEDS: JSON.stringify(needs) },
    });
    assert.equal(gate.status, 1);
    assert.match(gate.stderr, /build-test: required=true, result=failure/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
