#!/usr/bin/env node
// OPS-12: package the existing CLI and adapters with a checksum-pinned runtime.
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { cliPath, inventory, nodePath, sha256, targetName, validateClient } from "./client-artifact.mjs";

const root = resolve(import.meta.dirname, "..");
const windows = process.platform === "win32";
const [version, target, binary, runtimeArchive, output, sourceSha] = process.argv.slice(2);
if (!sourceSha || process.argv.length !== 8) throw new Error("usage: package-client.mjs VERSION TARGET CLI NODE_ARCHIVE OUTPUT SOURCE_SHA");
execFileSync("sh", [join(root, "scripts/release-version.sh").replaceAll("\\", "/"), version]);
if (!/^[0-9a-f]{40}$/.test(sourceSha)) throw new Error("invalid source SHA");
const lock = JSON.parse(readFileSync(join(root, "scripts/node-runtimes.json")));
const pin = lock.targets[target];
if (!pin || target !== targetName()) throw new Error("client packaging requires its native host target");
const dirty = execFileSync("git", ["status", "--porcelain", "--untracked-files=normal", "--", "scripts", "crates", "adapters", "docs", ".github", "Cargo.toml", "Cargo.lock"],
  { cwd: root, encoding: "utf8" }).trim().length > 0;
for (const file of [binary, runtimeArchive]) {
  const info = lstatSync(file);
  if (!info.isFile() || info.nlink !== 1 || info.size > 256 * 1024 * 1024) throw new Error("package inputs must be bounded regular unlinked files");
}
if (sha256(readFileSync(runtimeArchive)) !== pin.sha256) throw new Error("private Node archive checksum mismatch");
mkdirSync(output, { recursive: true });
const scratch = mkdtempSync(join(resolve(output), ".client-package-"));
const client = join(scratch, "client");
const run = (command, args, extra = {}) => execFileSync(command, args, { encoding: "utf8", timeout: 120_000,
  maxBuffer: 8 * 1024 * 1024, env: { ...process.env, NODE_OPTIONS: "", NODE_PATH: "" }, ...extra });
try {
  mkdirSync(client);
  run("bash", [join(root, "scripts/package-plugin.sh").replaceAll("\\", "/"), version, scratch.replaceAll("\\", "/")]);
  run("tar", ["-xzf", join(scratch, `synveda-plugin-${version}.tar.gz`), "-C", client]);
  // Claude's source package also carries test outputs; consumers need only the
  // existing executable module closure. Codex/Copilot are already closed lists.
  const dist = join(client, "plugin/synveda/dist");
  for (const file of readdirSync(dist)) {
    if (!file.endsWith(".mjs") || file.includes(".test.") || file === "mock-gateway.mjs") rmSync(join(dist, file));
  }
  const upstreamRoot = pin.archive.replace(/\.(?:tar\.gz|zip)$/, "");
  const upstreamNode = windows ? "node.exe" : "bin/node";
  run("tar", ["-xf", resolve(runtimeArchive), "-C", scratch, `${upstreamRoot}/${upstreamNode}`, `${upstreamRoot}/LICENSE`]);
  const runtime = join(client, "plugin/synveda/runtime");
  mkdirSync(runtime);
  const runtimeNode = windows ? "node.exe" : "node";
  copyFileSync(join(scratch, upstreamRoot, upstreamNode), join(runtime, runtimeNode));
  copyFileSync(join(scratch, upstreamRoot, "LICENSE"), join(runtime, "LICENSE"));
  if (!windows) chmodSync(join(runtime, runtimeNode), 0o755);
  if (run(join(runtime, runtimeNode), ["--version"]).trim() !== `v${lock.version}`) throw new Error("private Node version mismatch");
  for (const relative of ["plugin/synveda/hooks/hooks.json", "plugin/synveda/.mcp.json"]) {
    const path = join(client, relative);
    const manifest = JSON.parse(readFileSync(path));
    function rewrite(value) {
      if (value === null || typeof value !== "object") return;
      if (value.command === "node") value.command = `\${CLAUDE_PLUGIN_ROOT}/runtime/${runtimeNode}`;
      for (const child of Object.values(value)) rewrite(child);
    }
    rewrite(manifest);
    writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`);
  }
  mkdirSync(join(client, "bin"));
  copyFileSync(binary, join(client, cliPath));
  if (!windows) chmodSync(join(client, cliPath), 0o755);
  const cliVersion = run(join(client, cliPath), ["--version"]).trim();
  for (const file of ["LICENSE", "NOTICE"]) copyFileSync(join(root, file), join(client, file));
  mkdirSync(join(client, "lib"));
  for (const file of ["client-install.mjs", "client-artifact.mjs"]) copyFileSync(join(root, "scripts", file), join(client, "lib", file));
  writeFileSync(join(client, "client.json"), `${JSON.stringify({ schema_version: 1, version, source_sha: sourceSha,
    target, source_tree_dirty: dirty, cli_version: cliVersion, node: { version: lock.version, archive: pin.archive, archive_sha256: pin.sha256 },
    files: inventory(client) }, null, 2)}\n`);
  validateClient(client);
  for (const adapter of ["synveda", "codex", "copilot-cli"]) {
    run(join(client, nodePath), [join(client, "plugin", adapter, "dist/hook.mjs")], { cwd: scratch, input: "{}",
      env: { PATH: process.env.PATH, HOME: join(scratch, "home"), XDG_CONFIG_HOME: join(scratch, "config"),
        XDG_STATE_HOME: join(scratch, "state"), SYNVEDA_DISABLED: "1" } });
  }
  const archive = join(resolve(output), `synveda-client-${version}-${target}.${windows ? "zip" : "tar.gz"}`);
  run("tar", windows ? ["-a", "-cf", archive, "-C", scratch, "client"] : ["-czf", archive, "-C", scratch, "client"]);
  console.log(JSON.stringify({ archive, target, node: lock.version, cli_version: cliVersion, sha256: sha256(readFileSync(archive)) }));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
