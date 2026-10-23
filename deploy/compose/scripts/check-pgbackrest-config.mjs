#!/usr/bin/env node
// The Compose selector accepts one narrow S3 repository grammar. Do not print
// parsed values: this file contains storage credentials and the cipher pass.
import { readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ALLOWED = new Set([
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

export class ConfigRefused extends Error {}

function refuse() {
  throw new ConfigRefused("pgBackRest S3 configuration was refused");
}

function sections(source) {
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
      if (!ALLOWED.has(key) || options.has(key)) refuse();
      options.set(key, value);
    } else {
      if (key !== "pg1-path" || stanzaOptions.has(key)) refuse();
      stanzaOptions.set(key, value);
    }
  }
  if (seenSections.size !== 2 ||
      stanzaOptions.get("pg1-path") !== "/var/lib/postgresql/data") refuse();
  return options;
}

/** Validates one repository configuration and returns its global options. */
export function parsePgbackrestConfig(source, caPresent) {
  if (typeof source !== "string" || source.includes("\0") || source.includes("\r")) refuse();
  const options = sections(source);
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
  if ((caPath !== undefined) !== caPresent) refuse();
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
  if (options.has("repo1-storage-port") &&
      !/^[1-9][0-9]{0,4}$/.test(options.get("repo1-storage-port"))) refuse();
  return options;
}

/** Reads a private repository configuration with the same bounds as Compose. */
export function readPgbackrestConfig(path, caPresent) {
  let source;
  try {
    if (statSync(path).size > 16 * 1024) refuse();
    source = new TextDecoder("utf-8", { fatal: true }).decode(readFileSync(path));
  } catch (error) {
    if (error instanceof ConfigRefused) throw error;
    refuse();
  }
  return parsePgbackrestConfig(source, caPresent);
}

function main() {
  const path = process.argv[2];
  const caPresent = process.argv[3];
  if (!path || !["true", "false"].includes(caPresent) ||
      process.argv.length !== 4) process.exit(64);
  try {
    readPgbackrestConfig(path, caPresent === "true");
  } catch {
    console.error("compose: pgBackRest S3 configuration was refused");
    process.exit(78);
  }
  console.log("compose: pgBackRest S3 configuration shape valid");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
