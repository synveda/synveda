#!/usr/bin/env node
import {
  X509Certificate,
  createPrivateKey,
} from "node:crypto";
import {
  closeSync,
  constants,
  fstatSync,
  openSync,
  readSync,
} from "node:fs";
import process from "node:process";
import { fileURLToPath } from "node:url";

const CERTIFICATE_LIMIT = 256 * 1024;
const PRIVATE_KEY_LIMIT = 64 * 1024;
const CHAIN_LIMIT = 8;
const SERVER_AUTH_OID = "1.3.6.1.5.5.7.3.1";
const ANY_EXTENDED_KEY_USAGE_OID = "2.5.29.37.0";
const HOST_OPTIONS = Object.freeze({
  subject: "never",
  wildcards: true,
  partialWildcards: false,
  multiLabelWildcards: false,
  singleLabelSubdomains: false,
});

class TlsRefusal extends Error {}

class SafeReadError extends Error {
  constructor(status) {
    super("safe input read failed");
    this.status = status;
  }
}

function refuse(message) {
  throw new TlsRefusal(message);
}

function asciiPem(buffer) {
  for (const byte of buffer) {
    if (byte === 0 || byte > 0x7f) refuse("PEM encoding was refused");
  }
  return buffer.toString("ascii");
}

function pemBlocks(buffer, allowedLabels, maximum) {
  const text = asciiPem(buffer);
  const pattern = /-----BEGIN ([A-Z0-9][A-Z0-9 ]{0,31})-----[\s\S]*?-----END \1-----/g;
  const blocks = [];
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    if (!/^[\t\n\r ]*$/.test(text.slice(cursor, match.index))) {
      refuse("PEM structure was refused");
    }
    if (!allowedLabels.has(match[1])) refuse("PEM type was refused");
    blocks.push(match[0]);
    if (blocks.length > maximum) refuse("PEM block count was refused");
    cursor = match.index + match[0].length;
  }
  if (blocks.length === 0 || !/^[\t\n\r ]*$/.test(text.slice(cursor))) {
    refuse("PEM structure was refused");
  }
  return blocks;
}

function parsedCertificates(certificateBytes) {
  const blocks = pemBlocks(certificateBytes, new Set(["CERTIFICATE"]), CHAIN_LIMIT);
  try {
    return blocks.map((block) => new X509Certificate(block));
  } catch {
    refuse("certificate bundle was refused");
  }
}

function parsedPrivateKey(privateKeyBytes) {
  const blocks = pemBlocks(
    privateKeyBytes,
    new Set(["PRIVATE KEY", "RSA PRIVATE KEY", "EC PRIVATE KEY"]),
    1,
  );
  try {
    return createPrivateKey({ key: blocks[0], format: "pem" });
  } catch {
    refuse("private key was refused");
  }
}

function safeHostname(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 253 &&
    /^[a-z0-9.-]+$/.test(value) &&
    value.includes(".") &&
    !value.startsWith(".") &&
    !value.endsWith(".") &&
    !value.includes("..")
  );
}

export function referenceTlsFindings({
  certificateBytes,
  privateKeyBytes,
  hosts,
  nowMs = Date.now(),
  validThroughMs = nowMs,
}) {
  try {
    if (
      !Buffer.isBuffer(certificateBytes) ||
      !Buffer.isBuffer(privateKeyBytes) ||
      !Array.isArray(hosts) ||
      hosts.length < 1 ||
      hosts.length > 2 ||
      hosts.some((host) => !safeHostname(host)) ||
      new Set(hosts).size !== hosts.length ||
      !Number.isFinite(nowMs) ||
      !Number.isFinite(validThroughMs) ||
      validThroughMs < nowMs
    ) {
      refuse("TLS validation input was refused");
    }

    const certificates = parsedCertificates(certificateBytes);
    const fingerprints = new Set();
    for (const certificate of certificates) {
      if (fingerprints.has(certificate.fingerprint256)) {
        refuse("certificate chain contains a duplicate");
      }
      fingerprints.add(certificate.fingerprint256);
      const validFrom = Date.parse(certificate.validFrom);
      const validTo = Date.parse(certificate.validTo);
      if (!Number.isFinite(validFrom) || !Number.isFinite(validTo)) {
        refuse("certificate validity was refused");
      }
      if (validFrom > nowMs) refuse("certificate chain is not yet valid");
      if (validTo <= validThroughMs) {
        refuse("certificate chain expires before lifecycle completion");
      }
    }

    const leaf = certificates[0];
    if (leaf.ca) refuse("leaf certificate must not be a certificate authority");
    const extendedKeyUsage = leaf.keyUsage ?? leaf.toLegacyObject()?.ext_key_usage;
    if (
      Array.isArray(extendedKeyUsage) &&
      extendedKeyUsage.length > 0 &&
      !extendedKeyUsage.includes(SERVER_AUTH_OID) &&
      !extendedKeyUsage.includes(ANY_EXTENDED_KEY_USAGE_OID)
    ) {
      refuse("leaf certificate is not valid for TLS server authentication");
    }
    for (const host of hosts) {
      if (leaf.checkHost(host, HOST_OPTIONS) === undefined) {
        refuse("required hostname is not covered by a certificate DNS SAN");
      }
    }

    for (let index = 0; index < certificates.length - 1; index += 1) {
      const child = certificates[index];
      const parent = certificates[index + 1];
      if (!parent.ca || !child.checkIssued(parent) || !child.verify(parent.publicKey)) {
        refuse("certificate chain order or signature was refused");
      }
    }
    const terminal = certificates.at(-1);
    if (terminal.ca && terminal.checkIssued(terminal) && terminal.verify(terminal.publicKey)) {
      refuse("certificate bundle must omit its self-signed trust root");
    }

    const privateKey = parsedPrivateKey(privateKeyBytes);
    if (!leaf.checkPrivateKey(privateKey)) {
      refuse("certificate and private key do not match");
    }
    return [];
  } catch (error) {
    return [error instanceof TlsRefusal ? error.message : "TLS inputs were refused"];
  }
}

