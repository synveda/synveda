import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdtemp, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const GENERATOR = join(ROOT, "deploy/compose/scripts/generate-issuer.sh");
const TENANT_A = "019b53c0-7c00-7000-8000-000000000045";
const TENANT_B = "019b53c0-7c00-7000-8000-000000000046";

function environment(fixture, extra = {}) {
  return {
    ...process.env,
    SYNVEDA_COMPOSE_RUNTIME: "development",
    SYNVEDA_OIDC_MODE: "bundled",
    SYNVEDA_APP_HOST: "app.synveda.test",
    SYNVEDA_AUTH_HOST: "auth.synveda.test",
    SYNVEDA_PUBLIC_SCHEME: "http",
    SYNVEDA_DEV_HTTP_PORT: "8080",
    SYNVEDA_BOOTSTRAP_TENANT_ID: TENANT_A,
    SYNVEDA_OIDC_ISSUERS_FILE: fixture.target,
    SYNVEDA_COMPOSE_PROJECT_SUFFIX: fixture.suffix,
    SYNVEDA_CONFIRM_ISSUER_REPLACEMENT: "",
    ...extra,
  };
}

async function privateTarget() {
  const root = await mkdtemp(join(tmpdir(), "synveda-issuer-test-"));
  const suffix = `acceptance-issuer${randomBytes(4).toString("hex")}`;
  const project = `synveda-development-${suffix}`;
  const parent = join(root, project);
  await mkdir(parent, { mode: 0o700 });
  return {
    root,
    target: join(parent, "issuers.json"),
    suffix,
    project,
  };
}

