/** Local Git fixtures exercise the real Git CLI without network or remotes. */

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { observeGit } from "./git.mjs";

test("checkout observation handles worktrees, detached and unborn HEAD, nested repos, and no Git", () => {
  const fixture = mkdtempSync(join(tmpdir(), "synveda-git-observe-"));
  const main = join(fixture, "main");
  const worktree = join(fixture, "worktree");
  const nested = join(main, "nested");
  const git = (cwd: string, ...args: string[]) =>
    execFileSync("git", ["-C", cwd, ...args], { stdio: "pipe", timeout: 5000 });
  try {
    execFileSync("git", ["init", "-b", "main", main], { stdio: "pipe", timeout: 5000 });
    const unborn = observeGit(main, "installation-1");
    assert.equal(unborn?.branch, "main");
    assert.equal(unborn?.commit, undefined);
    assert.equal(unborn?.dirty, false);

    writeFileSync(join(main, "README"), "one\n");
    git(main, "add", "README");
    git(main, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "commit", "-m", "initial");
    git(main, "worktree", "add", "-b", "parallel", worktree);
    git(main, "remote", "add", "origin", "https://token:topsecret@example.test/acme/repo.git");
    const first = observeGit(main, "installation-1");
    const second = observeGit(worktree, "installation-1");
    assert.ok(first?.commit);
    assert.equal(second?.commit, first.commit);
    assert.notEqual(second?.ref, first.ref);
    assert.equal(second?.branch, "parallel");
    assert.equal(first.dirty, false);
    assert.equal(JSON.stringify(first).includes("topsecret"), false);

    writeFileSync(join(worktree, "untracked"), "unsaved\n");
    assert.equal(observeGit(worktree, "installation-1")?.dirty, true);
    git(main, "checkout", "--detach", "HEAD");
    assert.equal(observeGit(main, "installation-1")?.branch, undefined);

    mkdirSync(nested);
    execFileSync("git", ["init", "-b", "nested", nested], { stdio: "pipe", timeout: 5000 });
    assert.notEqual(observeGit(nested, "installation-1")?.ref, first.ref);
    assert.equal(observeGit(fixture, "installation-1"), undefined);
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});