function readBoundedRegularFile(path, maximum) {
  if (
    typeof constants.O_NOFOLLOW !== "number" ||
    typeof constants.O_NONBLOCK !== "number"
  ) {
    throw new SafeReadError(69);
  }
  let descriptor;
  let staging;
  try {
    try {
      descriptor = openSync(
        path,
        constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW,
      );
    } catch (error) {
      throw new SafeReadError(error?.code === "ELOOP" ? 78 : 69);
    }
    const before = fstatSync(descriptor, { bigint: true });
    if (!before.isFile() || before.size < 1n || before.size > BigInt(maximum)) {
      throw new SafeReadError(78);
    }
    staging = Buffer.alloc(maximum + 1);
    let offset = 0;
    while (offset < staging.length) {
      const count = readSync(descriptor, staging, offset, staging.length - offset, null);
      if (count === 0) break;
      offset += count;
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (
      offset < 1 ||
      offset > maximum ||
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      before.mtimeNs !== after.mtimeNs ||
      before.ctimeNs !== after.ctimeNs ||
      after.size !== BigInt(offset)
    ) {
      throw new SafeReadError(78);
    }
    return Buffer.from(staging.subarray(0, offset));
  } finally {
    staging?.fill(0);
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || values.has(name)) return undefined;
    values.set(name, value);
  }
  const allowed = new Set([
    "--cert-file",
    "--key-file",
    "--oidc",
    "--app-host",
    "--auth-host",
    "--valid-for-seconds",
  ]);
  if ([...values.keys()].some((name) => !allowed.has(name))) return undefined;
  const certificateFile = values.get("--cert-file");
  const privateKeyFile = values.get("--key-file");
  const oidc = values.get("--oidc");
  const appHost = values.get("--app-host");
  const authHost = values.get("--auth-host");
  const validFor = values.get("--valid-for-seconds");
  if (
    !certificateFile ||
    !privateKeyFile ||
    !new Set(["bundled", "external"]).has(oidc) ||
    !safeHostname(appHost) ||
    (oidc === "bundled" ? !safeHostname(authHost) : authHost !== undefined) ||
    (oidc === "bundled" && appHost === authHost) ||
    !/^[1-9][0-9]{0,4}$/.test(validFor ?? "")
  ) {
    return undefined;
  }
  const validForSeconds = Number(validFor);
  if (validForSeconds > 3_600) return undefined;
  return {
    certificateFile,
    privateKeyFile,
    hosts: oidc === "bundled" ? [appHost, authHost] : [appHost],
    validForSeconds,
  };
}

export function main(argv = process.argv.slice(2)) {
  const selection = parseArguments(argv);
  if (selection === undefined) {
    console.error("compose-tls: configuration was refused");
    process.exitCode = 64;
    return;
  }
  if (
    Number(process.versions.node.split(".")[0]) < 22 ||
    !new Set(["darwin", "linux"]).has(process.platform)
  ) {
    console.error("compose-tls: Node 22 on macOS or Linux is required");
    process.exitCode = 69;
    return;
  }

  let certificateBytes;
  let privateKeyBytes;
  try {
    try {
      certificateBytes = readBoundedRegularFile(
        selection.certificateFile,
        CERTIFICATE_LIMIT,
      );
    } catch (error) {
      const status = error instanceof SafeReadError ? error.status : 69;
      console.error("compose-tls: certificate file could not be read safely");
      process.exitCode = status;
      return;
    }
    try {
      privateKeyBytes = readBoundedRegularFile(
        selection.privateKeyFile,
        PRIVATE_KEY_LIMIT,
      );
    } catch (error) {
      const status = error instanceof SafeReadError ? error.status : 69;
      console.error("compose-tls: private-key file could not be read safely");
      process.exitCode = status;
      return;
    }
    const nowMs = Date.now();
    const findings = referenceTlsFindings({
      certificateBytes,
      privateKeyBytes,
      hosts: selection.hosts,
      nowMs,
      validThroughMs: nowMs + selection.validForSeconds * 1_000,
    });
    if (findings.length > 0) {
      console.error(`compose-tls: ${findings[0]}`);
      process.exitCode = 78;
      return;
    }
    console.log("reference TLS inputs validated");
  } finally {
    privateKeyBytes?.fill(0);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
