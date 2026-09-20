import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

const root = resolve(import.meta.dirname, "..");
const installer = join(root, "scripts/install.sh");
const version = "0.2.0";
const sourceSha = "1".repeat(40);
const digest = (digit) => `sha256:${digit.repeat(64)}`;
const assetNames = (releaseVersion) => [
  `synveda-${releaseVersion}-darwin-arm64.tar.gz`,
  `synveda-${releaseVersion}-linux-x86_64.tar.gz`,
  `synveda-console-${releaseVersion}.tar.gz`,
  `synveda-reference-${releaseVersion}.tar.gz`,
  `synveda-plugin-${releaseVersion}.tar.gz`,
];

function archive(directory, output, entries) {
  execFileSync("tar", ["-czf", output, "-C", directory, ...entries]);
}

function writeChecksums(assets, releaseVersion) {
  const sums = assetNames(releaseVersion).map((name) => {
    const hash = createHash("sha256").update(readFileSync(join(assets, name))).digest("hex");
    return `${hash} ${name}`;
  });
  writeFileSync(join(assets, "SHA256SUMS"), `${sums.join("\n")}\n`);
}

function buildAssets(scratch, releaseVersion = version, releaseSourceSha = sourceSha) {
  const assets = join(scratch, "assets");
  const native = join(scratch, "native");
  const consoleStage = join(scratch, "console-stage");
  const pluginStage = join(scratch, "plugin-stage");
  mkdirSync(assets);
  mkdirSync(native);
  for (const name of ["synveda", "synveda-gateway", "synveda-worker"]) {
    const path = join(native, name);
    writeFileSync(path, `#!/bin/sh\necho ${name}\n`);
    chmodSync(path, 0o755);
  }
  for (const target of ["darwin-arm64", "linux-x86_64"]) {
    archive(native, join(assets, `synveda-${releaseVersion}-${target}.tar.gz`), [
      "synveda",
      "synveda-gateway",
      "synveda-worker",
    ]);
  }
  mkdirSync(join(consoleStage, "console"), { recursive: true });
  writeFileSync(join(consoleStage, "console/index.html"), "console\n");
  archive(consoleStage, join(assets, `synveda-console-${releaseVersion}.tar.gz`), ["console"]);
  mkdirSync(join(pluginStage, "plugin"), { recursive: true });
  writeFileSync(join(pluginStage, "plugin/manifest.json"), "{}\n");
  for (const client of ["codex", "copilot-cli"]) {
    const runtime = join(pluginStage, "plugin", client);
    mkdirSync(join(runtime, "dist"), { recursive: true });
    writeFileSync(join(runtime, "dist/hook.mjs"), `// fixture ${releaseVersion}\n`);
    writeFileSync(join(runtime, "dist/transcript.mjs"), "// fixture\n");
    writeFileSync(join(runtime, "package.json"), "{}\n");
    const shared = join(runtime, "node_modules/@synveda/claude-code-adapter");
    mkdirSync(join(shared, "dist"), { recursive: true });
    writeFileSync(join(shared, "package.json"), "{}\n");
    writeFileSync(join(shared, "dist/session-runtime.mjs"), "// fixture\n");
  }
  archive(pluginStage, join(assets, `synveda-plugin-${releaseVersion}.tar.gz`), ["plugin"]);
  execFileSync(
    "bash",
    [
      "scripts/package-release.sh",
      releaseVersion,
      assets,
      releaseSourceSha,
      digest("2"),
      digest("3"),
      digest("4"),
      digest("5"),
      digest("6"),
      digest("7"),
    ],
    { cwd: root, stdio: "pipe" },
  );
  writeChecksums(assets, releaseVersion);
  return assets;
}

function installEnv(scratch, assets, releaseVersion = version) {
  const home = join(scratch, "home");
  const bin = join(scratch, "bin");
  return {
    home,
    bin,
    env: {
      ...process.env,
      HOME: join(scratch, "user-home"),
      SYNVEDA_BASE_URL: `file://${assets}`,
      SYNVEDA_BIN: bin,
      SYNVEDA_HOME: home,
      SYNVEDA_VERSION: releaseVersion,
    },
  };
}

