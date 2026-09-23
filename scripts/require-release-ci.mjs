#!/usr/bin/env node
// OPS-8: a tag may release only an exact commit validated by full, trusted CI.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";

export function validatedRun(runs, repository, source) {
  const matching = runs
    .filter(
      (run) =>
        run.head_sha === source &&
        run.event === "push" &&
        run.head_branch === "main" &&
        run.head_repository?.full_name === repository &&
        run.path === ".github/workflows/ci.yml",
    )
    .sort((a, b) => b.id - a.id);
  const run = matching[0];
  assert.ok(
    run?.status === "completed" && run.conclusion === "success",
    "latest full CI main-push run for the exact source must succeed",
  );
  return run;
}
export function requireResult(jobs) {
  const gates = jobs.filter((job) => job.name === "CI Result");
  assert.ok(
    gates.length === 1 && gates[0].conclusion === "success",
    "missing or unsuccessful CI Result for the exact source",
  );
  assert.ok(
    jobs.every((job) => job.conclusion === "success"),
    "full main CI contains a failed, cancelled or skipped job",
  );
}
if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const { GITHUB_REPOSITORY: repository, GITHUB_SHA: source } = process.env;
  assert.match(repository ?? "", /^[\w.-]+\/[\w.-]+$/);
  assert.match(source ?? "", /^[a-f0-9]{40}$/);
  execFileSync("git", ["merge-base", "--is-ancestor", source, "origin/main"]);
  const api = (endpoint) =>
    JSON.parse(
      execFileSync("gh", ["api", "--paginate", "--slurp", endpoint], {
        encoding: "utf8",
        timeout: 60000,
      }),
    );
  const runs = api(
    `repos/${repository}/actions/workflows/ci.yml/runs?event=push&branch=main&head_sha=${source}&per_page=100`,
  ).flatMap((page) => page.workflow_runs);
  const run = validatedRun(runs, repository, source);
  const jobs = api(
    `repos/${repository}/actions/runs/${run.id}/jobs?filter=latest&per_page=100`,
  ).flatMap((page) => page.jobs);
  requireResult(jobs);
  const evidence = `Validated source ${source}: ${run.html_url}\n`;
  console.log(evidence);
  if (process.env.GITHUB_STEP_SUMMARY)
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, evidence);
}
