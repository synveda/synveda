import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  inspectConsoleImage,
  inspectNodeImage,
  noticeHashes,
  requireAnonymousConfig,
  validateEnvironment,
  validateIndex,
  verifyImages,
} from "./verify-release-images.mjs";

import { consolePackageFixture } from "./fixtures/console-package.mjs";
import { nodeImageFixture } from "./fixtures/node-image.mjs";
import { nodePins } from "./node-runtime-inventory.mjs";
import { productPackageFixture } from "./fixtures/product-packages.mjs";

const version = "0.3.0-rc.1";
const source = "a".repeat(40);
const digest = `sha256:${"1".repeat(64)}`;
const manifest = () => ({
  schema_version: 1,
  deployment_contract: "CPR-45/ADR-0102",
  release_version: version,
  source_sha: source,
  images: Object.fromEntries(
    [
      ["product", "product"],
      ["postgres", "postgres"],
      ["keycloak", "keycloak"],
      ["proxy", "proxy"],
      ["browser_acceptance", "browser-acceptance"],
      ["helm_postgres", "cnpg-postgres"],
    ].map(([name, repo]) => [name, `ghcr.io/synveda/${repo}@${digest}`]),
  ),
  external_images: {
    otel_collector: `example.com/otel:1.0@${digest}`,
    prometheus: `example.com/prometheus:v1.0@${digest}`,
  },
});
const index = () => ({
  manifests: [
    { digest, platform: { os: "linux", architecture: "amd64" } },
    { digest, platform: { os: "linux", architecture: "arm64" } },
    { digest, platform: { os: "unknown", architecture: "unknown" } },
  ],
});

function fixture(t, override = () => undefined) {
  const console = consolePackageFixture(t, { source, version });
  const node = nodeImageFixture(console.root);
  const packages = productPackageFixture("arm64", digest);
  const calls = [];
  const run = (args, _timeout, binary) => {
    calls.push(args);
    const result = override(args);
    if (result !== undefined) return result;
    if (args[0] === "create") return "c".repeat(64);
    if (args[0] === "cp") {
      assert.equal(binary, true);
      const path = args[1].split(":")[1];
      if (packages.bytes.has(path)) return packages.tar(path);
      return args[1].endsWith("/console") ? console.imageTar() : node.tar(args[1].split("/").at(-1));
    }
    if (args[0] === "start") return JSON.stringify(node.metadata);
    if (args[0] === "info") return "linux/aarch64";
    if (args[0] === "buildx") return JSON.stringify(index());
    if (args[0] === "image")
      return JSON.stringify([
        {
          Os: "linux",
          Architecture: "arm64",
          Id: digest,
          RepoDigests: [args[2].replace(/:[^/:@]+(?=@sha256:)/, "")],
          Config: {
            Labels: {
              "org.opencontainers.image.source":
                "https://github.com/synveda/synveda",
              "org.opencontainers.image.revision": source,
              "org.opencontainers.image.version": version,
            },
          },
        },
      ]);
    if (args[0] === "run" && args.includes("/usr/local/bin/synveda"))
      return `synveda ${version}`;
    if (args[0] === "run" && args.at(-1).startsWith("sha256sum "))
      return Object.entries(noticeHashes).map(([name, hash]) => `${hash}  /usr/share/licenses/synveda/${name}`).join("\n");
    return "";
  };
  return { calls, run, root: console.root, console, node, packages };
}

