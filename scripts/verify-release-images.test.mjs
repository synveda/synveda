import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  requireAnonymousConfig,
  validateEnvironment,
  validateIndex,
  verifyImages,
} from "./verify-release-images.mjs";

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

function fixture(override = () => undefined) {
  const calls = [];
  const run = (args) => {
    calls.push(args);
    const result = override(args);
    if (result !== undefined) return result;
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
    return "";
  };
  return { calls, run };
}

test("a native pull check covers all six artifacts, upstream pulls and isolated runtime smoke", () => {
  const f = fixture();
  const report = verifyImages(
    manifest(),
    "linux/arm64",
    version,
    source,
    f.run,
  );
  assert.equal(report.images.length, 8);
  assert.equal(report.platform, "linux/arm64");
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

test("loopback OCI candidates remain separate from public release verification", () => {
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
  const f = fixture((args) => args[0] === "buildx" && args.at(-1).startsWith("localhost:") ? JSON.stringify(single) : undefined);
  const report = verifyImages(candidate, "linux/arm64", version, source, f.run, true);
  assert.equal(report.local_candidate, true);
  assert.equal(report.anonymous_pull, false);
  const unattested = fixture((args) => args[0] === "buildx" ? JSON.stringify({ manifests: [single.manifests[0]] }) : undefined);
  assert.throws(() => verifyImages(candidate, "linux/arm64", version, source, unattested.run, true), /attestation descriptor/);
  for (const manifests of [[], index().manifests, [{ digest, platform: { os: "linux", architecture: "amd64" } }]]) {
    const broken = fixture((args) => args[0] === "buildx" ? JSON.stringify({ manifests }) : undefined);
    assert.throws(() => verifyImages(candidate, "linux/arm64", version, source, broken.run, true), /exactly the native image/);
  }
  candidate.image_namespace = "untrusted.example/team";
  assert.throws(() => validateEnvironment(candidate, version, source, true), /image namespace/);
});

test("missing artifacts, mutable tags and mismatched release identity fail before Docker", () => {
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
    const f = fixture();
    assert.throws(() =>
      verifyImages(value, "linux/arm64", version, source, f.run),
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

test("wrong engine, private image and runtime failures propagate; started containers are cleaned up", () => {
  const wrongEngine = fixture((args) =>
    args[0] === "info" ? "linux/amd64" : undefined,
  );
  assert.throws(
    () =>
      verifyImages(manifest(), "linux/arm64", version, source, wrongEngine.run),
    /native/,
  );
  assert.equal(wrongEngine.calls.length, 1);
  const privateImage = fixture((args) => {
    if (args[0] === "pull") throw new Error("denied");
  });
  assert.throws(
    () =>
      verifyImages(
        manifest(),
        "linux/arm64",
        version,
        source,
        privateImage.run,
      ),
    /denied/,
  );
  assert.equal(
    privateImage.calls.some(([verb]) => verb === "run"),
    false,
  );
  const runtimeFailure = fixture((args) => {
    if (args[0] === "run") throw new Error("executable failed");
  });
  assert.throws(
    () =>
      verifyImages(
        manifest(),
        "linux/arm64",
        version,
        source,
        runtimeFailure.run,
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

test("wrong image metadata, lost digests and stale compiled binaries block verification", () => {
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
    const normal = fixture();
    const f = fixture((args) => {
      if (args[0] !== "image") return;
      const [value] = JSON.parse(normal.run(args));
      mutate(value);
      return JSON.stringify([value]);
    });
    assert.throws(() =>
      verifyImages(manifest(), "linux/arm64", version, source, f.run),
    );
    assert.equal(
      f.calls.some(([verb]) => verb === "run"),
      false,
    );
  }
  const stale = fixture((args) =>
    args[0] === "run" ? "synveda 0.2.0" : undefined,
  );
  assert.throws(
    () => verifyImages(manifest(), "linux/arm64", version, source, stale.run),
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
