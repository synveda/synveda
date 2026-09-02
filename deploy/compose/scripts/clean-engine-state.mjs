#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  closeSync,
  constants,
  existsSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readlinkSync,
  readSync,
  realpathSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";
import {
  ReceiptFailure,
  buildEnvironmentManifest,
  createFinalization,
  createNextReceipt,
  receiptFileName,
  validateReceiptChain,
} from "./clean-engine-receipts.mjs";

const REGISTRY_IMAGE =
  "registry:3.1.1@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33";
const MAX_FILE_BYTES = 256 * 1024;
const MAX_CONTEXT_ENTRIES = 100_000;
const MAX_INERT_STAGING = 8;
const MAX_CONTEXT_FILE_BYTES = 128n * 1024n * 1024n;
const MAX_CONTEXT_TOTAL_BYTES = 2n * 1024n * 1024n * 1024n;
const OWNER_UID = BigInt(process.getuid());
const ZERO_SHA256 = "0".repeat(64);
const RECEIPT_STAGING_NAME = ".receipt-publish";
const ENVIRONMENT_NAME = "environment.json";
const ENVIRONMENT_STAGING_NAME = ".environment-publish";
const LEGACY_MUTATION_LEASE_NAME = ".mutation-lease";
const MUTATION_SLOT_PREFIX = ".mutation-slot-";
const MUTATION_CLOSE_PREFIX = ".mutation-close-";
const MUTATION_SLOT_SCHEMA = "synveda.clean-engine.mutation-slot.v1";
const MUTATION_CLOSE_SCHEMA = "synveda.clean-engine.mutation-close.v1";
const MUTATION_RECOVERY_PREFIX = ".mutation-recovery-";
const MUTATION_RECOVERY_SCHEMA = "synveda.clean-engine.mutation-recovery.v1";
const MUTATION_STAGE_PREFIX = ".mutation-stage-";
const MUTATION_OWNER_PROBE = "opaque-process-instance-v1";
const MAX_MUTATION_RECOVERIES = 8;
const MAX_MUTATION_SLOTS = 64;
const MAX_MUTATION_STAGES = 16;
const MAX_MUTATION_PUBLICATION_ATTEMPTS = 16;
const FAKE_PROVIDER_ADAPTER_FIELDS = Object.freeze([
  "close_prelink_hold_milliseconds",
  "execute_outcome",
  "execute_result",
  "hold_milliseconds",
  "kind",
  "prelink_hold_milliseconds",
  "publication_hold_milliseconds",
  "reconcile_hold_milliseconds",
  "reconcile_outcome",
  "reconcile_result",
]);
const FAKE_PROVIDER_CONTRACT = Object.freeze({
  adapter_fields: FAKE_PROVIDER_ADAPTER_FIELDS,
  execute_outcomes: Object.freeze(["failed", "passed"]),
  kind: "deterministic-fake-provider-v1",
  max_hold_milliseconds: 30_000,
  reconcile_outcomes: Object.freeze(["failed", "passed", "unknown"]),
  result_contract: "clean-engine-provider-receipt-v2",
  schema: "synveda.clean-engine.fake-provider-contract.v1",
});
const FAKE_PROVIDER_CONTRACT_SHA256 = digest(canonicalBytes(FAKE_PROVIDER_CONTRACT));
const REQUESTED_ASSERTIONS = Object.freeze([
  "browser-pkce-admin-logout-no-capture",
  "builder-canary-zero-connections",
  "canonical-proxy-values-empty",
  "clean-engine-initial-state",
  "disposable-engine-destroyed",
  "docker-client-proxy-active",
  "exact-local-embedded-builder",
  "exact-project-cleanup",
  "registry-auth-negative-positive",
  "source-closure-unchanged",
]);
const EXCLUDED_CLAIMS = Object.freeze([
  "disaster-recovery",
  "docker-desktop-parity",
  "enterprise-certification",
  "high-availability",
  "host-loss-tolerance",
  "native-linux-parity",
  "production-saas-readiness",
  "reference-https",
  "signed-provenance",
  "zero-downtime-upgrades",
]);
const PROXY_TEMPLATE = Object.freeze({
  auths: Object.freeze({}),
  proxies: Object.freeze({
    default: Object.freeze({
      allProxy: "socks5://all-proxy-canary.invalid:65535",
      ftpProxy: "http://ftp-proxy-canary.invalid:65535",
      httpProxy: "http://http-proxy-canary.invalid:65535",
      httpsProxy: "http://https-proxy-canary.invalid:65535",
      noProxy: "proxy-bypass-canary.invalid",
    }),
  }),
});
const DOCKERIGNORE_CONTRACT = Object.freeze([
  ".git",
  ".git/**",
  ".agents",
  ".agents/**",
  ".codex",
  ".codex/**",
  ".claude",
  ".claude/**",
  "target",
  "target/**",
  "**/target",
  "**/target/**",
  "node_modules",
  "node_modules/**",
  "**/node_modules",
  "**/node_modules/**",
  ".pnpm-store",
  ".pnpm-store/**",
  "**/dist",
  "**/dist/**",
  "**/dist-test",
  "**/dist-test/**",
  "data",
  "data/**",
  "evals/fixtures/longmemeval/longmemeval_*.json",
  "evals/fixtures/longmemeval/LICENSE",
  "evals/fixtures/longmemeval/LICENSE.*",
  ".env",
  ".env.*",
  "**/.env",
  "**/.env.*",
  "!**/.env.example",
  "deploy/compose/secrets",
  "deploy/compose/secrets/**",
  "deploy/compose/runtime",
  "deploy/compose/runtime/**",
  "deploy/compose/backups",
  "deploy/compose/backups/**",
  ".DS_Store",
  "**/.DS_Store",
  "Thumbs.db",
  "**/Thumbs.db",
  "*.log",
  "**/*.log",
  "*.tmp",
  "**/*.tmp",
]);

class ClosedFailure extends Error {
  constructor(code, status) {
    super(code);
    this.exitStatus = status;
  }
}

function fail(code, status = 78) {
  throw new ClosedFailure(code, status);
}

function parseArgs(argv) {
  const action = argv[2];
  if (!new Set(["plan", "status", "verify"]).has(action)) {
    fail("invalid action", 64);
  }
  const values = {};
  for (let index = 3; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || value === "") {
      fail("invalid arguments", 64);
    }
    const key = name.slice(2);
    if (values[key] !== undefined) fail("duplicate argument", 64);
    values[key] = value;
  }
  const common = new Set(["repo-root", "state-base"]);
  const allowed = action === "plan" ? new Set([...common, "ipv4-pool", "provider"]) : common;
  if (
    Object.keys(values).some((name) => !allowed.has(name)) ||
    [...common].some((name) => values[name] === undefined) ||
    (action === "plan" && (values["ipv4-pool"] === undefined || values.provider === undefined))
  ) {
    fail("invalid arguments", 64);
  }
  return { action, values };
}

function canonical(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number" && Number.isSafeInteger(value)) return String(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(",")}}`;
  }
  fail("unsupported canonical value", 70);
}

function canonicalBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

function digest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function exactLstat(path) {
  return lstatSync(path, { bigint: true });
}

function exactFstat(descriptor) {
  return fstatSync(descriptor, { bigint: true });
}

function exactStat(path) {
  return statSync(path, { bigint: true });
}

function runGit(repoRoot, args, label) {
  const result = spawnSync("git", ["-C", repoRoot, ...args], {
    encoding: null,
    env: {
      GIT_CONFIG_GLOBAL: "/dev/null",
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_OPTIONAL_LOCKS: "0",
      GIT_TERMINAL_PROMPT: "0",
      LANG: "C",
      LC_ALL: "C",
      PATH: process.env.PATH ?? "/usr/bin:/bin",
    },
    maxBuffer: 16 * 1024 * 1024,
    timeout: 30_000,
    killSignal: "SIGKILL",
  });
  if (result.error !== undefined || result.status !== 0) fail(`${label} was unavailable`, 69);
  return result.stdout;
}

function onlyLowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function pathIsOutsideBuildContext(path) {
  if (path.includes("\0") || path.startsWith("/") || path.split("/").includes("..")) return false;
  const segments = path.split("/");
  const base = segments.at(-1);
  if ([".DS_Store", "Thumbs.db"].includes(base) || base.endsWith(".log") || base.endsWith(".tmp")) {
    return true;
  }
  if (base === ".env.example") return false;
  if (base === ".env" || base.startsWith(".env.")) return true;
  if (new Set([".git", "target", "node_modules", ".pnpm-store", "dist", "dist-test"]).has(segments[0])) {
    return true;
  }
  if (segments.some((segment) => new Set(["target", "node_modules", "dist", "dist-test"]).has(segment))) {
    return true;
  }
  if (new Set([".agents", ".claude", ".codex", "data"]).has(segments[0])) return true;
  if (
    segments.length === 4 &&
    segments[0] === "evals" &&
    segments[1] === "fixtures" &&
    segments[2] === "longmemeval" &&
    (segments[3] === "LICENSE" ||
      segments[3].startsWith("LICENSE.") ||
      /^longmemeval_.*\.json$/.test(segments[3]))
  ) {
    return true;
  }
  return (
    segments[0] === "deploy" &&
    segments[1] === "compose" &&
    new Set(["backups", "runtime", "secrets"]).has(segments[2])
  );
}

function dockerIgnoreContract(repoRoot) {
  let source;
  try {
    source = readFileSync(join(repoRoot, ".dockerignore"), "utf8");
  } catch {
    fail("Docker ignore contract was unavailable", 69);
  }
  const patterns = source
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("#"));
  if (
    new Set(patterns).size !== patterns.length ||
    JSON.stringify(patterns) !== JSON.stringify(DOCKERIGNORE_CONTRACT)
  ) {
    fail("Docker ignore contract was refused");
  }
}

function nulFields(buffer, label) {
  if (buffer.length === 0) return [];
  if (buffer.at(-1) !== 0) fail(`${label} was malformed`, 69);
  return buffer
    .subarray(0, -1)
    .toString("utf8")
    .split("\0");
}

function nulBufferFields(buffer, label) {
  if (buffer.length === 0) return [];
  if (buffer.at(-1) !== 0) fail(`${label} was malformed`, 69);
  const fields = [];
  let start = 0;
  for (let index = 0; index < buffer.length; index += 1) {
    if (buffer[index] !== 0) continue;
    fields.push(buffer.subarray(start, index));
    start = index + 1;
  }
  return fields;
}

function portablePath(bytes, label) {
  let path;
  try {
    path = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    fail(`${label} path was refused`, 69);
  }
  if (
    path === "" ||
    path.startsWith("/") ||
    /[\0-\x1f\x7f]/.test(path) ||
    path.split("/").some((part) => part === "" || part === "." || part === "..") ||
    !Buffer.from(path, "utf8").equals(bytes)
  ) {
    fail(`${label} path was refused`, 69);
  }
  return path;
}

function trackedEntries(indexBytes) {
  const entries = new Map();
  for (const field of nulBufferFields(indexBytes, "tracked source inventory")) {
    const separator = field.indexOf(0x09);
    if (separator < 1) fail("tracked source inventory was malformed", 69);
    const header = field.subarray(0, separator).toString("ascii");
    const match = header.match(/^([0-7]{6}) ([0-9a-f]{40}|[0-9a-f]{64}) ([0-3])$/);
    if (match === null || match[3] !== "0" || !new Set(["100644", "100755", "120000"]).has(match[1])) {
      fail("tracked source inventory was refused");
    }
    const path = portablePath(field.subarray(separator + 1), "tracked source");
    if (entries.has(path)) fail("tracked source inventory was refused");
    entries.set(path, { mode: match[1], object: match[2] });
  }
  if (entries.size === 0) fail("tracked source inventory was malformed", 69);
  return entries;
}

function sameMetadata(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.nlink === right.nlink &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs
  );
}

function modeString(metadata) {
  return (metadata.mode & 0o7777n).toString(8).padStart(4, "0");
}

function deploymentInput(path) {
  return (
    path === ".dockerignore" ||
    path === "Makefile" ||
    path === "deploy/compose" ||
    path.startsWith("deploy/compose/") ||
    path === "docs/DEPLOYMENT_CONTRACT.md" ||
    path === "docs/SECURITY.md"
  );
}

