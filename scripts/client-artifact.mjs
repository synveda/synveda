// OPS-12: shared closed archive inventory, used before installation and switching.
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

export const nodePath = `plugin/synveda/runtime/node${process.platform === "win32" ? ".exe" : ""}`;
export const cliPath = `bin/synveda${process.platform === "win32" ? ".exe" : ""}`;
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
export const targetName = () => `${process.platform === "win32" ? "windows" : process.platform}-${process.arch === "x64" ? "x86_64" : process.arch}`;

export function inventory(root) {
  const files = Object.create(null);
  let total = 0;
  let entries = 0;
  function walk(relative) {
    if (++entries > 1024 || relative.length > 512) throw new Error("client archive exceeds its entry/path bound");
    const path = join(root, relative);
    const stat = lstatSync(path);
    if (stat.isDirectory()) {
      for (const name of readdirSync(path).sort()) {
        if (!/^[a-zA-Z0-9@_.-]+$/.test(name) || name === "." || name === "..") throw new Error("unsafe client archive path");
        walk(relative ? `${relative}/${name}` : name);
      }
    } else {
      if (!stat.isFile() || stat.nlink !== 1) throw new Error("client archive contains a link or special file");
      total += stat.size;
      if (total > 512 * 1024 * 1024 || Object.keys(files).length >= 512) throw new Error("client archive exceeds its size/file bound");
      if (relative !== "client.json") files[relative] = { sha256: sha256(readFileSync(path)), executable: (stat.mode & 0o111) !== 0 };
    }
  }
  walk("");
  return files;
}

export function validateClient(root, expectedTarget = targetName()) {
  const manifestPath = join(root, "client.json");
  const stat = lstatSync(manifestPath);
  if (!stat.isFile() || stat.nlink !== 1 || stat.size > 256 * 1024) throw new Error("invalid client manifest file");
  const bytes = readFileSync(manifestPath);
  const manifest = JSON.parse(bytes);
  if (manifest.schema_version !== 1 || manifest.target !== expectedTarget ||
      !/^[0-9a-f]{40}$/.test(manifest.source_sha) ||
      !/^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(manifest.version) ||
      typeof manifest.cli_version !== "string" || !/^synveda [0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(manifest.cli_version) ||
      !/^24\.[0-9]+\.[0-9]+$/.test(manifest.node?.version) ||
      !/^[0-9a-f]{64}$/.test(manifest.node?.archive_sha256)) throw new Error("client identity or native target mismatch");
  const files = inventory(root);
  if (JSON.stringify(files) !== JSON.stringify(manifest.files)) throw new Error("client content inventory mismatch");
  for (const path of [cliPath, nodePath, "LICENSE", "NOTICE", "plugin/synveda/runtime/LICENSE",
    "lib/client-install.mjs", "lib/client-artifact.mjs", "plugin/synveda/consumer-setup.json",
    "plugin/synveda/dist/hook.mjs", "plugin/synveda/dist/mcp-server.mjs",
    "plugin/.claude-plugin/marketplace.json", "plugin/synveda/.mcp.json", "plugin/synveda/hooks/hooks.json",
    "plugin/codex/dist/hook.mjs", "plugin/copilot-cli/dist/hook.mjs"]) {
    if (!files[path]) throw new Error(`client archive lacks ${path}`);
  }
  if (process.platform !== "win32" && (!files[nodePath].executable || !files[cliPath].executable)) throw new Error("client executables lack execute permission");
  const command = `\${CLAUDE_PLUGIN_ROOT}/runtime/node${process.platform === "win32" ? ".exe" : ""}`;
  const mcp = JSON.parse(readFileSync(join(root, "plugin/synveda/.mcp.json")));
  const hooks = JSON.parse(readFileSync(join(root, "plugin/synveda/hooks/hooks.json")));
  if (mcp.synveda?.command !== command ||
      Object.values(hooks.hooks).flatMap((event) => event.flatMap((group) => group.hooks))
        .some((hook) => hook.command !== command)) throw new Error("Claude launchers must use private Node");
  return { manifest, digest: sha256(bytes) };
}
