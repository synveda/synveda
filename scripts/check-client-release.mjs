#!/usr/bin/env node
// OPS-12: archive execution is a required producer result, never an optional log.
import { lstatSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { sha256 } from "./client-artifact.mjs";

export function checkClientRelease(directory, version, source, publish, lock) {
  for (const [target, pin] of Object.entries(lock.targets)) {
    const archive = join(directory, `synveda-client-${version}-${target}.tar.gz`);
    const reportFile = join(directory, `synveda-client-report-${target}.json`);
    if (![archive, reportFile].every((path) => lstatSync(path).isFile() && !lstatSync(path).isSymbolicLink())) throw new Error("client release inputs must be regular files");
    const report = JSON.parse(readFileSync(reportFile));
    if (report.schema_version !== 1 || report.evidence !== "native-client-archive" || report.target !== target ||
        report.version !== version || report.source_sha !== source || report.node?.version !== lock.version ||
        report.node.archive_sha256 !== pin.sha256 || report.node.archive !== pin.archive ||
        report.archive_sha256 !== sha256(readFileSync(archive)) || report.archive_bytes !== lstatSync(archive).size ||
        (publish && (report.cli_version !== `synveda ${version}` || report.source_tree_dirty !== false))) {
      throw new Error(`native client identity, pin or archive evidence mismatch: ${target}`);
    }
    for (const check of ["native-identity-and-client-only-inventory", "restricted-path-install-cli-and-three-hook-launches",
      "private-install-without-harness-or-credential-mutation", "repeat-install-preserves-deployment-state",
      "codex-extracted-lifecycle-replay", "copilot-cli-extracted-lifecycle-replay"]) {
      if (!report.checks?.includes(check)) throw new Error(`missing native client check ${check}: ${target}`);
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const [directory, version, source, publish] = process.argv.slice(2);
  if (process.argv.length !== 6 || !["true", "false"].includes(publish)) throw new Error("usage: check-client-release.mjs DIRECTORY VERSION SOURCE_SHA true|false");
  const lock = JSON.parse(readFileSync(new URL("./node-runtimes.json", import.meta.url)));
  checkClientRelease(directory, version, source, publish === "true", lock);
  console.log("four native client archives and their execution reports agree");
}
