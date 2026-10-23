import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const launcher = new URL("../deploy/compose/scripts/evaluation.sh", import.meta.url);
const image = (name) => `docker.io/synveda/${name}@sha256:${name === "from" ? "a".repeat(64) : "b".repeat(64)}`;
const fragments = ["compose.yaml", "compose.postgres.yaml", "compose.keycloak.yaml", "compose.keycloak-postgres.yaml", "compose.evaluation.yaml", "compose.demo.yaml"];

// The fake engine logs one line per invocation, fails the Compose call whose
// arguments end with FAKE_FAIL, and simulates only the effects the launcher
// reads back: preparation rewriting the env and a snapshot set.
const fakeDocker = `#!/bin/sh
printf '%s\\n' "$*" >> "$FAKE_DOCKER_LOG"
case "$1" in
  context) [ "$2" = show ] && echo default || echo unix:///var/run/docker.sock; exit 0 ;;
  volume) exit 0 ;;
  ps) case "$*" in *"service=$FAKE_RUNNING"*) echo running-container ;; esac; exit 0 ;;
  run)
    case "$*" in
      *prepare-evaluation.mjs*)
        printf "SYNVEDA_PRODUCT_IMAGE='%s'\\n" "$FAKE_TARGET_IMAGE" > "$FAKE_STATE/evaluation.env" ;;
      *"evaluation-recovery.mjs snapshot"*)
        : > "$FAKE_STATE/backups/$FAKE_BACKUP_ID/configuration.json" ;;
    esac
    exit 0 ;;
  compose)
    case "$*" in *"$FAKE_FAIL") [ -n "$FAKE_FAIL" ] && exit 1 ;; esac
    exit 0 ;;
esac
exit 0
`;

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "synveda-evaluation-upgrade-"));
  const bin = join(root, "bin"), home = join(root, "home"), state = join(home, ".synveda/state");
  for (const [bundle, name] of [["from", "from"], ["to", "to"]]) {
    mkdirSync(join(root, bundle, "deploy/compose/scripts"), { recursive: true });
    writeFileSync(join(root, bundle, "product-image"), `${image(name)}\n`);
  }
  copyFileSync(launcher, join(root, "to/deploy/compose/scripts/evaluation.sh"));
  mkdirSync(bin, { recursive: true });
  writeFileSync(join(bin, "docker"), fakeDocker);
  chmodSync(join(bin, "docker"), 0o755);
  mkdirSync(join(state, "synveda-evaluation/secrets"), { recursive: true, mode: 0o700 });
  writeFileSync(join(state, "synveda-evaluation/secrets/.synveda-private-directory"), "");
  writeFileSync(join(state, "evaluation.env"), `SYNVEDA_PRODUCT_IMAGE='${image("from")}'\n`);
  writeFileSync(join(state, "evaluation-files"), `${fragments.join("\n")}\n`);
  return { root, bin, home, state, log: join(root, "docker.log"), receipt: join(state, "upgrades/pre-upgrade") };
}

function upgrade(context, env = {}) {
  writeFileSync(context.log, "");
  const result = spawnSync("sh", [join(context.root, "to/deploy/compose/scripts/evaluation.sh"), "upgrade", "pre-upgrade"], {
    encoding: "utf8",
    env: {
      PATH: `${context.bin}:${process.env.PATH}`, HOME: context.home,
      SYNVEDA_UPGRADE_FROM: join(context.root, "from"),
      SYNVEDA_CONFIRM_UPGRADE: "upgrade:synveda-evaluation:pre-upgrade",
      FAKE_DOCKER_LOG: context.log, FAKE_STATE: context.state, FAKE_BACKUP_ID: "pre-upgrade",
      FAKE_TARGET_IMAGE: image("to"), FAKE_FAIL: "", FAKE_RUNNING: "none",
      ...env,
    },
  });
  const calls = readFileSync(context.log, "utf8").split("\n").filter((line) => line && !line.startsWith("context "));
  return { ...result, calls };
}

const phase = (context) => readFileSync(context.receipt, "utf8").match(/^phase=(.*)$/m)?.[1];
const step = (calls, pattern) => calls.findIndex((call) => pattern.test(call));

test("upgrade stops previous writers, backs up with the installed release and checks before migrating", (t) => {
  const context = fixture();
  t.after(() => rmSync(context.root, { recursive: true, force: true }));
  const result = upgrade(context);
  assert.equal(result.status, 0, result.stderr);
  const { calls } = result;
  const from = join(context.root, "from/deploy/compose"), to = join(context.root, "to/deploy/compose");
  const order = [
    step(calls, new RegExp(`^compose .*-f ${from}/compose\\.yaml .*stop --timeout 210 gateway worker keycloak-realm-convergence keycloak$`)),
    step(calls, new RegExp(`^compose .*-f ${from}/compose\\.backup\\.yaml run --rm --no-deps database-backup$`)),
    step(calls, new RegExp(`^run .*${image("from").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} .*evaluation-recovery\\.mjs snapshot pre-upgrade$`)),
    step(calls, /^run .*prepare-evaluation\.mjs$/),
    step(calls, new RegExp(`^compose .*-f ${to}/compose\\.yaml .*up --detach --no-build --wait --wait-timeout 900 postgres$`)),
    step(calls, /run --rm --no-deps database-bootstrap$/),
    step(calls, /run --rm --no-deps keycloak-database-bootstrap$/),
    step(calls, /run --rm --no-deps database-preflight$/),
    step(calls, /run --rm --no-deps --no-TTY migrate migration-check$/),
    step(calls, /run --rm --no-deps --no-TTY migrate$/),
    step(calls, /up --detach --no-build --wait --wait-timeout 900$/),
  ];
  assert.ok(order.every((index) => index >= 0), `missing step in:\n${calls.join("\n")}`);
  assert.deepEqual([...order].sort((a, b) => a - b), order, "steps ran out of order");
  assert.equal(calls.filter((call) => / database-bootstrap$/.test(call)).length, 1);
  assert.equal(calls.filter((call) => / keycloak-database-bootstrap$/.test(call)).length, 1);
  assert.equal(phase(context), "complete");
  const receipt = readFileSync(context.receipt, "utf8");
  for (const recorded of ["started", "writers-stopped", "backed-up", "prepared", "migrating", "migrated", "complete"]) {
    assert.match(receipt, new RegExp(`^${recorded}_at=\\d+$`, "m"), recorded);
  }
  assert.match(receipt, new RegExp(`^from_image=${image("from").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`, "m"));
});

