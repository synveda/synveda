import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  linkSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  HostsFileError,
  classifyHostsBytes,
  expectedBlock,
  expectedConfirmation,
  linuxAclOutputIsBase,
  manageHostsPathForTest,
  validateSelection,
} from "../deploy/compose/scripts/manage-hosts-file.mjs";

const UID = process.getuid?.() ?? 0;
const GID = process.getgid?.() ?? 0;
const BUNDLED = Object.freeze({
  project: "synveda-development",
  oidc: "bundled",
  appHost: "app.synveda.test",
  authHost: "auth.synveda.test",
});
const EXTERNAL = Object.freeze({
  project: "synveda-development-acceptance-fixture1",
  oidc: "external",
  appHost: "external-app.synveda.test",
  authHost: null,
});
const STATE = ".synveda-hosts-state-v1.json";
const BACKUP = ".synveda-hosts-backup-v1.json";
const LOCK = ".synveda-hosts-lock-v1";
const MANAGER_URL = new URL("../deploy/compose/scripts/manage-hosts-file.mjs", import.meta.url).href;
const MANAGER_PATH = fileURLToPath(MANAGER_URL);

function fixture(source = "127.0.0.1 localhost\n") {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-hosts-manager-"));
  chmodSync(scratch, 0o700);
  const hosts = join(scratch, "hosts");
  const sourceBytes = Buffer.isBuffer(source) ? source : Buffer.from(source);
  writeFileSync(hosts, sourceBytes, { mode: 0o644 });
  chmodSync(hosts, 0o644);
  return { scratch, hosts, source: sourceBytes };
}

function confirmation(action, selection = BUNDLED) {
  return expectedConfirmation(action, selection);
}

function manage(state, action, selection = BUNDLED, value, options = {}) {
  const confirmationValue = value ?? (action === "status" ? undefined : confirmation(action, selection));
  return manageHostsPathForTest(state.hosts, action, selection, confirmationValue, {
    expectedUid: UID,
    expectedGid: GID,
    aclInspector: () => {},
    ...options,
  });
}

function assertRefused(callback, pattern, status = 78) {
  assert.throws(callback, (error) => {
    assert.equal(error instanceof HostsFileError, true);
    if (status !== null) assert.equal(error.status, status);
    assert.match(error.message, pattern);
    return true;
  });
}

test("bundled and external plans are exact and configuration-bound", () => {
  assert.equal(
    expectedBlock(BUNDLED).toString(),
    "# BEGIN SYNVEDA synveda-development\n" +
      "127.0.0.1 app.synveda.test auth.synveda.test\n" +
      "# END SYNVEDA synveda-development\n",
  );
  assert.equal(
    expectedBlock(EXTERNAL).toString(),
    "# BEGIN SYNVEDA synveda-development-acceptance-fixture1\n" +
      "127.0.0.1 external-app.synveda.test\n" +
      "# END SYNVEDA synveda-development-acceptance-fixture1\n",
  );
  assert.equal(
    confirmation("install"),
    "install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test",
  );
  assert.equal(
    confirmation("remove", EXTERNAL),
    "remove:127.0.0.1:synveda-development-acceptance-fixture1:external-app.synveda.test:-",
  );
});

test("the production CLI exposes no host-file path override", () => {
  const arguments_ = [
    "--runtime",
    "development",
    "--project",
    BUNDLED.project,
    "--oidc",
    BUNDLED.oidc,
    "--app-host",
    BUNDLED.appHost,
    "--auth-host",
    BUNDLED.authHost,
  ];
  const plan = spawnSync(process.execPath, [MANAGER_PATH, "plan", ...arguments_], {
    encoding: "utf8",
  });
  assert.equal(plan.status, 0, plan.stderr);
  assert.equal(plan.stdout, expectedBlock(BUNDLED).toString());

  const injected = spawnSync(
    process.execPath,
    [MANAGER_PATH, "plan", ...arguments_, "--path", "/tmp/not-allowed"],
    { encoding: "utf8" },
  );
  assert.equal(injected.status, 64, injected.stderr);
  assert.match(injected.stderr, /configuration was refused/);
  assert.equal(injected.stdout, "");
});

