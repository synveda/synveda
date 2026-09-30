// OPS-12 / ADR-0133: execute only a reviewed native scanner, outside the client.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chmodSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { sha256, targetName } from "./client-artifact.mjs";

export const syftPins = JSON.parse(readFileSync(new URL("./syft-runtimes.json", import.meta.url)));

export async function downloadSyft(target, output) {
  assert.equal(target, targetName(), "scanner must match the native host");
  const pin = syftPins.targets[target];
  assert.ok(pin, "unreviewed scanner target");
  const response = await fetch(`https://github.com/anchore/syft/releases/download/v${syftPins.version}/${pin.archive}`,
    { signal: AbortSignal.timeout(120_000) });
  assert.ok(response.ok, "native Syft download failed");
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    assert.ok(size <= 128 * 1024 * 1024, "native Syft download exceeds its bound");
    chunks.push(chunk);
  }
  const bytes = Buffer.concat(chunks);
  assert.equal(bytes.length, pin.bytes, "native Syft archive size differs");
  assert.equal(sha256(bytes), pin.sha256, "native Syft archive checksum differs");
  // Exclusive creation prevents overwriting tools from another invocation.
  mkdirSync(output, { mode: 0o700 });
  const archive = join(output, pin.archive);
  const member = target.startsWith("windows-") ? "syft.exe" : "syft";
  try {
    writeFileSync(archive, bytes, { flag: "wx", mode: 0o600 });
    const options = { timeout: 120_000, stdio: ["ignore", "pipe", "pipe"] };
    const listing = execFileSync("tar", ["-tvf", archive, member], { ...options, encoding: "utf8", maxBuffer: 32 * 1024 }).trim().split("\n");
    assert.ok(listing.length === 1 && listing[0].startsWith("-"), "native Syft must be one regular archive member");
    const binary = execFileSync("tar", ["-xOf", archive, member], { ...options, maxBuffer: 256 * 1024 * 1024 });
    assert.equal(binary.length, pin.binary_bytes, "native Syft executable size differs");
    assert.equal(sha256(binary), pin.binary_sha256, "native Syft executable checksum differs");
    const path = join(output, member);
    writeFileSync(path, binary, { flag: "wx", mode: 0o700 });
    if (process.platform !== "win32") chmodSync(path, 0o700);
    return path;
  } finally {
    rmSync(archive, { force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  assert.equal(process.argv.length, 4, "usage: download-syft.mjs NATIVE_TARGET OUTPUT_DIRECTORY");
  console.log(await downloadSyft(...process.argv.slice(2)));
}
