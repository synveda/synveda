import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, realpathSync,
  rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { gzipSync } from "node:zlib";
import { inventory, nodePath, sha256, targetName, validateClient } from "./client-artifact.mjs";
import { installClient } from "./client-install.mjs";
import { checkClientRelease } from "./check-client-release.mjs";

const root = resolve(import.meta.dirname, "..");
const version = "0.4.0";
const quote = (text) => `'${text.replaceAll("'", "'\\''")}'`;
function fixture(t) {
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-client-test-")));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const source = join(scratch, "client");
  const write = (path, bytes) => { mkdirSync(join(source, path, ".."), { recursive: true }); writeFileSync(join(source, path), bytes); };
  for (const path of ["LICENSE", "NOTICE", "plugin/synveda/runtime/LICENSE", "plugin/synveda/consumer-setup.json",
    "plugin/synveda/dist/hook.mjs", "plugin/synveda/dist/mcp-server.mjs", "plugin/.claude-plugin/marketplace.json",
    "plugin/codex/dist/hook.mjs", "plugin/copilot-cli/dist/hook.mjs"]) write(path, "fixture\n");
  write("bin/synveda", `#!/bin/sh\nprintf 'synveda ${version}\\n'\n`);
  // Fixture-only interpreter: a production packager always verifies the exact
  // upstream archive before copying Node. Unit tests need no network/runtime pin.
  write(nodePath, `#!/bin/sh\nif [ "$1" = --version ]; then printf 'v24.21.0\\n'; else exec ${quote(process.execPath)} "$@"; fi\n`);
  chmodSync(join(source, "bin/synveda"), 0o755);
  chmodSync(join(source, nodePath), 0o755);
  write("plugin/synveda/.mcp.json", JSON.stringify({ synveda: { command: "${CLAUDE_PLUGIN_ROOT}/runtime/node" } }));
  write("plugin/synveda/hooks/hooks.json", JSON.stringify({ hooks: { SessionStart: [{ hooks: [{ command: "${CLAUDE_PLUGIN_ROOT}/runtime/node" }] }] } }));
  mkdirSync(join(source, "lib"));
  for (const file of ["client-install.mjs", "client-artifact.mjs"]) copyFileSync(join(root, "scripts", file), join(source, "lib", file));
  const seal = (changes = {}) => write("client.json", JSON.stringify({ schema_version: 1, version, target: targetName(),
    source_sha: "1".repeat(40), cli_version: `synveda ${version}`, node: { version: "24.21.0", archive_sha256: "2".repeat(64) },
    ...changes, files: inventory(source) }));
  seal();
  const home = join(scratch, "user ' ✓", ".synveda");
  const bin = join(home, "bin");
  return { scratch, source, home, bin, write, seal, install: () => installClient(source, home, bin) };
}

test("client install repeats and upgrades atomically without changing retained state or prior bytes", (t) => {
  const f = fixture(t);
  mkdirSync(join(f.home, "state"), { recursive: true });
  writeFileSync(join(f.home, "state/retained"), "keys and data");
  const first = f.install();
  const prior = realpathSync(first.current);
  assert.equal(statSync(join(f.home, "client")).mode & 0o777, 0o700);
  assert.equal(statSync(join(f.home, "client/install.json")).mode & 0o777, 0o600);
  assert.equal(execFileSync(first.launcher, ["--version"], { encoding: "utf8" }).trim(), `synveda ${version}`);
  assert.equal(f.install().digest, first.digest);
  f.write("NOTICE", "new notice");
  f.seal({ source_sha: "3".repeat(40) });
  const upgraded = f.install();
  assert.notEqual(upgraded.digest, first.digest);
  assert.equal(validateClient(prior).digest, first.digest);
  assert.equal(readFileSync(join(f.home, "state/retained"), "utf8"), "keys and data");
  assert.equal(existsSync(join(f.home, "bin/synveda-gateway")), false);
});

test("damaged input and foreign native targets fail before creating an installation", (t) => {
  const f = fixture(t);
  f.write("NOTICE", "damaged");
  assert.throws(f.install, /inventory mismatch/);
  assert.equal(existsSync(f.home), false);
  f.seal({ target: "windows-arm64" });
  assert.throws(f.install, /target mismatch/);
  assert.equal(existsSync(f.home), false);
});

test("archive inventories refuse symbolic links and hardlinks", (t) => {
  const f = fixture(t);
  const link = join(f.source, "linked");
  symlinkSync(join(f.source, "NOTICE"), link);
  assert.throws(() => inventory(f.source), /link or special/);
  rmSync(link);
  linkSync(join(f.source, "NOTICE"), link);
  assert.throws(() => inventory(f.source), /link or special/);
});

