#!/usr/bin/env node
// OPS-12: download exactly one reviewed private runtime, without a package manager.
import { writeFileSync, readFileSync } from "node:fs";
import { sha256 } from "./client-artifact.mjs";
const [target, output] = process.argv.slice(2);
const lock = JSON.parse(
  readFileSync(new URL("./node-runtimes.json", import.meta.url)),
);
const pin = lock.targets[target];
if (!pin || !output)
  throw new Error("usage: download-client-node.mjs TARGET OUTPUT");
const response = await fetch(
  `https://nodejs.org/dist/v${lock.version}/${pin.archive}`,
  { signal: AbortSignal.timeout(120000) },
);
if (!response.ok) throw new Error("private Node download failed");
const chunks = [];
let size = 0;
for await (const chunk of response.body) {
  size += chunk.length;
  if (size > 128 * 1024 * 1024)
    throw new Error("private Node download exceeds its size bound");
  chunks.push(chunk);
}
const bytes = Buffer.concat(chunks);
if (sha256(bytes) !== pin.sha256)
  throw new Error("private Node checksum mismatch");
writeFileSync(output, bytes, { flag: "wx" });
