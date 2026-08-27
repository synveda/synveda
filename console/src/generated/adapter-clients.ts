// Generated from adapters/registry.json by scripts/check-adapter-conformance.mjs.
// Do not edit by hand: support claims and connection choices share one authority.

export const GENERATED_AGENT_CLIENTS = [
  {
    "id": "claude-code",
    "label": "Claude Code",
    "via": "plugin",
    "supportLevel": "verified",
    "note": "verified: Stop and PreCompact cross only the atomic local-spool boundary synchronously; SessionEnd or the next SessionStart delivers them."
  },
  {
    "id": "cursor",
    "label": "Cursor",
    "via": "mcp",
    "supportLevel": "experimental",
    "note": "experimental: No Cursor executable or authenticated client was available on 2026-08-25."
  },
  {
    "id": "vscode",
    "label": "Visual Studio Code",
    "via": "mcp",
    "supportLevel": "configured",
    "note": "configured: The documented Preview contract has no SessionEnd event; Stop explicitly does not mean the session became inactive."
  },
  {
    "id": "claude-desktop",
    "label": "Claude Desktop",
    "via": "mcp",
    "supportLevel": "captured",
    "note": "captured: Authentic discovery and tool-call frames are replayed, but MCP alone does not prove session capture or end semantics."
  },
  {
    "id": "zed",
    "label": "Zed",
    "via": "mcp",
    "supportLevel": "captured",
    "note": "captured: Authentic non-Anthropic tool frames are replayed, but no session lifecycle/capture contract is available."
  },
  {
    "id": "windsurf",
    "label": "Windsurf",
    "via": "mcp",
    "supportLevel": "configured",
    "note": "configured: Documented config shape only; no authentic exchange or lifecycle run is claimed."
  },
  {
    "id": "continue",
    "label": "Continue",
    "via": "mcp",
    "supportLevel": "configured",
    "note": "configured: Documented legacy JSON config shape only; use --print for YAML-only installations. No authentic run is claimed."
  }
] as const;

export type GeneratedAgentClient = (typeof GENERATED_AGENT_CLIENTS)[number];
