#!/usr/bin/env node
import { createHash, randomBytes } from "node:crypto";
import {
  closeSync,
  constants,
  fchmodSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  openSync,
  writeSync,
  unlinkSync,
} from "node:fs";
import { isAbsolute, join } from "node:path";

function canonical(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number" && Number.isSafeInteger(value)) return String(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  }
  throw new Error("unsupported value");
}

function lowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function writeEffect(path, value) {
  const bytes = Buffer.from(`${canonical(value)}\n`, "utf8");
  const directory = join(path, "..");
  const stagePath = join(directory, `.fake-effect-stage-${randomBytes(16).toString("hex")}`);
  let descriptor;
  try {
    descriptor = openSync(
      stagePath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    );
    fchmodSync(descriptor, 0o600);
    let offset = 0;
    while (offset < bytes.length) {
      const written = writeSync(descriptor, bytes, offset, bytes.length - offset);
      if (written < 1) throw new Error("write failed");
      offset += written;
    }
    fsyncSync(descriptor);
    const staged = fstatSync(descriptor, { bigint: true });
    const named = lstatSync(stagePath, { bigint: true });
    if (
      !staged.isFile() ||
      staged.uid !== BigInt(process.getuid()) ||
      staged.nlink !== 1n ||
      (staged.mode & 0o7777n) !== 0o600n ||
      staged.dev !== named.dev ||
      staged.ino !== named.ino ||
      staged.uid !== named.uid ||
      staged.mode !== named.mode ||
      staged.nlink !== named.nlink
    ) {
      throw new Error("effect stage identity changed");
    }
    linkSync(stagePath, path);
    const final = lstatSync(path, { bigint: true });
    if (
      final.dev !== staged.dev ||
      final.ino !== staged.ino ||
      final.uid !== staged.uid ||
      final.mode !== staged.mode ||
      final.nlink !== 2n
    ) {
      throw new Error("effect link identity changed");
    }
    closeSync(descriptor);
    descriptor = undefined;
    const directoryDescriptor = openSync(
      directory,
      constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
    );
    try {
      fsyncSync(directoryDescriptor);
    } finally {
      closeSync(directoryDescriptor);
    }
    unlinkSync(stagePath);
    const finalDirectoryDescriptor = openSync(
      directory,
      constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
    );
    try {
      fsyncSync(finalDirectoryDescriptor);
    } finally {
      closeSync(finalDirectoryDescriptor);
    }
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

const [scenario, rootPath, fixtureId, intentSha256, rootOwnerSha256, witnessSha256] =
  process.argv.slice(2);
process.umask(0o077);
if (
  !new Set(["descendant", "fail", "hang", "orphan", "pass"]).has(scenario) ||
  !isAbsolute(rootPath ?? "") ||
  !lowerHex(fixtureId, 32) ||
  !lowerHex(intentSha256, 64) ||
  !lowerHex(rootOwnerSha256, 64) ||
  !lowerHex(witnessSha256, 64)
) {
  process.exit(64);
}

if (scenario === "descendant") {
  process.on("SIGTERM", () => process.exit(75));
  process.on("disconnect", () => process.exit(75));
  process.channel?.unref();
  if (!process.connected) process.exit(75);
  setInterval(() => {}, 1_000);
} else {
  process.on("disconnect", () => process.exit(75));
  process.channel?.unref();
  if (!process.connected) process.exit(75);
  const environmentKeys = Object.keys(process.env).sort();
  const expectedKeys = [
    "COLIMA_CACHE_HOME",
    "COLIMA_HOME",
    "DOCKER_CONFIG",
    "LANG",
    "LC_ALL",
    "LIMA_HOME",
    "TMPDIR",
  ];
  if (process.platform === "darwin") expectedKeys.push("__CF_USER_TEXT_ENCODING");
  expectedKeys.sort();
  if (JSON.stringify(environmentKeys) !== JSON.stringify(expectedKeys)) process.exit(78);
  writeEffect(join(rootPath, "t", "fake-effect.json"), {
    environment_keys: environmentKeys,
    fixture_id: fixtureId,
    provider_intent_sha256: intentSha256,
    provider_root_owner_sha256: rootOwnerSha256,
    scenario,
    schema: "synveda.clean-engine.controlled-fake-effect.v1",
    witness_sha256: witnessSha256,
  });
  if (scenario === "fail") process.exit(42);
  if (scenario === "hang") {
    process.on("SIGTERM", () => {});
    if (!process.connected) process.exit(75);
    setInterval(() => {}, 1_000);
  }
}