test("private runtime lookup cannot regress to system Node", (t) => {
  const f = fixture(t);
  f.write("plugin/synveda/.mcp.json", JSON.stringify({ synveda: { command: "node" } }));
  f.seal();
  assert.throws(f.install, /private Node/);
});

test("unowned launchers and destination links are preserved", (t) => {
  const f = fixture(t);
  mkdirSync(f.bin, { recursive: true });
  const launcher = join(f.bin, "synveda");
  writeFileSync(launcher, "someone else's CLI");
  assert.throws(f.install, /unowned/);
  assert.equal(readFileSync(launcher, "utf8"), "someone else's CLI");
  rmSync(launcher);
  symlinkSync(f.source, join(f.home, "client"));
  assert.throws(f.install, /link or non-directory/);
  assert.equal(readlinkSync(join(f.home, "client")), f.source);
});

test("changed installed files and launcher prevent switching releases", (t) => {
  const f = fixture(t);
  const first = f.install();
  const current = readlinkSync(first.current);
  writeFileSync(join(first.current, "NOTICE"), "operator edit");
  assert.throws(f.install, /inventory mismatch/);
  assert.equal(readlinkSync(first.current), current);
  assert.equal(readFileSync(join(first.current, "NOTICE"), "utf8"), "operator edit");
  writeFileSync(first.launcher, "operator launcher");
  assert.throws(f.install, /launcher changed/);
  assert.equal(readFileSync(first.launcher, "utf8"), "operator launcher");
});

test("foreign current links and interrupted installer locks are refused and retained", (t) => {
  const f = fixture(t);
  const first = f.install();
  rmSync(first.current);
  symlinkSync(f.source, first.current);
  assert.throws(f.install, /ownership or content/);
  const lock = join(f.home, ".client-install.lock");
  mkdirSync(lock);
  writeFileSync(join(lock, "sentinel"), "other installer");
  assert.throws(f.install, /busy or interrupted/);
  assert.equal(readFileSync(join(lock, "sentinel"), "utf8"), "other installer");
});

function shellFixture(t) {
  const f = fixture(t);
  const assets = join(f.scratch, "assets");
  mkdirSync(assets);
  const name = `synveda-client-${version}-${targetName()}.tar.gz`;
  const pack = () => {
    execFileSync("tar", ["-czf", join(assets, name), "-C", f.scratch, "client"]);
    writeFileSync(join(assets, "SHA256SUMS"), `${sha256(readFileSync(join(assets, name)))}  ${name}\n`);
  };
  pack();
  const env = { ...process.env, HOME: dirnameForHome(f.home), SYNVEDA_HOME: f.home, SYNVEDA_BIN: f.bin,
    SYNVEDA_VERSION: version, SYNVEDA_INSTALL_MODE: "client", SYNVEDA_BASE_URL: `file://${assets}`, SYNVEDA_SOURCE_SHA: "" };
  return { ...f, assets, name, pack, env, run: () => spawnSync("/bin/sh", [join(root, "scripts/install.sh")], { env, encoding: "utf8" }) };
}
function dirnameForHome(path) { return resolve(path, ".."); }

test("shell client mode downloads only its archive and checksums", (t) => {
  const f = shellFixture(t);
  mkdirSync(join(f.home, "profile"), { recursive: true });
  writeFileSync(join(f.home, "profile/retained"), "unrelated retained deployment");
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Local development candidate: publisher identity was not verified/);
  assert.match(result.stdout, /No deployment was downloaded or started/);
  assert.equal(existsSync(join(f.home, "reference")), false);
  assert.equal(readFileSync(join(f.home, "profile/retained"), "utf8"), "unrelated retained deployment");
});

