#!/usr/bin/env node
// OPS-12: one native candidate path shared by CI and the existing release job.
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { sha256, targetName } from "./client-artifact.mjs";

const [binary, version, target, output, source] = process.argv.slice(2);
if (!source || process.platform !== "win32" || target !== targetName()) throw new Error("usage: native Windows candidate CLI VERSION TARGET OUTPUT SOURCE_SHA");
const root = resolve(import.meta.dirname, "..");
const lock = JSON.parse(readFileSync(join(root, "scripts/node-runtimes.json")));
const pin = lock.targets[target];
if (!pin || pin.platform !== "win32" || pin.arch !== process.arch) throw new Error("private Node target is not native");
mkdirSync(output, { recursive: true });
const runtime = join(resolve(output), `.private-node-${randomUUID()}.zip`);
try {
  const response = await fetch(`https://nodejs.org/dist/v${lock.version}/${pin.archive}`, { signal: AbortSignal.timeout(120000) });
  if (!response.ok) throw new Error("private Node download failed");
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > 128 * 1024 * 1024) throw new Error("private Node download exceeds its size bound");
    chunks.push(chunk);
  }
  const bytes = Buffer.concat(chunks);
  if (sha256(bytes) !== pin.sha256) throw new Error("private Node checksum mismatch");
  writeFileSync(runtime, bytes, { flag: "wx" });
  for (const args of [
    ["scripts/package-client.mjs", version, target, resolve(binary), runtime, resolve(output), source],
    ["scripts/check-windows-client-package.mjs", version, join(resolve(output), `synveda-client-${version}-${target}.zip`),
      join(resolve(output), `synveda-client-report-${target}.json`)],
  ]) execFileSync(process.execPath, args, { cwd: root, timeout: 300000, stdio: "inherit" });
} finally { rmSync(runtime, { force: true }); }
