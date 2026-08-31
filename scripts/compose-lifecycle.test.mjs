import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { randomBytes } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const WRAPPER = join(ROOT, "deploy/compose/scripts/compose.sh");
const PROJECT_LOCK = join(ROOT, "deploy/compose/scripts/project-lock.sh");
const REAL_ENV = spawnSync("/bin/sh", ["-c", "command -v env"], {
  encoding: "utf8",
}).stdout.trim();
const REAL_OPENSSL = spawnSync("/bin/sh", ["-c", "command -v openssl"], {
  encoding: "utf8",
}).stdout.trim();

function executable(path, content) {
  writeFileSync(path, content, { mode: 0o700 });
  chmodSync(path, 0o700);
}

function fixture() {
  const suffix = `acceptance-life${randomBytes(6).toString("hex")}`;
  const project = `synveda-development-${suffix}`;
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-compose-lifecycle-")));
  chmodSync(scratch, 0o700);
  const bin = join(scratch, "bin");
  mkdirSync(bin, { mode: 0o700 });
  const log = join(scratch, "calls.log");
  const fakeDocker = join(bin, "docker");
  executable(
    fakeDocker,
    `#!/bin/sh
set -eu
if [ "\${SYNVEDA_FAKE_REQUIRE_PINNED_DOCKER:-0}" = 1 ]; then
  [ "\${DOCKER_HOST:-}" = "$SYNVEDA_FAKE_LOCAL_DOCKER_ENDPOINT" ] &&
    [ -z "\${DOCKER_CONTEXT:-}" ] || exit 97
fi
printf 'docker' >> "$SYNVEDA_FAKE_CALL_LOG"
for argument in "$@"; do printf ' <%s>' "$argument" >> "$SYNVEDA_FAKE_CALL_LOG"; done
printf '\n' >> "$SYNVEDA_FAKE_CALL_LOG"
if [ "$1" = compose ] && [ "$2" = version ]; then
  echo 2.33.1
  exit 0
fi
if [ "$1" = volume ] && [ "$2" = ls ]; then
  [ "\${SYNVEDA_FAKE_VOLUME_INVENTORY_ERROR:-0}" = 0 ] || exit 1
  if grep -q 'docker <volume> <rm>' "$SYNVEDA_FAKE_CALL_LOG"; then
    [ "\${SYNVEDA_FAKE_POST_REMOVE_INVENTORY_ERROR:-0}" = 0 ] || exit 1
    exit 0
  fi
  case " $* " in
    *"name=^"*) query=named ;;
    *) query=labelled ;;
  esac
  case "\${SYNVEDA_FAKE_VOLUME_MODE:-absent}:$query" in
    exact:named|exact:labelled|named-only:named)
      printf '%s_postgres-data\n' "$SYNVEDA_FAKE_PROJECT"
      ;;
    labelled-wrong-name:labelled)
      printf '%s_foreign-data\n' "$SYNVEDA_FAKE_PROJECT"
      ;;
  esac
  exit 0
fi
if [ "$1" = volume ] && [ "$2" = inspect ]; then
  case "\${SYNVEDA_FAKE_VOLUME_CONTRACT:-valid}" in
    inspect-error) exit 1 ;;
    valid)
      printf '%s_postgres-data|local|local|null|%s|postgres-data|cpr-45|postgres-data\n' \
        "$SYNVEDA_FAKE_PROJECT" "$SYNVEDA_FAKE_PROJECT"
      ;;
    wrong-driver)
      printf '%s_postgres-data|foreign|local|null|%s|postgres-data|cpr-45|postgres-data\n' \
        "$SYNVEDA_FAKE_PROJECT" "$SYNVEDA_FAKE_PROJECT"
      ;;
    wrong-scope)
      printf '%s_postgres-data|local|global|null|%s|postgres-data|cpr-45|postgres-data\n' \
        "$SYNVEDA_FAKE_PROJECT" "$SYNVEDA_FAKE_PROJECT"
      ;;
    wrong-options)
      printf '%s_postgres-data|local|local|{"type":"none"}|%s|postgres-data|cpr-45|postgres-data\n' \
        "$SYNVEDA_FAKE_PROJECT" "$SYNVEDA_FAKE_PROJECT"
      ;;
    drift-after-down)
      if grep -q ' <down>' "$SYNVEDA_FAKE_CALL_LOG"; then
        printf '%s_postgres-data|foreign|local|null|%s|postgres-data|cpr-45|postgres-data\n' \
          "$SYNVEDA_FAKE_PROJECT" "$SYNVEDA_FAKE_PROJECT"
      else
        printf '%s_postgres-data|local|local|null|%s|postgres-data|cpr-45|postgres-data\n' \
          "$SYNVEDA_FAKE_PROJECT" "$SYNVEDA_FAKE_PROJECT"
      fi
      ;;
  esac
  exit 0
fi
case " $* " in
  *" up "*)
    if [ "\${SYNVEDA_FAKE_BLOCK_UP:-0}" = 1 ]; then
      : > "$SYNVEDA_FAKE_UP_ENTERED"
      while [ ! -f "$SYNVEDA_FAKE_UP_RELEASE" ]; do /bin/sleep 0.02; done
    fi
    ;;
esac
exit 0
`,
  );
  const fakeNode = join(bin, "node");
  executable(
    fakeNode,
    `#!/bin/sh
set -eu
case "$1" in
  */monotonic-seconds.mjs)
    if [ -n "\${SYNVEDA_FAKE_CLOCK_FILE:-}" ]; then
      IFS= read -r now < "$SYNVEDA_FAKE_CLOCK_FILE"
      printf '%s\n' "$now"
      printf '%s\n' "$((now + SYNVEDA_FAKE_CLOCK_STEP))" > "$SYNVEDA_FAKE_CLOCK_FILE"
      exit 0
    fi
    exec ${JSON.stringify(process.execPath)} "$@"
    ;;
  */run-with-deadline.mjs)
    printf 'deadline <%s>\n' "$3" >> "$SYNVEDA_FAKE_CALL_LOG"
    if [ -n "\${SYNVEDA_FAKE_RUNNER_PID_FILE:-}" ]; then
      printf '%s\n' "$$" > "$SYNVEDA_FAKE_RUNNER_PID_FILE"
    fi
    if [ "\${SYNVEDA_FAKE_RUNNER_125:-0}" = 1 ]; then exit 125; fi
    exec ${JSON.stringify(process.execPath)} "$@"
    ;;
  */check-host-resolution.mjs|*/check-network-preflight.mjs|*/check-compose-assets.mjs|*/check-runtime-smoke.mjs)
    printf 'node <%s>\n' "$1" >> "$SYNVEDA_FAKE_CALL_LOG"
    if [ "\${SYNVEDA_FAKE_REAL_HOST_CHECK:-0}" = 1 ]; then
      case "$1" in
        */check-host-resolution.mjs) exec ${JSON.stringify(process.execPath)} "$@" ;;
      esac
    fi
    case " $* " in
      *" --print-docker-endpoint true "*)
        printf '%s\n' "\${SYNVEDA_FAKE_LOCAL_DOCKER_ENDPOINT:-unix:///var/run/docker.sock}"
        ;;
    esac
    exit 0
    ;;
esac
exec ${JSON.stringify(process.execPath)} "$@"
`,
  );
  executable(
    join(bin, "env"),
    `#!/bin/sh
set -eu
if [ -n "\${SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE:-}" ]; then
  for argument in "$@"; do
    case "$argument" in
      */generate-secrets.sh)
        printf '%s\n' "$$" > "$SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE"
        if [ "\${SYNVEDA_FAKE_BLOCK_AUTHORITY_ENV:-0}" = 1 ]; then
          if [ "\${SYNVEDA_FAKE_AUTHORITY_IGNORE_SIGNALS:-0}" = 1 ]; then
            trap '' HUP INT TERM
          fi
          : > "$SYNVEDA_FAKE_AUTHORITY_ENV_ENTERED"
          while [ ! -f "$SYNVEDA_FAKE_AUTHORITY_ENV_RELEASE" ]; do :; done
        fi
        break
        ;;
    esac
  done
fi
exec ${JSON.stringify(REAL_ENV)} "$@"
`,
  );
  executable(
    join(bin, "openssl"),
    `#!/bin/sh
set -eu
if [ "\${SYNVEDA_FAKE_BLOCK_OPENSSL:-0}" = 1 ]; then
  : > "$SYNVEDA_FAKE_OPENSSL_ENTERED"
  while [ ! -f "$SYNVEDA_FAKE_OPENSSL_RELEASE" ]; do /bin/sleep 0.02; done
fi
exec ${JSON.stringify(REAL_OPENSSL)} "$@"
`,
  );
  const fakeRm = join(bin, "rm");
  executable(
    fakeRm,
    `#!/bin/sh
set -eu
if [ "\${SYNVEDA_FAKE_BLOCK_ASSET_CLEANUP:-0}" = 1 ]; then
  for argument in "$@"; do
    case "$argument" in
      */synveda-compose-assets.*)
        : > "$SYNVEDA_FAKE_CLEANUP_ENTERED"
        while [ ! -f "$SYNVEDA_FAKE_CLEANUP_RELEASE" ]; do /bin/sleep 0.02; done
        ;;
    esac
  done
fi
exec /bin/rm "$@"
`,
  );
  const runtime = join(scratch, project);
  mkdirSync(runtime, { mode: 0o700 });
  const secrets = join(runtime, "secrets");
  const authority = join(runtime, "database-authority");
  const gate = join(runtime, "keycloak-public-gate");
  const issuer = join(runtime, "issuers.json");
  return {
    scratch,
    bin,
    log,
    fakeDocker,
    runtime,
    secrets,
    authority,
    gate,
    issuer,
    project,
    suffix,
  };
}