test("bundled issuer generation is private, exact, and content-free", async () => {
  const fixture = await privateTarget();
  const { root, target } = fixture;
  try {
    const output = execFileSync(GENERATOR, [], {
      cwd: ROOT,
      env: environment(fixture),
      encoding: "utf8",
    });
    assert.equal(output, "generated project-scoped issuer configuration\n");

    const metadata = await stat(target);
    assert.equal(metadata.mode & 0o777, 0o600);
    const parsed = JSON.parse(await readFile(target, "utf8"));
    assert.deepEqual(parsed, [
      {
        issuer: "http://auth.synveda.test:8080/realms/synveda",
        client_id: "synveda",
        audience: "synveda-api",
        tenant: { static: { tenant_id: TENANT_A } },
        login_scopes: ["openid", "profile", "email"],
      },
    ]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("replacement needs both force and the exact project confirmation", async () => {
  const fixture = await privateTarget();
  const { root, target, project } = fixture;
  try {
    execFileSync(GENERATOR, [], { cwd: ROOT, env: environment(fixture) });
    const original = await readFile(target, "utf8");

    for (const [args, extra] of [
      [[], {}],
      [["--force"], {}],
      [["--force"], { SYNVEDA_CONFIRM_ISSUER_REPLACEMENT: "wrong-project" }],
    ]) {
      const refused = spawnSync(GENERATOR, args, {
        cwd: ROOT,
        env: environment(fixture, extra),
        encoding: "utf8",
      });
      assert.equal(refused.status, 73, refused.stderr);
      assert.equal(await readFile(target, "utf8"), original);
    }

    execFileSync(GENERATOR, ["--force"], {
      cwd: ROOT,
      env: environment(fixture, {
        SYNVEDA_BOOTSTRAP_TENANT_ID: TENANT_B,
        SYNVEDA_CONFIRM_ISSUER_REPLACEMENT: project,
      }),
    });
    assert.equal(await readFile(`${target}.previous`, "utf8"), original);
    const replaced = JSON.parse(await readFile(target, "utf8"));
    assert.equal(replaced[0].tenant.static.tenant_id, TENANT_B);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("failed forced publication retains the authoritative and preserved inputs", async () => {
  const fixture = await privateTarget();
  const { root, target, project } = fixture;
  try {
    execFileSync(GENERATOR, [], { cwd: ROOT, env: environment(fixture) });
    const original = await readFile(target, "utf8");
    const fakeBin = join(root, "bin");
    await mkdir(fakeBin, { mode: 0o700 });
    const realMv = execFileSync("/bin/sh", ["-c", "command -v mv"], {
      encoding: "utf8",
    }).trim();
    await writeFile(
      join(fakeBin, "mv"),
      `#!/bin/sh
case " $* " in
  *".claim."*) exec "$SYNVEDA_TEST_REAL_MV" "$@" ;;
esac
exit 1
`,
      { mode: 0o700 },
    );

    const refused = spawnSync(GENERATOR, ["--force"], {
      cwd: ROOT,
      env: environment(fixture, {
        PATH: `${fakeBin}:${process.env.PATH}`,
        SYNVEDA_TEST_REAL_MV: realMv,
        SYNVEDA_BOOTSTRAP_TENANT_ID: TENANT_B,
        SYNVEDA_CONFIRM_ISSUER_REPLACEMENT: project,
      }),
      encoding: "utf8",
    });
    assert.equal(refused.status, 73, refused.stderr);
    assert.equal(await readFile(target, "utf8"), original);
    assert.equal(await readFile(`${target}.previous`, "utf8"), original);
    assert.deepEqual(
      (await readdir(dirname(target))).filter((name) => name.startsWith(".issuers.")),
      [],
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a late fresh issuer target is preserved and publication fails closed", async () => {
  const fixture = await privateTarget();
  const { root, target } = fixture;
  try {
    const fakeBin = join(root, "bin");
    await mkdir(fakeBin, { mode: 0o700 });
    const realChmod = execFileSync("/bin/sh", ["-c", "command -v chmod"], {
      encoding: "utf8",
    }).trim();
    await writeFile(
      join(fakeBin, "chmod"),
      `#!/bin/sh
set -eu
case " $* " in
  *".issuers."*)
    if [ ! -e "$SYNVEDA_TEST_RACE_TARGET" ]; then
      printf 'foreign-issuer\n' > "$SYNVEDA_TEST_RACE_TARGET"
      "$SYNVEDA_TEST_REAL_CHMOD" 600 "$SYNVEDA_TEST_RACE_TARGET"
    fi
    ;;
esac
exec "$SYNVEDA_TEST_REAL_CHMOD" "$@"
`,
      { mode: 0o700 },
    );
    const refused = spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: environment(fixture, {
        PATH: `${fakeBin}:${process.env.PATH}`,
        SYNVEDA_TEST_RACE_TARGET: target,
        SYNVEDA_TEST_REAL_CHMOD: realChmod,
      }),
      encoding: "utf8",
    });
    assert.equal(refused.status, 73, refused.stderr);
    assert.match(refused.stderr, /staged input could not be installed/);
    assert.equal(await readFile(target, "utf8"), "foreign-issuer\n");
    await assert.rejects(stat(`${target}.previous`), { code: "ENOENT" });
    assert.deepEqual(
      (await readdir(dirname(target))).filter((name) => name.startsWith(".issuers.")),
      [],
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("unsafe issuer inputs fail before a file is written", async () => {
  for (const extra of [
    { SYNVEDA_COMPOSE_PROJECT_SUFFIX: "acceptance-invalid-" },
    { SYNVEDA_AUTH_HOST: "-auth.synveda.test" },
    { SYNVEDA_AUTH_HOST: "auth.example.invalid" },
    { SYNVEDA_AUTH_HOST: "127.0.0.2" },
    { SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7g00-8000-000000000045" },
  ]) {
    const fixture = await privateTarget();
    const { root, target } = fixture;
    try {
      const refused = spawnSync(GENERATOR, [], {
        cwd: ROOT,
        env: environment(fixture, extra),
        encoding: "utf8",
      });
      assert.equal(refused.status, 64, refused.stderr);
      await assert.rejects(stat(target), { code: "ENOENT" });
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }
});

test("a custom relative issuer target is refused", () => {
  const suffix = `acceptance-issuer${randomBytes(4).toString("hex")}`;
  const refused = spawnSync(GENERATOR, [], {
    cwd: ROOT,
    env: environment({ target: "./runtime/not-project-scoped.json", suffix }),
    encoding: "utf8",
  });
  assert.equal(refused.status, 73, refused.stderr);
});

test("a bundled issuer target scoped to another Compose project is refused", async () => {
  const fixture = await privateTarget();
  const foreignParent = join(fixture.root, `${fixture.project}-foreign`);
  await mkdir(foreignParent, { mode: 0o700 });
  const foreign = { ...fixture, target: join(foreignParent, "issuers.json") };
  try {
    const refused = spawnSync(GENERATOR, [], {
      cwd: ROOT,
      env: environment(foreign),
      encoding: "utf8",
    });
    assert.equal(refused.status, 73, refused.stderr);
    assert.match(refused.stderr, /issuer input must be scoped to project/);
    await assert.rejects(stat(foreign.target), { code: "ENOENT" });
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});
