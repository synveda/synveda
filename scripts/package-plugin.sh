#!/usr/bin/env bash
# Assembles the Claude marketplace and Codex/Copilot hook runtimes (OPS-8,
# ADPT-9, ADR-0065 amendments 2, 9 and 10). Client configuration is separate.
#
# Usage: scripts/package-plugin.sh <version> <output-dir>
#
# Produces <output-dir>/synveda-plugin-<version>.tar.gz containing:
#
#   plugin/.claude-plugin/marketplace.json    the marketplace, one plugin in it
#   plugin/synveda/.claude-plugin/plugin.json the plugin manifest, version pinned
#   plugin/synveda/.mcp.json                  the MCP server, auto-discovered
#   plugin/synveda/hooks/hooks.json           the four seams, auto-discovered
#   plugin/synveda/dist/                      the prebuilt, dependency-free JS
#   plugin/codex/                            the private, self-contained hook runtime
#   plugin/copilot-cli/                      the private, self-contained hook runtime
#
# A **marketplace** rather than a bare plugin directory because that is the
# unit Claude Code installs: `claude plugin marketplace add <path>` then
# `claude plugin install synveda@synveda`. Dropping a plugin into
# `~/.claude/plugins/synveda/` — which is what this repository's docs said
# to do, and what the ADPT-1 demo does — installs nothing, because that is
# not a location Claude Code reads.
#
# `dist/` is gitignored, so the caller builds it first:
#   pnpm --filter @synveda/codex-adapter... --filter @synveda/copilot-cli-adapter... build
#
# It writes nothing outside <output-dir>.
set -euo pipefail

cd "$(dirname "$0")/.."

version="${1:?usage: package-plugin.sh <version> <output-dir>}"
outdir="${2:?usage: package-plugin.sh <version> <output-dir>}"

# Validate before the version can become an archive path or manifest value.
sh scripts/release-version.sh "$version"

adapter="adapters/claude-code"
[ -f "$adapter/dist/hook.mjs" ] || {
  echo "package-plugin: $adapter/dist is not built." >&2
  echo "  pnpm --filter @synveda/claude-code-adapter build" >&2
  exit 1
}
[ -f "$adapter/dist/mcp-server.mjs" ] || {
  echo "package-plugin: $adapter/dist/mcp-server.mjs is missing —" >&2
  echo "  .mcp.json names it, so the plugin would install and its MCP" >&2
  echo "  server would fail at spawn." >&2
  exit 1
}

# Keep the existing shared runtime's artifact closure explicit. Extracted
# lifecycle tests catch missing modules; no workspace symlink reaches users.
shared_modules="client config credentials deliver events install-id log paths private-state session-runtime session-start spool transcript turn"
for module in $shared_modules; do
  [ -f "$adapter/dist/$module.mjs" ] && [ ! -L "$adapter/dist/$module.mjs" ] || {
    echo "package-plugin: shared runtime module $module is not built as a regular file" >&2
    exit 1
  }
done
for client in codex copilot-cli; do
  for module in hook transcript; do
    [ -f "adapters/$client/dist/$module.mjs" ] && [ ! -L "adapters/$client/dist/$module.mjs" ] || {
      echo "package-plugin: build the adapters with pnpm --filter @synveda/codex-adapter... --filter @synveda/copilot-cli-adapter... build" >&2
      exit 1
    }
  done
done

stage="$outdir/plugin"
rm -rf "$stage"
mkdir -p "$stage/.claude-plugin" "$stage/synveda"

cp "$adapter/marketplace.json" "$stage/.claude-plugin/marketplace.json"
cp -R "$adapter/.claude-plugin" "$stage/synveda/.claude-plugin"
cp "$adapter/.mcp.json" "$stage/synveda/.mcp.json"
cp -R "$adapter/hooks" "$stage/synveda/hooks"
cp -R "$adapter/dist" "$stage/synveda/dist"

# Native managed registration requires the receipt-aware observation runtime.
# Hash the shipped module so a historical same-version bundle is refused.
node --input-type=module - "$stage/synveda" <<'JS'
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
const root = process.argv[2];
const config = readFileSync(`${root}/dist/config.mjs`);
if (!config.includes(Buffer.from("managed_observation"))) throw new Error("rebuild the receipt-aware adapter before packaging");
writeFileSync(`${root}/consumer-setup.json`, JSON.stringify({ version: 1,
  contract: "OPS-12/ADR-0116", config_sha256: createHash("sha256").update(config).digest("hex") }) + "\n");
JS

for client in codex copilot-cli; do
  runtime="$stage/$client"
  shared="$runtime/node_modules/@synveda/claude-code-adapter"
  mkdir -p "$runtime/dist" "$shared/dist"
  for module in hook transcript; do
    cp "adapters/$client/dist/$module.mjs" "$runtime/dist/$module.mjs"
  done
  for module in $shared_modules; do
    cp "$adapter/dist/$module.mjs" "$shared/dist/$module.mjs"
  done
  node -e '
    const fs = require("node:fs");
    const [version, client, runtime, shared] = process.argv.slice(1);
    for (const [source, destination] of [[`adapters/${client}`, runtime], ["adapters/claude-code", shared]]) {
      const sourceManifest = JSON.parse(fs.readFileSync(`${source}/package.json`, "utf8"));
      const manifest = { name: sourceManifest.name, version, private: true, type: sourceManifest.type };
      if (sourceManifest.exports) manifest.exports = sourceManifest.exports;
      if (sourceManifest.dependencies) manifest.dependencies = { "@synveda/claude-code-adapter": version };
      fs.writeFileSync(`${destination}/package.json`, JSON.stringify(manifest, null, 2) + "\n");
    }
  ' "$version" "$client" "$runtime" "$shared"
done

# The plugin's version is the release's. `synveda plugin install` reports
# what it installed and `claude plugin list` shows it, so a plugin claiming
# a version the adjacent release artifact does not have is the same confusion
# ADR-0065 decision 5 refuses.
manifest="$stage/synveda/.claude-plugin/plugin.json"
node -e '
  const fs = require("node:fs");
  const [path, version] = process.argv.slice(1);
  const manifest = JSON.parse(fs.readFileSync(path, "utf8"));
  manifest.version = version;
  // Two keys that must NOT be here, both learned by installing the plugin
  // rather than by reading anything — and they fail differently, which is
  // why the manifest carried both for so long:
  //
  //   hooks       naming `./hooks/hooks.json` is a *duplicate load*. The
  //               standard path is read automatically, so declaring it too
  //               leaves the plugin "✘ failed to load" — everything else
  //               about the install looking perfectly healthy.
  //   mcpServers  silently *ignored*. The server map belongs in `.mcp.json`
  //               at the plugin root; an inline `mcpServers` registers
  //               nothing and reports nothing, which is why the plugin
  //               advertised an MCP server it never had.
  for (const key of ["hooks", "mcpServers"]) {
    if (key in manifest) {
      console.error(`package-plugin: plugin.json declares "${key}", which Claude Code discovers on its own.`);
      console.error(key === "hooks"
        ? "  Declaring it double-loads the file and the plugin fails to load. Remove it."
        : "  It is ignored — the server map belongs in .mcp.json at the plugin root. Remove it.");
      process.exit(1);
    }
  }
  fs.writeFileSync(path, JSON.stringify(manifest, null, 2) + "\n");
' "$manifest" "$version"

tar -czf "$outdir/synveda-plugin-$version.tar.gz" -C "$outdir" plugin
rm -rf "$stage"
echo "packaged $outdir/synveda-plugin-$version.tar.gz"
