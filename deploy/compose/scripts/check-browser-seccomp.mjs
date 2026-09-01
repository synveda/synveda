#!/usr/bin/env node
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, realpathSync } from "node:fs";

const EXPECTED_SHA256 = "cc3e61cabda6bbc1e53e54d27ba4d55a9d3be829b6dd1a596f4a7b31b1cc7849";
const EXPECTED_BYTES = 12_997;

function fail(message, status = 78) {
  process.stderr.write(`browser-seccomp: ${message}\n`);
  process.exit(status);
}

if (process.argv.length !== 4 || process.argv[2] !== "--profile") {
  fail("expected --profile PATH", 64);
}
const path = process.argv[3];
if (typeof path !== "string" || !path.startsWith("/") || path.length > 1024) {
  fail("profile path was refused", 64);
}
let metadata;
let bytes;
try {
  metadata = lstatSync(path);
  if (realpathSync(path) !== path) fail("profile path was not canonical");
  bytes = readFileSync(path);
} catch (error) {
  if (error?.message?.startsWith("profile path")) throw error;
  fail("profile was unavailable", 69);
}
if (
  !metadata.isFile() ||
  metadata.isSymbolicLink() ||
  metadata.nlink !== 1 ||
  metadata.size !== EXPECTED_BYTES ||
  bytes.length !== EXPECTED_BYTES
) fail("profile filesystem contract was refused");
if (createHash("sha256").update(bytes).digest("hex") !== EXPECTED_SHA256) {
  fail("profile digest was refused");
}
let profile;
try {
  profile = JSON.parse(bytes.toString("utf8"));
} catch {
  fail("profile JSON was refused");
}
const namespaceRule = profile?.syscalls?.[0];
if (
  profile?.defaultAction !== "SCMP_ACT_ERRNO" ||
  !profile?.archMap?.some(({ architecture }) => architecture === "SCMP_ARCH_X86_64") ||
  !profile?.archMap?.some(({ architecture }) => architecture === "SCMP_ARCH_AARCH64") ||
  JSON.stringify(namespaceRule) !== JSON.stringify({
    comment: "Allow create user namespaces",
    names: ["clone", "setns", "unshare"],
    action: "SCMP_ACT_ALLOW",
    args: [],
    includes: {},
    excludes: {},
  })
) fail("profile sandbox structure was refused");

process.stdout.write("reviewed Playwright seccomp profile validated\n");
