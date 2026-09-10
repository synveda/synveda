import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const fixtureCache = new Map();

function safeDnsName(value, wildcard = false) {
  const grammar = wildcard ? /^[a-z0-9*.-]+$/ : /^[a-z0-9.-]+$/;
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 253 &&
    grammar.test(value) &&
    !value.includes("..")
  );
}

function runOpenSsl(argumentsList) {
  const result = spawnSync("openssl", argumentsList, {
    encoding: "utf8",
    maxBuffer: 16 * 1024,
    timeout: 15_000,
    stdio: ["ignore", "ignore", "pipe"],
  });
  if (result.error !== undefined || result.status !== 0) {
    throw new Error("ephemeral TLS fixture generation failed");
  }
}

function privateWrite(path, value) {
  writeFileSync(path, value, { mode: 0o600 });
  chmodSync(path, 0o600);
}

export function generateTestTlsChain({
  commonName,
  sanHosts,
  extendedKeyUsage = "serverAuth",
} = {}) {
  if (!safeDnsName(commonName) || !Array.isArray(sanHosts)) {
    throw new TypeError("safe test certificate names are required");
  }
  if (
    sanHosts.length > 8 ||
    sanHosts.some((host) => !safeDnsName(host, true)) ||
    new Set(sanHosts).size !== sanHosts.length
  ) {
    throw new TypeError("safe unique test certificate SANs are required");
  }
  if (!new Set(["serverAuth", "clientAuth", undefined]).has(extendedKeyUsage)) {
    throw new TypeError("safe test certificate extended-key usage is required");
  }
  const cacheKey = JSON.stringify({ commonName, sanHosts, extendedKeyUsage });
  const cached = fixtureCache.get(cacheKey);
  if (cached !== undefined) return cached;

  const scratch = mkdtempSync(join(tmpdir(), "synveda-test-tls-"));
  chmodSync(scratch, 0o700);
  const rootConfig = join(scratch, "root.cnf");
  const rootKey = join(scratch, "root.key");
  const rootCertificate = join(scratch, "root.pem");
  const intermediateConfig = join(scratch, "intermediate.cnf");
  const intermediateKey = join(scratch, "intermediate.key");
  const intermediateRequest = join(scratch, "intermediate.csr");
  const intermediateCertificate = join(scratch, "intermediate.pem");
  const leafConfig = join(scratch, "leaf.cnf");
  const leafKey = join(scratch, "leaf.key");
  const leafRequest = join(scratch, "leaf.csr");
  const leafCertificate = join(scratch, "leaf.pem");
  const sanExtension = sanHosts.length === 0 ? "" : "subjectAltName=@san\n";
  const extendedKeyUsageExtension =
    extendedKeyUsage === undefined ? "" : `extendedKeyUsage=${extendedKeyUsage}\n`;
  const sanSection =
    sanHosts.length === 0
      ? ""
      : `\n[san]\n${sanHosts.map((host, index) => `DNS.${index + 1}=${host}`).join("\n")}\n`;

  try {
    privateWrite(
      rootConfig,
      `[req]
prompt=no
distinguished_name=dn

[dn]
CN=Synveda TLS Fixture Root

[v3_root]
basicConstraints=critical,CA:true,pathlen:1
keyUsage=critical,keyCertSign,cRLSign
subjectKeyIdentifier=hash
`,
    );
    privateWrite(
      intermediateConfig,
      `[req]
prompt=no
distinguished_name=dn

[dn]
CN=Synveda TLS Fixture Intermediate

[v3_intermediate]
basicConstraints=critical,CA:true,pathlen:0
keyUsage=critical,keyCertSign,cRLSign
subjectKeyIdentifier=hash
authorityKeyIdentifier=keyid,issuer
`,
    );
    privateWrite(
      leafConfig,
      `[req]
prompt=no
distinguished_name=dn

[dn]
CN=${commonName}

[v3_leaf]
basicConstraints=critical,CA:false
keyUsage=critical,digitalSignature,keyEncipherment
${extendedKeyUsageExtension}${sanExtension}subjectKeyIdentifier=hash
authorityKeyIdentifier=keyid,issuer
${sanSection}`,
    );
    runOpenSsl([
      "req",
      "-x509",
      "-newkey",
      "rsa:2048",
      "-nodes",
      "-sha256",
      "-days",
      "2",
      "-keyout",
      rootKey,
      "-out",
      rootCertificate,
      "-config",
      rootConfig,
      "-extensions",
      "v3_root",
    ]);
    runOpenSsl([
      "req",
      "-new",
      "-newkey",
      "rsa:2048",
      "-nodes",
      "-sha256",
      "-keyout",
      intermediateKey,
      "-out",
      intermediateRequest,
      "-config",
      intermediateConfig,
    ]);
    runOpenSsl([
      "x509",
      "-req",
      "-in",
      intermediateRequest,
      "-CA",
      rootCertificate,
      "-CAkey",
      rootKey,
      "-set_serial",
      "2",
      "-days",
      "2",
      "-sha256",
      "-out",
      intermediateCertificate,
      "-extfile",
      intermediateConfig,
      "-extensions",
      "v3_intermediate",
    ]);
    runOpenSsl([
      "req",
      "-new",
      "-newkey",
      "rsa:2048",
      "-nodes",
      "-sha256",
      "-keyout",
      leafKey,
      "-out",
      leafRequest,
      "-config",
      leafConfig,
    ]);
    runOpenSsl([
      "x509",
      "-req",
      "-in",
      leafRequest,
      "-CA",
      intermediateCertificate,
      "-CAkey",
      intermediateKey,
      "-set_serial",
      "3",
      "-days",
      "2",
      "-sha256",
      "-out",
      leafCertificate,
      "-extfile",
      leafConfig,
      "-extensions",
      "v3_leaf",
    ]);
    for (const path of [
      rootKey,
      rootCertificate,
      intermediateKey,
      intermediateRequest,
      intermediateCertificate,
      leafKey,
      leafRequest,
      leafCertificate,
    ]) {
      chmodSync(path, 0o600);
    }
    const leaf = readFileSync(leafCertificate, "utf8");
    const intermediate = readFileSync(intermediateCertificate, "utf8");
    const root = readFileSync(rootCertificate, "utf8");
    const fixture = Object.freeze({
      leafCertificate: leaf,
      intermediateCertificate: intermediate,
      rootCertificate: root,
      certificateChain: `${leaf.trimEnd()}\n${intermediate}`,
      privateKey: readFileSync(leafKey, "utf8"),
    });
    fixtureCache.set(cacheKey, fixture);
    return fixture;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}
