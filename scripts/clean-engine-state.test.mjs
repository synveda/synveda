#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  lstatSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  readdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";

const stateTool = resolve("deploy/compose/scripts/clean-engine-state.mjs");

function command(binary, args, options = {}) {
  return spawnSync(binary, args, {
    encoding: "utf8",
    env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
    ...options,
  });
}

function git(repo, args) {
  const result = command("git", ["-C", repo, ...args]);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

function fixture() {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "synveda-clean-engine-state-")));
  chmodSync(root, 0o700);
  const repo = join(root, "repo");
  const state = join(root, "state");
  mkdirSync(repo, { mode: 0o700 });
  mkdirSync(state, { mode: 0o700 });
  const files = {
    ".dockerignore": `
.git
.git/**
.agents
.agents/**
.codex
.codex/**
target
target/**
**/target
**/target/**
node_modules
node_modules/**
**/node_modules
**/node_modules/**
.pnpm-store
.pnpm-store/**
**/dist
**/dist/**
**/dist-test
**/dist-test/**
data
data/**
.env
.env.*
**/.env
**/.env.*
!**/.env.example
deploy/compose/secrets
deploy/compose/secrets/**
deploy/compose/runtime
deploy/compose/runtime/**
deploy/compose/backups
deploy/compose/backups/**
.DS_Store
**/.DS_Store
Thumbs.db
**/Thumbs.db
*.log
**/*.log
*.tmp
**/*.tmp
`.trimStart(),
    ".gitignore": ".codex/\ntarget/\nnode_modules/\n",
    ".env.example": "NON_SECRET_EXAMPLE=true\n",
    ".env.secret.example": "excluded example-shaped residue\n",
    Makefile: "compose-config:\n\t@true\n",
    "deploy/compose/compose.yaml": "name: fixture\nservices: {}\n",
    "docs/DEPLOYMENT_CONTRACT.md": "# Fixture deployment contract\n",
    "docs/SECURITY.md": "# Fixture security contract\n",
    "source.txt": "fixture source\n",
  };
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(repo, path)), { recursive: true, mode: 0o700 });
    writeFileSync(join(repo, path), content, { mode: 0o600 });
  }
  git(repo, ["init", "-q"]);
  git(repo, ["add", "."]);
  git(repo, [
    "-c",
    "user.name=Synveda Test",
    "-c",
    "user.email=synveda-test@example.invalid",
    "commit",
    "-q",
    "-m",
    "fixture",
  ]);
  return { root, repo, state };
}

function run(state, action, extra = []) {
  return command(process.execPath, toolArgs(state, action, extra));
}

function toolArgs(state, action, extra = []) {
  const args = [
    stateTool,
    action,
    "--repo-root",
    state.repo,
    "--state-base",
    state.state,
  ];
  if (action === "plan") {
    args.push(
      "--ipv4-pool",
      "10.239.17.0/24",
      "--provider",
      "colima",
    );
  }
  args.push(...extra);
  return args;
}