function environment(state, extra = {}) {
  const clean = { ...process.env };
  for (const name of Object.keys(clean)) {
    if (name.startsWith("SYNVEDA_") || name.startsWith("COMPOSE_")) delete clean[name];
  }
  for (const name of [
    "DATABASE_URL",
    "POSTGRES_PASSWORD",
    "KC_DB_PASSWORD",
    "KC_BOOTSTRAP_ADMIN_USERNAME",
    "KC_BOOTSTRAP_ADMIN_PASSWORD",
  ]) {
    delete clean[name];
  }
  return {
    ...clean,
    PATH: `${state.bin}:${clean.PATH}`,
    SYNVEDA_DOCKER_BIN: state.fakeDocker,
    SYNVEDA_FAKE_CALL_LOG: state.log,
    SYNVEDA_FAKE_PROJECT: state.project,
    SYNVEDA_COMPOSE_RUNTIME: "development",
    SYNVEDA_POSTGRES_MODE: "bundled",
    SYNVEDA_OIDC_MODE: "bundled",
    SYNVEDA_COMPOSE_PROFILES: "demo",
    SYNVEDA_COMPOSE_PROJECT_SUFFIX: state.suffix,
    SYNVEDA_COMPOSE_IPV4_POOL: "10.231.44.0/24",
    SYNVEDA_SECRETS_DIR: state.secrets,
    SYNVEDA_DATABASE_AUTHORITY_DIR: state.authority,
    SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR: state.gate,
    SYNVEDA_OIDC_ISSUERS_FILE: state.issuer,
    ...extra,
  };
}