test("a native pull check covers all six artifacts, upstream pulls and isolated runtime smoke", t => {
  const f = fixture(t);
  const report = verifyImages(
    manifest(),
    "linux/arm64",
    version,
    source,
    f.run, false, f.root, f.packages.plan,
  );
  assert.equal(report.images.length, 8);
  assert.equal(report.platform, "linux/arm64");
  for (const entry of report.images) {
    assert.deepEqual(entry.notice_sha256,
      Object.hasOwn(manifest().images, entry.name) ? noticeHashes : undefined);
    assert.deepEqual(entry.console_inventory, entry.name === "product"
      ? { ...f.console.check(), source_sha: source, version } : undefined);
    assert.deepEqual(entry.node_inventory, entry.name === "product" ? f.node.report : undefined);
    assert.deepEqual(entry.package_inventory, entry.name === "product" ? f.packages.report : undefined);
  }
  assert.match(report.scope, /no deployment or OIDC acceptance/);
  const pulled = f.calls.filter(([verb]) => verb === "pull");
  assert.deepEqual(
    pulled.map((args) => args.at(-1)),
    Object.values({ ...manifest().images, ...manifest().external_images }),
  );
  for (const args of f.calls.filter(([verb]) => verb === "run")) {
    assert.ok(args.includes("--network=none"));
    assert.ok(args.includes("--pull=never"));
    assert.ok(args.includes("--read-only"));
    assert.ok(args.includes("--user=65532:65532"));
    assert.equal(
      args.some((arg) => /^(--volume|--mount|--env)/.test(arg)),
      false,
    );
  }
  assert.equal(
    f.calls.some(([verb]) => ["build", "login", "push"].includes(verb)),
    false,
  );
});

test("Node inspection verifies stopped bytes before isolated metadata execution and cleanup", t => {
  const f = fixture(t);
  assert.deepEqual(inspectNodeImage(digest, "linux/arm64", f.run, f.root), f.node.report);
  assert.deepEqual(f.calls.map(args => args[0]), ["create", ...Array(f.node.files.size).fill("cp"), "start", "rm"]);
  const create = f.calls[0], container = create[create.indexOf("--name") + 1];
  for (const value of ["--pull=never", "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--env=NODE_OPTIONS=", "--env=NODE_PATH="])
    assert.ok(create.includes(value));
  assert.equal(create[create.indexOf("--entrypoint") + 1], "/usr/local/bin/node");
  assert.deepEqual(f.calls.at(-2), ["start", "--attach", container]);
  assert.deepEqual(f.calls.at(-1), ["rm", "--force", "--volumes", container]);
});

test("changed Node bytes, truncated notices and duplicate streams refuse before execution", t => {
  for (const name of ["node", "LICENSE", ...Object.keys(nodePins.product.supplementary_notices)]) {
    const f = fixture(t); const bytes = Buffer.from(f.node.files.get(name)); bytes[bytes.length - 1] ^= 1; f.node.files.set(name, bytes);
    assert.throws(() => inspectNodeImage(digest, "linux/arm64", f.run, f.root), /hash differs/);
    assert.equal(f.calls.some(args => args[0] === "start"), false); assert.equal(f.calls.at(-1)[0], "rm");
  }
  const f = fixture(t);
  const run = (args, timeout, binary) => args[0] === "cp" && args[1].endsWith("/LICENSE")
    ? f.node.tar("LICENSE", ["LICENSE"]) : f.run(args, timeout, binary);
  assert.throws(() => inspectNodeImage(digest, "linux/arm64", run, f.root), /one regular file/);
  assert.equal(f.calls.some(args => args[0] === "start"), false);
});

test("Node stream/metadata bounds and every failure path retain their cause and clean up", t => {
  const original = new Error("Node inspection failed"), cleanup = new Error("Node cleanup failed");
  for (const stage of ["create", "cp", "start"]) {
    const f = fixture(t, args => { if (args[0] === stage) throw original; if (args[0] === "rm") throw cleanup; });
    assert.throws(() => inspectNodeImage(digest, "linux/arm64", f.run, f.root), error => error === original);
    assert.equal(f.calls.at(-1)[0], "rm");
  }
  const f = fixture(t, args => { if (args[0] === "rm") throw cleanup; });
  assert.throws(() => inspectNodeImage(digest, "linux/arm64", f.run, f.root), error => error === cleanup);
  for (const value of ["invalid stream", Buffer.alloc(0)]) {
    const f = fixture(t, args => args[0] === "cp" ? value : undefined);
    assert.throws(() => inspectNodeImage(digest, "linux/arm64", f.run, f.root), /stream exceeds/);
    assert.equal(f.calls.some(args => args[0] === "start"), false);
  }
  const large = fixture(t, args => args[0] === "start" ? "x".repeat(64 * 1024 + 1) : undefined);
  assert.throws(() => inspectNodeImage(digest, "linux/arm64", large.run, large.root), /metadata exceeds/);
  const changed = fixture(t); changed.node.metadata.versions.openssl = "0.0.0";
  assert.throws(() => inspectNodeImage(digest, "linux/arm64", changed.run, changed.root), /dependency versions/);
});