function signedShellFixture(t) {
  const f = shellFixture(t);
  const tools = join(f.scratch, "tools");
  mkdirSync(tools);
  const downloads = join(f.scratch, "downloads");
  const verification = join(f.scratch, "verification");
  // These stubs test invocation policy and ordering, not Sigstore cryptography.
  // Published-bundle verification is a separate live acceptance check.
  writeFileSync(join(f.assets, "SHA256SUMS.sigstore.json"), "fixture bundle\n");
  writeFileSync(join(tools, "curl"), `#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output=$2; shift ;;
    https://*) url=$1 ;;
  esac
  shift
done
asset=\${url##*/}
printf '%s\\n' "$asset" >> "$FAKE_DOWNLOAD_LOG"
exec /bin/cp "$FAKE_ASSETS/$asset" "$output"
`);
  writeFileSync(join(tools, "gh"), `#!/bin/sh
printf '%s\\n' "$@" > "$FAKE_VERIFICATION_LOG"
exit "\${FAKE_VERIFICATION_STATUS:-0}"
`);
  for (const tool of ["curl", "gh"]) chmodSync(join(tools, tool), 0o755);
  Object.assign(f.env, { PATH: `${tools}:${process.env.PATH}`, SYNVEDA_BASE_URL: "https://mirror.example/releases/v0.4.0",
    SYNVEDA_SOURCE_SHA: "1".repeat(40), FAKE_ASSETS: f.assets, FAKE_DOWNLOAD_LOG: downloads, FAKE_VERIFICATION_LOG: verification });
  return { ...f, tools, downloads, verification };
}

test("remote client installation pins the publisher, workflow, tag, commit and runner before fetching code", (t) => {
  const f = signedShellFixture(t);
  f.env.GH_HOST = "untrusted.example";
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  const args = readFileSync(f.verification, "utf8").trim().split("\n");
  assert.deepEqual(args.slice(0, 2), ["attestation", "verify"]);
  assert.match(args[2], /\/SHA256SUMS$/);
  assert.equal(args[3], "--bundle");
  assert.equal(args[4], `${args[2]}.sigstore.json`);
  assert.deepEqual(args.slice(5), ["--hostname", "github.com", "--repo", "synveda/synveda",
    "--signer-workflow", "synveda/synveda/.github/workflows/release.yml", "--source-ref", "refs/tags/v0.4.0",
    "--source-digest", "1".repeat(40), "--cert-oidc-issuer", "https://token.actions.githubusercontent.com",
    "--predicate-type", "https://slsa.dev/provenance/v1", "--deny-self-hosted-runners"]);
  assert.deepEqual(readFileSync(f.downloads, "utf8").trim().split("\n"), ["SHA256SUMS", "SHA256SUMS.sigstore.json", f.name]);
});

test("publisher rejection preserves the current client and never downloads archive code", (t) => {
  const f = signedShellFixture(t);
  const previous = f.install();
  const current = readlinkSync(previous.current);
  f.env.FAKE_VERIFICATION_STATUS = "1";
  const result = f.run();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /release publisher verification failed/);
  assert.deepEqual(readFileSync(f.downloads, "utf8").trim().split("\n"), ["SHA256SUMS", "SHA256SUMS.sigstore.json"]);
  assert.equal(readlinkSync(previous.current), current);
  assert.equal(validateClient(realpathSync(previous.current)).digest, previous.digest);
});

test("a missing or oversized attestation stops before archive download", (t) => {
  const f = signedShellFixture(t);
  const bundle = join(f.assets, "SHA256SUMS.sigstore.json");
  rmSync(bundle);
  assert.match(f.run().stderr, /no readable publisher attestation/);
  assert.equal(existsSync(f.verification), false);
  writeFileSync(bundle, Buffer.alloc(4194305));
  assert.match(f.run().stderr, /publisher attestation exceeds its size bound/);
  assert.equal(existsSync(f.verification), false);
  assert.equal(existsSync(f.home), false);
});

test("remote inputs cannot relax source, publisher or URL policy", (t) => {
  const f = signedShellFixture(t);
  for (const [changes, diagnostic] of [
    [{ SYNVEDA_SOURCE_SHA: "" }, /requires SYNVEDA_SOURCE_SHA/],
    [{ SYNVEDA_SOURCE_SHA: "1".repeat(39) }, /40-character lowercase Git commit/],
    [{ SYNVEDA_SOURCE_SHA: "g".repeat(40) }, /40-character lowercase Git commit/],
    [{ SYNVEDA_REPO: "other/publisher" }, /publisher must be synveda\/synveda/],
    [{ SYNVEDA_BASE_URL: "http://mirror.example/assets" }, /HTTPS release URL/],
    [{ SYNVEDA_BASE_URL: "file://remote.example/assets" }, /absolute local file:\/\/\//],
    [{ SYNVEDA_BASE_URL: "https://user:password@mirror.example/assets" }, /without credentials/],
  ]) {
    const result = spawnSync("/bin/sh", [join(root, "scripts/install.sh")], {
      env: { ...f.env, ...changes }, encoding: "utf8",
    });
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stderr, diagnostic);
    assert.equal(existsSync(f.downloads), false);
    assert.equal(existsSync(f.home), false);
  }
});