function parse(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function assertPrivate(path, mode, directory = false, links = 1) {
  const metadata = lstatSync(path);
  assert.equal(metadata.isSymbolicLink(), false);
  assert.equal(directory ? metadata.isDirectory() : metadata.isFile(), true);
  if (!directory) assert.equal(metadata.nlink, links);
  assert.equal(metadata.mode & 0o777, mode);
  assert.equal(metadata.uid, process.getuid());
}

function activeRun(state) {
  const receipt = parse(join(state.state, "active"));
  return join(state.state, `.run-${receipt.fixture_id}`);
}

test("plan publishes one canonical content-free candidate, receipt and proxy template", () => {
  const state = fixture();
  try {
    mkdirSync(join(state.repo, ".codex"), { mode: 0o700 });
    writeFileSync(join(state.repo, ".codex", "journal.md"), "ignored journal\n", { mode: 0o600 });
    const result = run(state, "plan");
    assert.equal(result.status, 0, result.stderr);
    assert.match(
      result.stdout,
      /^clean-engine: plan [0-9a-f]{32} prepared for synveda-development-acceptance-[0-9a-f]{24}\n$/,
    );
    assert.equal(result.stderr, "");

    const active = activeRun(state);
    const candidatePath = join(active, "candidate.json");
    const receiptPath = join(active, "00-plan.json");
    const proxyPath = join(active, "client", "proxy-template.json");
    assertPrivate(active, 0o700, true);
    assertPrivate(candidatePath, 0o600);
    assertPrivate(receiptPath, 0o600, false, 2);
    assertPrivate(join(state.state, "active"), 0o600, false, 2);
    assert.equal(lstatSync(receiptPath).ino, lstatSync(join(state.state, "active")).ino);
    const exactRun = lstatSync(active, { bigint: true });
    assertPrivate(proxyPath, 0o600);

    const candidateRaw = readFileSync(candidatePath, "utf8");
    const receiptRaw = readFileSync(receiptPath, "utf8");
    const proxyRaw = readFileSync(proxyPath, "utf8");
    for (const raw of [candidateRaw, receiptRaw]) {
      assert.equal(raw.endsWith("\n"), true);
      assert.doesNotMatch(
        raw,
        /password|private[_ -]?key|authorization|bearer|cookie|token|database_url|Users\/|home\//i,
      );
    }
    const candidate = JSON.parse(candidateRaw);
    const receipt = JSON.parse(receiptRaw);
    assert.equal(candidate.run_id, receipt.fixture_id);
    assert.equal(candidate.selection.project_suffix, `acceptance-${candidate.run_id.slice(0, 24)}`);
    assert.equal(candidate.selection.ipv4_pool, "10.239.17.0/24");
    assert.equal(candidate.fixtures.registry_transport, "loopback-tls-ephemeral");
    assert.equal(candidate.source.worktree_clean, true);
    assert.match(candidate.source.build_context_manifest_sha256, /^[0-9a-f]{64}$/);
    assert.match(candidate.source.tracked_index_manifest_sha256, /^[0-9a-f]{64}$/);
    assert.equal(receipt.result.state_device, String(exactRun.dev));
    assert.equal(receipt.result.state_inode, String(exactRun.ino));
    assert.deepEqual(JSON.parse(proxyRaw), {
      auths: {},
      proxies: {
        default: {
          allProxy: "socks5://all-proxy-canary.invalid:65535",
          ftpProxy: "http://ftp-proxy-canary.invalid:65535",
          httpProxy: "http://http-proxy-canary.invalid:65535",
          httpsProxy: "http://https-proxy-canary.invalid:65535",
          noProxy: "proxy-bypass-canary.invalid",
        },
      },
    });

    const status = run(state, "status");
    assert.equal(status.status, 0, status.stderr);
    assert.match(status.stdout, / is prepared\n$/);
    const verify = run(state, "verify");
    assert.equal(verify.status, 0, verify.stderr);
    assert.match(verify.stdout, / is source-verified\n$/);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("one active plan is a durable lease and cannot be replaced", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const original = readFileSync(join(activeRun(state), "candidate.json"));
    const second = run(state, "plan");
    assert.equal(second.status, 73);
    assert.equal(second.stdout, "");
    assert.equal(second.stderr, "clean-engine: an active clean-engine plan already exists\n");
    assert.deepEqual(readFileSync(join(activeRun(state), "candidate.json")), original);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("dirty tracked, untracked and ignored build-context inputs leave no active receipt", () => {
  const mutations = [
    (state) => writeFileSync(join(state.repo, "source.txt"), "dirty tracked\n"),
    (state) => writeFileSync(join(state.repo, "untracked.txt"), "dirty untracked\n"),
    (state) => {
      git(state.repo, ["update-index", "--assume-unchanged", "source.txt"]);
      writeFileSync(join(state.repo, "source.txt"), "hidden tracked drift\n");
    },
    (state) => {
      git(state.repo, ["update-index", "--skip-worktree", "source.txt"]);
      writeFileSync(join(state.repo, "source.txt"), "hidden skip-worktree drift\n");
    },
    (state) => {
      writeFileSync(join(state.repo, ".gitignore"), ".codex/\ntarget/\nnode_modules/\n.idea/\n");
      git(state.repo, ["add", ".gitignore"]);
      git(state.repo, [
        "-c",
        "user.name=Synveda Test",
        "-c",
        "user.email=synveda-test@example.invalid",
        "commit",
        "-q",
        "-m",
        "ignore mutation",
      ]);
      mkdirSync(join(state.repo, ".idea"), { mode: 0o700 });
      writeFileSync(join(state.repo, ".idea", "leak"), "ignored context leak\n");
    },
  ];
  for (const mutate of mutations) {
    const state = fixture();
    try {
      mutate(state);
      const result = run(state, "plan");
      assert.equal(result.status, 78);
      assert.equal(result.stdout, "");
      assert.match(
        result.stderr,
        /^clean-engine: (?:source worktree|source index|ignored source input)/,
      );
      assert.equal(lstatSync(state.state).isDirectory(), true);
      assert.throws(() => lstatSync(join(state.state, "active")), { code: "ENOENT" });
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("verification detects source drift while status remains content-free and resumable", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    writeFileSync(join(state.repo, "source.txt"), "source drift\n");
    const verify = run(state, "verify");
    assert.equal(verify.status, 78);
    assert.equal(verify.stdout, "");
    assert.equal(verify.stderr, "clean-engine: source worktree is not clean\n");
    const status = run(state, "status");
    assert.equal(status.status, 0, status.stderr);
    assert.match(status.stdout, / is prepared\n$/);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("canonical schema, receipt chain, private modes and link identity are fail-closed", () => {
  const mutations = [
    (active) => {
      const path = join(active, "candidate.json");
      const parsed = parse(path);
      parsed.unreviewed = true;
      writeFileSync(path, `${JSON.stringify(parsed)}\n`, { mode: 0o600 });
    },
    (active) => {
      const path = join(active, "candidate.json");
      writeFileSync(path, ` ${readFileSync(path, "utf8")}`, { mode: 0o600 });
    },
    (active) => chmodSync(join(active, "00-plan.json"), 0o644),
    (active) => linkSync(join(active, "candidate.json"), join(active, "candidate-hardlink.json")),
    (active) => {
      const path = join(active, "00-plan.json");
      const parsed = parse(path);
      parsed.previous_sha256 = "1".repeat(64);
      const ordered = Object.fromEntries(Object.entries(parsed).sort(([left], [right]) => left.localeCompare(right)));
      writeFileSync(path, `${JSON.stringify(ordered)}\n`, { mode: 0o600 });
    },
    (active) => writeFileSync(join(active, "unreviewed"), "unexpected\n", { mode: 0o600 }),
    (active) => chmodSync(join(active, "provider"), 0o711),
    (active) => writeFileSync(join(active, "runtime", "unexpected"), "state\n", { mode: 0o600 }),
    (active) => {
      const client = join(active, "client");
      const external = join(dirname(dirname(active)), "client-substitute");
      rmSync(client, { recursive: true, force: false });
      mkdirSync(external, { mode: 0o700 });
      writeFileSync(join(external, "proxy-template.json"), "{}\n", { mode: 0o600 });
      symlinkSync(external, client);
    },
    (active) => linkSync(join(active, "00-plan.json"), join(active, "third-plan-link.json")),
    (active) => {
      const lease = join(dirname(active), "active");
      const replacement = join(dirname(active), "replacement-active");
      copyFileSync(lease, replacement);
      chmodSync(replacement, 0o600);
      rmSync(lease);
      copyFileSync(replacement, lease);
      chmodSync(lease, 0o600);
      rmSync(replacement);
    },
  ];
  for (const mutate of mutations) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      mutate(activeRun(state));
      const result = run(state, "status");
      assert.equal(result.status, 78);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /^clean-engine: /);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("an interrupted pre-provider staging run is inert and does not gain authority", async () => {
  const state = fixture();
  const bin = join(state.root, "bin");
  const entered = join(state.root, "git-entered");
  const realGit = command("/bin/sh", ["-c", "command -v git"]).stdout.trim();
  mkdirSync(bin, { mode: 0o700 });
  const fakeGit = join(bin, "git");
  writeFileSync(
    fakeGit,
    `#!/bin/sh\nset -eu\ncase " $* " in *" status "*) : > ${JSON.stringify(entered)}; while :; do /bin/sleep 1; done ;; esac\nexec ${JSON.stringify(realGit)} "$@"\n`,
    { mode: 0o700 },
  );
  chmodSync(fakeGit, 0o700);
  try {
    const child = spawn(process.execPath, toolArgs(state, "plan"), {
      detached: true,
      env: { PATH: `${bin}:${process.env.PATH}`, LANG: "C", LC_ALL: "C" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.stdout.resume();
    child.stderr.resume();
    const deadline = Date.now() + 8_000;
    while (!existsSync(entered)) {
      assert.ok(Date.now() < deadline, "timed out waiting for staged source inventory");
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    process.kill(-child.pid, "SIGKILL");
    const [, signal] = await once(child, "close");
    assert.equal(signal, "SIGKILL");
    const entries = readdirSync(state.state);
    assert.equal(entries.length, 1);
    assert.match(entries[0], /^\.pending-[0-9a-f]{32}$/);
    assert.equal(existsSync(join(state.state, "active")), false);

    const resumed = run(state, "plan");
    assert.equal(resumed.status, 0, resumed.stderr);
    assert.equal(run(state, "verify").status, 0);
    const finalEntries = readdirSync(state.state).sort();
    assert.equal(finalEntries.includes("active"), true);
    assert.equal(finalEntries.filter((entry) => /^\.run-[0-9a-f]{32}$/.test(entry)).length, 1);
    assert.equal(finalEntries.filter((entry) => /^\.pending-[0-9a-f]{32}$/.test(entry)).length, 1);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a completed run interrupted before active-link publication remains inert", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const orphan = activeRun(state);
    rmSync(join(state.state, "active"));
    assert.equal(lstatSync(join(orphan, "00-plan.json")).nlink, 1);

    const resumed = run(state, "plan");
    assert.equal(resumed.status, 0, resumed.stderr);
    assert.equal(run(state, "verify").status, 0);
    assert.equal(
      readdirSync(state.state).filter((entry) => /^\.run-[0-9a-f]{32}$/.test(entry)).length,
      2,
    );
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("planning refuses inert staging with foreign leaves or external-mutation state", () => {
  const mutations = [
    (pending) => writeFileSync(join(pending, "foreign"), "foreign\n", { mode: 0o600 }),
    (pending) => {
      mkdirSync(join(pending, "provider"), { mode: 0o700 });
      writeFileSync(join(pending, "provider", "resource"), "resource\n", { mode: 0o600 });
    },
  ];
  for (const mutate of mutations) {
    const state = fixture();
    const pending = join(state.state, `.pending-${"1".repeat(32)}`);
    try {
      mkdirSync(pending, { mode: 0o700 });
      mutate(pending);
      const result = run(state, "plan");
      assert.equal(result.status, 78);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /^clean-engine: pending /);
      assert.equal(existsSync(pending), true);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("retained inert staging is bounded", () => {
  const state = fixture();
  try {
    for (let index = 0; index < 9; index += 1) {
      mkdirSync(join(state.state, `.pending-${index.toString(16).padStart(32, "0")}`), {
        mode: 0o700,
      });
    }
    const result = run(state, "plan");
    assert.equal(result.status, 73);
    assert.equal(result.stdout, "");
    assert.equal(result.stderr, "clean-engine: inert staging limit was exceeded\n");
    assert.equal(existsSync(join(state.state, "active")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("two simultaneous planners publish one no-replace active receipt", async () => {
  const state = fixture();
  try {
    const launch = () => {
      const child = spawn(process.execPath, toolArgs(state, "plan"), {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stdout = "";
      let stderr = "";
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", (chunk) => { stdout += chunk; });
      child.stderr.on("data", (chunk) => { stderr += chunk; });
      return new Promise((resolvePromise) => {
        child.on("close", (status, signal) => resolvePromise({ status, signal, stdout, stderr }));
      });
    };
    const results = await Promise.all([launch(), launch()]);
    assert.deepEqual(results.map((result) => result.status).sort((a, b) => a - b), [0, 73]);
    assert.equal(results.every((result) => result.signal === null), true);
    assert.equal(results.filter((result) => result.status === 0)[0].stderr, "");
    assert.match(
      results.filter((result) => result.status === 73)[0].stderr,
      /active clean-engine plan already exists/,
    );
    assert.equal(run(state, "verify").status, 0);
    const entries = readdirSync(state.state).sort();
    assert.equal(entries.length, 2);
    assert.equal(entries.includes("active"), true);
    assert.equal(entries.filter((entry) => /^\.run-[0-9a-f]{32}$/.test(entry)).length, 1);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("actual Docker-context modes and empty directories are part of source closure", () => {
  const modeState = fixture();
  try {
    assert.equal(run(modeState, "plan").status, 0);
    chmodSync(join(modeState.repo, "source.txt"), 0o644);
    assert.equal(git(modeState.repo, ["status", "--porcelain"]), "");
    const result = run(modeState, "verify");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: source closure changed\n");
  } finally {
    rmSync(modeState.root, { recursive: true, force: true });
  }

  const emptyState = fixture();
  try {
    assert.equal(run(emptyState, "plan").status, 0);
    mkdirSync(join(emptyState.repo, "included-empty"), { mode: 0o700 });
    assert.equal(git(emptyState.repo, ["status", "--porcelain"]), "");
    const result = run(emptyState, "verify");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: source worktree/context is not clean\n");
  } finally {
    rmSync(emptyState.root, { recursive: true, force: true });
  }

  const ignoredState = fixture();
  try {
    assert.equal(run(ignoredState, "plan").status, 0);
    mkdirSync(join(ignoredState.repo, ".codex", "empty"), {
      mode: 0o700,
      recursive: true,
    });
    mkdirSync(join(ignoredState.repo, "target", "empty"), {
      mode: 0o700,
      recursive: true,
    });
    assert.equal(run(ignoredState, "verify").status, 0);
  } finally {
    rmSync(ignoredState.root, { recursive: true, force: true });
  }
});

test("only the exact .env.example basename is re-included in the Docker context", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    chmodSync(join(state.repo, ".env.secret.example"), 0o644);
    assert.equal(git(state.repo, ["status", "--porcelain"]), "");
    assert.equal(run(state, "verify").status, 0);
    chmodSync(join(state.repo, ".env.example"), 0o644);
    const result = run(state, "verify");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: source closure changed\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("tracked symlinks are accepted and Dockerignore order is exact", () => {
  const symlinkState = fixture();
  try {
    symlinkSync("source.txt", join(symlinkState.repo, "source-link"));
    git(symlinkState.repo, ["add", "source-link"]);
    git(symlinkState.repo, [
      "-c",
      "user.name=Synveda Test",
      "-c",
      "user.email=synveda-test@example.invalid",
      "commit",
      "-q",
      "-m",
      "symlink fixture",
    ]);
    assert.equal(run(symlinkState, "plan").status, 0);
    assert.equal(run(symlinkState, "verify").status, 0);
  } finally {
    rmSync(symlinkState.root, { recursive: true, force: true });
  }

  const orderState = fixture();
  try {
    const ignorePath = join(orderState.repo, ".dockerignore");
    const reordered = readFileSync(ignorePath, "utf8").replace(
      "**/.env.*\n!**/.env.example",
      "!**/.env.example\n**/.env.*",
    );
    writeFileSync(ignorePath, reordered, { mode: 0o600 });
    git(orderState.repo, ["add", ".dockerignore"]);
    git(orderState.repo, [
      "-c",
      "user.name=Synveda Test",
      "-c",
      "user.email=synveda-test@example.invalid",
      "commit",
      "-q",
      "-m",
      "reordered ignore",
    ]);
    const result = run(orderState, "plan");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: Docker ignore contract was refused\n");
  } finally {
    rmSync(orderState.root, { recursive: true, force: true });
  }
});

test("read-only actions never create a missing state root", () => {
  const state = fixture();
  try {
    rmSync(state.state, { recursive: true, force: false });
    const result = run(state, "status");
    assert.equal(result.status, 69);
    assert.equal(result.stderr, "clean-engine: state base was unavailable\n");
    assert.equal(existsSync(state.state), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("unsafe pools, providers, repository-local state and unknown arguments are refused", () => {
  const state = fixture();
  try {
    const base = [
      stateTool,
      "plan",
      "--repo-root",
      state.repo,
      "--state-base",
      state.state,
      "--ipv4-pool",
    ];
    for (const pool of ["172.15.1.0/24", "10.1.2.1/24", "10.01.2.0/24", "public.invalid/24"]) {
      const result = command(process.execPath, [...base, pool, "--provider", "colima"]);
      assert.equal(result.status, 64);
      assert.equal(result.stderr, "clean-engine: IPv4 pool must be a canonical private /24\n");
    }
    const provider = command(process.execPath, [...base, "10.1.2.0/24", "--provider", "desktop"]);
    assert.equal(provider.status, 64);
    assert.equal(provider.stderr, "clean-engine: provider must be colima\n");
    const local = command(process.execPath, [
      stateTool,
      "plan",
      "--repo-root",
      state.repo,
      "--state-base",
      join(state.repo, "state"),
      "--ipv4-pool",
      "10.1.2.0/24",
      "--provider",
      "colima",
    ]);
    assert.equal(local.status, 78);
    assert.equal(local.stderr, "clean-engine: state base must be outside the repository\n");
    const unknown = run(state, "status", ["--unreviewed", "value"]);
    assert.equal(unknown.status, 64);
    assert.equal(unknown.stderr, "clean-engine: invalid arguments\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});
