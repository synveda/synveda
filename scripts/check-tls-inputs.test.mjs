import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { X509Certificate, generateKeyPairSync } from "node:crypto";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { referenceTlsFindings } from "../deploy/compose/scripts/check-tls-inputs.mjs";
import { generateTestTlsChain } from "./test-certificate.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CHECKER = join(ROOT, "deploy/compose/scripts/check-tls-inputs.mjs");
const WRAPPER = join(ROOT, "deploy/compose/scripts/compose.sh");
const APP_HOST = "app.reference.example";
const AUTH_HOST = "auth.reference.example";

function fixture(sanHosts = [APP_HOST, AUTH_HOST], commonName = APP_HOST) {
  return generateTestTlsChain({ commonName, sanHosts });
}

function findings(
  certificate,
  privateKey,
  hosts = [APP_HOST, AUTH_HOST],
  timing = {},
) {
  const nowMs = timing.nowMs ?? Date.now();
  return referenceTlsFindings({
    certificateBytes: Buffer.from(certificate),
    privateKeyBytes: Buffer.from(privateKey),
    hosts,
    nowMs,
    validThroughMs: timing.validThroughMs ?? nowMs + 60_000,
  });
}

function privateKeyPem() {
  return generateKeyPairSync("rsa", { modulusLength: 2048 }).privateKey.export({
    format: "pem",
    type: "pkcs8",
  });
}

test("matching leaf and leaf-first chain pass for bundled reference TLS", () => {
  const generated = fixture();
  assert.deepEqual(findings(generated.leafCertificate, generated.privateKey), []);
  assert.deepEqual(findings(generated.certificateChain, generated.privateKey), []);
  assert.deepEqual(
    findings(generated.certificateChain.replaceAll("\n", "\r\n"), generated.privateKey),
    [],
  );
});

test("external OIDC requires only the application SAN", () => {
  const generated = fixture([APP_HOST]);
  assert.deepEqual(findings(generated.certificateChain, generated.privateKey, [APP_HOST]), []);
  assert.match(
    findings(generated.certificateChain, generated.privateKey)[0],
    /required hostname.*DNS SAN/,
  );
});

test("SAN matching permits only conventional whole-label wildcards", () => {
  const wildcard = fixture(["*.reference.example"], "reference.example");
  assert.deepEqual(findings(wildcard.certificateChain, wildcard.privateKey), []);

  const partial = fixture(["app*.reference.example"]);
  assert.match(
    findings(partial.certificateChain, partial.privateKey)[0],
    /required hostname.*DNS SAN/,
  );

  const commonNameOnly = fixture([]);
  assert.match(
    findings(commonNameOnly.certificateChain, commonNameOnly.privateKey)[0],
    /required hostname.*DNS SAN/,
  );
});

test("private-key parsing is exact and the leaf key must match", () => {
  const generated = fixture();
  const wrongKey = privateKeyPem();
  assert.match(
    findings(generated.certificateChain, wrongKey)[0],
    /certificate and private key do not match/,
  );
  assert.match(
    findings(generated.certificateChain, `${generated.privateKey}\n${generated.privateKey}`)[0],
    /PEM block count/,
  );
  const encryptedLabel = generated.privateKey
    .replace("BEGIN PRIVATE KEY", "BEGIN ENCRYPTED PRIVATE KEY")
    .replace("END PRIVATE KEY", "END ENCRYPTED PRIVATE KEY");
  assert.match(
    findings(generated.certificateChain, encryptedLabel)[0],
    /PEM type/,
  );
  const publicKey = generateKeyPairSync("rsa", { modulusLength: 2048 }).publicKey.export({
    format: "pem",
    type: "spki",
  });
  assert.match(
    findings(generated.certificateChain, `${generated.privateKey}\n${publicKey}`)[0],
    /PEM type/,
  );
});

test("an explicit extended-key usage must permit TLS server authentication", () => {
  const clientOnly = generateTestTlsChain({
    commonName: APP_HOST,
    sanHosts: [APP_HOST, AUTH_HOST],
    extendedKeyUsage: "clientAuth",
  });
  assert.match(
    findings(clientOnly.certificateChain, clientOnly.privateKey)[0],
    /not valid for TLS server authentication/,
  );
});

test("certificate bundles are bounded to one ordered unique chain", () => {
  const generated = fixture();
  const unrelated = fixture(["unrelated.reference.example"], "unrelated.reference.example");
  assert.match(
    findings(
      `${generated.leafCertificate}${unrelated.intermediateCertificate}`,
      generated.privateKey,
    )[0],
    /chain order or signature/,
  );
  assert.match(
    findings(
      `${generated.intermediateCertificate}${generated.leafCertificate}`,
      generated.privateKey,
    )[0],
    /leaf certificate must not be a certificate authority/,
  );
  assert.match(
    findings(
      `${generated.certificateChain}${generated.intermediateCertificate}`,
      generated.privateKey,
    )[0],
    /duplicate/,
  );
  assert.match(
    findings(
      `${generated.certificateChain}${generated.rootCertificate}`,
      generated.privateKey,
    )[0],
    /omit its self-signed trust root/,
  );
  assert.match(
    findings(`not pem\n${generated.certificateChain}`, generated.privateKey)[0],
    /PEM structure/,
  );
  assert.match(
    findings(`\u0000${generated.certificateChain}`, generated.privateKey)[0],
    /PEM encoding/,
  );
  const corrupted = generated.leafCertificate.replace(/\n([A-Za-z0-9])/, "\n!");
  assert.match(findings(corrupted, generated.privateKey)[0], /certificate bundle/);
  assert.match(
    findings(
      generated.leafCertificate.replace("END CERTIFICATE", "END CERTIFICATE BROKEN"),
      generated.privateKey,
    )[0],
    /PEM structure/,
  );
});