test("the production CLI executes from a checkout path containing spaces", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda hosts manager "));
  try {
    const copiedManager = join(scratch, "manage hosts file.mjs");
    copyFileSync(MANAGER_PATH, copiedManager);
    const plan = spawnSync(
      process.execPath,
      [
        copiedManager,
        "plan",
        "--runtime",
        "development",
        "--project",
        BUNDLED.project,
        "--oidc",
        BUNDLED.oidc,
        "--app-host",
        BUNDLED.appHost,
        "--auth-host",
        BUNDLED.authHost,
      ],
      { encoding: "utf8" },
    );
    assert.equal(plan.status, 0, plan.stderr);
    assert.equal(plan.stdout, expectedBlock(BUNDLED).toString());
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("selection grammar refuses non-development projects and ambiguous identity hosts", () => {
  for (const selection of [
    { ...BUNDLED, project: "synveda-reference" },
    { ...BUNDLED, appHost: "app.example.com" },
    { ...BUNDLED, authHost: BUNDLED.appHost },
    { ...BUNDLED, oidc: "external" },
    { ...EXTERNAL, authHost: "unexpected.synveda.test" },
  ]) {
    assertRefused(() => validateSelection(selection), /selection/, 64);
  }
});

test("install and remove preserve unrelated bytes and POSIX ownership metadata", () => {
  const state = fixture("# retained comment\n127.0.0.1 localhost\n");
  try {
    const before = statSync(state.hosts);
    assert.equal(manage(state, "status", BUNDLED, undefined), "absent");
    assert.equal(manage(state, "install"), "installed");
    assert.deepEqual(readFileSync(state.hosts), Buffer.concat([state.source, expectedBlock(BUNDLED)]));
    assert.equal(manage(state, "status", BUNDLED, undefined), "installed");
    assert.equal(manage(state, "install"), "installed");
    const installed = statSync(state.hosts);
    assert.equal(installed.uid, before.uid);
    assert.equal(installed.gid, before.gid);
    assert.equal(installed.mode & 0o7777, before.mode & 0o7777);
    assert.equal(installed.ino, before.ino);
    assert.equal(lstatSync(join(state.scratch, STATE)).mode & 0o7777, 0o644);
    assert.equal(lstatSync(join(state.scratch, BACKUP)).mode & 0o7777, 0o600);

    assert.equal(manage(state, "remove"), "absent");
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.equal(existsSync(join(state.scratch, STATE)), false);
    assert.equal(existsSync(join(state.scratch, BACKUP)), false);
    assert.equal(manage(state, "remove"), "absent");
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("same-inode mutation preserves an available filesystem xattr", (context) => {
  const state = fixture();
  const attribute = process.platform === "darwin" ? "com.synveda.hosts-test" : "user.synveda_hosts_test";
  const setTool = process.platform === "darwin" ? "/usr/bin/xattr" : "/usr/bin/setfattr";
  const getTool = process.platform === "darwin" ? "/usr/bin/xattr" : "/usr/bin/getfattr";
  if (!existsSync(setTool) || !existsSync(getTool)) {
    context.skip("no platform xattr fixture tool");
    rmSync(state.scratch, { recursive: true, force: true });
    return;
  }
  const setArguments =
    process.platform === "darwin"
      ? ["-w", attribute, "synveda-xattr-value", state.hosts]
      : ["-n", attribute, "-v", "synveda-xattr-value", state.hosts];
  const getArguments =
    process.platform === "darwin"
      ? ["-p", attribute, state.hosts]
      : ["--only-values", "-n", attribute, state.hosts];
  const setResult = spawnSync(setTool, setArguments, { encoding: "utf8" });
  if (setResult.status !== 0) {
    context.skip("scratch filesystem refused xattr fixture");
    rmSync(state.scratch, { recursive: true, force: true });
    return;
  }
  try {
    const inode = statSync(state.hosts).ino;
    const aclOptions = process.platform === "darwin" ? { aclInspector: undefined } : {};
    assert.equal(manage(state, "install", BUNDLED, confirmation("install"), aclOptions), "installed");
    assert.equal(statSync(state.hosts).ino, inode);
    assert.equal(spawnSync(getTool, getArguments, { encoding: "utf8" }).stdout.trim(), "synveda-xattr-value");
    assert.equal(manage(state, "remove", BUNDLED, confirmation("remove"), aclOptions), "absent");
    assert.equal(statSync(state.hosts).ino, inode);
    assert.equal(spawnSync(getTool, getArguments, { encoding: "utf8" }).stdout.trim(), "synveda-xattr-value");
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("ACL grammar accepts only Linux base entries", () => {
  assert.equal(linuxAclOutputIsBase("user::rw-\ngroup::r--\nother::r--\n", 0o644), true);
  for (const output of [
    "user::rw-\nuser:1000:r--\ngroup::r--\nmask::r--\nother::r--\n",
    "user::rwx\ngroup::r-x\nother::r-x\ndefault:user::rwx\n",
    "# file: hosts\nuser::rw-\ngroup::r--\nother::r--\n",
    "user::rw-\ngroup::rw-\nother::r--\n",
  ]) {
    assert.equal(linuxAclOutputIsBase(output, 0o644), false);
  }
});

test("Darwin target and inheritable-parent ACLs are refused before sidecar bytes", (context) => {
  if (process.platform !== "darwin" || !existsSync("/bin/chmod")) {
    context.skip("Darwin ACL fixture unavailable");
    return;
  }
  for (const kind of ["target", "parent"]) {
    const state = fixture();
    const aclTarget = kind === "target" ? state.hosts : state.scratch;
    const entry =
      kind === "target"
        ? "everyone allow read"
        : "everyone allow read,readattr,readextattr,readsecurity,file_inherit";
    try {
      const added = spawnSync("/bin/chmod", ["+a", entry, aclTarget], { encoding: "utf8" });
      assert.equal(added.status, 0, added.stderr);
      assertRefused(
        () =>
          manage(state, "install", BUNDLED, confirmation("install"), {
            aclInspector: undefined,
          }),
        /access ACL/,
      );
      assert.deepEqual(readFileSync(state.hosts), state.source);
      assert.deepEqual(readdirSync(state.scratch).sort(), ["hosts"]);
    } finally {
      spawnSync("/bin/chmod", ["-N", aclTarget], { encoding: "utf8" });
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("empty hosts files round-trip exactly", () => {
  const state = fixture("");
  try {
    assert.equal(manage(state, "install"), "installed");
    assert.deepEqual(readFileSync(state.hosts), expectedBlock(BUNDLED));
    assert.equal(manage(state, "remove"), "absent");
    assert.equal(readFileSync(state.hosts).length, 0);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("the installed result is bounded before recovery or target mutation", () => {
  const blockLength = expectedBlock(BUNDLED).length;
  const exactBytes = Buffer.alloc(1024 * 1024 - blockLength, 0x20);
  exactBytes[exactBytes.length - 1] = 0x0a;
  const exact = fixture(exactBytes);
  try {
    assert.equal(manage(exact, "install"), "installed");
    assert.equal(readFileSync(exact.hosts).length, 1024 * 1024);
    assert.equal(manage(exact, "remove"), "absent");
  } finally {
    rmSync(exact.scratch, { recursive: true, force: true });
  }

  const oversizedResultBytes = Buffer.alloc(1024 * 1024 - blockLength + 1, 0x20);
  oversizedResultBytes[oversizedResultBytes.length - 1] = 0x0a;
  const oversizedResult = fixture(oversizedResultBytes);
  try {
    assertRefused(() => manage(oversizedResult, "install"), /installed hosts file size/);
    assert.deepEqual(readFileSync(oversizedResult.hosts), oversizedResult.source);
    assert.equal(existsSync(join(oversizedResult.scratch, STATE)), false);
    assert.equal(existsSync(join(oversizedResult.scratch, BACKUP)), false);
    assert.equal(existsSync(join(oversizedResult.scratch, LOCK)), false);
  } finally {
    rmSync(oversizedResult.scratch, { recursive: true, force: true });
  }
});

test("unmarked mappings, aliases in comments, case variants, and trailing dots are refused", () => {
  for (const source of [
    "127.0.0.1 app.synveda.test auth.synveda.test\n",
    "# app.synveda.test\n127.0.0.1 localhost\n",
    "127.0.0.1 APP.SYNVEDA.TEST.\n",
    "127.0.0.1 auth.synveda.test.\n",
  ]) {
    const state = fixture(source);
    try {
      assertRefused(() => manage(state, "install"), /hostname collision/);
      assert.equal(existsSync(join(state.scratch, STATE)), false);
      assert.equal(existsSync(join(state.scratch, BACKUP)), false);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("partial, nested, duplicate, foreign, and non-terminal markers are refused", () => {
  const exact = expectedBlock(BUNDLED).toString();
  for (const source of [
    "# BEGIN SYNVEDA synveda-development\n127.0.0.1 localhost\n",
    "# END SYNVEDA synveda-development\n",
    `${exact}${exact}`,
    "# BEGIN SYNVEDA synveda-development-acceptance-foreign\n" +
      "127.0.0.1 other.synveda.test\n" +
      "# END SYNVEDA synveda-development-acceptance-foreign\n",
    `${exact}127.0.0.1 localhost\n`,
    `not-a-line-boundary${exact}`,
  ]) {
    const state = fixture(source);
    try {
      assertRefused(() => manage(state, "status", BUNDLED, undefined), /marker|block|unowned/);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("a byte-exact block without helper recovery state is not adopted", () => {
  const state = fixture(`127.0.0.1 localhost\n${expectedBlock(BUNDLED).toString()}`);
  try {
    assertRefused(() => manage(state, "status", BUNDLED, undefined), /unowned/);
    assertRefused(() => manage(state, "install"), /unowned/);
    assertRefused(() => manage(state, "remove"), /unowned/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("configuration-bound confirmation is required before sidecar or target mutation", () => {
  const state = fixture();
  try {
    for (const value of ["", "synveda-development", confirmation("remove")]) {
      assertRefused(() => manage(state, "install", BUNDLED, value), /confirmation/, 64);
    }
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.equal(existsSync(join(state.scratch, STATE)), false);
    assert.equal(existsSync(join(state.scratch, BACKUP)), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("removal also refuses missing, stale, and install-bound confirmation without mutation", () => {
  const state = fixture();
  try {
    manage(state, "install");
    const installed = readFileSync(state.hosts);
    const ownership = readFileSync(join(state.scratch, STATE));
    const recovery = readFileSync(join(state.scratch, BACKUP));
    for (const value of [undefined, "synveda-development", confirmation("install")]) {
      assertRefused(
        () =>
          manageHostsPathForTest(state.hosts, "remove", BUNDLED, value, {
            expectedUid: UID,
            expectedGid: GID,
          }),
        /confirmation/,
        64,
      );
      assert.deepEqual(readFileSync(state.hosts), installed);
      assert.deepEqual(readFileSync(join(state.scratch, STATE)), ownership);
      assert.deepEqual(readFileSync(join(state.scratch, BACKUP)), recovery);
      assert.equal(existsSync(join(state.scratch, LOCK)), false);
    }
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("installation resumes after recovery publication interruption", () => {
  const state = fixture();
  try {
    assert.throws(
      () =>
        manage(state, "install", BUNDLED, confirmation("install"), {
          hooks: { afterBackupPublished: () => refuseFixture("interrupted") },
        }),
      /interrupted/,
    );
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.equal(existsSync(join(state.scratch, BACKUP)), true);
    assert.equal(existsSync(join(state.scratch, STATE)), false);
    assert.equal(existsSync(join(state.scratch, LOCK)), false);
    assert.equal(manage(state, "install"), "installed");
    assert.equal(manage(state, "status", BUNDLED, undefined), "installed");
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("install repairs a missing ownership record for an exact installed target", () => {
  const state = fixture();
  try {
    assert.equal(manage(state, "install"), "installed");
    unlinkSync(join(state.scratch, STATE));
    assert.equal(manage(state, "install"), "installed");
    assert.equal(existsSync(join(state.scratch, STATE)), true);
    assert.equal(manage(state, "status", BUNDLED, undefined), "installed");
    assert.equal(manage(state, "remove"), "absent");
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a non-writable target is refused before sidecar or target mutation", () => {
  const state = fixture();
  try {
    chmodSync(state.hosts, 0o444);
    assertRefused(() => manage(state, "install"), /permissions/);
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.equal(existsSync(join(state.scratch, STATE)), false);
    assert.equal(existsSync(join(state.scratch, BACKUP)), false);
    assert.equal(existsSync(join(state.scratch, LOCK)), false);
  } finally {
    chmodSync(state.hosts, 0o644);
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("confirmed removal of exact source state skips mutation writability preflight", () => {
  const state = fixture();
  try {
    assert.throws(
      () =>
        manage(state, "install", BUNDLED, confirmation("install"), {
          hooks: { afterStatePublished: () => refuseFixture("interrupted-before-target") },
        }),
      /interrupted-before-target/,
    );
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.equal(existsSync(join(state.scratch, STATE)), true);
    assert.equal(existsSync(join(state.scratch, BACKUP)), true);
    assert.equal(
      manage(state, "remove", BUNDLED, confirmation("remove"), {
        hooks: { beforeWritablePreflight: () => refuseFixture("unexpected-write-preflight") },
      }),
      "absent",
    );
    assert.equal(existsSync(join(state.scratch, STATE)), false);
    assert.equal(existsSync(join(state.scratch, BACKUP)), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("removal resumes after ownership-state cleanup interruption", () => {
  const state = fixture();
  try {
    manage(state, "install");
    assert.throws(
      () =>
        manage(state, "remove", BUNDLED, confirmation("remove"), {
          hooks: { afterStateRemoved: () => refuseFixture("interrupted") },
        }),
      /interrupted/,
    );
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.equal(existsSync(join(state.scratch, STATE)), false);
    assert.equal(existsSync(join(state.scratch, BACKUP)), true);
    assert.equal(existsSync(join(state.scratch, LOCK)), false);
    assert.equal(manage(state, "remove"), "absent");
    assert.equal(existsSync(join(state.scratch, BACKUP)), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a terminated post-mutation process is recovered as an exact installed state", () => {
  const state = fixture();
  try {
    const child = spawnSync(
      process.execPath,
      [
        "--input-type=module",
        "--eval",
        `import { manageHostsPathForTest } from ${JSON.stringify(MANAGER_URL)};
const selection = ${JSON.stringify(BUNDLED)};
try {
  manageHostsPathForTest(process.argv[1], "install", selection,
    "install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test", {
      expectedUid: ${UID}, expectedGid: ${GID},
      hooks: { afterTargetMutation() { throw new Error("simulated-post-mutation-exit"); } },
    });
  process.exit(21);
} catch (error) {
  process.exit(error?.uncertain === true ? 23 : 22);
}`,
        state.hosts,
      ],
      { encoding: "utf8" },
    );
    assert.equal(child.status, 23, `${child.stdout}${child.stderr}`);
    assert.equal(existsSync(join(state.scratch, LOCK)), true);
    assert.deepEqual(readFileSync(state.hosts), Buffer.concat([state.source, expectedBlock(BUNDLED)]));
    let synced = false;
    assert.equal(
      manage(state, "install", BUNDLED, confirmation("install"), {
        hooks: { afterTargetSync: () => { synced = true; } },
      }),
      "installed",
    );
    assert.equal(synced, true);
    assert.equal(existsSync(join(state.scratch, LOCK)), false);
    assert.equal(manage(state, "status", BUNDLED, undefined), "installed");
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("uncertain removal is fsynced before recovery authority is cleared", () => {
  const state = fixture();
  try {
    assert.equal(manage(state, "install"), "installed");
    const child = spawnSync(
      process.execPath,
      [
        "--input-type=module",
        "--eval",
        `import { expectedConfirmation, manageHostsPathForTest } from ${JSON.stringify(MANAGER_URL)};
const selection = ${JSON.stringify(BUNDLED)};
try {
  manageHostsPathForTest(process.argv[1], "remove", selection,
    expectedConfirmation("remove", selection), {
      expectedUid: ${UID}, expectedGid: ${GID}, aclInspector: () => {},
      hooks: { afterTargetMutation() { throw new Error("simulated-remove-exit"); } },
    });
  process.exit(21);
} catch (error) {
  process.exit(error?.uncertain === true ? 23 : 22);
}`,
        state.hosts,
      ],
      { encoding: "utf8" },
    );
    assert.equal(child.status, 23, `${child.stdout}${child.stderr}`);
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.equal(existsSync(join(state.scratch, LOCK)), true);
    let synced = false;
    assert.equal(
      manage(state, "remove", BUNDLED, confirmation("remove"), {
        hooks: { afterTargetSync: () => { synced = true; } },
      }),
      "absent",
    );
    assert.equal(synced, true);
    assert.equal(existsSync(join(state.scratch, LOCK)), false);
    assert.equal(existsSync(join(state.scratch, STATE)), false);
    assert.equal(existsSync(join(state.scratch, BACKUP)), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("SIGKILL during append leaves an exact recoverable prefix and no target stage", () => {
  const state = fixture();
  try {
    const before = statSync(state.hosts);
    const child = spawnSync(
      process.execPath,
      [
        "--input-type=module",
        "--eval",
        `import { expectedConfirmation, manageHostsPathForTest } from ${JSON.stringify(MANAGER_URL)};
const selection = ${JSON.stringify(BUNDLED)};
manageHostsPathForTest(process.argv[1], "install", selection,
  expectedConfirmation("install", selection), {
    expectedUid: ${UID}, expectedGid: ${GID},
    hooks: { afterTargetPartialWrite() { process.kill(process.pid, "SIGKILL"); } },
  });`,
        state.hosts,
      ],
      { encoding: "utf8" },
    );
    assert.equal(child.status, null, `${child.stdout}${child.stderr}`);
    assert.equal(child.signal, "SIGKILL");
    assert.deepEqual(readFileSync(state.hosts), Buffer.concat([state.source, Buffer.from("#")]));
    assert.equal(existsSync(join(state.scratch, LOCK)), true);
    assert.equal(readdirSync(state.scratch).some((name) => name.includes("target-stage")), false);

    assert.equal(manage(state, "install"), "installed");
    assert.equal(statSync(state.hosts).ino, before.ino);
    assert.equal(readdirSync(state.scratch).some((name) => name.includes("-stage-v1-")), false);
    assert.equal(manage(state, "remove"), "absent");
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a confirmed action recovers an exact interrupted prefix after lock loss", () => {
  const state = fixture();
  try {
    const child = spawnSync(
      process.execPath,
      [
        "--input-type=module",
        "--eval",
        `import { expectedConfirmation, manageHostsPathForTest } from ${JSON.stringify(MANAGER_URL)};
const selection = ${JSON.stringify(BUNDLED)};
manageHostsPathForTest(process.argv[1], "install", selection,
  expectedConfirmation("install", selection), {
    expectedUid: ${UID}, expectedGid: ${GID},
    hooks: { afterTargetPartialWrite() { process.kill(process.pid, "SIGKILL"); } },
  });`,
        state.hosts,
      ],
      { encoding: "utf8" },
    );
    assert.equal(child.signal, "SIGKILL");
    unlinkSync(join(state.scratch, LOCK));
    assertRefused(
      () => manage(state, "status", BUNDLED, undefined),
      /termination|recovery is incomplete|block collision|ownership/,
      null,
    );
    assert.equal(manage(state, "remove"), "absent");
    assert.deepEqual(readFileSync(state.hosts), state.source);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("SIGKILL sidecar stages are lock-bound and recovered before retry", () => {
  for (const point of ["backup-prepared", "backup-linked", "state-prepared", "state-linked"]) {
    const state = fixture();
    try {
      const child = spawnSync(
        process.execPath,
        [
          "--input-type=module",
          "--eval",
          `import { expectedConfirmation, manageHostsPathForTest } from ${JSON.stringify(MANAGER_URL)};
const selection = ${JSON.stringify(BUNDLED)};
const point = ${JSON.stringify(point)};
manageHostsPathForTest(process.argv[1], "install", selection,
  expectedConfirmation("install", selection), {
    expectedUid: ${UID}, expectedGid: ${GID},
    hooks: {
      afterStagePrepared(kind) {
        if (point === \`${"${kind}"}-prepared\`) process.kill(process.pid, "SIGKILL");
      },
      afterStageLinked(kind) {
        if (point === \`${"${kind}"}-linked\`) process.kill(process.pid, "SIGKILL");
      },
    },
  });`,
          state.hosts,
        ],
        { encoding: "utf8" },
      );
      assert.equal(child.status, null, `${point}: ${child.stdout}${child.stderr}`);
      assert.equal(child.signal, "SIGKILL", point);
      assert.equal(readdirSync(state.scratch).some((name) => name.includes("-stage-v1-")), true);
      assert.equal(existsSync(join(state.scratch, LOCK)), true);

      assert.equal(manage(state, "install"), "installed");
      assert.equal(readdirSync(state.scratch).some((name) => name.includes("-stage-v1-")), false);
      assert.equal(existsSync(join(state.scratch, LOCK)), false);
      assert.equal(manage(state, "remove"), "absent");
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("stage and target close failures retain recoverable authority", () => {
  for (const point of ["stage", "target"]) {
    const state = fixture();
    try {
      const child = spawnSync(
        process.execPath,
        [
          "--input-type=module",
          "--eval",
          `import { expectedConfirmation, manageHostsPathForTest } from ${JSON.stringify(MANAGER_URL)};
const selection = ${JSON.stringify(BUNDLED)};
const point = ${JSON.stringify(point)};
try {
  manageHostsPathForTest(process.argv[1], "install", selection,
    expectedConfirmation("install", selection), {
      expectedUid: ${UID}, expectedGid: ${GID}, aclInspector: () => {},
      hooks: {
        beforeStageClose() { if (point === "stage") throw new Error("stage-close"); },
        beforeTargetClose() { if (point === "target") throw new Error("target-close"); },
      },
    });
  process.exit(21);
} catch (error) {
  process.exit(error?.uncertain === true ? 23 : 22);
}`,
          state.hosts,
        ],
        { encoding: "utf8" },
      );
      assert.equal(child.status, 23, `${point}: ${child.stdout}${child.stderr}`);
      assert.equal(existsSync(join(state.scratch, LOCK)), true);
      assert.equal(manage(state, "install"), "installed");
      assert.equal(readdirSync(state.scratch).some((name) => name.includes("-stage-v1-")), false);
      assert.equal(existsSync(join(state.scratch, LOCK)), false);
      assert.equal(manage(state, "remove"), "absent");
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

function refuseFixture(message) {
  throw new Error(message);
}

test("a same-inode concurrent edit is detected before mutation and never overwritten", () => {
  const state = fixture();
  try {
    assertRefused(
      () =>
        manage(state, "install", BUNDLED, confirmation("install"), {
          hooks: {
            beforeTargetRevalidate: () =>
              writeFileSync(state.hosts, `${state.source.toString()}127.0.0.2 raced.example.test\n`),
          },
        }),
      /changed before replacement/,
      75,
    );
    assert.equal(readFileSync(state.hosts, "utf8").endsWith("127.0.0.2 raced.example.test\n"), true);
    assert.equal(existsSync(join(state.scratch, LOCK)), false);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("active and stale cooperative locks are distinguished", () => {
  const active = fixture();
  try {
    writeFileSync(join(active.scratch, LOCK), `synveda-hosts-lock-v1 ${process.pid} 00000000-0000-4000-8000-000000000000\n`, {
      mode: 0o600,
    });
    chmodSync(join(active.scratch, LOCK), 0o600);
    assertRefused(() => manage(active, "status", BUNDLED, undefined), /in progress/, 75);
    assertRefused(() => manage(active, "install"), /another/, 75);
  } finally {
    rmSync(active.scratch, { recursive: true, force: true });
  }

  const stale = fixture();
  try {
    writeFileSync(join(stale.scratch, LOCK), "synveda-hosts-lock-v1 2147483647 00000000-0000-4000-8000-000000000000\n", {
      mode: 0o600,
    });
    chmodSync(join(stale.scratch, LOCK), 0o600);
    assert.equal(manage(stale, "install"), "installed");
  } finally {
    rmSync(stale.scratch, { recursive: true, force: true });
  }
});

test("stale nlink-two stages must link to the exact published sidecar", () => {
  const state = fixture();
  const nonce = "00000000-0000-4000-8000-000000000000";
  const stage = join(state.scratch, `.synveda-hosts-backup-stage-v1-${nonce}`);
  const peer = join(state.scratch, "unrelated-stage-peer");
  try {
    writeFileSync(stage, "poison", { mode: 0o600 });
    chmodSync(stage, 0o600);
    linkSync(stage, peer);
    writeFileSync(join(state.scratch, LOCK), `synveda-hosts-lock-v1 2147483647 ${nonce}\n`, {
      mode: 0o600,
    });
    chmodSync(join(state.scratch, LOCK), 0o600);
    assertRefused(() => manage(state, "install"), /stage linkage/, 75);
    assert.equal(existsSync(stage), true);
    assert.equal(existsSync(peer), true);
    assert.equal(existsSync(join(state.scratch, LOCK)), true);
    assert.deepEqual(readFileSync(state.hosts), state.source);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("target drift and recovery-record drift fail closed", () => {
  const targetDrift = fixture();
  try {
    manage(targetDrift, "install");
    writeFileSync(targetDrift.hosts, `# drift\n${readFileSync(targetDrift.hosts, "utf8")}`);
    assertRefused(() => manage(targetDrift, "status", BUNDLED, undefined), /ownership/);
    assertRefused(() => manage(targetDrift, "remove"), /drift/);
  } finally {
    rmSync(targetDrift.scratch, { recursive: true, force: true });
  }

  const metadataDrift = fixture();
  try {
    manage(metadataDrift, "install");
    chmodSync(metadataDrift.hosts, 0o600);
    assertRefused(() => manage(metadataDrift, "status", BUNDLED, undefined), /permissions/);
    assertRefused(() => manage(metadataDrift, "remove"), /permissions/);
    chmodSync(metadataDrift.hosts, 0o644);
    assert.equal(manage(metadataDrift, "remove"), "absent");
  } finally {
    rmSync(metadataDrift.scratch, { recursive: true, force: true });
  }

  const recordDrift = fixture();
  try {
    manage(recordDrift, "install");
    const statePath = join(recordDrift.scratch, STATE);
    const value = JSON.parse(readFileSync(statePath, "utf8"));
    value.selection.project = "synveda-development-acceptance-foreign";
    writeFileSync(statePath, `${JSON.stringify(value)}\n`);
    chmodSync(statePath, 0o644);
    assertRefused(() => manage(recordDrift, "status", BUNDLED, undefined), /ownership state/);
  } finally {
    rmSync(recordDrift.scratch, { recursive: true, force: true });
  }

  const backupDrift = fixture();
  try {
    manage(backupDrift, "install");
    const backupPath = join(backupDrift.scratch, BACKUP);
    writeFileSync(backupPath, "x".repeat(256), { mode: 0o600 });
    chmodSync(backupPath, 0o600);
    assertRefused(
      () => manage(backupDrift, "status", BUNDLED, undefined),
      /ownership state/,
    );
  } finally {
    rmSync(backupDrift.scratch, { recursive: true, force: true });
  }
});

test("ownership-state metadata must remain the exact target audience", () => {
  const state = fixture();
  try {
    assert.equal(manage(state, "install"), "installed");
    assert.equal(lstatSync(join(state.scratch, STATE)).mode & 0o7777, 0o644);
    assert.equal(lstatSync(state.hosts).mode & 0o7777, 0o644);
    assert.equal(manage(state, "status", BUNDLED, undefined), "installed");
    chmodSync(join(state.scratch, STATE), 0o600);
    assertRefused(() => manage(state, "status", BUNDLED, undefined), /audience/);
    assertRefused(() => manage(state, "remove"), /audience/);
    assert.deepEqual(readFileSync(state.hosts), Buffer.concat([state.source, expectedBlock(BUNDLED)]));
    chmodSync(join(state.scratch, STATE), 0o644);
    assert.equal(manage(state, "remove"), "absent");
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("noncanonical target modes are refused before sidecars", () => {
  for (const mode of [0o600, 0o664, 0o444]) {
    const state = fixture();
    try {
      chmodSync(state.hosts, mode);
      assertRefused(() => manage(state, "install"), /permissions/);
      assert.deepEqual(readFileSync(state.hosts), state.source);
      assert.deepEqual(readdirSync(state.scratch).sort(), ["hosts"]);
    } finally {
      chmodSync(state.hosts, 0o644);
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("a group/world-writable physical parent is refused before mutation", () => {
  const state = fixture();
  try {
    chmodSync(state.scratch, 0o777);
    assertRefused(() => manage(state, "status", BUNDLED, undefined), /parent directory authority/);
    assertRefused(() => manage(state, "install"), /parent directory authority/);
    assert.deepEqual(readFileSync(state.hosts), state.source);
    assert.deepEqual(readdirSync(state.scratch).sort(), ["hosts"]);
  } finally {
    chmodSync(state.scratch, 0o700);
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("symlink, directory, hard-link, unsafe-mode, unterminated, binary, and oversized targets are refused", () => {
  const cases = [];

  const symlink = fixture();
  rmSync(symlink.hosts);
  writeFileSync(join(symlink.scratch, "real-hosts"), "127.0.0.1 localhost\n");
  symlinkSync(join(symlink.scratch, "real-hosts"), symlink.hosts);
  cases.push(symlink);

  const directory = fixture();
  rmSync(directory.hosts);
  mkdirSync(directory.hosts);
  cases.push(directory);

  const hardlink = fixture();
  linkSync(hardlink.hosts, join(hardlink.scratch, "hosts-peer"));
  cases.push(hardlink);

  const unsafe = fixture();
  chmodSync(unsafe.hosts, 0o666);
  cases.push(unsafe);

  const unterminated = fixture("127.0.0.1 localhost");
  cases.push(unterminated);

  const binary = fixture("127.0.0.1 localhost\n");
  writeFileSync(binary.hosts, Buffer.from([0xff, 0x0a]));
  chmodSync(binary.hosts, 0o644);
  cases.push(binary);

  const oversized = fixture();
  writeFileSync(oversized.hosts, Buffer.alloc(1024 * 1024 + 1, 0x0a));
  chmodSync(oversized.hosts, 0o644);
  cases.push(oversized);

  try {
    for (const state of cases) {
      assertRefused(
        () => manage(state, "status", BUNDLED, undefined),
        /refused|unavailable/,
        null,
      );
    }
  } finally {
    for (const state of cases) rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("classification never rewrites or returns unrelated hosts content", () => {
  assert.deepEqual(classifyHostsBytes(Buffer.from("127.0.0.1 localhost\n"), BUNDLED), {
    state: "absent",
  });
  const exact = Buffer.concat([Buffer.from("127.0.0.1 localhost\n"), expectedBlock(BUNDLED)]);
  assert.deepEqual(classifyHostsBytes(exact, BUNDLED), {
    state: "exact",
    offset: 20,
  });
});