function run(state, action, extra = {}) {
  return spawnSync(WRAPPER, [action], {
    cwd: ROOT,
    env: environment(state, extra),
    encoding: "utf8",
  });
}

test("canonical up prepares once, reruns convergence, and keeps credentials stable", () => {
  const state = fixture();
  try {
    const first = run(state, "up");
    assert.equal(first.status, 0, first.stderr);
    const password = readFileSync(join(state.secrets, "keycloak_demo_admin_password"), "utf8");
    const issuer = readFileSync(state.issuer, "utf8");
    assert.doesNotMatch(`${first.stdout}${first.stderr}${readFileSync(state.log, "utf8")}`, new RegExp(password.trim()));

    const second = run(state, "up");
    assert.equal(second.status, 0, second.stderr);
    assert.equal(readFileSync(join(state.secrets, "keycloak_demo_admin_password"), "utf8"), password);
    assert.equal(readFileSync(state.issuer, "utf8"), issuer);
    assert.match(second.stdout, /existing secret set validated/);
    assert.match(second.stdout, /existing project-scoped issuer configuration validated/);

    const calls = readFileSync(state.log, "utf8");
    const resolver = calls.indexOf("check-host-resolution.mjs");
    const network = calls.indexOf("check-network-preflight.mjs");
    const up = calls.indexOf(" <up>");
    assert.ok(resolver >= 0 && network > resolver && up > network, calls);
    assert.match(
      calls,
      / <up> <--build> <--detach> <--wait> <--wait-timeout> <900> <--force-recreate>/,
    );
    assert.doesNotMatch(calls, /--remove-orphans/);
    assert.match(calls, /compose\.demo\.yaml/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("down retains data and reset requires exact confirmation", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    const secret = readFileSync(join(state.secrets, "synveda_kms_key"), "utf8");
    const issuer = readFileSync(state.issuer, "utf8");

    const down = run(state, "down");
    assert.equal(down.status, 0, down.stderr);
    assert.equal(existsSync(state.secrets), true);
    assert.equal(readFileSync(join(state.secrets, "synveda_kms_key"), "utf8"), secret);

    const refused = run(state, "reset", { SYNVEDA_FAKE_VOLUME_MODE: "exact" });
    assert.equal(refused.status, 64);
    assert.match(refused.stderr, new RegExp(`SYNVEDA_CONFIRM_RESET=${state.project}`));

    const reset = run(state, "reset", {
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_CONFIRM_RESET: state.project,
    });
    assert.equal(reset.status, 0, reset.stderr);
    assert.equal(readFileSync(join(state.secrets, "synveda_kms_key"), "utf8"), secret);
    assert.equal(readFileSync(state.issuer, "utf8"), issuer);
    const calls = readFileSync(state.log, "utf8");
    assert.match(
      calls,
      new RegExp(`docker <volume> <rm> <${state.project}_postgres-data>`),
    );
    assert.doesNotMatch(calls, /down> <-v>|prune|system/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reset refuses unavailable, ambiguous, or drifted volume authority before mutation", () => {
  const cases = [
    {
      extra: { SYNVEDA_FAKE_VOLUME_INVENTORY_ERROR: "1" },
      status: 69,
      error: /volume inventory was unavailable/,
    },
    {
      extra: { SYNVEDA_FAKE_VOLUME_MODE: "named-only" },
      status: 78,
      error: /exact project data volume inventory was refused/,
    },
    {
      extra: { SYNVEDA_FAKE_VOLUME_MODE: "labelled-wrong-name" },
      status: 78,
      error: /exact project data volume inventory was refused/,
    },
    ...["wrong-driver", "wrong-scope", "wrong-options"].map((contract) => ({
      extra: {
        SYNVEDA_FAKE_VOLUME_MODE: "exact",
        SYNVEDA_FAKE_VOLUME_CONTRACT: contract,
      },
      status: 78,
      error: /exact project data volume contract was refused/,
    })),
    {
      extra: {
        SYNVEDA_FAKE_VOLUME_MODE: "exact",
        SYNVEDA_FAKE_VOLUME_CONTRACT: "inspect-error",
      },
      status: 69,
      error: /exact project data volume inspection failed/,
    },
  ];

  for (const item of cases) {
    const state = fixture();
    try {
      assert.equal(run(state, "up").status, 0);
      writeFileSync(state.log, "");
      const refused = run(state, "reset", {
        SYNVEDA_CONFIRM_RESET: state.project,
        ...item.extra,
      });
      assert.equal(refused.status, item.status, refused.stderr);
      assert.match(refused.stderr, item.error);
      const calls = readFileSync(state.log, "utf8");
      assert.doesNotMatch(calls, / <down>|docker <volume> <rm>/, calls);
    } finally {
      rmSync(state.scratch, { recursive: true, force: true });
    }
  }
});

test("reset preserves authority state when the volume contract changes after shutdown", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const refused = run(state, "reset", {
      SYNVEDA_CONFIRM_RESET: state.project,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_FAKE_VOLUME_CONTRACT: "drift-after-down",
    });
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /exact project data volume changed during reset/);
    assert.equal(existsSync(state.authority), true);
    assert.equal(existsSync(state.gate), true);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, / <down>/);
    assert.doesNotMatch(calls, /docker <volume> <rm>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("reset refuses an unavailable post-removal inventory before authority deletion", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "up").status, 0);
    writeFileSync(state.log, "");
    const refused = run(state, "reset", {
      SYNVEDA_CONFIRM_RESET: state.project,
      SYNVEDA_FAKE_VOLUME_MODE: "exact",
      SYNVEDA_FAKE_POST_REMOVE_INVENTORY_ERROR: "1",
    });
    assert.equal(refused.status, 69, refused.stderr);
    assert.match(refused.stderr, /inventory was unavailable after removal/);
    assert.equal(existsSync(state.authority), true);
    assert.equal(existsSync(state.gate), true);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("hosts plan is content-free and smoke is routed through bounded checkers", () => {
  const state = fixture();
  try {
    const plan = run(state, "hosts-plan");
    assert.equal(plan.status, 0, plan.stderr);
    assert.equal(
      plan.stdout,
      `# BEGIN SYNVEDA ${state.project}\n127.0.0.1 app.synveda.test auth.synveda.test\n# END SYNVEDA ${state.project}\n`,
    );
    assert.equal(run(state, "up").status, 0);
    const smoke = run(state, "smoke");
    assert.equal(smoke.status, 0, smoke.stderr);
    const calls = readFileSync(state.log, "utf8");
    assert.match(calls, /docker .* <ps> <--all> <--format> <json>/);
    assert.match(calls, /node <.*check-runtime-smoke\.mjs>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an interrupted Docker mutation retains its exact lock through signal-reentrant cleanup", async () => {
  const state = fixture();
  const entered = join(state.scratch, "up-entered");
  const release = join(state.scratch, "up-release");
  const cleanupEntered = join(state.scratch, "cleanup-entered");
  const cleanupRelease = join(state.scratch, "cleanup-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let expectedOwnerMarker;
  const blockingEnvironment = environment(state, {
    SYNVEDA_FAKE_BLOCK_UP: "1",
    SYNVEDA_FAKE_UP_ENTERED: entered,
    SYNVEDA_FAKE_UP_RELEASE: release,
    SYNVEDA_FAKE_BLOCK_ASSET_CLEANUP: "1",
    SYNVEDA_FAKE_CLEANUP_ENTERED: cleanupEntered,
    SYNVEDA_FAKE_CLEANUP_RELEASE: cleanupRelease,
  });
  async function waitFor(path) {
    const deadline = Date.now() + 8_000;
    while (!existsSync(path)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${path}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
  }
  try {
    const first = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: blockingEnvironment,
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    expectedOwnerMarker = `${state.project}:${first.pid}\n`;
    let firstError = "";
    first.stderr.setEncoding("utf8");
    first.stderr.on("data", (chunk) => { firstError += chunk; });
    await waitFor(entered);
    const concurrent = run(state, "up");
    assert.equal(concurrent.status, 75, concurrent.stderr);
    assert.match(concurrent.stderr, /another lifecycle or authority action owns/);

    process.kill(first.pid, "SIGTERM");
    await waitFor(cleanupEntered);
    // Cleanup ignores re-entrant lifecycle signals while removing only its
    // private staging file. Mutation uncertainty deliberately retains the lock.
    process.kill(first.pid, "SIGHUP");
    writeFileSync(cleanupRelease, "");
    const [signalCode, signal] = await once(first, "close");
    assert.equal(signal, null);
    assert.equal(signalCode, 143, firstError);
    assert.equal(existsSync(release), false);
    assert.match(firstError, /retained exact-project lock because Docker mutation state is uncertain/);
    assert.equal(existsSync(lockFile), true);
    assert.equal(readFileSync(lockFile, "utf8"), expectedOwnerMarker);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);

    // This unique fixture owns the exact marker, and the spawning process has
    // closed, so the test may perform the documented operator recovery step.
    rmSync(lockFile);
    expectedOwnerMarker = undefined;
    const resumed = run(state, "up");
    assert.equal(resumed.status, 0, resumed.stderr);
  } finally {
    if (
      expectedOwnerMarker !== undefined && existsSync(lockFile) &&
      readFileSync(lockFile, "utf8") === expectedOwnerMarker
    ) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("one elapsed lifecycle budget bounds all sequential preparation phases", () => {
  const state = fixture();
  const clock = join(state.scratch, "clock");
  writeFileSync(clock, "1000\n", { mode: 0o600 });
  try {
    const result = run(state, "up", {
      SYNVEDA_COMPOSE_LIFECYCLE_TIMEOUT_SECONDS: "240",
      SYNVEDA_FAKE_CLOCK_FILE: clock,
      SYNVEDA_FAKE_CLOCK_STEP: "40",
    });
    assert.equal(result.status, 124, result.stderr);
    assert.match(result.stderr, /whole-operation lifecycle deadline expired/);
    const calls = readFileSync(state.log, "utf8");
    const budgets = [...calls.matchAll(/deadline <([0-9]+)>/g)].map((match) =>
      Number(match[1]),
    );
    assert.deepEqual(budgets, [200, 160, 120, 80, 40]);
    assert.match(calls, /check-host-resolution\.mjs/);
    assert.match(calls, /check-network-preflight\.mjs/);
    assert.doesNotMatch(calls, / <up>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("bounded-child uncertainty before Docker mutation retains the exact project lock", () => {
  const state = fixture();
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const uncertain = run(state, "up", { SYNVEDA_FAKE_RUNNER_125: "1" });
    assert.equal(uncertain.status, 125, uncertain.stderr);
    assert.match(uncertain.stderr, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${uncertain.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an uncatchably killed deadline runner cannot orphan an authority writer past lock release", async () => {
  const state = fixture();
  const runnerPidFile = join(state.scratch, "runner.pid");
  const childPidFile = join(state.scratch, "authority-child.pid");
  const entered = join(state.scratch, "openssl-entered");
  const release = join(state.scratch, "openssl-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let childPid;
  let marker;
  async function waitForFile(path) {
    const deadline = Date.now() + 8_000;
    while (!existsSync(path)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${path}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
  }
  async function waitForExit(pid) {
    const deadline = Date.now() + 8_000;
    while (true) {
      try {
        process.kill(pid, 0);
      } catch (error) {
        if (error.code === "ESRCH") return;
        throw error;
      }
      assert.ok(Date.now() < deadline, `timed out waiting for process ${pid}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
  }
  try {
    const wrapper = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: environment(state, {
        SYNVEDA_FAKE_RUNNER_PID_FILE: runnerPidFile,
        SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE: childPidFile,
        SYNVEDA_FAKE_BLOCK_OPENSSL: "1",
        SYNVEDA_FAKE_OPENSSL_ENTERED: entered,
        SYNVEDA_FAKE_OPENSSL_RELEASE: release,
      }),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let errorOutput = "";
    wrapper.stderr.setEncoding("utf8");
    wrapper.stderr.on("data", (chunk) => { errorOutput += chunk; });
    wrapper.stdout.resume();
    await waitForFile(entered);
    const runnerPid = Number(readFileSync(runnerPidFile, "utf8").trim());
    childPid = Number(readFileSync(childPidFile, "utf8").trim());
    assert.ok(Number.isSafeInteger(runnerPid) && runnerPid > 1);
    assert.ok(Number.isSafeInteger(childPid) && childPid > 1);
    process.kill(runnerPid, "SIGKILL");
    const [wrapperStatus, wrapperSignal] = await once(wrapper, "exit");
    assert.equal(wrapperSignal, null);
    assert.equal(wrapperStatus, 137, errorOutput);
    assert.match(errorOutput, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${wrapper.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    writeFileSync(release, "continue\n", { mode: 0o600 });
    await waitForExit(childPid);
    childPid = undefined;

    assert.equal(readFileSync(lockFile, "utf8"), marker);
    rmSync(lockFile);
    marker = undefined;
    const resumed = run(state, "up");
    assert.equal(resumed.status, 0, resumed.stderr);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (Number.isSafeInteger(childPid) && childPid > 1) {
      try {
        process.kill(-childPid, "SIGKILL");
      } catch {
        // The unique authority process group normally exits after release.
      }
    }
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an uncatchably killed authority writer retains its exact project lock", async () => {
  const state = fixture();
  const childPidFile = join(state.scratch, "authority-child.pid");
  const entered = join(state.scratch, "authority-entered");
  const release = join(state.scratch, "authority-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const wrapper = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: environment(state, {
        SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE: childPidFile,
        SYNVEDA_FAKE_BLOCK_AUTHORITY_ENV: "1",
        SYNVEDA_FAKE_AUTHORITY_ENV_ENTERED: entered,
        SYNVEDA_FAKE_AUTHORITY_ENV_RELEASE: release,
      }),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let errorOutput = "";
    wrapper.stderr.setEncoding("utf8");
    wrapper.stderr.on("data", (chunk) => { errorOutput += chunk; });
    wrapper.stdout.resume();
    const deadline = Date.now() + 8_000;
    while (!existsSync(entered)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${entered}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    const childPid = Number(readFileSync(childPidFile, "utf8").trim());
    assert.ok(Number.isSafeInteger(childPid) && childPid > 1);
    process.kill(childPid, "SIGKILL");
    const [wrapperStatus, wrapperSignal] = await once(wrapper, "exit");
    assert.equal(wrapperSignal, null);
    assert.equal(wrapperStatus, 137, errorOutput);
    assert.match(errorOutput, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${wrapper.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a catchable parent signal retains the lock when authority cleanup needs SIGKILL", async () => {
  const state = fixture();
  const childPidFile = join(state.scratch, "authority-child.pid");
  const entered = join(state.scratch, "authority-entered");
  const release = join(state.scratch, "authority-release");
  const lockFile = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
    `${state.project}.lock`,
  );
  let marker;
  try {
    const wrapper = spawn(WRAPPER, ["up"], {
      cwd: ROOT,
      env: environment(state, {
        SYNVEDA_FAKE_AUTHORITY_CHILD_PID_FILE: childPidFile,
        SYNVEDA_FAKE_BLOCK_AUTHORITY_ENV: "1",
        SYNVEDA_FAKE_AUTHORITY_IGNORE_SIGNALS: "1",
        SYNVEDA_FAKE_AUTHORITY_ENV_ENTERED: entered,
        SYNVEDA_FAKE_AUTHORITY_ENV_RELEASE: release,
      }),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let errorOutput = "";
    wrapper.stderr.setEncoding("utf8");
    wrapper.stderr.on("data", (chunk) => { errorOutput += chunk; });
    wrapper.stdout.resume();
    const deadline = Date.now() + 8_000;
    while (!existsSync(entered)) {
      assert.ok(Date.now() < deadline, `timed out waiting for ${entered}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    process.kill(wrapper.pid, "SIGTERM");
    const [wrapperStatus, wrapperSignal] = await once(wrapper, "exit");
    assert.equal(wrapperSignal, null);
    assert.equal(wrapperStatus, 143, errorOutput);
    assert.match(errorOutput, /forced stop bypassed command cleanup/);
    assert.match(errorOutput, /bounded child process group was not cleanly reaped/);
    marker = readFileSync(lockFile, "utf8");
    assert.equal(marker, `${state.project}:${wrapper.pid}\n`);

    const blocked = run(state, "up");
    assert.equal(blocked.status, 75, blocked.stderr);
    assert.match(blocked.stderr, /another lifecycle or authority action owns/);
  } finally {
    writeFileSync(release, "continue\n", { mode: 0o600 });
    if (marker !== undefined && existsSync(lockFile) && readFileSync(lockFile, "utf8") === marker) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("runner launch publishes pending uncertainty before its PID", () => {
  const source = readFileSync(WRAPPER, "utf8");
  const pending = source.indexOf("bounded_runner_pending=true");
  const launch = source.indexOf('node "$script_dir/run-with-deadline.mjs"', pending);
  const pid = source.indexOf("bounded_runner_pid=$!", launch);
  const clear = source.indexOf("bounded_runner_pending=false", pid);
  const handler = source.indexOf('elif [ "$bounded_runner_pending" = true ]');
  const retain = source.indexOf("lifecycle_child_uncertain=true", handler);
  assert.ok(pending >= 0 && launch > pending && pid > launch && clear > pid);
  assert.ok(handler >= 0 && retain > handler);
});

test("runner completion witness is settled before PID state is cleared", () => {
  const source = readFileSync(WRAPPER, "utf8");
  const staged = source.indexOf("bounded_status_file=$(mktemp");
  const launch = source.indexOf("--status-file \"$bounded_status_file\"", staged);
  const waiting = source.indexOf("bounded_runner_waiting=true", launch);
  const wait = source.indexOf('wait "$bounded_runner_pid" || bounded_status=$?', waiting);
  const settlement = source.indexOf('settle_bounded_runner "$bounded_status"', wait);
  const killed = source.indexOf('if [ "$bounded_status" -ge 128 ]', settlement);
  const settled = source.indexOf("bounded_runner_waiting=false", killed);
  const cleared = source.indexOf("bounded_runner_pid=", settled);
  assert.ok(staged >= 0 && launch > staged && waiting > launch && wait > waiting);
  assert.ok(settlement > wait && killed > settlement && settled > killed);
  assert.ok(cleared > settled);
  assert.match(source, /recorded_bounded_status" = "clean:\$settled_status"/);
  assert.match(
    source,
    /if \[ "\$settled_status" -eq 125 \] \|\| \[ "\$bounded_group_clean" != true \]; then\n\s+lifecycle_child_uncertain=true/,
  );
  assert.match(
    source,
    /wait "\$bounded_runner_pid" 2>\/dev\/null \|\| signal_wait_status=\$\?[\s\S]*settle_bounded_runner "\$signal_wait_status"/,
  );
});

test("the project lock publishes and owns one verified hard-link inode", () => {
  const source = readFileSync(PROJECT_LOCK, "utf8");
  const link = source.indexOf('ln "$project_lock_claim_file" "$project_lock_file"');
  const verify = source.indexOf(
    '"$project_lock_claim_identity" ]; then',
    link,
  );
  const identity = source.indexOf("project_lock_identity=$project_lock_claim_identity", verify);
  const owned = source.indexOf("project_lock_owned=true", identity);
  assert.ok(link >= 0 && verify > link && identity > verify && owned > identity);
  assert.doesNotMatch(source, /mkdir[^\n]*\$project_lock_file/);
});

test("down refuses a remote Docker endpoint before mutation", () => {
  const state = fixture();
  try {
    const prepared = run(state, "up");
    assert.equal(prepared.status, 0, prepared.stderr);
    writeFileSync(state.log, "");
    const refused = run(state, "down", {
      DOCKER_HOST: "tcp://remote.example:2376",
      SYNVEDA_FAKE_REAL_HOST_CHECK: "1",
    });
    assert.equal(refused.status, 69, refused.stderr);
    assert.match(refused.stderr, /local Docker endpoint is required/);
    const calls = readFileSync(state.log, "utf8");
    assert.doesNotMatch(calls, / <down>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("the validated local Docker socket is pinned across later lifecycle calls", () => {
  const state = fixture();
  try {
    const endpoint = `unix://${join(state.scratch, "docker.sock")}`;
    const result = run(state, "up", {
      DOCKER_CONTEXT: "context-that-may-change",
      SYNVEDA_FAKE_LOCAL_DOCKER_ENDPOINT: endpoint,
      SYNVEDA_FAKE_REQUIRE_PINNED_DOCKER: "1",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(state.log, "utf8"), / <up>/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an abandoned exact-project lock fails closed", () => {
  const state = fixture();
  const lockRoot = join(
    "/tmp",
    `.synveda-compose-locks-${process.getuid?.() ?? 0}`,
  );
  const lockFile = join(lockRoot, `${state.project}.lock`);
  const ownerMarker = `${state.project}:999999\n`;
  let fixtureLockCreated = false;
  try {
    mkdirSync(lockRoot, { recursive: true, mode: 0o700 });
    writeFileSync(lockFile, ownerMarker, { mode: 0o600 });
    fixtureLockCreated = true;
    const refused = run(state, "up");
    assert.equal(refused.status, 75, refused.stderr);
    assert.match(refused.stderr, /another lifecycle or authority action owns/);
    assert.equal(existsSync(lockFile), true);
  } finally {
    if (
      fixtureLockCreated && existsSync(lockFile) &&
      readFileSync(lockFile, "utf8") === ownerMarker
    ) {
      rmSync(lockFile);
    }
    rmSync(state.scratch, { recursive: true, force: true });
  }
});