test("every supplied certificate must remain valid through the lifecycle deadline", () => {
  const generated = fixture();
  const certificates = [
    new X509Certificate(generated.leafCertificate),
    new X509Certificate(generated.intermediateCertificate),
  ];
  const validFrom = Math.max(...certificates.map((certificate) => Date.parse(certificate.validFrom)));
  const validTo = Math.min(...certificates.map((certificate) => Date.parse(certificate.validTo)));
  assert.deepEqual(
    findings(generated.certificateChain, generated.privateKey, [APP_HOST, AUTH_HOST], {
      nowMs: validFrom,
      validThroughMs: validTo - 1,
    }),
    [],
  );
  assert.match(
    findings(generated.certificateChain, generated.privateKey, [APP_HOST, AUTH_HOST], {
      nowMs: validFrom - 1,
      validThroughMs: validFrom - 1,
    })[0],
    /not yet valid/,
  );
  assert.match(
    findings(generated.certificateChain, generated.privateKey, [APP_HOST, AUTH_HOST], {
      nowMs: validFrom,
      validThroughMs: validTo,
    })[0],
    /expires before lifecycle completion/,
  );
});

test("the CLI reads bounded no-follow files and emits no key or path content", () => {
  const generated = fixture();
  const scratch = mkdtempSync(join(tmpdir(), "synveda-tls-check-"));
  chmodSync(scratch, 0o700);
  const certificate = join(scratch, "certificate.pem");
  const privateKey = join(scratch, "private-key.pem");
  const linkedKey = join(scratch, "linked-key.pem");
  const fifoKey = join(scratch, "fifo-key.pem");
  writeFileSync(certificate, generated.certificateChain, { mode: 0o600 });
  writeFileSync(privateKey, generated.privateKey, { mode: 0o600 });
  chmodSync(certificate, 0o600);
  chmodSync(privateKey, 0o600);
  const argumentsList = [
    CHECKER,
    "--cert-file",
    certificate,
    "--key-file",
    privateKey,
    "--oidc",
    "bundled",
    "--app-host",
    APP_HOST,
    "--auth-host",
    AUTH_HOST,
    "--valid-for-seconds",
    "60",
  ];
  const keySentinel = generated.privateKey
    .split("\n")
    .find((line) => line.length > 40);
  assert.ok(keySentinel);
  try {
    const accepted = spawnSync(process.execPath, argumentsList, {
      cwd: ROOT,
      encoding: "utf8",
    });
    assert.equal(accepted.status, 0, accepted.stderr);
    assert.match(accepted.stdout, /reference TLS inputs validated/);

    const wrongHost = [...argumentsList];
    wrongHost[wrongHost.indexOf(APP_HOST)] = "wrong.reference.example";
    const refused = spawnSync(process.execPath, wrongHost, {
      cwd: ROOT,
      encoding: "utf8",
    });
    assert.equal(refused.status, 78, refused.stderr);
    const output = `${refused.stdout}${refused.stderr}`;
    for (const forbidden of [
      keySentinel,
      certificate,
      privateKey,
      "BEGIN CERTIFICATE",
      "BEGIN PRIVATE KEY",
    ]) {
      assert.ok(!output.includes(forbidden), `output leaked ${forbidden}`);
    }

    symlinkSync(privateKey, linkedKey);
    const linkedArguments = argumentsList.map((value) =>
      value === privateKey ? linkedKey : value,
    );
    const linked = spawnSync(process.execPath, linkedArguments, {
      cwd: ROOT,
      encoding: "utf8",
    });
    assert.equal(linked.status, 78, linked.stderr);
    assert.match(linked.stderr, /private-key file could not be read safely/);

    execFileSync("mkfifo", [fifoKey]);
    const fifoArguments = argumentsList.map((value) =>
      value === privateKey ? fifoKey : value,
    );
    const started = Date.now();
    const fifo = spawnSync(process.execPath, fifoArguments, {
      cwd: ROOT,
      encoding: "utf8",
      timeout: 2_000,
    });
    assert.equal(fifo.signal, null, "writerless FIFO reached the test timeout");
    assert.ok(Date.now() - started < 2_000, "writerless FIFO blocked the validator");
    assert.equal(fifo.status, 78, fifo.stderr);
    assert.match(fifo.stderr, /private-key file could not be read safely/);

    writeFileSync(certificate, Buffer.alloc(256 * 1024 + 1, 65), { mode: 0o600 });
    const oversized = spawnSync(process.execPath, argumentsList, {
      cwd: ROOT,
      encoding: "utf8",
    });
    assert.equal(oversized.status, 78, oversized.stderr);
    assert.match(oversized.stderr, /certificate file could not be read safely/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the wrapper validates startup evidence but never TLS-validity-blocks teardown", () => {
  const source = readFileSync(WRAPPER, "utf8");
  const validation = source.indexOf('--valid-for-seconds "$lifecycle_remaining"');
  const actionCase = source.lastIndexOf('case "$action" in', validation);
  const actionEnd = source.indexOf("esac", validation);
  const branch = source.slice(actionCase, actionEnd);
  assert.ok(actionCase >= 0 && validation > actionCase && actionEnd > validation);
  assert.match(branch, /config\|up\|smoke\|restart-gateway\)/);
  assert.doesNotMatch(branch, /down|reset/);
  assert.ok(
    validation < source.indexOf('capture_bounded_output 30 "$docker_bin" compose version'),
  );
});