test("installer converges the canonical reference and preserves mutable state", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  const fixture = installEnv(scratch, assets);
  const project = join(scratch, "project");
  mkdirSync(project);

  const first = spawnSync("/bin/sh", [installer], {
    cwd: project,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(first.status, 0, first.stderr);
  assert.equal(statSync(fixture.bin).isDirectory(), true);
  assert.match(first.stdout, /synveda-compose up/);
  assert.doesNotMatch(first.stdout, /synveda-compose config/);
  assert.match(first.stdout, new RegExp(`/blob/${sourceSha}/docs/INSTALL\\.md`));
  assert.doesNotMatch(first.stdout, /\/blob\/main\/docs\/INSTALL\.md/);
  assert.equal(
    readlinkSync(join(fixture.home, "reference/current")),
    `releases/${version}-${sourceSha}`,
  );
  const current = join(fixture.home, "reference/current");
  assert.equal(readFileSync(join(current, "version"), "utf8"), `${version}\n`);
  const guide = readFileSync(join(current, "INSTALL.md"), "utf8");
  assert.match(guide, /synveda-compose secrets/);
  assert.ok(guide.includes(`/blob/${sourceSha}/docs/PRODUCTION_READINESS.md`));
  assert.doesNotMatch(guide, /\/blob\/main\//);
  for (const document of ["LICENSE", "NOTICE"]) {
    assert.equal(readFileSync(join(current, document), "utf8"), readFileSync(join(root, document), "utf8"));
  }
  assert.match(
    readFileSync(join(current, "environment.json"), "utf8"),
    /ghcr\.io\/synveda\/product@sha256:2{64}/,
  );
  assert.equal(existsSync(join(current, "deploy/compose/rauthy")), false);
  assert.equal(existsSync(join(current, "deploy/compose/compose.dev.yaml")), false);
  assert.equal(statSync(join(fixture.home, "state")).mode & 0o777, 0o700);
  const codexHook = join(fixture.home, "plugin/codex/dist/hook.mjs");
  assert.ok(first.stdout.includes(codexHook));
  assert.equal(readFileSync(codexHook, "utf8"), `// fixture ${version}\n`);
  const clientConfig = join(fixture.env.HOME, ".codex/config.toml");
  assert.equal(existsSync(clientConfig), false, "install must not configure Codex");
  mkdirSync(join(fixture.env.HOME, ".codex"), { recursive: true });
  writeFileSync(clientConfig, "# user-owned configuration\n");
  const copilotHook = join(fixture.home, "plugin/copilot-cli/dist/hook.mjs");
  assert.ok(first.stdout.includes(copilotHook));
  assert.match(first.stdout, new RegExp(`/blob/${sourceSha}/docs/integrations/copilot-cli\\.md`));
  assert.equal(readFileSync(copilotHook, "utf8"), `// fixture ${version}\n`);
  const copilotConfig = join(fixture.env.HOME, ".copilot/mcp-config.json");
  const projectHooks = join(project, ".github/hooks/synveda.json");
  assert.equal(existsSync(copilotConfig), false, "install must not configure Copilot MCP");
  assert.equal(existsSync(projectHooks), false, "install must not configure project hooks");
  mkdirSync(join(fixture.env.HOME, ".copilot"), { recursive: true });
  mkdirSync(join(project, ".github/hooks"), { recursive: true });
  const copilotConfigBytes = '{"mcpServers":{"user-owned":{"command":"custom-server"}}}\n';
  const projectHookBytes = '{"version":1,"hooks":{"sessionStart":[]}}\n';
  writeFileSync(copilotConfig, copilotConfigBytes);
  writeFileSync(projectHooks, projectHookBytes);

  const sentinel = join(fixture.home, "state/synveda-reference/operator-sentinel");
  writeFileSync(sentinel, "preserve\n");
  const second = spawnSync("/bin/sh", [installer], {
    cwd: project,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(second.status, 0, second.stderr);
  assert.equal(readFileSync(codexHook, "utf8"), `// fixture ${version}\n`);
  assert.equal(readFileSync(clientConfig, "utf8"), "# user-owned configuration\n");
  assert.equal(readFileSync(copilotHook, "utf8"), `// fixture ${version}\n`);
  assert.equal(readFileSync(copilotConfig, "utf8"), copilotConfigBytes);
  assert.equal(readFileSync(projectHooks, "utf8"), projectHookBytes);
  assert.equal(readFileSync(sentinel, "utf8"), "preserve\n");
  assert.equal(readFileSync(join(fixture.bin, "synveda"), "utf8").startsWith("#!/bin/sh"), true);

  const nextVersion = "0.2.1";
  const nextSourceSha = "8".repeat(40);
  const upgradeScratch = join(scratch, "upgrade");
  mkdirSync(upgradeScratch);
  const upgradeAssets = buildAssets(upgradeScratch, nextVersion, nextSourceSha);
  const upgrade = spawnSync("/bin/sh", [installer], {
    cwd: project,
    encoding: "utf8",
    env: {
      ...fixture.env,
      SYNVEDA_BASE_URL: `file://${upgradeAssets}`,
      SYNVEDA_VERSION: nextVersion,
    },
  });
  assert.equal(upgrade.status, 0, upgrade.stderr);
  assert.equal(readFileSync(codexHook, "utf8"), `// fixture ${nextVersion}\n`);
  assert.equal(readFileSync(clientConfig, "utf8"), "# user-owned configuration\n");
  assert.equal(readFileSync(copilotHook, "utf8"), `// fixture ${nextVersion}\n`);
  assert.equal(readFileSync(copilotConfig, "utf8"), copilotConfigBytes);
  assert.equal(readFileSync(projectHooks, "utf8"), projectHookBytes);
  assert.equal(
    readlinkSync(join(fixture.home, "reference/current")),
    `releases/${nextVersion}-${nextSourceSha}`,
  );
  assert.equal(
    existsSync(join(fixture.home, `reference/releases/${version}-${sourceSha}`)),
    true,
  );
  assert.equal(readFileSync(sentinel, "utf8"), "preserve\n");

  const delegate = join(current, "deploy/compose/scripts/compose.sh");
  writeFileSync(
    delegate,
    [
      "#!/bin/sh",
      'printf "%s|%s|%s|%s|%s|%s\\n" "$SYNVEDA_COMPOSE_RUNTIME" "$SYNVEDA_PUBLIC_SCHEME" "$SYNVEDA_COMPOSE_IPV4_POOL" "$SYNVEDA_SECRETS_DIR" "$SYNVEDA_DATABASE_BACKUP_ROOT" "$1"',
      "",
    ].join("\n"),
  );
  chmodSync(delegate, 0o755);
  const launch = spawnSync("/bin/sh", [join(current, "synveda-compose"), "config"], {
    cwd: scratch,
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: fixture.env.HOME,
      SYNVEDA_HOME: fixture.home,
      SYNVEDA_APP_HOST: "app.example.com",
      SYNVEDA_AUTH_HOST: "auth.example.com",
    },
  });
  assert.equal(launch.status, 0, launch.stderr);
  const installedHome = realpathSync(fixture.home);
  assert.equal(
    launch.stdout,
    `reference|https|172.30.240.0/24|${installedHome}/state/synveda-reference/secrets|${installedHome}/backups/database/synveda-reference|config\n`,
  );
});

test("installer refuses a symlinked explicit binary directory before mutation", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-bin-link-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  const fixture = installEnv(scratch, assets);
  const linkTarget = join(scratch, "unrelated-bin");
  mkdirSync(linkTarget);
  symlinkSync(linkTarget, fixture.bin, "dir");

  const result = spawnSync("/bin/sh", [installer], {
    cwd: root,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /SYNVEDA_BIN is not a real directory/);
  assert.equal(existsSync(fixture.home), false);
  assert.equal(existsSync(join(linkTarget, "synveda")), false);
});

test("server-only archive prepares private inputs without native binaries and preserves keys on repeat", (t) => {
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-server-only-")));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  const extracted = join(scratch, "extracted");
  mkdirSync(extracted);
  execFileSync("tar", ["-xzf", join(assets, `synveda-reference-${version}.tar.gz`), "-C", extracted]);
  const bundle = join(extracted, `synveda-reference-${version}`);
  const project = `synveda-reference-acceptance-prebuilt-${process.pid}`;
  const installRoot = join(scratch, "server");
  const state = join(installRoot, "state", project);
  mkdirSync(state, { recursive: true, mode: 0o700 });
  const env = {
    ...process.env,
    SYNVEDA_HOME: installRoot,
    SYNVEDA_APP_HOST: "app.example.com",
    SYNVEDA_AUTH_HOST: "auth.example.com",
    SYNVEDA_COMPOSE_PROJECT_SUFFIX: `acceptance-prebuilt-${process.pid}`,
  };
  const launch = (args, extra = {}) => spawnSync("sh", [join(bundle, "synveda-compose"), ...args], {
    cwd: scratch, env: { ...env, ...extra }, encoding: "utf8",
  });
  for (const action of ["secrets", "issuer"]) {
    const result = launch([action]);
    assert.equal(result.status, 0, result.stderr);
  }
  const keyPath = join(state, "secrets/synveda_kms_key");
  const hash = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
  const before = [hash(keyPath), hash(join(state, "issuers.json"))];
  for (const action of ["secrets", "issuer"]) {
    const result = launch([action]);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(launch([action, "--force"]).status, 64);
  }
  assert.deepEqual([hash(keyPath), hash(join(state, "issuers.json"))], before);
  assert.equal(statSync(keyPath).mode & 0o777, 0o600);
  assert.equal(statSync(join(state, "secrets")).mode & 0o777, 0o700);
  assert.match(readFileSync(join(state, "issuers.json"), "utf8"), /https:\/\/auth.example.com\/realms\/synveda/);
  assert.equal(existsSync(join(bundle, "deploy/compose/runtime")), false);
  assert.equal(existsSync(join(installRoot, "bin")), false);
  assert.equal(launch(["secrets"], { SYNVEDA_POSTGRES_MODE: "external" }).status, 69);
  assert.equal(launch(["issuer"], { SYNVEDA_OIDC_MODE: "external" }).status, 64);
});

test("installer refuses a non-directory reference root before mutation", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-reference-file-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  const fixture = installEnv(scratch, assets);
  mkdirSync(fixture.home);
  const sentinel = join(fixture.home, "reference");
  writeFileSync(sentinel, "unrelated\n");

  const result = spawnSync("/bin/sh", [installer], {
    cwd: root,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /reference install root is not a real directory/);
  assert.equal(readFileSync(sentinel, "utf8"), "unrelated\n");
  assert.equal(existsSync(fixture.bin), false);
  assert.equal(existsSync(join(fixture.home, "bin")), false);
});

test("installer refuses symlinks in checksum-valid release archives before mutation", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-archive-link-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  const malformed = join(scratch, "malformed-console");
  mkdirSync(join(malformed, "console"), { recursive: true });
  symlinkSync("../outside", join(malformed, "console/index.html"));
  archive(malformed, join(assets, `synveda-console-${version}.tar.gz`), ["console"]);
  writeChecksums(assets, version);
  const fixture = installEnv(scratch, assets);

  const result = spawnSync("/bin/sh", [installer], {
    cwd: root,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /release archives must not contain symbolic links/);
  assert.equal(existsSync(fixture.home), false);
  assert.equal(existsSync(fixture.bin), false);
});

for (const { client, asset, label, error } of [
  { client: "codex", asset: "node_modules/@synveda/claude-code-adapter/dist/session-runtime.mjs",
    label: "Codex shared runtime", error: /Codex .*session-runtime\.mjs was missing/ },
  { client: "copilot-cli", asset: "dist/hook.mjs",
    label: "Copilot hook", error: /Copilot dist\/hook\.mjs was missing/ },
  { client: "copilot-cli", asset: "node_modules/@synveda/claude-code-adapter/dist/session-runtime.mjs",
    label: "Copilot shared runtime", error: /Copilot .*session-runtime\.mjs was missing/ },
]) {
  test(`installer refuses a missing ${label} before mutation`, (t) => {
    const scratch = mkdtempSync(join(tmpdir(), "synveda-install-runtime-missing-"));
    t.after(() => rmSync(scratch, { recursive: true, force: true }));
    const assets = buildAssets(scratch);
    const fixture = installEnv(scratch, assets);
    const stage = join(scratch, "plugin-stage");
    rmSync(join(stage, "plugin", client, asset));
    archive(stage, join(assets, `synveda-plugin-${version}.tar.gz`), ["plugin"]);
    writeChecksums(assets, version);

    const result = spawnSync("/bin/sh", [installer], {
      cwd: root, encoding: "utf8", env: fixture.env,
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, error);
    assert.equal(existsSync(fixture.home), false);
    assert.equal(existsSync(fixture.bin), false);
  });
}

test("installer refuses a directory at a binary destination before mutation", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-binary-dir-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  const fixture = installEnv(scratch, assets);
  const gatewayTarget = join(fixture.home, "bin/synveda-gateway");
  mkdirSync(gatewayTarget, { recursive: true });
  writeFileSync(join(gatewayTarget, "sentinel"), "unrelated\n");

  const result = spawnSync("/bin/sh", [installer], {
    cwd: root,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /binary install target is not a regular file/);
  assert.equal(readFileSync(join(gatewayTarget, "sentinel"), "utf8"), "unrelated\n");
  assert.equal(existsSync(fixture.bin), false);
});

test("installer refuses a symlink at a binary temporary path before mutation", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-binary-temp-link-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  const fixture = installEnv(scratch, assets);
  mkdirSync(fixture.bin);
  const unrelated = join(scratch, "unrelated");
  writeFileSync(unrelated, "preserve\n");
  symlinkSync(unrelated, join(fixture.bin, "synveda.tmp"));

  const result = spawnSync("/bin/sh", [installer], {
    cwd: root,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /temporary binary install path already exists/);
  assert.equal(readFileSync(unrelated, "utf8"), "preserve\n");
  assert.equal(existsSync(fixture.home), false);
});

test("installer refuses an explicit binary directory that aliases managed paths", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-bin-overlap-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));

  for (const leaf of ["synveda-gateway", "synveda-gateway.tmp"]) {
    const home = join(scratch, leaf);
    const result = spawnSync("/bin/sh", [installer], {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        HOME: join(scratch, "user-home"),
        SYNVEDA_BASE_URL: `file://${scratch}/missing`,
        SYNVEDA_BIN: join(home, "bin", leaf),
        SYNVEDA_HOME: home,
        SYNVEDA_VERSION: version,
      },
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /SYNVEDA_BIN must stay outside SYNVEDA_HOME/);
    assert.doesNotMatch(result.stderr, /no asset/);
    assert.equal(existsSync(home), false);
  }
});

test("installer refuses a legacy profile before download or mutation", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-legacy-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const home = join(scratch, "home");
  const legacy = join(home, "profile/sentinel");
  mkdirSync(join(home, "profile"), { recursive: true });
  writeFileSync(legacy, "preserve\n");
  const result = spawnSync("/bin/sh", [installer], {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: join(scratch, "user-home"),
      SYNVEDA_BASE_URL: `file://${scratch}/missing`,
      SYNVEDA_HOME: home,
      SYNVEDA_VERSION: version,
    },
  });
  assert.equal(result.status, 1);
  assert.equal(readFileSync(legacy, "utf8"), "preserve\n");
  assert.match(result.stderr, /legacy .*profile exists/);
  assert.doesNotMatch(result.stderr, /no asset/);
});

test("installer requires the published checksum inventory before mutation", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-install-checksums-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const assets = buildAssets(scratch);
  rmSync(join(assets, "SHA256SUMS"));
  const fixture = installEnv(scratch, assets);
  const result = spawnSync("/bin/sh", [installer], {
    cwd: root,
    encoding: "utf8",
    env: fixture.env,
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /has no readable SHA256SUMS/);
  assert.equal(existsSync(fixture.home), false);
});