test("a signed local mirror still requires the verifier; no absence fallback exists", (t) => {
  const f = signedShellFixture(t);
  rmSync(join(f.tools, "gh"));
  for (const tool of ["uname", "tar", "grep"]) {
    const executable = execFileSync("/bin/sh", ["-c", `command -v ${tool}`], { encoding: "utf8" }).trim();
    symlinkSync(executable, join(f.tools, tool));
  }
  f.env.PATH = f.tools;
  f.env.SYNVEDA_BASE_URL = `file://${f.assets}`;
  const result = f.run();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /trusted GitHub CLI/);
  assert.equal(existsSync(f.downloads), false);
  assert.equal(existsSync(f.home), false);
});

test("deadline expiry or timer failure kills a blocked verifier before archive download", (t) => {
  for (const timerStatus of [0, 1]) {
    const f = signedShellFixture(t);
    writeFileSync(join(f.tools, "gh"), "#!/bin/sh\nexec /bin/sleep 30\n");
    // Advance or fail only the watchdog's timer; production has no override.
    writeFileSync(join(f.tools, "sleep"), `#!/bin/sh\nexit ${timerStatus}\n`, { mode: 0o755 });
    const result = spawnSync("/bin/sh", [join(root, "scripts/install.sh")], { env: f.env, encoding: "utf8", timeout: 5000 });
    assert.ifError(result.error);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /failed or exceeded 120 seconds/);
    assert.deepEqual(readFileSync(f.downloads, "utf8").trim().split("\n"), ["SHA256SUMS", "SHA256SUMS.sigstore.json"]);
    assert.equal(existsSync(f.home), false);
  }
});

test("shell client mode rejects duplicate and mismatched checksums before mutation", (t) => {
  const f = shellFixture(t);
  const sums = join(f.assets, "SHA256SUMS");
  writeFileSync(sums, readFileSync(sums, "utf8").repeat(2));
  assert.match(f.run().stderr, /exactly once/);
  writeFileSync(sums, `${"0".repeat(64)}  ${f.name}\n`);
  assert.match(f.run().stderr, /failed its checksum/);
  assert.equal(existsSync(f.home), false);
});

test("shell refuses links before extracting or invoking archive code", (t) => {
  const f = shellFixture(t);
  symlinkSync("../../outside", join(f.source, "escape"));
  f.pack();
  assert.match(f.run().stderr, /links or special entries/);
  assert.equal(existsSync(f.home), false);
});

test("shell requires the downloaded manifest to match the requested release", (t) => {
  const f = shellFixture(t);
  f.seal({ version: "9.9.9" });
  f.pack();
  assert.match(f.run().stderr, /requested release or target/);
  assert.equal(existsSync(f.home), false);
});

test("shell rejects traversal, foreign roots and duplicate tar entries before extraction", (t) => {
  const f = shellFixture(t);
  function tarEntry(name) {
    const header = Buffer.alloc(512);
    header.write(name);
    for (const [offset, length, value] of [[100, 8, 0o644], [108, 8, 0], [116, 8, 0], [124, 12, 0], [136, 12, 0]]) {
      header.write(`${value.toString(8).padStart(length - 1, "0")}\0`, offset, length);
    }
    header.fill(32, 148, 156);
    header[156] = 48;
    header.write("ustar\0", 257);
    header.write("00", 263);
    const sum = header.reduce((total, byte) => total + byte, 0);
    header.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148, 8);
    return header;
  }
  for (const names of [["client/../outside", "client/file"], ["../outside", "client/file"],
    ["/absolute", "client/file"], ["foreign/file", "client/file"], ["client/file", "client/file"]]) {
    const bytes = gzipSync(Buffer.concat([...names.map(tarEntry), Buffer.alloc(1024)]));
    writeFileSync(join(f.assets, f.name), bytes);
    writeFileSync(join(f.assets, "SHA256SUMS"), `${sha256(bytes)}  ${f.name}\n`);
    const result = f.run();
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /unsafe client archive path inventory|invalid client archive/);
    assert.equal(existsSync(f.home), false);
  }
});