test("console inspection reads a stopped immutable image and removes only its own container", t => {
  const f = fixture(t);
  assert.deepEqual(inspectConsoleImage(digest, "linux/arm64", version, source, f.run, f.root),
    { ...f.console.check(), source_sha: source, version });
  assert.deepEqual(f.calls.map(args => args[0]), ["create", "cp", "rm"]);
  const created = f.calls[0], container = created[created.indexOf("--name") + 1];
  for (const required of ["--pull=never", "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges"])
    assert.ok(created.includes(required));
  assert.equal(created.some(arg => /^(--volume|--mount|--env)/.test(arg)), false);
  assert.deepEqual(created.slice(-2), [digest, "--version"]);
  assert.deepEqual(f.calls[1], ["cp", `${container}:/usr/share/synveda/console`, "-"]);
  assert.deepEqual(f.calls[2], ["rm", "--force", "--volumes", container]);
  assert.throws(() => inspectConsoleImage("product:latest", "linux/arm64", version, source, f.run, f.root));
  assert.equal(f.calls.length, 3);
});

test("actual image TAR content cannot omit metadata, alter output or transplant the source", t => {
  for (const mutate of [
    c => c.files.delete("sbom.cdx.json"),
    c => c.files.set("assets/index.js", Buffer.from("changed image JavaScript")),
    c => { c.files.set("THIRD-PARTY-NOTICES.txt", Buffer.from("truncated notices")); c.seal(); },
    c => { c.inventory.source_sha = "b".repeat(40); c.seal(); },
  ]) {
    const f = fixture(t); mutate(f.console);
    assert.throws(() => inspectConsoleImage(digest, "linux/arm64", version, source, f.run, f.root));
    assert.equal(f.calls.at(-1)[0], "rm");
    assert.equal(f.calls.some(args => ["run", "start", "exec"].includes(args[0])), false);
  }
  const f = fixture(t); f.console.pack(["console/index.html"], true);
  const duplicate = readFileSync(f.console.archive);
  const run = (args, timeout, binary) => args[0] === "cp" ? duplicate : f.run(args, timeout, binary);
  assert.throws(() => inspectConsoleImage(digest, "linux/arm64", version, source, run, f.root), /duplicate/);
  assert.equal(f.calls.at(-1)[0], "rm");
});

test("invalid or over-budget Docker streams fail before inspection and still remove the container", t => {
  for (const bytes of ["not a binary stream", Buffer.alloc(0), Buffer.alloc(34 * 1024 * 1024 + 1)]) {
    const f = fixture(t, args => args[0] === "cp" ? bytes : undefined);
    assert.throws(() => inspectConsoleImage(digest, "linux/arm64", version, source, f.run, f.root), /stream exceeds its bound/);
    assert.equal(f.calls.at(-1)[0], "rm");
  }
});

test("inspection cleanup cannot turn a failure into a pass or obscure its original cause", t => {
  const cleanup = new Error("container cleanup failed"), original = new Error("Docker copy failed");
  const f = fixture(t, args => { if (args[0] === "rm") throw cleanup; });
  assert.throws(() => inspectConsoleImage(digest, "linux/arm64", version, source, f.run, f.root), error => error === cleanup);
  for (const stage of ["create", "cp"]) {
    const broken = fixture(t, args => {
      if (args[0] === stage) throw original;
      if (args[0] === "rm") throw cleanup;
    });
    assert.throws(() => inspectConsoleImage(digest, "linux/arm64", version, source, broken.run, broken.root), error => error === original);
    assert.equal(broken.calls.at(-1)[0], "rm");
  }
});

