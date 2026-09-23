#!/usr/bin/env node
// OPS-8: incomplete uploads remain drafts and cannot become a stable release.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { regularAssets, releaseAssets } from "./check-release-assets.mjs";

export function uploadedAssets(release, expected) {
  assert.equal(
    release.draft,
    true,
    "release must remain a draft until all uploads are verified",
  );
  assert.deepEqual(
    release.assets
      .map(({ name, size, state }) => ({ name, size, state }))
      .sort((a, b) => a.name.localeCompare(b.name)),
    expected
      .map((entry) => ({ ...entry, state: "uploaded" }))
      .sort((a, b) => a.name.localeCompare(b.name)),
    "uploaded release asset set differs",
  );
}
export function publishRelease(
  directory,
  version,
  source,
  repository,
  namespace,
  run = execFileSync,
) {
  run("sh", ["scripts/release-version.sh", version]);
  assert.match(source, /^[a-f0-9]{40}$/);
  const tag = `v${version}`;
  const names = [
    ...releaseAssets(version, true),
    "SHA256SUMS",
    "SHA256SUMS.sigstore.json",
  ];
  const expected = regularAssets(directory, names);
  run("sha256sum", ["--check", "SHA256SUMS"], {
    cwd: directory,
    stdio: "inherit",
  });
  // --target does not move an existing tag. Resolve it again immediately before
  // creation so a moved tag cannot announce another commit's tested artifacts.
  const commit = JSON.parse(
    run("gh", ["api", `repos/${repository}/commits/${tag}`], {
      encoding: "utf8",
    }),
  );
  assert.equal(commit.sha, source, "release tag moved after source validation");
  const template = readFileSync(
    new URL("./release-notes.md", import.meta.url),
    "utf8",
  );
  const notes = template
    .replaceAll("{{version}}", version)
    .replaceAll("{{tag}}", tag)
    .replaceAll("{{repository}}", repository)
    .replaceAll("{{namespace}}", namespace);
  const notesFile = join(directory, "release-notes.md");
  writeFileSync(notesFile, notes);
  run(
    "gh",
    [
      "release",
      "create",
      tag,
      "--draft",
      "--verify-tag",
      "--target",
      source,
      "--title",
      `Synveda ${tag}`,
      "--notes-file",
      notesFile,
      ...names.map((name) => join(directory, name)),
    ],
    { stdio: "inherit" },
  );
  const release = JSON.parse(
    run("gh", ["api", `repos/${repository}/releases/tags/${tag}`], {
      encoding: "utf8",
    }),
  );
  assert.ok(Number.isSafeInteger(release.id) && release.id > 0);
  // The expected set is smaller than one explicit page; an extra asset makes
  // the exact comparison fail, even if a later page exists.
  release.assets = JSON.parse(
    run(
      "gh",
      ["api", `repos/${repository}/releases/${release.id}/assets?per_page=100`],
      { encoding: "utf8" },
    ),
  );
  uploadedAssets(release, expected);
  run("gh", ["release", "edit", tag, "--draft=false"], { stdio: "inherit" });
}
if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const [directory, version] = process.argv.slice(2);
  publishRelease(
    directory,
    version,
    process.env.GITHUB_SHA,
    process.env.GH_REPO,
    process.env.DOCKERHUB_NAMESPACE,
  );
}
