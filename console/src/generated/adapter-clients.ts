// Generated from adapters/registry.json by scripts/check-adapter-conformance.mjs.
// Do not edit by hand: support claims and connection choices share one authority.

export const GENERATED_AGENT_CLIENTS = [
  {
    "id": "claude-code",
    "label": "Claude Code",
    "via": "plugin",
    "supportLevel": "verified",
    "note": "Automatic context on start/resume, durable Session observations and governed Skill sync.",
    "registration": "automatic",
    "testedVersions": [
      "2.1.220",
      "2.1.241"
    ],
    "limits": "Claude Code 2.1.241 has named lifecycle evidence. Published-package loading and broader platforms need separate qualification.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/CONSUMER_CLI.md"
  },
  {
    "id": "copilot-cli",
    "label": "GitHub Copilot CLI",
    "via": "mcp",
    "supportLevel": "verified",
    "note": "MCP recall and start/resume context; optional trusted hooks record Session observations.",
    "registration": "manual",
    "testedVersions": [
      "1.0.83"
    ],
    "limits": "Copilot CLI 1.0.83 on macOS arm64 has source-build lifecycle evidence. Native outage/compaction and other platforms remain unqualified.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/integrations/copilot-cli.md"
  },
  {
    "id": "cursor",
    "label": "Cursor",
    "via": "mcp",
    "supportLevel": "experimental",
    "note": "MCP scoped Knowledge recall and explicit Session context. Automatic observation requires a separately tested hook contract.",
    "registration": "automatic",
    "testedVersions": [],
    "limits": "Experimental recipe; a complete native lifecycle is not qualified. See tested versions and platform evidence.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md"
  },
  {
    "id": "vscode",
    "label": "Visual Studio Code",
    "via": "mcp",
    "supportLevel": "configured",
    "note": "MCP scoped Knowledge recall and explicit Session context. Automatic observation requires a separately tested hook contract.",
    "registration": "automatic",
    "testedVersions": [],
    "limits": "Configured recipe; a complete native lifecycle is not qualified. See tested versions and platform evidence.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md"
  },
  {
    "id": "codex",
    "label": "Codex CLI",
    "via": "mcp",
    "supportLevel": "verified",
    "note": "Task-bound MCP recall/context; optional trusted hooks record observations and reinject context after compaction.",
    "registration": "manual",
    "testedVersions": [
      "0.152.0"
    ],
    "limits": "Codex CLI 0.152.0 on macOS arm64 has source-build lifecycle evidence. Other platforms and published-package native runs remain unqualified.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/integrations/codex.md"
  },
  {
    "id": "claude-desktop",
    "label": "Claude Desktop",
    "via": "mcp",
    "supportLevel": "captured",
    "note": "MCP scoped Knowledge recall and explicit Session context. Automatic observation requires a separately tested hook contract.",
    "registration": "automatic",
    "testedVersions": [
      "1.25927.0"
    ],
    "limits": "Captured recipe; a complete native lifecycle is not qualified. See tested versions and platform evidence.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md"
  },
  {
    "id": "zed",
    "label": "Zed",
    "via": "mcp",
    "supportLevel": "captured",
    "note": "MCP scoped Knowledge recall and explicit Session context. Automatic observation requires a separately tested hook contract.",
    "registration": "automatic",
    "testedVersions": [
      "1.13.2"
    ],
    "limits": "Captured recipe; a complete native lifecycle is not qualified. See tested versions and platform evidence.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md"
  },
  {
    "id": "windsurf",
    "label": "Windsurf",
    "via": "mcp",
    "supportLevel": "configured",
    "note": "MCP scoped Knowledge recall and explicit Session context. Automatic observation requires a separately tested hook contract.",
    "registration": "automatic",
    "testedVersions": [],
    "limits": "Configured recipe; a complete native lifecycle is not qualified. See tested versions and platform evidence.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md"
  },
  {
    "id": "continue",
    "label": "Continue",
    "via": "mcp",
    "supportLevel": "configured",
    "note": "MCP scoped Knowledge recall and explicit Session context. Automatic observation requires a separately tested hook contract.",
    "registration": "automatic",
    "testedVersions": [],
    "limits": "Configured recipe; a complete native lifecycle is not qualified. See tested versions and platform evidence.",
    "guide": "https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md"
  }
] as const;

export type GeneratedAgentClient = (typeof GENERATED_AGENT_CLIENTS)[number];