test("missing or changed first-party image notices fail and clean up the verifier container", t => {
  for (const output of ["", "wrong licence hash"]) {
    const f = fixture(t, (args) => args[0] === "run" && args.at(-1).startsWith("sha256sum ") ? output : undefined);
    assert.throws(() => verifyImages(manifest(), "linux/arm64", version, source, f.run, false, f.root, f.packages.plan), /packaged licence\/notice hashes/);
    const started = f.calls.find((args) => args[0] === "run" && args.at(-1).startsWith("sha256sum "));
    assert.deepEqual(f.calls.at(-1), ["rm", "--force", started[started.indexOf("--name") + 1]]);
  }
});

test("loopback OCI candidates remain separate from public release verification", t => {
  const candidate = manifest();
  candidate.image_namespace = "localhost:5000/synveda";
  candidate.images = Object.fromEntries(Object.entries(candidate.images).map(([name, image]) => [name, image.replace("ghcr.io/synveda", candidate.image_namespace)]));
  assert.throws(() => validateEnvironment(candidate, version, source), /image namespace/);
  validateEnvironment(candidate, version, source, true);
  const single = index();
  single.manifests = single.manifests.filter((entry) => entry.platform.architecture !== "amd64");
  single.manifests.find((entry) => entry.platform.os === "unknown").annotations = {
    "vnd.docker.reference.type": "attestation-manifest",
    "vnd.docker.reference.digest": digest,
  };
  const f = fixture(t, (args) => args[0] === "buildx" && args.at(-1).startsWith("localhost:") ? JSON.stringify(single) : undefined);
  const report = verifyImages(candidate, "linux/arm64", version, source, f.run, true, f.root, f.packages.plan);
  assert.equal(report.local_candidate, true);
  assert.equal(report.anonymous_pull, false);
  const unattested = fixture(t, (args) => args[0] === "buildx" ? JSON.stringify({ manifests: [single.manifests[0]] }) : undefined);
  assert.throws(() => verifyImages(candidate, "linux/arm64", version, source, unattested.run, true, unattested.root, unattested.packages.plan), /attestation descriptor/);
  for (const manifests of [[], index().manifests, [{ digest, platform: { os: "linux", architecture: "amd64" } }]]) {
    const broken = fixture(t, (args) => args[0] === "buildx" ? JSON.stringify({ manifests }) : undefined);
    assert.throws(() => verifyImages(candidate, "linux/arm64", version, source, broken.run, true, broken.root, broken.packages.plan), /exactly the native image/);
  }
  candidate.image_namespace = "untrusted.example/team";
  assert.throws(() => validateEnvironment(candidate, version, source, true), /image namespace/);
});

test("missing artifacts, mutable tags and mismatched release identity fail before Docker", t => {
  const mutants = [
    (m) => {
      delete m.images.browser_acceptance;
    },
    (m) => {
      m.images.product = "ghcr.io/synveda/product:latest";
    },
    (m) => {
      m.images.keycloak = `ghcr.io/other/keycloak@${digest}`;
    },
    (m) => {
      m.source_sha = "b".repeat(40);
    },
    (m) => {
      m.release_version = "0.2.0";
    },
    (m) => {
      m.external_images.prometheus = "example.com/prometheus:latest";
    },
  ];
  for (const mutate of mutants) {
    const value = manifest();
    mutate(value);
    const f = fixture(t);
    assert.throws(() =>
      verifyImages(value, "linux/arm64", version, source, f.run, false, f.root, f.packages.plan),
    );
    assert.equal(f.calls.length, 0);
  }
  assert.doesNotThrow(() => validateEnvironment(manifest(), version, source));
});