function readContextFile(path, initial, budget) {
  if (!initial.isFile() || initial.nlink !== 1n || initial.size > MAX_CONTEXT_FILE_BYTES) {
    fail("Docker build-context file was refused");
  }
  if (budget.bytes + initial.size > MAX_CONTEXT_TOTAL_BYTES) {
    fail("Docker build-context size was refused");
  }
  let descriptor;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = exactFstat(descriptor);
    if (!sameMetadata(initial, before) || !before.isFile() || before.nlink !== 1n) {
      fail("Docker build-context file identity was refused");
    }
    const hash = createHash("sha256");
    const chunk = Buffer.allocUnsafe(64 * 1024);
    let offset = 0;
    while (offset < Number(before.size)) {
      const length = Math.min(chunk.length, Number(before.size) - offset);
      const count = readSync(descriptor, chunk, 0, length, offset);
      if (count < 1) fail("Docker build-context file changed while it was read");
      hash.update(chunk.subarray(0, count));
      offset += count;
    }
    const after = exactFstat(descriptor);
    const current = exactLstat(path);
    if (!sameMetadata(before, after) || !sameMetadata(after, current)) {
      fail("Docker build-context file changed while it was read");
    }
    budget.bytes += before.size;
    return { sha256: hash.digest("hex"), size: String(before.size) };
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail("Docker build-context file was unavailable", 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function readContextSymlink(path, initial, budget) {
  if (!initial.isSymbolicLink() || initial.nlink !== 1n) {
    fail("Docker build-context symlink was refused");
  }
  let target;
  try {
    target = readlinkSync(path, { encoding: "buffer" });
  } catch {
    fail("Docker build-context symlink was unavailable", 69);
  }
  const current = exactLstat(path);
  if (!sameMetadata(initial, current) || target.length > 16 * 1024) {
    fail("Docker build-context symlink changed while it was read");
  }
  if (budget.bytes + BigInt(target.length) > MAX_CONTEXT_TOTAL_BYTES) {
    fail("Docker build-context size was refused");
  }
  budget.bytes += BigInt(target.length);
  return { sha256: digest(target), size: String(target.length) };
}

function actualContextManifest(repoRoot, indexBytes) {
  const tracked = trackedEntries(indexBytes);
  const expectedDirectories = new Set();
  for (const path of tracked.keys()) {
    const parts = path.split("/");
    for (let index = 1; index < parts.length; index += 1) {
      const directory = parts.slice(0, index).join("/");
      if (pathIsOutsideBuildContext(directory)) break;
      expectedDirectories.add(directory);
    }
  }
  const seen = new Set();
  const contextHash = createHash("sha256");
  const deploymentHash = createHash("sha256");
  contextHash.update("synveda.docker-context-manifest.v1\0");
  deploymentHash.update("synveda.deployment-input-manifest.v1\0");
  const budget = { bytes: 0n, entries: 0 };
  const contractPath = "docs/DEPLOYMENT_CONTRACT.md";
  let contract;

  function record(path, value) {
    budget.entries += 1;
    if (budget.entries > MAX_CONTEXT_ENTRIES) fail("Docker build-context entry count was refused");
    const bytes = canonicalBytes({ path, ...value });
    contextHash.update(bytes);
    if (deploymentInput(path)) deploymentHash.update(bytes);
    if (path === contractPath) contract = value;
  }

  function walk(directory, relativeDirectory = "") {
    const initial = exactLstat(directory);
    if (!initial.isDirectory() || initial.isSymbolicLink()) {
      fail("Docker build-context directory was refused");
    }
    let names;
    try {
      names = readdirSync(directory, { encoding: "buffer" }).sort(Buffer.compare);
    } catch {
      fail("Docker build-context directory was unavailable", 69);
    }
    for (const nameBytes of names) {
      const name = portablePath(nameBytes, "Docker build-context");
      const path = relativeDirectory === "" ? name : `${relativeDirectory}/${name}`;
      if (pathIsOutsideBuildContext(path)) continue;
      const absolute = join(repoRoot, ...path.split("/"));
      const metadata = exactLstat(absolute);
      if (metadata.isDirectory() && !metadata.isSymbolicLink()) {
        if (!expectedDirectories.has(path)) fail("source worktree/context is not clean");
        record(path, { mode: modeString(metadata), type: "directory" });
        walk(absolute, path);
        continue;
      }
      const trackedEntry = tracked.get(path);
      if (trackedEntry === undefined) fail("source worktree/context is not clean");
      if (metadata.isFile() && trackedEntry.mode !== "120000") {
        const content = readContextFile(absolute, metadata, budget);
        record(path, { mode: modeString(metadata), type: "file", ...content });
      } else if (metadata.isSymbolicLink() && trackedEntry.mode === "120000") {
        const content = readContextSymlink(absolute, metadata, budget);
        record(path, { mode: modeString(metadata), type: "symlink", ...content });
      } else {
        fail("Docker build-context entry type was refused");
      }
      seen.add(path);
    }
    let finalNames;
    try {
      finalNames = readdirSync(directory, { encoding: "buffer" }).sort(Buffer.compare);
    } catch {
      fail("Docker build-context directory was unavailable", 69);
    }
    const current = exactLstat(directory);
    if (
      !sameMetadata(initial, current) ||
      names.length !== finalNames.length ||
      names.some((name, index) => !name.equals(finalNames[index]))
    ) {
      fail("Docker build-context directory changed while it was read");
    }
  }

  walk(repoRoot);
  for (const path of tracked.keys()) {
    if (!pathIsOutsideBuildContext(path) && !seen.has(path)) {
      fail("tracked Docker build-context input was unavailable", 69);
    }
  }
  if (
    contract === undefined ||
    contract.type !== "file" ||
    BigInt(contract.size) < 1n ||
    BigInt(contract.size) > 2n * 1024n * 1024n
  ) {
    fail("deployment contract size was refused", 69);
  }
  return {
    build_context_manifest_sha256: contextHash.digest("hex"),
    deployment_contract_sha256: contract.sha256,
    deployment_input_manifest_sha256: deploymentHash.digest("hex"),
  };
}

function sourceStatus(repoRoot) {
  return runGit(
    repoRoot,
    ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    "worktree inventory",
  );
}

function sourceClosure(repoRoot) {
  dockerIgnoreContract(repoRoot);
  const status = sourceStatus(repoRoot);
  if (status.length !== 0) fail("source worktree is not clean");

  const indexFlagBytes = runGit(repoRoot, ["ls-files", "-v", "-z"], "source index flags");
  const indexFlags = nulFields(indexFlagBytes, "source index flags");
  if (
    indexFlags.length === 0 ||
    indexFlags.some((entry) => entry.length < 3 || entry[0] !== "H" || entry[1] !== " ")
  ) {
    fail("source index contains hidden worktree state");
  }

  const ignored = nulFields(
    runGit(
      repoRoot,
      ["ls-files", "--others", "--ignored", "--exclude-standard", "-z"],
      "ignored source inventory",
    ),
    "ignored source inventory",
  );
  if (ignored.some((path) => !pathIsOutsideBuildContext(path))) {
    fail("ignored source input is not excluded from the build context");
  }

  const commitSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD"], "source commit")
    .toString("ascii")
    .trim();
  const treeSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD^{tree}"], "source tree")
    .toString("ascii")
    .trim();
  if (!onlyLowerHex(commitSha, 40) || !onlyLowerHex(treeSha, 40)) {
    fail("source identity was malformed", 69);
  }

  const buildInputs = runGit(repoRoot, ["ls-files", "--stage", "-z"], "tracked source inventory");
  const actual = actualContextManifest(repoRoot, buildInputs);
  const finalStatus = sourceStatus(repoRoot);
  const finalIndexFlags = runGit(repoRoot, ["ls-files", "-v", "-z"], "source index flags");
  const finalBuildInputs = runGit(
    repoRoot,
    ["ls-files", "--stage", "-z"],
    "tracked source inventory",
  );
  const finalCommitSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD"], "source commit")
    .toString("ascii")
    .trim();
  const finalTreeSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD^{tree}"], "source tree")
    .toString("ascii")
    .trim();
  const finalActual = actualContextManifest(repoRoot, finalBuildInputs);
  if (
    finalStatus.length !== 0 ||
    !finalIndexFlags.equals(indexFlagBytes) ||
    !finalBuildInputs.equals(buildInputs) ||
    finalCommitSha !== commitSha ||
    finalTreeSha !== treeSha ||
    canonical(finalActual) !== canonical(actual)
  ) {
    fail("source closure changed while it was read");
  }
  return {
    ...actual,
    commit_sha: commitSha,
    tracked_index_manifest_sha256: digest(buildInputs),
    tree_sha: treeSha,
    worktree_clean: true,
  };
}

function privateIpv4Pool(value) {
  if (typeof value !== "string") return false;
  const match = value.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.0\/24$/);
  if (match === null) return false;
  const rawOctets = match.slice(1);
  const octets = rawOctets.map(Number);
  if (octets.some((octet, index) => octet > 255 || String(octet) !== rawOctets[index])) return false;
  const [first, second] = octets;
  return first === 10 || (first === 172 && second >= 16 && second <= 31) ||
    (first === 192 && second === 168);
}

function inside(parent, candidate) {
  const path = relative(parent, candidate);
  return path === "" || (!path.startsWith(`..${sep}`) && path !== "..");
}

function ownedPrivateDirectory(path, label) {
  let metadata;
  try {
    metadata = exactLstat(path);
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== OWNER_UID ||
    (metadata.mode & 0o7777n) !== 0o700n ||
    realpathSync(path) !== resolve(path)
  ) {
    fail(`${label} was refused`);
  }
  return metadata;
}

function prepareRoots(repoArgument, stateArgument, createState) {
  if (!isAbsolute(repoArgument) || !isAbsolute(stateArgument)) fail("paths must be absolute", 64);
  let repoRoot;
  try {
    repoRoot = realpathSync(repoArgument);
  } catch {
    fail("repository root was unavailable", 69);
  }
  if (repoRoot !== resolve(repoArgument) || !exactStat(repoRoot).isDirectory()) {
    fail("repository root was refused");
  }
  const stateBase = resolve(stateArgument);
  if (inside(repoRoot, stateBase) || inside(stateBase, repoRoot)) {
    fail("state base must be outside the repository");
  }
  if (!existsSync(stateBase)) {
    if (!createState) fail("state base was unavailable", 69);
    try {
      const missing = [];
      let ancestor = stateBase;
      while (!existsSync(ancestor)) {
        missing.unshift(ancestor);
        const parent = dirname(ancestor);
        if (parent === ancestor) fail("state base ancestor was unavailable", 69);
        ancestor = parent;
      }
      const ancestorMetadata = exactLstat(ancestor);
      if (
        !ancestorMetadata.isDirectory() ||
        ancestorMetadata.isSymbolicLink() ||
        realpathSync(ancestor) !== resolve(ancestor)
      ) {
        fail("state base ancestor was refused");
      }
      for (const directory of missing) {
        mkdirSync(directory, { mode: 0o700 });
        ownedPrivateDirectory(directory, "created state directory");
        syncDirectory(dirname(directory));
      }
    } catch {
      fail("state base creation failed", 70);
    }
  }
  ownedPrivateDirectory(stateBase, "state base");
  return { repoRoot, stateBase, active: join(stateBase, "active") };
}

function syncDirectory(path) {
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_DIRECTORY);
  try {
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

function writeExclusive(path, bytes, mode = 0o600) {
  let descriptor;
  try {
    descriptor = openSync(
      path,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      mode,
    );
    let offset = 0;
    while (offset < bytes.length) {
      const written = writeSync(descriptor, bytes, offset, bytes.length - offset);
      if (written < 1) fail("immutable state publication failed", 70);
      offset += written;
    }
    fsyncSync(descriptor);
  } catch {
    fail("immutable state publication failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  syncDirectory(dirname(path));
}

function readPrivate(path, label, expectedLinks = 1, minimumBytes = 2n) {
  let descriptor;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = exactFstat(descriptor);
    if (
      !before.isFile() ||
      before.uid !== OWNER_UID ||
      before.nlink !== BigInt(expectedLinks) ||
      (before.mode & 0o7777n) !== 0o600n ||
      before.size < minimumBytes ||
      before.size > BigInt(MAX_FILE_BYTES)
    ) {
      fail(`${label} file was refused`);
    }
    const bytes = readFileSync(descriptor);
    const after = exactFstat(descriptor);
    if (
      after.dev !== before.dev ||
      after.ino !== before.ino ||
      after.size !== before.size ||
      after.mode !== before.mode ||
      after.nlink !== BigInt(expectedLinks)
    ) {
      fail(`${label} file changed while it was read`);
    }
    return bytes;
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail(`${label} file was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function parseCanonical(path, label, expectedLinks = 1, minimumBytes = 2n) {
  const bytes = readPrivate(path, label, expectedLinks, minimumBytes);
  let value;
  try {
    value = JSON.parse(bytes.toString("utf8"));
  } catch {
    fail(`${label} was not canonical JSON`);
  }
  if (!Buffer.from(canonicalBytes(value)).equals(bytes)) fail(`${label} was not canonical JSON`);
  return { bytes, value };
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") fail(`${label} was malformed`);
  if (JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) {
    fail(`${label} fields were refused`);
  }
}

let currentProcessIdentityValue;

function currentBootSessionSha256() {
  let value;
  if (process.platform === "linux") {
    try {
      value = readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim();
    } catch {
      fail("process boot identity was unavailable", 69);
    }
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value)) {
      fail("process boot identity was refused", 69);
    }
  } else if (process.platform === "darwin") {
    let result = spawnSync("/usr/sbin/sysctl", ["-n", "kern.boottime"], {
      encoding: "utf8",
      env: { LANG: "C", LC_ALL: "C", PATH: "/usr/bin:/bin:/usr/sbin:/sbin", TZ: "UTC0" },
      maxBuffer: 4096,
      timeout: 5_000,
    });
    value = result.stdout?.trim();
    if (
      result.error !== undefined ||
      result.status !== 0 ||
      typeof value !== "string" ||
      !/^\{ sec = [0-9]+, usec = [0-9]+ \} .+$/.test(value)
    ) {
      // Sandboxed macOS runners can deny both kernel boot and process-table
      // probes. The closed fallback is only an opaque platform label. Recovery
      // never treats a boot-label mismatch as death and still requires the
      // recorded PID to be absent, so this restriction can only make recovery
      // more conservative.
      value = "restricted-kernel-probe-unavailable";
    }
  } else {
    fail("process identity platform was unsupported", 69);
  }
  return digest(Buffer.from(`synveda.clean-engine.boot.v1\0${process.platform}\0${value}`, "utf8"));
}

function currentProcessIdentity() {
  if (currentProcessIdentityValue !== undefined) return currentProcessIdentityValue;
  const bootSha256 = currentBootSessionSha256();
  const instanceSha256 = digest(
    canonicalBytes({
      boot_sha256: bootSha256,
      nonce: randomBytes(32).toString("hex"),
      pid: process.pid,
      schema: "synveda.clean-engine.process-instance.v1",
    }),
  );
  currentProcessIdentityValue = Object.freeze({
    boot_sha256: bootSha256,
    instance_sha256: instanceSha256,
    pid: process.pid,
    probe: MUTATION_OWNER_PROBE,
  });
  return currentProcessIdentityValue;
}

function validateMutationOwner(value, prefix, label) {
  if (
    !onlyLowerHex(value[`${prefix}_boot_sha256`], 64) ||
    !onlyLowerHex(value[`${prefix}_instance_sha256`], 64) ||
    !Number.isSafeInteger(value[`${prefix}_pid`]) ||
    value[`${prefix}_pid`] < 1 ||
    value[`${prefix}_probe`] !== MUTATION_OWNER_PROBE
  ) {
    fail(`${label} owner identity was refused`);
  }
}

function validateMutationLeaseValue(value, fixtureId) {
  exactKeys(
    value,
    [
      "action",
      "fixture_id",
      "intent_receipt_sha256",
      "journal_sequence",
      "nonce",
      "owner_boot_sha256",
      "owner_instance_sha256",
      "owner_pid",
      "owner_probe",
      "previous_close_sha256",
      "schema",
      "source_environment_sha256",
      "source_head_sha256",
      "source_sequence",
    ],
    "mutation slot",
  );
  if (
    value.schema !== MUTATION_SLOT_SCHEMA ||
    value.fixture_id !== fixtureId ||
    !new Set(["append-receipt", "finalize-environment", "provider-create"]).has(value.action) ||
    !onlyLowerHex(value.intent_receipt_sha256, 64) ||
    !Number.isSafeInteger(value.journal_sequence) ||
    value.journal_sequence < 0 ||
    value.journal_sequence >= MAX_MUTATION_SLOTS ||
    !onlyLowerHex(value.nonce, 32) ||
    !onlyLowerHex(value.previous_close_sha256, 64) ||
    !onlyLowerHex(value.source_environment_sha256, 64) ||
    !onlyLowerHex(value.source_head_sha256, 64) ||
    !Number.isSafeInteger(value.source_sequence) ||
    value.source_sequence < 0 ||
    value.source_sequence > 63
  ) {
    fail("mutation slot was refused");
  }
  validateMutationOwner(value, "owner", "mutation slot");
}

function mutationOwnerState(value, probe = defaultMutationOwnerProbe) {
  const state = probe({
    boot_sha256: value.owner_boot_sha256,
    instance_sha256: value.owner_instance_sha256,
    pid: value.owner_pid,
    probe: value.owner_probe,
  });
  if (!new Set(["absent", "current", "pid-reused", "unknown", "zombie"]).has(state)) {
    fail("mutation owner probe was refused", 70);
  }
  return state;
}

function defaultMutationOwnerProbe(owner) {
  let current;
  try {
    current = currentProcessIdentity();
  } catch {
    return "unknown";
  }
  if (owner.pid === current.pid) {
    return owner.boot_sha256 === current.boot_sha256 &&
      owner.instance_sha256 === current.instance_sha256
      ? "current"
      : "unknown";
  }
  try {
    process.kill(owner.pid, 0);
    // Another live PID cannot prove the opaque process-instance challenge. A
    // reused PID therefore blocks safely instead of being treated as the old
    // executor or reclaimed from a coarse timestamp.
    return "unknown";
  } catch (error) {
    return error?.code === "ESRCH" ? "absent" : "unknown";
  }
}

function mutationSlotFileName(sequence) {
  return `${MUTATION_SLOT_PREFIX}${String(sequence).padStart(2, "0")}`;
}

function mutationCloseFileName(sequence) {
  return `${MUTATION_CLOSE_PREFIX}${String(sequence).padStart(2, "0")}`;
}

function recoveryFileName(slotSequence, sequence) {
  return `${MUTATION_RECOVERY_PREFIX}${String(slotSequence).padStart(2, "0")}-${String(sequence).padStart(2, "0")}`;
}

function recoveryChainRootSha256(fixtureId, leaseSha256) {
  return digest(canonicalBytes({
    action: "provider-create",
    fixture_id: fixtureId,
    lease_sha256: leaseSha256,
    schema: "synveda.clean-engine.mutation-recovery-root.v1",
  }));
}

function validateRecoveryClaims(claims, fixtureId, leaseSha256) {
  let previous;
  for (const claim of claims) {
    exactKeys(
      claim.value,
      [
        "action",
        "chain_root_sha256",
        "fixture_id",
        "lease_sha256",
        "nonce",
        "owner_boot_sha256",
        "owner_instance_sha256",
        "owner_pid",
        "owner_probe",
        "parent_sha256",
        "schema",
        "sequence",
        "slot_sequence",
        "source_head_sha256",
      ],
      "mutation recovery claim",
    );
    if (
      claim.name !== recoveryFileName(claim.value.slot_sequence, claim.value.sequence) ||
      claim.value.schema !== MUTATION_RECOVERY_SCHEMA ||
      !Number.isSafeInteger(claim.value.sequence) ||
      claim.value.sequence < 0 ||
      claim.value.sequence >= MAX_MUTATION_RECOVERIES ||
      !Number.isSafeInteger(claim.value.slot_sequence) ||
      claim.value.slot_sequence < 0 ||
      claim.value.slot_sequence >= MAX_MUTATION_SLOTS ||
      claim.value.fixture_id !== fixtureId ||
      claim.value.action !== "provider-create" ||
      !onlyLowerHex(claim.value.lease_sha256, 64) ||
      claim.value.lease_sha256 !== (leaseSha256 ?? claims[0].value.lease_sha256) ||
      claim.value.chain_root_sha256 !==
        recoveryChainRootSha256(claim.value.fixture_id, claim.value.lease_sha256) ||
      !onlyLowerHex(claim.value.nonce, 32) ||
      !onlyLowerHex(claim.value.source_head_sha256, 64)
    ) {
      fail("mutation recovery claim was refused");
    }
    if (
      (previous === undefined &&
        (claim.value.sequence !== 0 || claim.value.parent_sha256 !== ZERO_SHA256)) ||
      (previous !== undefined &&
        (claim.value.sequence !== previous.value.sequence + 1 ||
          claim.value.parent_sha256 !== digest(previous.bytes)))
    ) {
      fail("mutation recovery claim chain was refused");
    }
    validateMutationOwner(claim.value, "owner", "mutation recovery claim");
    previous = claim;
  }
}

function validateMutationCloseValue(value, slot, fixtureId) {
  exactKeys(
    value,
    [
      "authority",
      "authority_sha256",
      "disposition",
      "fixture_id",
      "result_head_sha256",
      "result_environment_sha256",
      "result_sequence",
      "schema",
      "slot_sequence",
      "slot_sha256",
    ],
    "mutation close",
  );
  if (
    value.schema !== MUTATION_CLOSE_SCHEMA ||
    value.fixture_id !== fixtureId ||
    value.slot_sequence !== slot.value.journal_sequence ||
    value.slot_sha256 !== digest(slot.bytes) ||
    !new Set(["owner", "recovery"]).has(value.authority) ||
    !onlyLowerHex(value.authority_sha256, 64) ||
    !onlyLowerHex(value.result_environment_sha256, 64) ||
    !new Set(["aborted-before-effect", "completed"]).has(value.disposition) ||
    !Number.isSafeInteger(value.result_sequence) ||
    value.result_sequence < slot.value.source_sequence ||
    value.result_sequence > 63 ||
    !onlyLowerHex(value.result_head_sha256, 64)
  ) {
    fail("mutation close was refused");
  }
}

function validateCandidate(candidate) {
  exactKeys(
    candidate,
    [
      "created_at",
      "excluded_claims",
      "feature",
      "fixtures",
      "kind",
      "requested_assertions",
      "run_id",
      "schema_version",
      "selection",
      "source",
    ],
    "candidate",
  );
  if (
    candidate.kind !== "synveda-cpr45-clean-engine-candidate" ||
    candidate.schema_version !== 1 ||
    candidate.feature !== "CPR-45" ||
    !onlyLowerHex(candidate.run_id, 32) ||
    typeof candidate.created_at !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(candidate.created_at) ||
    Number.isNaN(Date.parse(candidate.created_at)) ||
    JSON.stringify(candidate.requested_assertions) !== JSON.stringify(REQUESTED_ASSERTIONS) ||
    JSON.stringify(candidate.excluded_claims) !== JSON.stringify(EXCLUDED_CLAIMS)
  ) {
    fail("candidate contract was refused");
  }
  exactKeys(
    candidate.source,
    [
      "build_context_manifest_sha256",
      "commit_sha",
      "deployment_contract_sha256",
      "deployment_input_manifest_sha256",
      "tracked_index_manifest_sha256",
      "tree_sha",
      "worktree_clean",
    ],
    "candidate source",
  );
  for (const name of [
    "build_context_manifest_sha256",
    "deployment_contract_sha256",
    "deployment_input_manifest_sha256",
    "tracked_index_manifest_sha256",
  ]) {
    if (!onlyLowerHex(candidate.source[name], 64)) fail("candidate source digest was refused");
  }
  if (
    !onlyLowerHex(candidate.source.commit_sha, 40) ||
    !onlyLowerHex(candidate.source.tree_sha, 40) ||
    candidate.source.worktree_clean !== true
  ) {
    fail("candidate source identity was refused");
  }
  exactKeys(
    candidate.selection,
    [
      "app_host",
      "auth_host",
      "ipv4_pool",
      "oidc",
      "port",
      "postgres",
      "profiles",
      "project",
      "project_suffix",
      "runtime",
      "scheme",
    ],
    "candidate selection",
  );
  const suffix = `acceptance-${candidate.run_id.slice(0, 24)}`;
  if (
    candidate.selection.runtime !== "development" ||
    candidate.selection.postgres !== "bundled" ||
    candidate.selection.oidc !== "bundled" ||
    candidate.selection.project_suffix !== suffix ||
    candidate.selection.project !== `synveda-development-${suffix}` ||
    JSON.stringify(candidate.selection.profiles) !==
      JSON.stringify(["browser-acceptance", "demo"]) ||
    !privateIpv4Pool(candidate.selection.ipv4_pool) ||
    candidate.selection.app_host !== "app.synveda.test" ||
    candidate.selection.auth_host !== "auth.synveda.test" ||
    candidate.selection.scheme !== "http" ||
    candidate.selection.port !== 8080
  ) {
    fail("candidate selection was refused");
  }
  exactKeys(
    candidate.fixtures,
    [
      "builder_canary",
      "docker_proxy",
      "registry_authentication",
      "registry_image",
      "registry_transport",
    ],
    "candidate fixtures",
  );
  if (
    candidate.fixtures.registry_image !== REGISTRY_IMAGE ||
    candidate.fixtures.registry_transport !== "loopback-tls-ephemeral" ||
    candidate.fixtures.registry_authentication !== "one-run-basic-bcrypt" ||
    candidate.fixtures.docker_proxy !== "synthetic-nonsecret-v1" ||
    candidate.fixtures.builder_canary !== "ambient-remote-inert-zero-read-v1"
  ) {
    fail("candidate fixtures were refused");
  }
}

function validatePlan(plan, candidateBytes, stateMetadata) {
  exactKeys(
    plan,
    ["fixture_id", "outcome", "phase", "previous_sha256", "result", "schema", "sequence"],
    "plan receipt",
  );
  if (
    plan.schema !== "synveda.clean-engine.receipt.v2" ||
    plan.sequence !== 0 ||
    plan.outcome !== "passed" ||
    plan.phase !== "plan" ||
    plan.previous_sha256 !== ZERO_SHA256 ||
    !onlyLowerHex(plan.fixture_id, 32)
  ) {
    fail("plan receipt contract was refused");
  }
  exactKeys(
    plan.result,
    [
      "candidate_sha256",
      "project",
      "provider",
      "provider_resource",
      "state_device",
      "state_inode",
    ],
    "plan result",
  );
  if (
    plan.result.candidate_sha256 !== digest(candidateBytes) ||
    plan.result.provider !== "colima" ||
    plan.result.project !== `synveda-development-acceptance-${plan.fixture_id.slice(0, 24)}` ||
    plan.result.provider_resource !== `synveda-cpr45-${plan.fixture_id}` ||
    plan.result.state_device !== String(stateMetadata.dev) ||
    plan.result.state_inode !== String(stateMetadata.ino)
  ) {
    fail("plan result was refused");
  }
}

function validateProxyTemplate(path) {
  const { value } = parseCanonical(path, "proxy template");
  if (canonical(value) !== canonical(PROXY_TEMPLATE)) fail("proxy template contract was refused");
}

function validatePlanRunInventory(run, stateMetadata) {
  const required = [
    "00-plan.json",
    "candidate.json",
    "client",
    "evidence",
    "provider",
    "registry",
    "runtime",
  ];
  const entries = readdirSync(run).sort();
  const receipts = entries.filter((entry) => /^[0-9]{2}-[a-z][a-z0-9-]*\.json$/.test(entry));
  const hasReceiptStaging = entries.includes(RECEIPT_STAGING_NAME);
  const hasEnvironment = entries.includes(ENVIRONMENT_NAME);
  const hasEnvironmentStaging = entries.includes(ENVIRONMENT_STAGING_NAME);
  const hasLegacyMutationLease = entries.includes(LEGACY_MUTATION_LEASE_NAME);
  const mutationSlotNames = entries.filter((entry) =>
    /^\.mutation-slot-[0-9]{2}$/.test(entry),
  );
  const mutationCloseNames = entries.filter((entry) =>
    /^\.mutation-close-[0-9]{2}$/.test(entry),
  );
  const mutationRecoveryNames = entries.filter((entry) =>
    /^\.mutation-recovery-[0-9]{2}-[0-9]{2}$/.test(entry),
  );
  const mutationStageNames = entries.filter((entry) =>
    /^\.mutation-stage-[0-9a-f]{32}$/.test(entry),
  );
  if (
    receipts.length < 1 ||
    receipts.length > 64 ||
    mutationSlotNames.length > MAX_MUTATION_SLOTS ||
    mutationCloseNames.length > MAX_MUTATION_SLOTS ||
    mutationRecoveryNames.length > MAX_MUTATION_SLOTS * MAX_MUTATION_RECOVERIES ||
    mutationStageNames.length > MAX_MUTATION_STAGES ||
    required.some((entry) => !entries.includes(entry)) ||
    entries.some(
      (entry) =>
        !required.includes(entry) &&
        !receipts.includes(entry) &&
        entry !== RECEIPT_STAGING_NAME &&
        entry !== ENVIRONMENT_NAME &&
        entry !== ENVIRONMENT_STAGING_NAME &&
        entry !== LEGACY_MUTATION_LEASE_NAME &&
        !mutationSlotNames.includes(entry) &&
        !mutationCloseNames.includes(entry) &&
        !mutationRecoveryNames.includes(entry) &&
        !mutationStageNames.includes(entry),
    )
  ) {
    fail("plan run inventory was refused");
  }
  if (hasLegacyMutationLease) {
    fail("legacy mutation lease was refused; prepare a fresh clean-engine plan");
  }
  for (const directory of ["client", "evidence", "provider", "registry", "runtime"]) {
    const path = join(run, directory);
    const metadata = ownedPrivateDirectory(path, "plan run directory");
    if (metadata.dev !== stateMetadata.dev) fail("plan run crossed a filesystem boundary");
    const children = readdirSync(path);
    if (directory === "client") {
      if (children.length !== 1 || children[0] !== "proxy-template.json") {
        fail("plan client inventory was refused");
      }
    } else if (children.length !== 0) {
      fail("pre-provider plan directory was not empty");
    }
  }
  let pendingPublication;
  if (hasReceiptStaging) {
    const path = join(run, RECEIPT_STAGING_NAME);
    const metadata = inspectPendingFile(
      path,
      "pending receipt publication",
      stateMetadata.dev,
      new Set([1n, 2n]),
    );
    const linkedReceipts = receipts.filter((name) => {
      const candidate = exactLstat(join(run, name));
      return candidate.dev === metadata.dev && candidate.ino === metadata.ino;
    });
    if (
      (metadata.nlink === 1n && linkedReceipts.length !== 0) ||
      (metadata.nlink === 2n && linkedReceipts.length !== 1) ||
      linkedReceipts.includes("00-plan.json")
    ) {
      fail("pending receipt publication link was refused");
    }
    pendingPublication = {
      linkedReceipt: linkedReceipts[0],
      links: Number(metadata.nlink),
      path,
    };
  }
  let environmentPublication;
  if (hasEnvironmentStaging) {
    const path = join(run, ENVIRONMENT_STAGING_NAME);
    const metadata = inspectPendingFile(
      path,
      "pending environment publication",
      stateMetadata.dev,
      new Set([1n, 2n]),
    );
    let linkedEnvironment = false;
    if (hasEnvironment) {
      const candidate = exactLstat(join(run, ENVIRONMENT_NAME));
      linkedEnvironment = candidate.dev === metadata.dev && candidate.ino === metadata.ino;
    }
    if (
      (metadata.nlink === 1n && linkedEnvironment) ||
      (metadata.nlink === 2n && !linkedEnvironment)
    ) {
      fail("pending environment publication link was refused");
    }
    environmentPublication = {
      linkedEnvironment,
      links: Number(metadata.nlink),
      path,
    };
  }
  if (hasEnvironment) {
    const expectedLinks = environmentPublication?.linkedEnvironment ? new Set([2n]) : new Set([1n]);
    inspectPendingFile(
      join(run, ENVIRONMENT_NAME),
      "environment manifest",
      stateMetadata.dev,
      expectedLinks,
    );
  }
  // Resolve mutation staging links before parsing their final destinations.
  // A crash after link(2) but before the staging alias is retired leaves both
  // names at link count two; that is a recoverable publication boundary, not
  // permission for any unrelated hard link.
  const mutationDestinations = [
    ...mutationSlotNames,
    ...mutationCloseNames,
    ...mutationRecoveryNames,
  ];
  const linkedMutationDestinations = new Map();
  const mutationStages = mutationStageNames.map((name) => {
    const path = join(run, name);
    const metadata = inspectPendingFile(
      path,
      "pending mutation publication",
      stateMetadata.dev,
      new Set([1n, 2n]),
    );
    const linkedDestinations = mutationDestinations.filter((destination) => {
      if (!entries.includes(destination)) return false;
      const candidate = exactLstat(join(run, destination));
      return candidate.dev === metadata.dev && candidate.ino === metadata.ino;
    });
    if (
      (metadata.nlink === 1n && linkedDestinations.length !== 0) ||
      (metadata.nlink === 2n && linkedDestinations.length !== 1)
    ) {
      fail("pending mutation publication link was refused");
    }
    const linkedDestination = linkedDestinations[0];
    if (linkedDestination !== undefined) {
      if (linkedMutationDestinations.has(linkedDestination)) {
        fail("pending mutation publication link was refused");
      }
      linkedMutationDestinations.set(linkedDestination, name);
    }
    return { linkedDestination, metadata, name, path };
  });
  const mutationSlots = mutationSlotNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "mutation slot",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  for (const [sequence, slot] of mutationSlots.entries()) {
    if (!onlyLowerHex(slot.value?.fixture_id, 32)) fail("mutation slot was refused");
    validateMutationLeaseValue(slot.value, slot.value.fixture_id);
    if (
      slot.name !== mutationSlotFileName(slot.value.journal_sequence) ||
      slot.value.journal_sequence !== sequence
    ) {
      fail("mutation slot sequence was refused");
    }
  }
  const mutationCloses = mutationCloseNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "mutation close",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  const closesBySlot = new Map();
  for (const close of mutationCloses) {
    const sequence = close.value?.slot_sequence;
    const slot = Number.isSafeInteger(sequence) ? mutationSlots[sequence] : undefined;
    if (
      slot === undefined ||
      close.name !== mutationCloseFileName(sequence) ||
      closesBySlot.has(sequence)
    ) {
      fail("mutation close sequence was refused");
    }
    validateMutationCloseValue(close.value, slot, slot.value.fixture_id);
    closesBySlot.set(sequence, close);
  }
  for (let sequence = 0; sequence < mutationSlots.length - 1; sequence += 1) {
    if (!closesBySlot.has(sequence)) {
      fail("mutation slot journal was not closed in order");
    }
  }
  const allMutationRecoveries = mutationRecoveryNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "mutation recovery claim",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  const recoveriesBySlot = new Map();
  for (const recovery of allMutationRecoveries) {
    const sequence = recovery.value?.slot_sequence;
    if (!Number.isSafeInteger(sequence) || mutationSlots[sequence] === undefined) {
      fail("mutation recovery slot was refused");
    }
    const group = recoveriesBySlot.get(sequence) ?? [];
    group.push(recovery);
    recoveriesBySlot.set(sequence, group);
  }
  for (const [sequence, claims] of recoveriesBySlot) {
    const slot = mutationSlots[sequence];
    if (slot.value.action !== "provider-create" || claims.length > MAX_MUTATION_RECOVERIES) {
      fail("mutation recovery action was refused");
    }
    validateRecoveryClaims(claims, slot.value.fixture_id, digest(slot.bytes));
  }
  const lastSlot = mutationSlots.at(-1);
  const activeMutationSlot =
    lastSlot !== undefined && !closesBySlot.has(lastSlot.value.journal_sequence)
      ? lastSlot
      : undefined;
  const activeMutationRecoveries =
    activeMutationSlot === undefined
      ? []
      : recoveriesBySlot.get(activeMutationSlot.value.journal_sequence) ?? [];
  return {
    activeMutationRecoveries,
    activeMutationSlot,
    allMutationRecoveries,
    environment: hasEnvironment ? join(run, ENVIRONMENT_NAME) : undefined,
    environmentPublication,
    mutationCloses,
    mutationLease: activeMutationSlot,
    mutationRecoveries: activeMutationRecoveries,
    mutationSlots,
    mutationStages,
    pendingPublication,
    receipts,
  };
}

function loadState(roots, checkSource, allowCompetingStaging = false) {
  const entries = readdirSync(roots.stateBase).sort();
  if (
    entries.length > MAX_INERT_STAGING + 2 ||
    !entries.includes("active") ||
    entries.some((entry) =>
      entry !== "active" && !/^\.(?:pending|run)-[0-9a-f]{32}$/.test(entry),
    )
  ) {
    fail("state base inventory was refused");
  }
  const activePlan = parseCanonical(roots.active, "active plan receipt", 2);
  if (!onlyLowerHex(activePlan.value?.fixture_id, 32)) fail("active plan identity was refused");
  const runName = `.run-${activePlan.value.fixture_id}`;
  if (!entries.includes(runName)) fail("state base inventory was refused");
  const run = join(roots.stateBase, runName);
  const stateMetadata = ownedPrivateDirectory(run, "active run state");
  const inventory = validatePlanRunInventory(run, stateMetadata);
  const candidate = parseCanonical(join(run, "candidate.json"), "candidate");
  validateCandidate(candidate.value);
  const plan = parseCanonical(join(run, "00-plan.json"), "plan receipt", 2);
  if (!plan.bytes.equals(activePlan.bytes)) fail("active plan receipt identity was refused");
  const activeMetadata = exactLstat(roots.active);
  const planMetadata = exactLstat(join(run, "00-plan.json"));
  if (
    activeMetadata.dev !== planMetadata.dev ||
    activeMetadata.ino !== planMetadata.ino ||
    activeMetadata.nlink !== 2n ||
    planMetadata.nlink !== 2n
  ) {
    fail("active plan receipt link was refused");
  }
  validatePlan(plan.value, candidate.bytes, stateMetadata);
  if (candidate.value.run_id !== plan.value.fixture_id) fail("state identity was refused");
  if (
    inventory.mutationSlots.some(
      (slot) => slot.value.fixture_id !== candidate.value.run_id,
    ) ||
    inventory.mutationCloses.some(
      (close) => close.value.fixture_id !== candidate.value.run_id,
    ) ||
    inventory.allMutationRecoveries.some(
      (recovery) => recovery.value.fixture_id !== candidate.value.run_id,
    )
  ) {
    fail("mutation journal identity was refused");
  }
  const receipts = [];
  for (const [index, name] of inventory.receipts.entries()) {
    const expectedLinks =
      index === 0 || name === inventory.pendingPublication?.linkedReceipt ? 2 : 1;
    const parsed =
      index === 0
        ? plan
        : parseCanonical(join(run, name), "phase receipt", expectedLinks);
    if (name !== receiptFileName(parsed.value)) fail("phase receipt filename was refused");
    receipts.push(parsed.value);
  }
  let receiptState;
  try {
    receiptState = validateReceiptChain(receipts, candidate.value.run_id);
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  const mutationClosesBySlot = new Map(
    inventory.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  const receiptSequencesBySha256 = new Map(
    receipts.map((receipt) => [digest(canonicalBytes(receipt)), receipt.sequence]),
  );
  for (const [sequence, slot] of inventory.mutationSlots.entries()) {
    const lease = slot.value;
    const sourceReceipt = receipts[lease.source_sequence];
    const providerIntentReceipt = receipts[lease.source_sequence + 1];
    if (
      sourceReceipt === undefined ||
      digest(canonicalBytes(sourceReceipt)) !== lease.source_head_sha256 ||
      (sequence === 0 &&
        (lease.source_sequence !== 0 ||
          lease.source_head_sha256 !== digest(canonicalBytes(plan.value)))) ||
      (lease.action === "provider-create" &&
        (lease.source_sequence !== 0 ||
          lease.intent_receipt_sha256 === ZERO_SHA256 ||
          (providerIntentReceipt !== undefined &&
            (providerIntentReceipt.phase !== "provider-create-intent" ||
              digest(canonicalBytes(providerIntentReceipt)) !==
                lease.intent_receipt_sha256)))) ||
      (lease.action !== "provider-create" && lease.intent_receipt_sha256 !== ZERO_SHA256)
    ) {
      fail("mutation slot receipt binding was refused");
    }
    if (
      sequence === 0 &&
      (lease.previous_close_sha256 !== ZERO_SHA256 ||
        lease.source_environment_sha256 !== ZERO_SHA256)
    ) {
      fail("mutation slot journal root was refused");
    }
    if (sequence > 0) {
      const previousClose = mutationClosesBySlot.get(sequence - 1);
      if (
        previousClose === undefined ||
        lease.previous_close_sha256 !== digest(previousClose.bytes) ||
        lease.source_environment_sha256 !== previousClose.value.result_environment_sha256 ||
        lease.source_sequence !== previousClose.value.result_sequence ||
        lease.source_head_sha256 !== previousClose.value.result_head_sha256
      ) {
        fail("mutation slot journal continuity was refused");
      }
    }
    const close = mutationClosesBySlot.get(sequence);
    if (close === undefined) continue;
    const resultReceipt = receipts[close.value.result_sequence];
    const resultDelta = close.value.result_sequence - lease.source_sequence;
    const slotRecoveries = inventory.allMutationRecoveries.filter(
      (recovery) => recovery.value.slot_sequence === sequence,
    );
    const authorityValid =
      (close.value.authority === "owner" &&
        close.value.authority_sha256 === digest(slot.bytes)) ||
      (close.value.authority === "recovery" &&
        slotRecoveries.some(
          (recovery) => digest(recovery.bytes) === close.value.authority_sha256,
        ));
    if (
      resultReceipt === undefined ||
      digest(canonicalBytes(resultReceipt)) !== close.value.result_head_sha256 ||
      !authorityValid ||
      ((lease.action !== "finalize-environment" ||
        close.value.disposition === "aborted-before-effect") &&
        close.value.result_environment_sha256 !== lease.source_environment_sha256) ||
      (close.value.disposition === "aborted-before-effect" &&
        (close.value.result_sequence !== lease.source_sequence ||
          close.value.result_head_sha256 !== lease.source_head_sha256)) ||
      (close.value.disposition === "completed" &&
        lease.action === "provider-create" &&
        (!new Set([2, 3]).has(resultDelta) ||
          providerIntentReceipt === undefined ||
          (resultDelta === 2 &&
            !new Set([
              "execution-failed",
              "provider-create-failed",
              "provider-create-passed",
            ]).has(resultReceipt.phase)) ||
          (resultDelta === 3 &&
            (receipts[lease.source_sequence + 2]?.phase !== "provider-create-passed" ||
              resultReceipt.phase !== "execution-failed")))) ||
      (close.value.disposition === "completed" &&
        lease.action === "finalize-environment" &&
        (!new Set([0, 1]).has(resultDelta) ||
          resultReceipt.phase !== "finalize-passed" ||
          (resultDelta === 0 && sourceReceipt.phase !== "finalize-passed") ||
          close.value.result_environment_sha256 === ZERO_SHA256)) ||
      (close.value.disposition === "completed" &&
        lease.action === "append-receipt" &&
        (resultDelta > 2 ||
          (resultDelta === 2 && !resultReceipt.phase.endsWith("-failed"))))
    ) {
      fail("mutation close receipt binding was refused");
    }
  }
  const lastRecoverySourceBySlot = new Map();
  for (const recovery of inventory.allMutationRecoveries) {
    const slot = inventory.mutationSlots[recovery.value.slot_sequence];
    const close = mutationClosesBySlot.get(recovery.value.slot_sequence);
    const sourceSequence = receiptSequencesBySha256.get(recovery.value.source_head_sha256);
    if (
      sourceSequence === undefined ||
      sourceSequence < slot.value.source_sequence ||
      (close !== undefined && sourceSequence > close.value.result_sequence) ||
      sourceSequence < (lastRecoverySourceBySlot.get(recovery.value.slot_sequence) ?? 0)
    ) {
      fail("mutation recovery receipt binding was refused");
    }
    lastRecoverySourceBySlot.set(recovery.value.slot_sequence, sourceSequence);
  }
  const latestMutationSlot = inventory.mutationSlots.at(-1);
  const latestMutationClose =
    latestMutationSlot === undefined
      ? undefined
      : mutationClosesBySlot.get(latestMutationSlot.value.journal_sequence);
  if (latestMutationSlot === undefined && receiptState.head.sequence !== 0) {
    fail("receipt head was outside the mutation journal");
  }
  if (
    latestMutationClose !== undefined &&
    (latestMutationClose.value.result_sequence !== receiptState.head.sequence ||
      latestMutationClose.value.result_head_sha256 !== receiptState.head_sha256)
  ) {
    fail("closed mutation journal did not cover the receipt head");
  }
  if (inventory.mutationLease !== undefined) {
    const lease = inventory.mutationLease.value;
    const delta = receiptState.head.sequence - lease.source_sequence;
    if (
      delta < 0 ||
      (lease.action === "append-receipt" &&
        (delta > 2 ||
          (delta === 2 && !receiptState.head.phase.endsWith("-failed")))) ||
      (lease.action === "provider-create" &&
        (delta > 3 ||
          (delta === 1 && receiptState.head.phase !== "provider-create-intent") ||
          (delta === 2 &&
            !new Set([
              "execution-failed",
              "provider-create-failed",
              "provider-create-passed",
            ]).has(receiptState.head.phase)) ||
          (delta === 3 &&
            (receipts[lease.source_sequence + 2]?.phase !== "provider-create-passed" ||
              receiptState.head.phase !== "execution-failed")))) ||
      (lease.action === "finalize-environment" &&
        (delta > 1 ||
          (delta === 1 && receiptState.head.phase !== "finalize-passed")))
    ) {
      fail("open mutation slot did not cover the receipt head");
    }
  }
  if (
    inventory.pendingPublication !== undefined &&
    inventory.mutationLease === undefined
  ) {
    fail("pending receipt publication was outside an open mutation slot");
  }
  if (
    inventory.environmentPublication !== undefined &&
    inventory.mutationLease?.value.action !== "finalize-environment"
  ) {
    fail("pending environment publication was outside a finalization slot");
  }
  for (const receipt of receipts) {
    let requiredAction;
    if (receipt.phase.startsWith("provider-create-")) requiredAction = "provider-create";
    if (receipt.phase === "finalize-passed") requiredAction = "finalize-environment";
    if (requiredAction === undefined) continue;
    const owner = inventory.mutationSlots.find((slot) => {
      const close = mutationClosesBySlot.get(slot.value.journal_sequence);
      const resultSequence = close?.value.result_sequence ?? receiptState.head.sequence;
      return receipt.sequence > slot.value.source_sequence && receipt.sequence <= resultSequence;
    });
    if (owner?.value.action !== requiredAction) {
      fail(`${receipt.phase} receipt was outside its mutation action`);
    }
  }
  const finalized = receiptState.head.phase === "finalize-passed";
  const manifestReceipts = finalized ? receipts.slice(0, -1) : receipts;
  let manifestState;
  try {
    manifestState = validateReceiptChain(manifestReceipts, candidate.value.run_id);
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  if (
    (inventory.environment !== undefined || inventory.environmentPublication !== undefined) &&
    !manifestState.manifest_eligible
  ) {
    fail("environment manifest publication was not eligible");
  }
  let environment;
  if (inventory.environment !== undefined) {
    const expectedLinks = inventory.environmentPublication?.linkedEnvironment ? 2 : 1;
    environment = parseCanonical(inventory.environment, "environment manifest", expectedLinks);
    let expected;
    try {
      expected = canonicalBytes(
        buildEnvironmentManifest(candidate.value, candidate.bytes, manifestReceipts),
      );
    } catch (error) {
      if (error instanceof ReceiptFailure) fail(error.message);
      throw error;
    }
    if (!environment.bytes.equals(expected)) fail("environment manifest content was refused");
    if (
      finalized &&
      receiptState.head.result.environment_manifest_sha256 !== digest(environment.bytes)
    ) {
      fail("final environment manifest digest was refused");
    }
  } else if (finalized) {
    fail("final environment manifest was unavailable", 69);
  }
  const environmentSha256 =
    environment === undefined ? ZERO_SHA256 : digest(environment.bytes);
  if (latestMutationSlot === undefined && environmentSha256 !== ZERO_SHA256) {
    fail("environment manifest was outside the mutation journal");
  }
  if (
    latestMutationClose !== undefined &&
    latestMutationClose.value.result_environment_sha256 !== environmentSha256
  ) {
    fail("closed mutation journal did not cover the environment manifest");
  }
  if (
    inventory.mutationLease !== undefined &&
    ((inventory.mutationLease.value.action !== "finalize-environment" &&
      inventory.mutationLease.value.source_environment_sha256 !== environmentSha256) ||
      (inventory.mutationLease.value.action === "finalize-environment" &&
        inventory.mutationLease.value.source_environment_sha256 !== ZERO_SHA256 &&
        inventory.mutationLease.value.source_environment_sha256 !== environmentSha256))
  ) {
    fail("open mutation slot did not cover the environment manifest");
  }
  validateProxyTemplate(join(run, "client", "proxy-template.json"));
  if (!allowCompetingStaging) {
    for (const entry of entries) {
      if (entry !== "active" && entry !== runName) {
        validateInertStaging(join(roots.stateBase, entry));
      }
    }
  }
  if (checkSource) {
    const current = sourceClosure(roots.repoRoot);
    if (canonical(current) !== canonical(candidate.value.source)) fail("source closure changed");
  }
  return {
    candidate: candidate.value,
    candidateBytes: candidate.bytes,
    environment,
    environmentPublication: inventory.environmentPublication,
    plan: plan.value,
    receiptState,
    receipts,
    run,
    allMutationRecoveries: inventory.allMutationRecoveries,
    mutationCloses: inventory.mutationCloses,
    mutationLease: inventory.mutationLease,
    mutationRecoveries: inventory.mutationRecoveries,
    mutationSlots: inventory.mutationSlots,
    mutationStages: inventory.mutationStages,
    pendingPublication: inventory.pendingPublication,
  };
}

function plan(roots, values) {
  if (values.provider !== "colima") fail("provider must be colima", 64);
  if (!privateIpv4Pool(values["ipv4-pool"])) fail("IPv4 pool must be a canonical private /24", 64);
  const initialEntries = readdirSync(roots.stateBase);
  if (initialEntries.includes("active")) fail("an active clean-engine plan already exists", 73);
  if (initialEntries.length > MAX_INERT_STAGING) fail("inert staging limit was exceeded", 73);
  for (const entry of initialEntries) {
    if (!/^\.(?:pending|run)-[0-9a-f]{32}$/.test(entry)) {
      fail("state base inventory was refused");
    }
    validateInertStaging(join(roots.stateBase, entry));
  }
  const runId = randomBytes(16).toString("hex");
  const pending = join(roots.stateBase, `.pending-${runId}`);
  const run = join(roots.stateBase, `.run-${runId}`);
  let pendingCreated = false;
  let pendingIdentity;
  let runPublished = false;
  let published = false;
  try {
    mkdirSync(pending, { mode: 0o700 });
    pendingCreated = true;
    pendingIdentity = exactLstat(pending);
    for (const directory of ["client", "evidence", "provider", "registry", "runtime"]) {
      mkdirSync(join(pending, directory), { mode: 0o700 });
    }
    syncDirectory(pending);
    syncDirectory(roots.stateBase);
    const stateMetadata = ownedPrivateDirectory(pending, "pending state");
    const suffix = `acceptance-${runId.slice(0, 24)}`;
    const candidate = {
      created_at: new Date().toISOString(),
      excluded_claims: EXCLUDED_CLAIMS,
      feature: "CPR-45",
      fixtures: {
        builder_canary: "ambient-remote-inert-zero-read-v1",
        docker_proxy: "synthetic-nonsecret-v1",
        registry_authentication: "one-run-basic-bcrypt",
        registry_image: REGISTRY_IMAGE,
        registry_transport: "loopback-tls-ephemeral",
      },
      kind: "synveda-cpr45-clean-engine-candidate",
      requested_assertions: REQUESTED_ASSERTIONS,
      run_id: runId,
      schema_version: 1,
      selection: {
        app_host: "app.synveda.test",
        auth_host: "auth.synveda.test",
        ipv4_pool: values["ipv4-pool"],
        oidc: "bundled",
        port: 8080,
        postgres: "bundled",
        profiles: ["browser-acceptance", "demo"],
        project: `synveda-development-${suffix}`,
        project_suffix: suffix,
        runtime: "development",
        scheme: "http",
      },
      source: sourceClosure(roots.repoRoot),
    };
    validateCandidate(candidate);
    const candidateBytes = canonicalBytes(candidate);
    writeExclusive(join(pending, "candidate.json"), candidateBytes);
    writeExclusive(
      join(pending, "client", "proxy-template.json"),
      canonicalBytes(PROXY_TEMPLATE),
    );
    const receipt = {
      fixture_id: runId,
      outcome: "passed",
      phase: "plan",
      previous_sha256: ZERO_SHA256,
      result: {
        candidate_sha256: digest(candidateBytes),
        project: candidate.selection.project,
        provider: "colima",
        provider_resource: `synveda-cpr45-${runId}`,
        state_device: String(stateMetadata.dev),
        state_inode: String(stateMetadata.ino),
      },
      schema: "synveda.clean-engine.receipt.v2",
      sequence: 0,
    };
    writeExclusive(join(pending, "00-plan.json"), canonicalBytes(receipt));
    syncDirectory(pending);
    try {
      renameSync(pending, run);
    } catch {
      fail("completed run publication failed", 70);
    }
    runPublished = true;
    syncDirectory(roots.stateBase);
    try {
      linkSync(join(run, "00-plan.json"), roots.active);
    } catch {
      if (existsSync(roots.active)) fail("an active clean-engine plan already exists", 73);
      fail("active plan publication failed", 70);
    }
    published = true;
    syncDirectory(roots.stateBase);
    loadState(roots, true, true);
    process.stdout.write(
      `clean-engine: plan ${runId} prepared for ${candidate.selection.project}\n`,
    );
  } catch (error) {
    if (pendingCreated && !published) {
      try {
        const cleanupPath = runPublished ? run : pending;
        const current = exactLstat(cleanupPath);
        if (
          current.isSymbolicLink() ||
          !current.isDirectory() ||
          current.dev !== pendingIdentity.dev ||
          current.ino !== pendingIdentity.ino ||
          current.uid !== OWNER_UID
        ) {
          throw new Error("pending state identity changed");
        }
        rmSync(cleanupPath, { recursive: true, force: false });
        syncDirectory(roots.stateBase);
      } catch {
        process.stderr.write("clean-engine: failed plan state was retained for inspection\n");
      }
    } else if (published) {
      process.stderr.write("clean-engine: published plan state was retained for inspection\n");
    }
    throw error;
  }
}

function activeMutationRun(roots) {
  const active = parseCanonical(roots.active, "active plan receipt", 2);
  if (!onlyLowerHex(active.value?.fixture_id, 32)) fail("active plan identity was refused");
  const run = join(roots.stateBase, `.run-${active.value.fixture_id}`);
  ownedPrivateDirectory(run, "active run state");
  return { fixtureId: active.value.fixture_id, run };
}

function mutationHeldIdentity(path) {
  const metadata = exactLstat(path);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== OWNER_UID ||
    metadata.nlink !== 1n ||
    (metadata.mode & 0o7777n) !== 0o600n
  ) {
    fail("mutation slot identity was refused");
  }
  return metadata;
}

function sameMutationArtifact(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.uid === right.uid &&
    left.mode === right.mode &&
    left.size === right.size
  );
}

function retireUnlinkedMutationStage(run, stagePath, identity) {
  const runMetadata = ownedPrivateDirectory(run, "active run state");
  let current;
  try {
    current = inspectPendingFile(
      stagePath,
      "pending mutation publication",
      runMetadata.dev,
      new Set([1n]),
    );
  } catch (error) {
    if (mutationStageWasRemoved(stagePath)) return;
    throw error;
  }
  if (!sameMetadata(identity, current)) {
    fail("pending mutation publication identity changed");
  }
  try {
    unlinkSync(stagePath);
    syncDirectory(run);
  } catch (error) {
    if (error?.code === "ENOENT") return;
    fail("pending mutation publication retirement failed", 70);
  }
}

function mutationStageWasRemoved(stagePath) {
  try {
    exactLstat(stagePath);
    return false;
  } catch (error) {
    if (error?.code === "ENOENT") return true;
    fail("pending mutation publication was unavailable", 69);
  }
}

// Publish a complete, fsynced blocker through an unguessable private staging
// name. The final name is created atomically with link(2), so interruption can
// leave only an inert one-link stage or a valid two-link final artifact.
function publishMutationBlocker(
  run,
  destinationName,
  bytes,
  {
    afterLinkMilliseconds = 0,
    beforeLinkMilliseconds = 0,
    reassertAuthority,
  } = {},
) {
  if (
    !/^\.mutation-(?:slot|close)-[0-9]{2}$/.test(destinationName) &&
    !/^\.mutation-recovery-[0-9]{2}-[0-9]{2}$/.test(destinationName)
  ) {
    fail("mutation publication destination was refused", 70);
  }
  for (const milliseconds of [afterLinkMilliseconds, beforeLinkMilliseconds]) {
    if (
      !Number.isSafeInteger(milliseconds) ||
      milliseconds < 0 ||
      milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds
    ) {
      fail("mutation publication hold was refused", 70);
    }
  }
  if (reassertAuthority !== undefined && typeof reassertAuthority !== "function") {
    fail("mutation publication authority was refused", 70);
  }
  const runMetadata = ownedPrivateDirectory(run, "active run state");
  const destinationPath = join(run, destinationName);
  for (let attempt = 0; attempt < MAX_MUTATION_PUBLICATION_ATTEMPTS; attempt += 1) {
    const stagePath = join(run, `${MUTATION_STAGE_PREFIX}${randomBytes(16).toString("hex")}`);
    writeExclusive(stagePath, bytes);
    let staged;
    try {
      staged = inspectPendingFile(
        stagePath,
        "pending mutation publication",
        runMetadata.dev,
        new Set([1n]),
      );
      if (!readPrivate(stagePath, "pending mutation publication", 1, 0n).equals(bytes)) {
        fail("pending mutation publication content changed");
      }
    } catch (error) {
      if (!mutationStageWasRemoved(stagePath)) throw error;
      reassertAuthority?.();
      continue;
    }
    holdFakeProvider(beforeLinkMilliseconds);
    try {
      linkSync(stagePath, destinationPath);
    } catch (error) {
      if (error?.code === "EEXIST") {
        retireUnlinkedMutationStage(run, stagePath, staged);
        return false;
      }
      if (error?.code === "ENOENT") {
        reassertAuthority?.();
        continue;
      }
      retireUnlinkedMutationStage(run, stagePath, staged);
      fail("mutation blocker publication failed", 70);
    }
    try {
      syncDirectory(run);
      holdFakeProvider(afterLinkMilliseconds);
      const linkedDestination = exactLstat(destinationPath);
      if (
        !linkedDestination.isFile() ||
        linkedDestination.isSymbolicLink() ||
        linkedDestination.uid !== OWNER_UID ||
        linkedDestination.dev !== runMetadata.dev ||
        !new Set([1n, 2n]).has(linkedDestination.nlink) ||
        (linkedDestination.mode & 0o7777n) !== 0o600n ||
        !sameMutationArtifact(staged, linkedDestination)
      ) {
        fail("mutation blocker publication identity changed");
      }
      try {
        const linkedStage = exactLstat(stagePath);
        if (
          !linkedStage.isFile() ||
          linkedStage.isSymbolicLink() ||
          linkedStage.nlink !== 2n ||
          !sameMutationArtifact(staged, linkedStage)
        ) {
          fail("mutation blocker publication identity changed");
        }
        unlinkSync(stagePath);
      } catch (error) {
        if (error?.code !== "ENOENT") throw error;
      }
      syncDirectory(run);
      if (!readPrivate(destinationPath, "published mutation blocker", 1, 0n).equals(bytes)) {
        fail("mutation blocker publication content changed");
      }
    } catch (error) {
      if (error instanceof ClosedFailure) throw error;
      fail("mutation blocker publication failed", 70);
    }
    return true;
  }
  fail("mutation publication was repeatedly superseded", 73);
}

function reconcileMutationStages(roots) {
  const state = loadState(roots, false);
  if (state.mutationStages.length === 0) return state;
  const runMetadata = ownedPrivateDirectory(state.run, "active run state");
  for (const stage of state.mutationStages) {
    const expectedLinks = stage.linkedDestination === undefined ? 1n : 2n;
    const current = inspectPendingFile(
      stage.path,
      "pending mutation publication",
      runMetadata.dev,
      new Set([expectedLinks]),
    );
    if (!sameMetadata(stage.metadata, current)) {
      fail("pending mutation publication identity changed");
    }
    if (stage.linkedDestination !== undefined) {
      const destination = inspectPendingFile(
        join(state.run, stage.linkedDestination),
        "published mutation blocker",
        runMetadata.dev,
        new Set([2n]),
      );
      if (!sameMutationArtifact(current, destination)) {
        fail("pending mutation publication link was refused");
      }
    }
    try {
      unlinkSync(stage.path);
      syncDirectory(state.run);
    } catch {
      fail("pending mutation publication recovery failed", 70);
    }
  }
  return loadState(roots, false);
}

function acquireMutationLease(
  roots,
  action,
  intentReceiptSha256 = ZERO_SHA256,
  expectedSource,
  publicationHolds = {},
) {
  const active = activeMutationRun(roots);
  const initial = reconcileMutationStages(roots);
  if (initial.mutationRecoveries.length > 0) {
    fail("a clean-engine mutation recovery is active or abandoned", 73);
  }
  if (initial.mutationLease !== undefined) {
    const ownerState = mutationOwnerState(initial.mutationLease.value);
    if (ownerState === "current" || ownerState === "unknown") {
      fail("another clean-engine mutation is active or could not be identified", 73);
    }
    fail("an abandoned clean-engine mutation requires explicit recovery", 73);
  }
  if (!new Set(["append-receipt", "finalize-environment", "provider-create"]).has(action)) {
    fail("mutation action was refused", 70);
  }
  if (!onlyLowerHex(intentReceiptSha256, 64)) fail("mutation intent binding was refused", 70);
  const journalSequence = initial.mutationSlots.length;
  if (journalSequence >= MAX_MUTATION_SLOTS) {
    fail("mutation slot journal capacity was exhausted", 73);
  }
  if (
    expectedSource !== undefined &&
    (initial.receiptState.head.sequence !== expectedSource.sequence ||
      initial.receiptState.head_sha256 !== expectedSource.sha256)
  ) {
    fail("mutation source head changed", 73);
  }
  const owner = currentProcessIdentity();
  const previousClose = initial.mutationCloses.at(-1);
  const lease = {
    action,
    fixture_id: active.fixtureId,
    intent_receipt_sha256: intentReceiptSha256,
    journal_sequence: journalSequence,
    nonce: randomBytes(16).toString("hex"),
    owner_boot_sha256: owner.boot_sha256,
    owner_instance_sha256: owner.instance_sha256,
    owner_pid: owner.pid,
    owner_probe: owner.probe,
    previous_close_sha256:
      previousClose === undefined ? ZERO_SHA256 : digest(previousClose.bytes),
    schema: MUTATION_SLOT_SCHEMA,
    source_environment_sha256:
      initial.environment === undefined ? ZERO_SHA256 : digest(initial.environment.bytes),
    source_head_sha256: initial.receiptState.head_sha256,
    source_sequence: initial.receiptState.head.sequence,
  };
  const leaseBytes = canonicalBytes(lease);
  const leaseName = mutationSlotFileName(journalSequence);
  const leasePath = join(active.run, leaseName);
  if (!publishMutationBlocker(
    active.run,
    leaseName,
    leaseBytes,
    publicationHolds,
  )) {
    fail("another clean-engine mutation is active", 73);
  }
  const leaseIdentity = mutationHeldIdentity(leasePath);
  const held = Object.freeze({
    active,
    identity: leaseIdentity,
    lease,
    leaseBytes,
    path: leasePath,
  });
  const verified = loadState(roots, false);
  if (
    verified.mutationLease === undefined ||
    !verified.mutationLease.bytes.equals(leaseBytes) ||
    verified.mutationRecoveries.length !== 0
  ) {
    fail("mutation slot publication was not durable", 70);
  }
  if (
    expectedSource !== undefined &&
    (verified.receiptState.head.sequence !== expectedSource.sequence ||
      verified.receiptState.head_sha256 !== expectedSource.sha256)
  ) {
    fail("mutation source head changed", 73);
  }
  return held;
}

function assertMutationLeaseHeld(roots, held, allowRecovery = false) {
  const current = parseCanonical(held.path, "mutation slot");
  const metadata = mutationHeldIdentity(held.path);
  if (!sameMetadata(held.identity, metadata) || !current.bytes.equals(held.leaseBytes)) {
    fail("mutation slot identity changed");
  }
  const state = loadState(roots, false);
  if (
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(held.leaseBytes) ||
    (!allowRecovery && state.mutationRecoveries.length !== 0)
  ) {
    fail("mutation slot ownership was refused", 73);
  }
  return state;
}

function publishMutationClose(
  roots,
  state,
  slot,
  disposition,
  reassertAuthority,
  publicationHolds = {},
) {
  if (
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(slot.bytes) ||
    state.pendingPublication !== undefined ||
    state.environmentPublication !== undefined
  ) {
    fail("mutation close ownership was refused", 73);
  }
  const result = state.receiptState;
  const resultEnvironmentSha256 =
    state.environment === undefined ? ZERO_SHA256 : digest(state.environment.bytes);
  if (
    disposition === "aborted-before-effect" &&
    (result.head.sequence !== slot.value.source_sequence ||
      result.head_sha256 !== slot.value.source_head_sha256 ||
      resultEnvironmentSha256 !== slot.value.source_environment_sha256)
  ) {
    fail("mutation close disposition was refused", 70);
  }
  if (
    disposition === "completed" &&
    ((slot.value.action === "provider-create" &&
      !new Set([
        "execution-failed",
        "provider-create-failed",
        "provider-create-passed",
      ]).has(result.head.phase)) ||
      (slot.value.action === "finalize-environment" &&
        result.head.phase !== "finalize-passed"))
  ) {
    fail("mutation close result was refused", 70);
  }
  const recoveries = state.mutationRecoveries;
  const authority = recoveries.length === 0 ? "owner" : "recovery";
  const authoritySha256 =
    authority === "owner" ? digest(slot.bytes) : digest(recoveries.at(-1).bytes);
  const close = {
    authority,
    authority_sha256: authoritySha256,
    disposition,
    fixture_id: slot.value.fixture_id,
    result_environment_sha256: resultEnvironmentSha256,
    result_head_sha256: result.head_sha256,
    result_sequence: result.head.sequence,
    schema: MUTATION_CLOSE_SCHEMA,
    slot_sequence: slot.value.journal_sequence,
    slot_sha256: digest(slot.bytes),
  };
  validateMutationCloseValue(close, slot, slot.value.fixture_id);
  const closeBytes = canonicalBytes(close);
  const closeName = mutationCloseFileName(slot.value.journal_sequence);
  const reassertClosePublication = () => {
    const current = reassertAuthority();
    const currentEnvironmentSha256 =
      current.environment === undefined ? ZERO_SHA256 : digest(current.environment.bytes);
    if (
      current.mutationLease === undefined ||
      !current.mutationLease.bytes.equals(slot.bytes) ||
      current.pendingPublication !== undefined ||
      current.environmentPublication !== undefined ||
      current.receiptState.head.sequence !== close.result_sequence ||
      current.receiptState.head_sha256 !== close.result_head_sha256 ||
      currentEnvironmentSha256 !== close.result_environment_sha256
    ) {
      fail("mutation close authority changed", 73);
    }
  };
  const published = publishMutationBlocker(state.run, closeName, closeBytes, {
    ...publicationHolds,
    reassertAuthority: reassertClosePublication,
  });
  if (!published) {
    const existing = parseCanonical(join(state.run, closeName), "mutation close");
    if (!existing.bytes.equals(closeBytes)) {
      fail("mutation close publication conflicted", 73);
    }
  }
  const verified = loadState(roots, false);
  const recorded = verified.mutationCloses.find(
    (candidate) => candidate.value.slot_sequence === slot.value.journal_sequence,
  );
  if (
    recorded === undefined ||
    !recorded.bytes.equals(closeBytes)
  ) {
    fail("mutation close publication was not durable", 70);
  }
  return verified;
}

function closeMutationLease(roots, held, disposition, publicationHolds = {}) {
  assertMutationLeaseHeld(roots, held, false);
  reconcileReceiptPublication(roots);
  reconcileEnvironmentPublication(roots);
  const state = assertMutationLeaseHeld(roots, held, false);
  return publishMutationClose(
    roots,
    state,
    state.mutationLease,
    disposition,
    () => assertMutationLeaseHeld(roots, held, false),
    publicationHolds,
  );
}

function withMutationLease(roots, action, callback) {
  const held = acquireMutationLease(roots, action);
  let result;
  let callbackError;
  try {
    result = callback();
  } catch (error) {
    callbackError = error;
  }
  assertMutationLeaseHeld(roots, held, false);
  reconcileReceiptPublication(roots);
  reconcileEnvironmentPublication(roots);
  const state = assertMutationLeaseHeld(roots, held, false);
  const receiptChanged =
    state.receiptState.head.sequence !== held.lease.source_sequence ||
    state.receiptState.head_sha256 !== held.lease.source_head_sha256;
  const environmentChanged =
    (state.environment === undefined ? ZERO_SHA256 : digest(state.environment.bytes)) !==
    held.lease.source_environment_sha256;
  if (
    callbackError !== undefined &&
    held.lease.action === "finalize-environment" &&
    environmentChanged &&
    !receiptChanged
  ) {
    throw callbackError;
  }
  closeMutationLease(
    roots,
    held,
    callbackError !== undefined && !receiptChanged && !environmentChanged
      ? "aborted-before-effect"
      : "completed",
  );
  if (callbackError !== undefined) throw callbackError;
  return result;
}

function validateFakeProviderAdapter(adapter) {
  exactKeys(
    adapter,
    FAKE_PROVIDER_ADAPTER_FIELDS,
    "fake provider adapter",
  );
  if (
    adapter.kind !== FAKE_PROVIDER_CONTRACT.kind ||
    !Number.isSafeInteger(adapter.close_prelink_hold_milliseconds) ||
    adapter.close_prelink_hold_milliseconds < 0 ||
    adapter.close_prelink_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.hold_milliseconds) ||
    adapter.hold_milliseconds < 0 ||
    adapter.hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.prelink_hold_milliseconds) ||
    adapter.prelink_hold_milliseconds < 0 ||
    adapter.prelink_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.publication_hold_milliseconds) ||
    adapter.publication_hold_milliseconds < 0 ||
    adapter.publication_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.reconcile_hold_milliseconds) ||
    adapter.reconcile_hold_milliseconds < 0 ||
    adapter.reconcile_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !new Set(["failed", "passed"]).has(adapter.execute_outcome) ||
    !new Set(["failed", "passed", "unknown"]).has(adapter.reconcile_outcome) ||
    adapter.execute_result === null ||
    Array.isArray(adapter.execute_result) ||
    typeof adapter.execute_result !== "object" ||
    adapter.reconcile_result === null ||
    Array.isArray(adapter.reconcile_result) ||
    typeof adapter.reconcile_result !== "object"
  ) {
    fail("fake provider adapter was refused", 64);
  }
}

function providerIntentResult(fixtureId) {
  return {
    cleanup_command: "colima-delete-data-force",
    preexisting_resource: "absent",
    provider_contract_sha256: FAKE_PROVIDER_CONTRACT_SHA256,
    provider_resource: `synveda-cpr45-${fixtureId}`,
    provider_root_key: `sv-c45-${fixtureId.slice(0, 16)}`,
  };
}

function providerAdapterReceipt(adapterResult) {
  if (
    adapterResult === null ||
    typeof adapterResult !== "object" ||
    adapterResult.then !== undefined ||
    !new Set(["failed", "passed"]).has(adapterResult.outcome) ||
    adapterResult.result === null ||
    Array.isArray(adapterResult.result) ||
    typeof adapterResult.result !== "object"
  ) {
    fail("fake provider result was refused", 70);
  }
  return {
    phase: adapterResult.outcome === "passed" ? "provider-create-passed" : "provider-create-failed",
    result: adapterResult.result,
  };
}

function holdFakeProvider(milliseconds) {
  if (milliseconds === 0) return;
  const cell = new Int32Array(new SharedArrayBuffer(4));
  Atomics.wait(cell, 0, 0, milliseconds);
}

function refusedProviderResult(safeCode = "evidence-refused") {
  return {
    cleanup_required: true,
    collision_resource: "none",
    resource_disposition: "receipt-owned-or-absent",
    safe_code: safeCode,
  };
}

export function executeProviderCreateForExecutor(argumentsValue) {
  validateFakeProviderAdapter(argumentsValue.adapter);
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const closePublicationHolds = {
    beforeLinkMilliseconds: argumentsValue.adapter.close_prelink_hold_milliseconds,
  };
  const initial = loadState(roots, true);
  if (
    initial.receiptState.head.phase !== "plan" ||
    initial.pendingPublication !== undefined ||
    initial.environment !== undefined ||
    initial.environmentPublication !== undefined
  ) {
    fail("provider creation state was refused", 73);
  }
  const intentResult = providerIntentResult(initial.candidate.run_id);
  let intendedReceipt;
  try {
    intendedReceipt = createNextReceipt(
      initial.receipts,
      initial.candidate.run_id,
      "provider-create-intent",
      intentResult,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  const held = acquireMutationLease(
    roots,
    "provider-create",
    digest(canonicalBytes(intendedReceipt)),
    {
      sequence: initial.receiptState.head.sequence,
      sha256: initial.receiptState.head_sha256,
    },
    {
      afterLinkMilliseconds: argumentsValue.adapter.publication_hold_milliseconds,
      beforeLinkMilliseconds: argumentsValue.adapter.prelink_hold_milliseconds,
    },
  );
  if (
    held.lease.source_sequence !== initial.receiptState.head.sequence ||
    held.lease.source_head_sha256 !== initial.receiptState.head_sha256
  ) {
    closeMutationLease(roots, held, "aborted-before-effect", closePublicationHolds);
    fail("provider creation source head changed", 73);
  }
  // Provider exceptions deliberately keep the durable slot open. Only an
  // exact result receipt, or explicit identity-bound recovery, may close it.
  appendReceiptWithLease(roots, { phase: "provider-create-intent", result: intentResult });
  assertMutationLeaseHeld(roots, held);
  holdFakeProvider(argumentsValue.adapter.hold_milliseconds);
  assertMutationLeaseHeld(roots, held);
  const next = providerAdapterReceipt({
    outcome: argumentsValue.adapter.execute_outcome,
    result: argumentsValue.adapter.execute_result,
  });
  const receipt = appendReceiptWithLease(roots, next);
  assertMutationLeaseHeld(roots, held);
  closeMutationLease(roots, held, "completed", closePublicationHolds);
  return receipt;
}

function providerRecoveryBase(state) {
  const lease = state.mutationLease;
  if (lease === undefined) {
    fail("no abandoned provider mutation was available", 73);
  }
  if (lease.value.action !== "provider-create") {
    fail("mutation recovery action was refused", 73);
  }
  return {
    fixtureId: lease.value.fixture_id,
    lease,
    leaseSha256: digest(lease.bytes),
    slotSequence: lease.value.journal_sequence,
  };
}

function providerRecoveryConfirmation(base) {
  return `recover:${base.fixtureId}:${String(base.slotSequence).padStart(2, "0")}:${base.leaseSha256}`;
}

export function providerRecoveryConfirmationForExecutor(argumentsValue) {
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const state = reconcileMutationStages(roots);
  const base = providerRecoveryBase(state);
  return providerRecoveryConfirmation(base);
}

function acquireProviderRecovery(roots, confirmation, ownerProbe, publicationHolds = {}) {
  let state = reconcileMutationStages(roots);
  const base = providerRecoveryBase(state);
  if (confirmation !== providerRecoveryConfirmation(base)) {
    fail("provider recovery confirmation was refused", 64);
  }
  const latest = state.mutationRecoveries.at(-1);
  if (latest !== undefined) {
    const latestState = mutationOwnerState(latest.value, ownerProbe);
    if (latestState === "current" || latestState === "unknown") {
      fail("another provider recovery is active or could not be identified", 73);
    }
  } else {
    const leaseOwnerState = mutationOwnerState(base.lease.value, ownerProbe);
    if (leaseOwnerState === "current" || leaseOwnerState === "unknown") {
      fail("provider mutation owner is active or could not be identified", 73);
    }
  }
  const sequence = (latest?.value.sequence ?? -1) + 1;
  if (sequence >= MAX_MUTATION_RECOVERIES) {
    fail("provider mutation recovery capacity was exhausted", 73);
  }
  const owner = currentProcessIdentity();
  const claim = {
    action: "provider-create",
    chain_root_sha256: recoveryChainRootSha256(base.fixtureId, base.leaseSha256),
    fixture_id: base.fixtureId,
    lease_sha256: base.leaseSha256,
    nonce: randomBytes(16).toString("hex"),
    owner_boot_sha256: owner.boot_sha256,
    owner_instance_sha256: owner.instance_sha256,
    owner_pid: owner.pid,
    owner_probe: owner.probe,
    parent_sha256: latest === undefined ? ZERO_SHA256 : digest(latest.bytes),
    schema: MUTATION_RECOVERY_SCHEMA,
    sequence,
    slot_sequence: base.slotSequence,
    source_head_sha256: state.receiptState.head_sha256,
  };
  const bytes = canonicalBytes(claim);
  const name = recoveryFileName(base.slotSequence, sequence);
  const path = join(state.run, name);
  if (!publishMutationBlocker(state.run, name, bytes, publicationHolds)) {
    fail("another provider recovery won the mutation claim", 73);
  }
  const identity = mutationHeldIdentity(path);
  state = loadState(roots, false);
  const durableClaim = state.allMutationRecoveries.find(
    (candidate) => candidate.name === name && candidate.bytes.equals(bytes),
  );
  if (durableClaim === undefined) {
    fail("provider recovery claim publication was not durable", 70);
  }
  if (state.mutationLease === undefined) {
    fail("provider mutation closed before the recovery claim became authoritative", 73);
  }
  const published = state.mutationRecoveries.at(-1);
  if (published === undefined || !published.bytes.equals(bytes)) {
    fail("provider recovery claim ownership was refused", 73);
  }
  const held = Object.freeze({ base, bytes, claim, identity, path });
  const predecessorPresent =
    state.mutationLease?.bytes.equals(base.lease.bytes) === true &&
    (latest === undefined ||
      state.mutationRecoveries.some((recovery) => recovery.bytes.equals(latest.bytes)));
  if (
    !predecessorPresent ||
    state.receiptState.head_sha256 !== claim.source_head_sha256
  ) {
    fail("provider recovery source changed", 73);
  }
  return held;
}

function assertProviderRecoveryHeld(roots, held) {
  const current = parseCanonical(held.path, "mutation recovery claim");
  const metadata = mutationHeldIdentity(held.path);
  if (!sameMetadata(held.identity, metadata) || !current.bytes.equals(held.bytes)) {
    fail("provider recovery claim identity changed");
  }
  const state = loadState(roots, false);
  const latest = state.mutationRecoveries.at(-1);
  if (latest === undefined || !latest.bytes.equals(held.bytes)) {
    fail("provider recovery claim ownership was refused", 73);
  }
  return state;
}

function closeRecoveredProviderMutation(roots, held, disposition, publicationHolds = {}) {
  assertProviderRecoveryHeld(roots, held);
  reconcileReceiptPublication(roots);
  reconcileEnvironmentPublication(roots);
  const state = assertProviderRecoveryHeld(roots, held);
  return publishMutationClose(
    roots,
    state,
    state.mutationLease,
    disposition,
    () => assertProviderRecoveryHeld(roots, held),
    publicationHolds,
  );
}

export function recoverProviderCreateForExecutor(argumentsValue) {
  validateFakeProviderAdapter(argumentsValue.adapter);
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const closePublicationHolds = {
    beforeLinkMilliseconds: argumentsValue.adapter.close_prelink_hold_milliseconds,
  };
  const held = acquireProviderRecovery(
    roots,
    argumentsValue.confirmation,
    defaultMutationOwnerProbe,
    {
      afterLinkMilliseconds: argumentsValue.adapter.publication_hold_milliseconds,
      beforeLinkMilliseconds: argumentsValue.adapter.prelink_hold_milliseconds,
    },
  );
  reconcileReceiptPublication(roots);
  let state = assertProviderRecoveryHeld(roots, held);
  const lease = state.mutationLease;
  const head = state.receiptState.head;
  if (head.phase === "plan") {
    if (
      head.sequence !== lease.value.source_sequence ||
      state.receiptState.head_sha256 !== lease.value.source_head_sha256
    ) {
      fail("pre-intent provider recovery binding was refused");
    }
    closeRecoveredProviderMutation(
      roots,
      held,
      "aborted-before-effect",
      closePublicationHolds,
    );
    return head;
  }
  if (head.phase === "provider-create-intent") {
    if (
      state.receiptState.head_sha256 !== lease.value.intent_receipt_sha256 ||
      head.result.provider_contract_sha256 !== FAKE_PROVIDER_CONTRACT_SHA256
    ) {
      fail("provider recovery intent binding was refused");
    }
    if (argumentsValue.adapter.reconcile_outcome === "unknown") {
      fail("provider effect remained uncertain", 73);
    }
    holdFakeProvider(argumentsValue.adapter.reconcile_hold_milliseconds);
    assertProviderRecoveryHeld(roots, held);
    let next = providerAdapterReceipt({
      outcome: argumentsValue.adapter.reconcile_outcome,
      result: argumentsValue.adapter.reconcile_result,
    });
    if (next.phase === "provider-create-passed") {
      let sourceMatches = false;
      try {
        sourceMatches = canonical(sourceClosure(roots.repoRoot)) === canonical(state.candidate.source);
      } catch (error) {
        if (!(error instanceof ClosedFailure)) throw error;
      }
      if (!sourceMatches) {
        next = { phase: "provider-create-failed", result: refusedProviderResult() };
      }
    }
    appendReceiptWithLease(roots, next);
    state = assertProviderRecoveryHeld(roots, held);
  }
  if (state.receiptState.head.phase === "provider-create-passed") {
    let sourceMatches = false;
    try {
      sourceMatches = canonical(sourceClosure(roots.repoRoot)) === canonical(state.candidate.source);
    } catch (error) {
      if (!(error instanceof ClosedFailure)) throw error;
    }
    if (!sourceMatches) {
      appendReceiptWithLease(roots, {
        phase: "execution-failed",
        result: refusedProviderResult(),
      });
      state = assertProviderRecoveryHeld(roots, held);
    }
  }
  if (!new Set(["execution-failed", "provider-create-failed", "provider-create-passed"]).has(
    state.receiptState.head.phase,
  )) {
    fail("provider recovery receipt state was refused");
  }
  closeRecoveredProviderMutation(roots, held, "completed", closePublicationHolds);
  return state.receiptState.head;
}

export function appendReceiptForExecutor(argumentsValue) {
  if (
    typeof argumentsValue.phase === "string" &&
    (argumentsValue.phase.startsWith("provider-create-") ||
      argumentsValue.phase === "finalize-passed")
  ) {
    fail("receipt phase requires its dedicated mutation executor", 64);
  }
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  return withMutationLease(roots, "append-receipt", () =>
    appendReceiptWithLease(roots, argumentsValue),
  );
}

function appendReceiptWithLease(roots, {
  phase,
  result,
}) {
  reconcileReceiptPublication(roots);
  const sourceRequired =
    !phase.endsWith("-failed") &&
    !phase.startsWith("failure-cleanup-") &&
    phase !== "preflight-refused" &&
      phase !== "project-cleanup-intent" &&
      phase !== "provider-cleanup-intent";
  const state = loadState(roots, sourceRequired);
  if (state.receiptState.head.phase === phase) {
    if (canonical(state.receiptState.head.result) !== canonical(result)) {
      fail("completed receipt result did not match retry");
    }
    return state.receiptState.head;
  }
  if (state.environment !== undefined || state.environmentPublication !== undefined) {
    fail("environment finalization is already in progress", 73);
  }
  if (
    state.receipts.length >= 64 ||
    ((phase.endsWith("-intent") || sourceRequired) && state.receipts.length >= 63)
  ) {
    fail("receipt chain capacity was exhausted", 73);
  }
  let receipt;
  try {
    receipt = createNextReceipt(state.receipts, state.candidate.run_id, phase, result);
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  publishReceipt(state.run, receipt);
  if (sourceRequired) {
    let sourceFailure;
    try {
      const current = sourceClosure(roots.repoRoot);
      if (canonical(current) !== canonical(state.candidate.source)) fail("source closure changed");
    } catch (error) {
      if (error instanceof ClosedFailure) sourceFailure = error;
      else throw error;
    }
    if (sourceFailure !== undefined) {
      const drifted = loadState(roots, false);
      if (drifted.receipts.length >= 64) throw sourceFailure;
      let failurePhase = "execution-failed";
      if (phase === "project-cleanup-intent" || phase === "provider-cleanup-intent") {
        failurePhase = phase.replace(/-intent$/, "-failed");
      }
      try {
        const failureReceipt = createNextReceipt(
          drifted.receipts,
          drifted.candidate.run_id,
          failurePhase,
          {
            cleanup_required: true,
            collision_resource: "none",
            resource_disposition: "receipt-owned-or-absent",
            safe_code: "evidence-refused",
          },
        );
        publishReceipt(drifted.run, failureReceipt);
      } catch (error) {
        if (!(error instanceof ReceiptFailure) && !(error instanceof ClosedFailure)) throw error;
      }
      throw sourceFailure;
    }
  }
  const verified = loadState(roots, sourceRequired);
  if (verified.receiptState.head_sha256 !== digest(canonicalBytes(receipt))) {
    fail("phase receipt publication was not durable", 70);
  }
  return receipt;
}

function publishReceipt(run, receipt) {
  publishPrivateArtifact(
    run,
    RECEIPT_STAGING_NAME,
    receiptFileName(receipt),
    canonicalBytes(receipt),
    "phase receipt",
  );
}

function publishPrivateArtifact(run, stagingName, destinationName, bytes, label) {
  const staging = join(run, stagingName);
  const destination = join(run, destinationName);
  writeExclusive(staging, bytes);
  try {
    linkSync(staging, destination);
    syncDirectory(run);
    unlinkSync(staging);
    syncDirectory(run);
  } catch {
    fail(`${label} publication failed`, 70);
  }
}

function reconcileReceiptPublication(roots) {
  const state = loadState(roots, false);
  const pending = state.pendingPublication;
  if (pending === undefined) return;
  if (pending.links === 2) {
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending receipt publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }

  let staged;
  try {
    staged = parseCanonical(pending.path, "pending receipt publication", 1, 0n);
  } catch (error) {
    if (
      !(error instanceof ClosedFailure) ||
      error.message !== "pending receipt publication was not canonical JSON"
    ) {
      throw error;
    }
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending receipt publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }
  let expected;
  const existing = state.receipts[staged.value?.sequence];
  try {
    if (existing !== undefined) {
      expected = existing;
    } else if (staged.value?.phase === "finalize-passed") {
      if (state.environment === undefined) {
        fail("complete final receipt publication requires the environment manifest");
      }
      expected = createFinalization(
        state.candidate,
        state.candidateBytes,
        state.receipts,
      ).receipt;
    } else {
      expected = createNextReceipt(
        state.receipts,
        state.candidate.run_id,
        staged.value?.phase,
        staged.value?.result,
      );
    }
  } catch (error) {
    if (error instanceof ReceiptFailure) {
      fail("complete pending receipt publication was refused");
    }
    throw error;
  }
  if (!staged.bytes.equals(canonicalBytes(expected))) {
    fail("complete pending receipt publication did not match");
  }
  try {
    if (existing === undefined) {
      linkSync(pending.path, join(state.run, receiptFileName(expected)));
      syncDirectory(state.run);
    }
    unlinkSync(pending.path);
    syncDirectory(state.run);
  } catch {
    fail("pending receipt publication recovery failed", 70);
  }
  loadState(roots, false);
}

function reconcileEnvironmentPublication(roots) {
  const state = loadState(roots, false);
  const pending = state.environmentPublication;
  if (pending === undefined) return;
  if (pending.links === 2) {
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending environment publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }

  let staged;
  try {
    staged = parseCanonical(pending.path, "pending environment publication", 1, 0n);
  } catch (error) {
    if (
      !(error instanceof ClosedFailure) ||
      error.message !== "pending environment publication was not canonical JSON"
    ) {
      throw error;
    }
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending environment publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }
  let finalization;
  try {
    const manifestReceipts =
      state.receiptState.head.phase === "finalize-passed"
        ? state.receipts.slice(0, -1)
        : state.receipts;
    finalization = createFinalization(
      state.candidate,
      state.candidateBytes,
      manifestReceipts,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) {
      fail("complete pending environment publication was refused");
    }
    throw error;
  }
  if (!staged.bytes.equals(finalization.manifestBytes)) {
    fail("complete pending environment publication did not match");
  }
  try {
    if (state.environment === undefined) {
      linkSync(pending.path, join(state.run, ENVIRONMENT_NAME));
      syncDirectory(state.run);
    }
    unlinkSync(pending.path);
    syncDirectory(state.run);
  } catch {
    fail("pending environment publication recovery failed", 70);
  }
  loadState(roots, false);
}

export function finalizeEnvironmentForExecutor(argumentsValue) {
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  return withMutationLease(roots, "finalize-environment", () =>
    finalizeEnvironmentWithLease(roots),
  );
}

function finalizeEnvironmentWithLease(roots) {
  reconcileReceiptPublication(roots);
  reconcileEnvironmentPublication(roots);
  let state = loadState(roots, false);
  const exactStateEntries = readdirSync(roots.stateBase).sort();
  if (
    JSON.stringify(exactStateEntries) !==
    JSON.stringify([`.run-${state.candidate.run_id}`, "active"].sort())
  ) {
    fail("environment finalization requires absent inert staging");
  }
  let sourceFailure;
  try {
    const current = sourceClosure(roots.repoRoot);
    if (canonical(current) !== canonical(state.candidate.source)) fail("source closure changed");
  } catch (error) {
    if (error instanceof ClosedFailure) sourceFailure = error;
    else throw error;
  }
  if (sourceFailure !== undefined) {
    if (
      state.environment !== undefined &&
      state.receiptState.head.phase !== "finalize-passed"
    ) {
      try {
        unlinkSync(join(state.run, ENVIRONMENT_NAME));
        syncDirectory(state.run);
      } catch {
        fail("stale environment manifest cleanup failed", 70);
      }
      loadState(roots, false);
    }
    throw sourceFailure;
  }
  state = loadState(roots, true);
  if (state.receiptState.head.phase === "finalize-passed") {
    return { manifest: state.environment.value, receipt: state.receiptState.head };
  }
  let finalization;
  try {
    finalization = createFinalization(
      state.candidate,
      state.candidateBytes,
      state.receipts,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  if (state.environment === undefined) {
    publishPrivateArtifact(
      state.run,
      ENVIRONMENT_STAGING_NAME,
      ENVIRONMENT_NAME,
      finalization.manifestBytes,
      "environment manifest",
    );
  }
  state = loadState(roots, true);
  if (!state.environment.bytes.equals(finalization.manifestBytes)) {
    fail("environment manifest retry did not match");
  }
  publishReceipt(state.run, finalization.receipt);
  const verified = loadState(roots, true);
  if (
    verified.receiptState.head.phase !== "finalize-passed" ||
    verified.receiptState.head_sha256 !== digest(canonicalBytes(finalization.receipt))
  ) {
    fail("environment finalization was not durable", 70);
  }
  return { manifest: verified.environment.value, receipt: verified.receiptState.head };
}

function inspectPendingFile(path, label, expectedDevice, expectedLinks = new Set([1n])) {
  let descriptor;
  let metadata;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    metadata = exactFstat(descriptor);
    if (
      !metadata.isFile() ||
      metadata.uid !== OWNER_UID ||
      metadata.dev !== expectedDevice ||
      !expectedLinks.has(metadata.nlink) ||
      (metadata.mode & 0o7777n) !== 0o600n ||
      metadata.size > BigInt(MAX_FILE_BYTES)
    ) {
      fail(`${label} was refused`);
    }
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail(`${label} was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  const current = exactLstat(path);
  if (!sameMetadata(metadata, current)) fail(`${label} identity changed`);
  return current;
}

function validateInertStaging(pending) {
  const identity = ownedPrivateDirectory(pending, "pending state");
  const expectedDirectories = new Set(["client", "evidence", "provider", "registry", "runtime"]);
  const expectedFiles = new Set(["00-plan.json", "candidate.json"]);
  for (const entry of readdirSync(pending, { withFileTypes: true })) {
    const path = join(pending, entry.name);
    if (expectedDirectories.has(entry.name)) {
      const metadata = ownedPrivateDirectory(path, "pending plan directory");
      if (metadata.dev !== identity.dev) fail("pending plan crossed a filesystem boundary");
      const children = readdirSync(path);
      if (entry.name === "client") {
        if (
          children.length > 1 ||
          (children.length === 1 && children[0] !== "proxy-template.json")
        ) {
          fail("pending client state was refused");
        }
        if (children.length === 1) {
          inspectPendingFile(join(path, children[0]), "pending proxy template", identity.dev);
        }
      } else if (children.length !== 0) {
        fail("pending external-mutation directory was not empty");
      }
      continue;
    }
    if (expectedFiles.has(entry.name)) {
      inspectPendingFile(path, "pending plan artifact", identity.dev);
      continue;
    }
    fail("pending plan leaf was refused");
  }
  const current = exactLstat(pending);
  if (
    current.isSymbolicLink() ||
    !current.isDirectory() ||
    current.dev !== identity.dev ||
    current.ino !== identity.ino ||
    current.uid !== OWNER_UID
  ) {
    fail("pending plan identity changed during validation");
  }
}

function main() {
  try {
    const { action, values } = parseArgs(process.argv);
    const roots = prepareRoots(values["repo-root"], values["state-base"], action === "plan");
    if (action === "plan") {
      plan(roots, values);
    } else {
      const state = loadState(roots, action === "verify");
      const suffix =
        state.receiptState.head.phase === "plan"
          ? action === "verify" ? "source-verified" : "prepared"
          : `${action === "verify" ? "source-verified" : "prepared"} at ${state.receiptState.head.phase}`;
      process.stdout.write(`clean-engine: plan ${state.candidate.run_id} is ${suffix}\n`);
    }
  } catch (error) {
    if (error instanceof ClosedFailure) {
      process.stderr.write(`clean-engine: ${error.message}\n`);
      process.exit(error.exitStatus);
    }
    process.stderr.write("clean-engine: unexpected closed-state failure\n");
    process.exit(70);
  }
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main();
}
