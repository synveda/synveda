#!/usr/bin/env bash
# Assembles the Claude marketplace and Codex hook runtime (OPS-8,
# ADR-0065 amendments 2 and 9). Client configuration is a separate step.
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
#
# A **marketplace** rather than a bare plugin directory because that is the
# unit Claude Code installs: `claude plugin marketplace add <path>` then
# `claude plugin install synveda@synveda`. Dropping a plugin into
# `~/.claude/plugins/synveda/` — which is what this repository's docs said
# to do, and what the ADPT-1 demo does — installs nothing, because that is
# not a location Claude Code reads.
#
# `dist/` is gitignored, so the caller builds it first:
#   pnpm --filter @synveda/codex-adapter... build
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
shared_modules="client config credentials deliver events install-id log paths session-runtime session-start spool transcript turn"
for module in $shared_modules; do
  [ -f "$adapter/dist/$module.mjs" ] && [ ! -L "$adapter/dist/$module.mjs" ] || {
    echo "package-plugin: shared runtime module $module is not built as a regular file" >&2
    exit 1
  }
done
for module in hook transcript; do
  [ -f "adapters/codex/dist/$module.mjs" ] && [ ! -L "adapters/codex/dist/$module.mjs" ] || {
    echo "package-plugin: build both adapters with pnpm --filter @synveda/codex-adapter... build" >&2
    exit 1
  }
done

stage="$outdir/plugin"
rm -rf "$stage"
mkdir -p "$stage/.claude-plugin" "$stage/synveda"

cp "$adapter/marketplace.json" "$stage/.claude-plugin/marketplace.json"
cp -R "$adapter/.claude-plugin" "$stage/synveda/.claude-plugin"
cp "$adapter/.mcp.json" "$stage/synveda/.mcp.json"
cp -R "$adapter/hooks" "$stage/synveda/hooks"
cp -R "$adapter/dist" "$stage/synveda/dist"

codex="$stage/codex"
shared="$codex/node_modules/@synveda/claude-code-adapter"
mkdir -p "$codex/dist" "$shared/dist"
for module in hook transcript; do
  cp "adapters/codex/dist/$module.mjs" "$codex/dist/$module.mjs"
done
for module in $shared_modules; do
  cp "$adapter/dist/$module.mjs" "$shared/dist/$module.mjs"
done
node -e '
  const fs = require("node:fs");
  const [version, codex, shared] = process.argv.slice(1);
  for (const [source, destination] of [["adapters/codex", codex], ["adapters/claude-code", shared]]) {
    const sourceManifest = JSON.parse(fs.readFileSync(`${source}/package.json`, "utf8"));
    const manifest = { name: sourceManifest.name, version, private: true, type: sourceManifest.type };
    if (sourceManifest.exports) manifest.exports = sourceManifest.exports;
    if (sourceManifest.dependencies) manifest.dependencies = { "@synveda/claude-code-adapter": version };
    fs.writeFileSync(`${destination}/package.json`, JSON.stringify(manifest, null, 2) + "\n");
  }
' "$version" "$codex" "$shared"

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