test("multi-platform indexes require both architectures exactly once and tolerate attestations", () => {
  assert.deepEqual(Object.keys(validateIndex(index(), "image")), [
    "linux/amd64",
    "linux/arm64",
  ]);
  assert.throws(
    () => validateIndex({ manifests: index().manifests.slice(0, 1) }, "image"),
    /linux\/arm64/,
  );
  assert.throws(
    () =>
      validateIndex(
        { manifests: [...index().manifests, index().manifests[0]] },
        "image",
      ),
    /linux\/amd64/,
  );
  assert.throws(() => validateIndex({ layers: [] }, "image"), /multi-platform/);
});

test("wrong engine, private image and runtime failures propagate; started containers are cleaned up", t => {
  const wrongEngine = fixture(t, (args) =>
    args[0] === "info" ? "linux/amd64" : undefined,
  );
  assert.throws(
    () =>
      verifyImages(manifest(), "linux/arm64", version, source, wrongEngine.run, false, wrongEngine.root, wrongEngine.packages.plan),
    /native/,
  );
  assert.equal(wrongEngine.calls.length, 1);
  const privateImage = fixture(t, (args) => {
    if (args[0] === "pull") throw new Error("denied");
  });
  assert.throws(
    () =>
      verifyImages(
        manifest(),
        "linux/arm64",
        version,
        source,
        privateImage.run, false, privateImage.root, privateImage.packages.plan,
      ),
    /denied/,
  );
  assert.equal(
    privateImage.calls.some(([verb]) => verb === "run"),
    false,
  );
  const runtimeFailure = fixture(t, (args) => {
    if (args[0] === "run") throw new Error("executable failed");
  });
  assert.throws(
    () =>
      verifyImages(
        manifest(),
        "linux/arm64",
        version,
        source,
        runtimeFailure.run, false, runtimeFailure.root, runtimeFailure.packages.plan,
      ),
    /executable failed/,
  );
  const started = runtimeFailure.calls.find(([verb]) => verb === "run");
  assert.deepEqual(runtimeFailure.calls.at(-1), [
    "rm",
    "--force",
    started[started.indexOf("--name") + 1],
  ]);
});

test("wrong image metadata, lost digests and stale compiled binaries block verification", t => {
  for (const mutate of [
    (i) => {
      i.Architecture = "amd64";
    },
    (i) => {
      i.Config.Labels["org.opencontainers.image.revision"] = "b".repeat(40);
    },
    (i) => {
      i.RepoDigests = [];
    },
  ]) {
    const normal = fixture(t);
    const f = fixture(t, (args) => {
      if (args[0] !== "image") return;
      const [value] = JSON.parse(normal.run(args));
      mutate(value);
      return JSON.stringify([value]);
    });
    assert.throws(() =>
      verifyImages(manifest(), "linux/arm64", version, source, f.run, false, f.root, f.packages.plan),
    );
    assert.equal(
      f.calls.some(([verb]) => verb === "run"),
      false,
    );
  }
  const stale = fixture(t, (args) =>
    args[0] === "run" ? "synveda 0.2.0" : undefined,
  );
  assert.throws(
    () => verifyImages(manifest(), "linux/arm64", version, source, stale.run, false, stale.root, stale.packages.plan),
    /compiled CLI/,
  );
});

test("registry credentials and credential helpers cannot turn a private pull into a pass", () => {
  const directory = mkdtempSync(join(tmpdir(), "synveda-anonymous-test-"));
  try {
    assert.throws(() => requireAnonymousConfig(undefined), /DOCKER_CONFIG/);
    requireAnonymousConfig(directory);
    writeFileSync(
      join(directory, "config.json"),
      JSON.stringify({ auths: {}, currentContext: "default" }),
    );
    requireAnonymousConfig(directory);
    for (const value of [
      { auths: { "ghcr.io": {} } },
      { credsStore: "desktop" },
      { credHelpers: { "ghcr.io": "helper" } },
    ]) {
      writeFileSync(join(directory, "config.json"), JSON.stringify(value));
      assert.throws(() => requireAnonymousConfig(directory), /anonymous/);
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