test("a refused compatibility check stops before the schema change and resumes without a second backup", (t) => {
  const context = fixture();
  t.after(() => rmSync(context.root, { recursive: true, force: true }));
  const refused = upgrade(context, { FAKE_FAIL: "migrate migration-check" });
  assert.equal(refused.status, 78);
  assert.match(refused.stderr, /before any schema change.*from\/synveda-compose up/s);
  assert.equal(phase(context), "prepared");
  assert.equal(step(refused.calls, /--no-TTY migrate$/), -1, "migration must not start after a refused check");

  const running = upgrade(context, { FAKE_RUNNING: "gateway" });
  assert.equal(running.status, 78);
  assert.match(running.stderr, /gateway is running again.*may be stale/);
  assert.equal(phase(context), "prepared");

  const resumed = upgrade(context);
  assert.equal(resumed.status, 0, resumed.stderr);
  for (const absent of [/ stop --timeout/, /database-backup$/, /evaluation-recovery\.mjs snapshot/]) {
    assert.equal(step(resumed.calls, absent), -1, `resume repeated ${absent}`);
  }
  assert.ok(step(resumed.calls, /migrate migration-check$/) < step(resumed.calls, /--no-TTY migrate$/));
  assert.equal(phase(context), "complete");
});

test("a failed migration directs roll-forward and resumes without repeating the read-only check", (t) => {
  const context = fixture();
  t.after(() => rmSync(context.root, { recursive: true, force: true }));
  const failed = upgrade(context, { FAKE_FAIL: "--no-TTY migrate" });
  assert.equal(failed.status, 78);
  assert.match(failed.stderr, /after the schema change began.*refuses this database.*restore backup pre-upgrade/s);
  assert.equal(phase(context), "migrating");

  const resumed = upgrade(context);
  assert.equal(resumed.status, 0, resumed.stderr);
  assert.equal(step(resumed.calls, /migration-check$/), -1);
  assert.ok(step(resumed.calls, / database-bootstrap$/) < step(resumed.calls, /keycloak-database-bootstrap$/));
  assert.ok(step(resumed.calls, /keycloak-database-bootstrap$/) < step(resumed.calls, /--no-TTY migrate$/));
  assert.equal(phase(context), "complete");
});

test("upgrade refuses before any Docker mutation without exact confirmation and the installed release", (t) => {
  const context = fixture();
  t.after(() => rmSync(context.root, { recursive: true, force: true }));
  const mutation = (calls) => calls.filter((call) => !/^(volume inspect|ps )/.test(call));
  for (const [env, message] of [
    [{ SYNVEDA_CONFIRM_UPGRADE: "upgrade:synveda-evaluation:other" }, /requires SYNVEDA_CONFIRM_UPGRADE/],
    [{ SYNVEDA_UPGRADE_FROM: "relative/bundle" }, /absolute path/],
    [{ SYNVEDA_UPGRADE_FROM: join(context.root, "to") }, /run upgrade from the target bundle/],
  ]) {
    const result = upgrade(context, env);
    assert.equal(result.status, 78);
    assert.match(result.stderr, message);
    assert.deepEqual(mutation(result.calls), []);
  }
  writeFileSync(join(context.root, "from/product-image"), `${image("to")}\n`);
  let result = upgrade(context);
  assert.equal(result.status, 78);
  assert.match(result.stderr, /same product image/);
  writeFileSync(join(context.root, "from/product-image"), `${image("from")}\n`);
  writeFileSync(join(context.state, "evaluation.env"), `SYNVEDA_PRODUCT_IMAGE='${image("other")}'\n`);
  result = upgrade(context);
  assert.equal(result.status, 78);
  assert.match(result.stderr, /not the release this installation runs/);
  assert.deepEqual(mutation(result.calls), []);
  assert.equal(existsSync(context.receipt), false);
  writeFileSync(join(context.state, "evaluation.env"), `SYNVEDA_PRODUCT_IMAGE='${image("from")}'\n`);
  mkdirSync(join(context.state, "backups/pre-upgrade"), { recursive: true });
  result = upgrade(context);
  assert.equal(result.status, 78);
  assert.match(result.stderr, /backup id already exists/);
  assert.deepEqual(mutation(result.calls), []);
});
