# ADR-0116: Native consumer commands retain existing authorities

- **Status**: Accepted
- **Date**: 2026-09-21
- **Feature(s)**: OPS-12
- **Deciders**: Synveda maintainers

## Context

The plain-Compose candidate has local lifecycle and paired recovery evidence.
Consumers still need separate deployment, public-API and harness commands.
Combining these must not silently adopt a retained deployment, grant authority,
enable observation across repositories or remove someone else's configuration.

## Decision

Add native `up`, `down`, `status`, `logs` and `doctor` commands over the extracted
consumer graph. Select a bundle explicitly or from the installed reference.
Keep Docker operator-owned, use fixed Compose arguments and a closed child
environment, verify a local Linux engine, and bind a private receipt to the
engine, project, canonical bundle path and bundle bytes before the first start.
Without a receipt, refuse retained project resources. Shutdown retains volumes.
Changed bytes require operator investigation; this is not an upgrade mechanism.
The native commands require an extracted bundle and do not operate the host-state
launcher. Compose pulls its declared images when absent.

`setup` lists accessible projects or verifies one explicit active project and its
workspace through the existing authenticated public APIs. It writes only the
project's workspace/project selection and an explicit observation choice to the
existing adapter configuration. The default is observation off. Managed hooks
require a matching private setup receipt, project/workspace and credential
profile before recording; copied repository configuration cannot grant consent.
Hooks resolve the same Git root when launched from a subdirectory. Creation remains
in the console's public-API onboarding flow. Gateway credentials remain in the
existing CLI profile store, and setup never puts a token or gateway override in
a repository. The selected profile must also be selected in the harness environment.

`adapter install|status|uninstall` wraps the existing Claude native installer
and registry-backed MCP installers. Claude registration is project/local only
in this increment: user-wide registration cannot express project-only consent.
Setup must precede this route. Registration and observation consent are separate;
normal vendor trust remains required. Clients without an existing installer
retain their documented manual setup, including Codex and Copilot hooks.
The packaged Claude runtime carries a versioned contract and module hash for
receipt-aware observation. Managed registration refuses an older plugin bundle;
the marker is compatibility evidence, not a signature or native load evidence.

A shared bounded OS lock serializes native setup, lifecycle and registration
mutations, including the older plugin/MCP entry points. Private receipts contain
only selection and ownership evidence, never tokens or transcript content.
Receipt intent precedes mutation so an interrupted install can be reconciled.
File edits use exclusive temporary files, atomic replacement and a final
comparison with the bytes read. Removal requires the receipt's exact current
entry, preserves unrelated entries, and refuses drift. An existing registration
without a receipt is not silently adopted. Receipts establish local ownership,
not gateway authority, vendor trust, loaded hooks or live conformance.

## Options considered

1. **Thin routes over existing boundaries** — chosen; keeps one Compose graph,
   credential authority and installer implementation.
2. **A second supervisor or installer** — duplicates lifecycle and ownership.
3. **Automatically configure every harness and enable observation** — invents
   unqualified vendor contracts and consent beyond the selected repository.

## Consequences

- Repeatable local commands can be tested through the built CLI and extracted
  artifacts without touching a user's deployment or harness configuration.
- Published v0.4.0 lacks the candidate graph and these commands. Publication,
  client-only artifacts, private Node runtimes and native platform qualification
  remain separate OPS-12 increments.
- Direct Compose/vendor commands do not participate in the CLI lock. Operators
  must not run those mutations concurrently. Files changed by other software
  are refused when detected, not forcibly repaired.
- Revisit registration scope only when runtime consent can be enforced for every
  repository reached by a user-wide hook installation.

## Compliance notes

Setup uses the public API and existing profile-bound credential refresh. Cedar,
forced RLS, VedaFlow and server audit retain all product authority. Local receipts
are not audit-chain evidence. Normal shutdown and adapter removal preserve keys,
databases, credentials, spools, shared marketplaces and persistent plugin data.
