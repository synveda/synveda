import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { evaluationEnvironment } from "../deploy/compose/scripts/prepare-evaluation.mjs";
import { validateSettings, validateAuthorizationUrl } from "../deploy/compose/browser/console-login-contract.mjs";

const image = (name) => `example.invalid/${name}@sha256:${"a".repeat(64)}`;
const manifest = { images: Object.fromEntries(["product", "postgres", "keycloak", "proxy", "browser_acceptance"].map((key) => [key, image(key)])), external_images: { otel_collector: image("otel") } };
const prepare = (options = { demoAccounts: true }, source = manifest) => evaluationEnvironment(source, options, "/private/state", 1000, 1000, "/bundle/deploy/compose");

test("loopback evaluation fixes public issuer and immutable images independently of the Docker backchannel", () => {
  const env = prepare({ port: 18080, subnet: "192.168.170.0/24", demoAccounts: false });
  assert.equal(env.SYNVEDA_OIDC_ISSUER, "http://localhost:18080/realms/synveda");
  assert.equal(env.SYNVEDA_RENDER_PROXY_IDENTITY_ADDRESS, "192.168.170.2");
  assert.equal(env.SYNVEDA_RENDER_DATA_SUBNET, "192.168.170.48/28");
  assert.equal(env.SYNVEDA_PRODUCT_IMAGE, manifest.images.product);
  assert.equal(env.SYNVEDA_KEYCLOAK_SSL_REQUIRED, "NONE");
});

test("contradictory settings and mutable images fail before secret preparation", () => {
  for (const options of [{}, { demoAccounts: "true" }, { demoAccounts: true, port: 80 }, { demoAccounts: true, port: 8443 }, { demoAccounts: true, subnet: "8.8.8.0/24" }, { demoAccounts: true, subnet: "10.256.1.0/24" }, { demoAccounts: true, issuer: "http://elsewhere" }]) assert.throws(() => prepare(options));
  for (const key of Object.keys(manifest.images)) assert.throws(() => prepare({ demoAccounts: true }, { ...manifest, images: { ...manifest.images, [key]: "example.invalid/product:latest" } }));
});

test("localhost front channels retain exact origin and callback checks for Compose and port-forward", () => {
  for (const port of [18080, 8081]) {
    const settings = validateSettings("http://localhost:18080", `http://localhost:${port}/realms/synveda`);
    const auth = new URL(`${settings.issuerOrigin}${settings.authorizationPath}`);
    for (const [key, value] of Object.entries({ client_id: "synveda", response_type: "code", redirect_uri: settings.callback, scope: "openid profile email", state: "s".repeat(43), nonce: "n".repeat(43), code_challenge: "c".repeat(43), code_challenge_method: "S256" })) auth.searchParams.set(key, value);
    assert.doesNotThrow(() => validateAuthorizationUrl(auth.href, settings));
    auth.searchParams.set("redirect_uri", "http://localhost:9999/auth/callback");
    assert.throws(() => validateAuthorizationUrl(auth.href, settings));
  }
  assert.throws(() => validateSettings("http://localhost:18080", "http://public.example:18080/realms/synveda"));
});

test("publication metadata stays aligned and unreleased commands cannot be silently presented as published", () => {
  const read = (path) => readFileSync(path, "utf8");
  const publication = JSON.parse(read("docs/installation.json"));
  assert.ok(["unreleased", "published"].includes(publication.publication));
  assert.match(read("Cargo.toml"), new RegExp(`version = "${publication.sourceVersion.replaceAll(".", "\\.")}"`));
  for (const path of ["README.md", "docs/INSTALL.md", "deploy/compose/PREBUILT.md", "deploy/helm/synveda/README.md"]) {
    assert.ok(read(path).includes(`<!-- installation-version: ${publication.sourceVersion}; publication: ${publication.publication} -->`), path);
  }
  assert.ok(read("README.md").includes(publication.dockerCommand));
  assert.ok(read("README.md").includes(publication.sampleCommand));
});
