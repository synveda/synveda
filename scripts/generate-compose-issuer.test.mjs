import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const GENERATOR = join(ROOT, "deploy/compose/scripts/generate-issuer.sh");
const TENANT_A = "019b53c0-7c00-7000-8000-000000000045";
const TENANT_B = "019b53c0-7c00-7000-8000-000000000046";

function environment(target, extra = {}) {
  return {
    ...process.env,
    SYNVEDA_COMPOSE_RUNTIME: "development",
    SYNVEDA_OIDC_MODE: "bundled",
    SYNVEDA_APP_HOST: "app.synveda.test",
    SYNVEDA_AUTH_HOST: "auth.synveda.test",
    SYNVEDA_PUBLIC_SCHEME: "http",
    SYNVEDA_DEV_HTTP_PORT: "8080",
    SYNVEDA_BOOTSTRAP_TENANT_ID: TENANT_A,
    SYNVEDA_OIDC_ISSUERS_FILE: target,
    SYNVEDA_COMPOSE_PROJECT_SUFFIX: "",
    SYNVEDA_CONFIRM_ISSUER_REPLACEMENT: "",
    ...extra,
  };
}

async function privateTarget() {
  const root = await mkdtemp(join(tmpdir(), "synveda-issuer-test-"));
  const parent = join(root, "runtime");
  await mkdir(parent, { mode: 0o700 });
  return { root, target: join(parent, "issuers.json") };
}

test("bundled issuer generation is private, exact, and content-free", async () => {
  const { root, target } = await privateTarget();
  try {
    const output = execFileSync(GENERATOR, [], {
      cwd: ROOT,
      env: environment(target),
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
  const { root, target } = await privateTarget();
  try {
    execFileSync(GENERATOR, [], { cwd: ROOT, env: environment(target) });
    const original = await readFile(target, "utf8");

    for (const [args, extra] of [
      [[], {}],
      [["--force"], {}],
      [["--force"], { SYNVEDA_CONFIRM_ISSUER_REPLACEMENT: "wrong-project" }],
    ]) {
      const refused = spawnSync(GENERATOR, args, {
        cwd: ROOT,
        env: environment(target, extra),
        encoding: "utf8",
      });
      assert.equal(refused.status, 73, refused.stderr);
      assert.equal(await readFile(target, "utf8"), original);
    }

    execFileSync(GENERATOR, ["--force"], {
      cwd: ROOT,
      env: environment(target, {
        SYNVEDA_BOOTSTRAP_TENANT_ID: TENANT_B,
        SYNVEDA_CONFIRM_ISSUER_REPLACEMENT: "synveda-development",
      }),
    });
    assert.equal(await readFile(`${target}.previous`, "utf8"), original);
    const replaced = JSON.parse(await readFile(target, "utf8"));
    assert.equal(replaced[0].tenant.static.tenant_id, TENANT_B);
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
    const { root, target } = await privateTarget();
    try {
      const refused = spawnSync(GENERATOR, [], {
        cwd: ROOT,
        env: environment(target, extra),
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
  const refused = spawnSync(GENERATOR, [], {
    cwd: ROOT,
    env: environment("./runtime/not-project-scoped.json"),
    encoding: "utf8",
  });
  assert.equal(refused.status, 73, refused.stderr);
});
