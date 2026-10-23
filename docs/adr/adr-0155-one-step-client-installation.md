# ADR-0155: One-step client installation and managed Codex/Copilot registration

- **Status**: Accepted
- **Date**: 2026-10-23
- **Feature(s)**: Consumer installation and harness setup
- **Deciders**: Synveda maintainers

## Context

Connecting an agent took a reviewed release commit pasted into the installer,
a manual PATH edit, and, for Codex and Copilot CLI, hand-written hook JSON with
absolute paths to a private Node runtime and hook script, a separate vendor
`mcp add`, a user-level `SYNVEDA_CLI` on Windows and knowledge of each vendor's
Git-root and trust rules. Each step was correct on its own; together they made
the most common task, "connect my agent", the hardest one to follow. Claude
Code already had a managed route (`synveda adapter install`, ADR-0116).

ADR-0132 required the expected source commit for every remote installation so
that publisher verification bound the exact reviewed bytes.

## Decision

1. **The installers default to the latest client release and verify the
   publisher without requiring a commit.** `install.sh` defaults to client
   mode; both installers resolve the latest release when no version is given.
   `gh attestation verify` always pins the repository, the release workflow,
   the tag ref, the OIDC issuer, the SLSA predicate and hosted runners; a
   supplied source commit additionally pins `--source-digest`. This amends
   ADR-0132: the tag and workflow identity, not a hand-copied commit, is the
   default trust anchor. Checksums still bind every archive before extraction.
2. **The installers make the CLI runnable.** The shell installer appends one
   PATH line to the login shell's profile; the PowerShell installer adds the
   launcher directory to the user `Path` and sets the user `SYNVEDA_CLI` that
   hooks need. `SYNVEDA_NO_MODIFY_PATH=1` / `-NoModifyEnvironment` opt out.
   Neither installer configures an agent.
3. **`synveda adapter install --client codex|copilot-cli` registers hooks and
   MCP.** It runs after `synveda setup`, at the Git repository root, and:
   - writes hook entries that run the installed release's private Node and
     hook, passing the CLI path in the hook environment (Copilot `env`; a
     POSIX environment prefix for Codex on Unix). Copilot gets an owned
     `.github/hooks/synveda.json`; Codex entries are merged into
     `.codex/hooks.json` and identified by the Synveda hook path, so other
     hooks survive install and uninstall;
   - runs the vendor's own `mcp add synveda -- <cli> mcp --writes host`, keeping
     the vendor's configuration format owned by the vendor, with a 30-second
     bound. A failure leaves the hooks in place and prints the exact command;
   - records an ownership receipt (hook hash and whether it added the MCP
     entry), refuses unowned or edited entries, and refreshes its own unchanged
     entries in place so an upgraded client can update paths.
4. Claude Code, Codex and Copilot default to `--scope project`; MCP clients
   default to `--scope user`.

## Options considered

1. **Publish npm packages or a vendor marketplace per agent** — familiar to
   users, but the hooks still need the native `synveda` CLI for credentials and
   private state, so a second distribution channel would add a release and
   trust path without removing the CLI install.
2. **Write Codex TOML and Copilot JSON MCP configuration directly** — testable
   without vendor binaries, but needs a TOML editor dependency and duplicates
   formats the vendors own and change.
3. **Keep the documented manual recipes** — no code, but the setup stays the
   main adoption barrier and the recipes drift from the qualified layout.

## Consequences

- Positive: four commands connect any of the three qualified agents, on Unix
  and Windows, with the same receipt-owned install, status and uninstall.
- Negative / accepted trade-offs: default installs trust the release
  workflow's signature for a tag rather than an independently reviewed commit;
  the installer edits a shell profile or user environment unless told not to;
  MCP registration depends on the vendor CLI being on PATH; Windows Codex hook
  command execution remains unqualified.
- Reversal trigger: a vendor changes `mcp add` semantics or hook loading so the
  managed route no longer matches the qualified layout, or release signing is
  found insufficient to bind tags to reviewed sources.

## Compliance notes

No change to PDP, RLS or audit paths. Hooks and MCP use the same public API and
credentials as before; registration grants no access and is not live-client
verification.
