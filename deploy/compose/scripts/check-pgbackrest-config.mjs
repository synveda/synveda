#!/usr/bin/env node
// The Compose selector accepts one narrow S3 repository grammar. Do not print
// parsed values: this file contains storage credentials and the cipher pass.
import { readFileSync, statSync } from "node:fs";

const path = process.argv[2];
const caPresent = process.argv[3];
if (!path || !["true", "false"].includes(caPresent) ||
    process.argv.length !== 4) process.exit(64);

function refuse() {
  console.error("compose: pgBackRest S3 configuration was refused");
  process.exit(78);
}

let source;
try {
  if (statSync(path).size > 16 * 1024) refuse();
  source = new TextDecoder("utf-8", { fatal: true }).decode(readFileSync(path));
} catch {
  refuse();
}
if (source.includes("\0") || source.includes("\r")) refuse();

const allowed = new Set([
  "repo1-type",
  "repo1-path",
  "repo1-cipher-type",
  "repo1-cipher-pass",
  "repo1-s3-bucket",
  "repo1-s3-endpoint",
  "repo1-s3-region",
  "repo1-s3-key-type",
  "repo1-s3-key",
  "repo1-s3-key-secret",
  "repo1-s3-token",
  "repo1-s3-uri-style",
  "repo1-storage-port",
  "repo1-storage-ca-file",
  "repo1-storage-verify-tls",
]);
const options = new Map();
const stanzaOptions = new Map();
const seenSections = new Set();
let section;
for (const raw of source.split("\n")) {
  const line = raw.trim();
  if (line === "" || line.startsWith("#")) continue;
  if (line.startsWith("[")) {
    if (!["[global]", "[synveda]"].includes(line) || seenSections.has(line)) refuse();
    section = line;
    seenSections.add(line);
    continue;
  }
  const equal = line.indexOf("=");
  if (!section || equal <= 0) refuse();
  const key = line.slice(0, equal).trim();
  const value = line.slice(equal + 1).trim();
  if (!value) refuse();
  if (section === "[global]") {
    if (!allowed.has(key) || options.has(key)) refuse();
    options.set(key, value);
  } else {
    if (key !== "pg1-path" || stanzaOptions.has(key)) refuse();
    stanzaOptions.set(key, value);
  }
}
if (seenSections.size !== 2 ||
    stanzaOptions.get("pg1-path") !== "/var/lib/postgresql/data") refuse();
for (const [key, value] of [
  ["repo1-type", "s3"],
  ["repo1-cipher-type", "aes-256-cbc"],
  ["repo1-storage-verify-tls", "y"],
]) {
  if (options.get(key) !== value) refuse();
}
for (const key of [
  "repo1-path",
  "repo1-s3-bucket",
  "repo1-s3-endpoint",
  "repo1-s3-region",
]) {
  if (!options.has(key)) refuse();
}
const repoPath = options.get("repo1-path");
if (!/^\/[A-Za-z0-9][A-Za-z0-9/_-]{0,127}$/.test(repoPath)) refuse();
const caPath = options.get("repo1-storage-ca-file");
if (caPath !== undefined && caPath !==
    "/var/run/postgresql/pgbackrest-ca.pem") refuse();
if ((caPath !== undefined) !== (caPresent === "true")) refuse();
if ((options.get("repo1-cipher-pass")?.length ?? 0) < 32) refuse();
const keyType = options.get("repo1-s3-key-type") ?? "shared";
if (keyType === "shared") {
  if (!options.has("repo1-s3-key") || !options.has("repo1-s3-key-secret")) refuse();
} else if (keyType === "auto") {
  if (options.has("repo1-s3-key") || options.has("repo1-s3-key-secret")) refuse();
} else {
  refuse();
}
if (options.has("repo1-s3-uri-style") &&
    !["host", "path"].includes(options.get("repo1-s3-uri-style"))) refuse();
console.log("compose: pgBackRest S3 configuration shape valid");