test("native runtime pins cover four Unix and two Windows candidates with immutable checksums", () => {
  const lock = JSON.parse(readFileSync(join(root, "scripts/node-runtimes.json")));
  assert.deepEqual(Object.keys(lock.targets).sort(), ["darwin-arm64", "darwin-x86_64", "linux-arm64", "linux-x86_64", "windows-arm64", "windows-x86_64"]);
  for (const pin of Object.values(lock.targets)) {
    assert.equal(pin.archive, `node-v${lock.version}-${pin.platform === "win32" ? "win" : pin.platform}-${pin.arch}.${pin.platform === "win32" ? "zip" : "tar.gz"}`);
    assert.match(pin.sha256, /^[0-9a-f]{64}$/);
  }
});

test("release verification rejects missing checks, changed archives and stale identities", (t) => {
  const f = fixture(t);
  const target = targetName();
  const bytes = Buffer.from("test archive bytes");
  const node = { version: "24.21.0", archive: "node-fixture.tar.gz", archive_sha256: "2".repeat(64) };
  const lock = { version: node.version, targets: { [target]: { archive: node.archive, sha256: node.archive_sha256 } } };
  const archive = join(f.scratch, `synveda-client-${version}-${target}.tar.gz`);
  const path = join(f.scratch, `synveda-client-report-${target}.json`);
  const report = { schema_version: 1, evidence: "native-client-archive", target, version, source_sha: "1".repeat(40),
    source_tree_dirty: false, cli_version: `synveda ${version}`, node, archive_sha256: sha256(bytes), archive_bytes: bytes.length,
    checks: ["licence-and-notice-carriage", "packaged-authentication-lifecycle", "native-identity-and-client-only-inventory", "restricted-path-install-cli-and-three-hook-launches",
      "private-install-without-harness-or-credential-mutation", "repeat-install-preserves-deployment-state",
      "codex-extracted-lifecycle-replay", "copilot-cli-extracted-lifecycle-replay"] };
  const check = () => checkClientRelease(f.scratch, version, "1".repeat(40), true, lock);
  writeFileSync(archive, bytes);
  writeFileSync(path, JSON.stringify(report));
  check();
  for (const changed of [{ source_tree_dirty: true }, { source_sha: "3".repeat(40) }, { target: "linux-mips" },
    { cli_version: "synveda 0.0.0" }, { node: { ...node, archive_sha256: "0".repeat(64) } }, { checks: report.checks.slice(1) }]) {
    writeFileSync(path, JSON.stringify({ ...report, ...changed }));
    assert.throws(check, /mismatch|missing native client check/);
  }
  writeFileSync(path, JSON.stringify(report));
  writeFileSync(archive, "changed");
  assert.throws(check, /mismatch/);
});

test("Windows release evidence requires both native ZIP reports and installer refusals", (t) => {
  const f = fixture(t);
  const lock = { version: "24.21.0", targets: {} };
  const reports = [];
  for (const target of ["windows-x86_64", "windows-arm64"]) {
    const bytes = Buffer.from(`fixture ${target}`);
    const node = { version: lock.version, archive: `node-${target}.zip`, archive_sha256: "2".repeat(64) };
    lock.targets[target] = { platform: "win32", archive: node.archive, sha256: node.archive_sha256 };
    writeFileSync(join(f.scratch, `synveda-client-${version}-${target}.zip`), bytes);
    const report = { schema_version: 1, evidence: "native-client-archive", target, version, source_sha: "1".repeat(40),
      source_tree_dirty: false, cli_version: `synveda ${version}`, node, archive_sha256: sha256(bytes), archive_bytes: bytes.length,
      checks: ["licence-and-notice-carriage", "packaged-authentication-lifecycle", "native-identity-and-client-only-inventory", "restricted-path-install-cli-and-three-hook-launches",
        "private-install-without-harness-or-credential-mutation", "repeat-install-preserves-deployment-state",
        "native-windows-private-storage-interoperability", "duplicate-checksum-launcher-drift-and-interrupted-lock-refusal",
        "unsafe-zip-and-overlapping-install-root-refusal", "publisher-policy-and-pre-execution-refusal", "bounded-download-sharing-lock-cleanup"] };
    const path = join(f.scratch, `synveda-client-report-${target}.json`);
    writeFileSync(path, JSON.stringify(report));
    reports.push({ path, report });
  }
  const check = () => checkClientRelease(f.scratch, version, "1".repeat(40), true, lock);
  check();
  for (const { path, report } of reports) {
    for (const omitted of report.checks) {
      writeFileSync(path, JSON.stringify({ ...report, checks: report.checks.filter((item) => item !== omitted) }));
      assert.throws(check, /missing native client check/);
    }
    rmSync(path);
    assert.throws(check, /ENOENT/);
    writeFileSync(path, JSON.stringify(report));
  }
});
