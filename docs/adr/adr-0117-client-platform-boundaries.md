# ADR-0117: Explicit client platform and private-state boundaries

- **Status**: Accepted; amended four times
- **Date**: 2026-09-21
- **Feature(s)**: Consumer installation and harness setup
- **Deciders**: Synveda maintainers

## Context

The Unix client candidate carries a native CLI and private Node. The CLI also
contains deployment bootstrap code with Unix descriptor/ownership checks.
Several non-Unix storage fallbacks omit those checks, and the CLI and hooks can
disagree on state paths or fall back to a repository-relative spool. Compiling
those fallbacks would not establish a private Windows client.

## Decision

### Amendment 4 (2026-10-17): native Windows registration, receipts and hook logs

Amendment 3 left five client surfaces refused on Windows: Claude plugin
registration, managed adapter registration, demo receipts, skill receipts, and
hook diagnostic logs. With all five ported, the native Windows client keeps one
refusal: the Compose deployment commands. Native Windows stays a client-only
platform, not a server.

**Vendor executables.**
- On Windows, a vendor CLI is the native `<name>.exe` found in an absolute
  `PATH` entry.
- npm `.cmd`/`.ps1` shims and extensionless scripts are named in the refusal
  and never run, because another shell would reparse their arguments.
- A relative `PATH` entry is ignored, so the working directory cannot choose
  the vendor.
- Unix lookup is unchanged.

**Paths.**
- Every path that is compared, recorded in a receipt, or passed to a vendor
  uses the conventional `C:\` spelling (`client_paths::conventional`). This
  covers the Claude `projectPath` comparison, the managed marketplace, adapter
  identities and generated commands.
- The vendor account home is `USERPROFILE` on Windows and `HOME` elsewhere. This
  covers the default `CLAUDE_CONFIG_DIR` and the `.claude`/`.codex` skill roots.

**Hook root parity.**
- Hooks resolve the repository root on win32 with `realpathSync.native`: true
  casing, no 8.3 names and no `\\?\` prefix. That is the root the CLI keys setup
  receipts by.
- The ancestor walk is bounded.

**Receipts.**
- Demo receipts are private and use the amendment 1 directory backend under
  the state root.
- Skill receipts are ordinary configuration on every platform. They use the
  amendment 3 reader and writer.
- Their directory is prepared and checked before any skill file is replaced.
  This fixes an install that wrote files and then failed on its receipt.

**Hook logs.**
- The closed `private-state` protocol gains one operation, `append_log`. It
  accepts 1–128 JSON-object lines, each at most 8 KiB with no line breaks,
  totalling at most 256 KiB.
- The CLI appends under a stable `log.lock` to a sealed `adapter.log` below the
  state root. At 4 MiB the log rotates, keeping one generation.
- A hook queues its lines and sends one request when the process exits, so
  logging spawns the CLI at most once per hook. The request times out after
  1.5 s, and a lost batch is never reported.
- Raw-frame capture stays refused on Windows; its live runner is Unix-only.

**Evidence.** Native Windows 11 x64 tests use a scripted vendor executable,
compiled from a test-only std source, and a loopback gateway. This is fixture
evidence. Live harness use is recorded separately and only from live runs.

### Amendment 3 (2026-10-16): native Windows project and vendor configuration edits

Native Windows `setup --project` and `mcp install|uninstall` stopped with
"private client state is unavailable on this platform until native ACL and
atomic replacement support is qualified". That message came from the
original Unix-only guard. It was still applied to ordinary repository and vendor
configuration edits and to the consumer-operation lock, although credentials,
receipts and spools already had amendment 1 and 2 backends.

Give ordinary configuration (`.synveda/config.json` and vendor MCP files) a
native Windows reader and writer beside the private directory backend. They are
not private state: no ancestor ACL policy applies, and the owner's DACL is kept.
As with a Unix rename, the writing account owns the replaced file afterwards.
Reads open the name without following a reparse point and refuse directories,
non-disk objects and multiply linked files. They bound the bytes, then recheck
that the name still denotes the object that was read.

A replacement creates an exclusive temporary in the same directory. Before any
byte is written, the temporary receives the destination's DACL, checked through
SDDL for protection and entries. A DACL that cannot be reproduced, or a null
DACL, refuses. A first creation is sealed private instead, matching the Unix
0600 temporary. The data is flushed and the destination bytes are compared
again. `std::fs::rename` then publishes; on the pinned toolchain this is
`MoveFileExW(MOVEFILE_REPLACE_EXISTING)` with a `FileRenameInfoEx` POSIX
fallback. A first creation uses `MoveFileExW` without replacement, so concurrent
creators cannot overwrite each other. Sharing and access refusals get at most
20 retries, 25 ms apart. A failure removes only this operation's own temporary.

`ReplaceFileW` is rejected. In native tests, concurrent readers found the name
absent between its internal renames in 216,807 of 484,663 reads. Without a
backup name, its `ERROR_UNABLE_TO_MOVE_REPLACEMENT` case also deletes the
previous file.

`setup` and `mcp install|uninstall` run natively under the existing
consumer-operation lock. These surfaces keep their refusal, which now names
them:
- Compose deployment (`up`, `down`, `status`, `logs`, `doctor`);
- Claude plugin install and uninstall;
- managed adapter registration;
- demo and skill receipts;
- Windows diagnostic logs.

Native ACL refusals now name the object, the refused owner or principal SID and
the next step. The amendment 1 ancestor policy is unchanged. So a profile whose
`AppData` lets another account write or delete entries refuses private state
until the XDG roots select admissible directories. One such account seen in
practice is a Codex Windows sandbox group. Synveda never changes existing ACLs.
Creating the first private directory with a creation-time security descriptor
would need unsafe FFI or a new wrapper, and remains a separate decision.

Rust's verbatim `\\?\C:\` canonical paths are written in conventional `C:\`
form for the setup receipt root and the generated MCP command. That is the
spelling hooks and vendor clients use.

### Amendment 2 (2026-09-21): shared Windows receipt and spool storage

Reuse the credential directory/ACL/identity implementation for private receipts
and the unchanged version-1 spool. A hidden, versioned `private-state` CLI
protocol gives Windows Node hooks bounded stdin/stdout access to spool files,
setup-receipt reads, installation identity and disclosure markers. It accepts
closed operations and leaf names below the existing config/state roots, never
arbitrary paths, credentials, shell commands or gateway requests. Payload bytes
travel as base64 on pipes, never process arguments or diagnostics. Unix hooks
keep their existing filesystem implementation.

Windows spool snapshots are limited to 16 MiB and directory scans to 4096
entries. Oversized, unsafe or unreadable state is held, never treated as absent.
Writers and removers share a stable OS lock and compare the digest of the bytes
they read before replacing or retiring them. A concurrent change refuses the
stale write or deletion and retains the newer queue. The native CLI uses this
same storage boundary for flush/purge. Exclusive random temporaries retain the
protected ACL and flushed sibling-rename behavior of amendment 1. Receipt
operations retain their separate consumer-operation lock. Existing unsafe ACLs
are not repaired. No format migration, same-account adversary guarantee or
power-loss durability claim is added.

Keep unported repository/vendor edits, deployment witnesses and diagnostic-log
writers behind their existing Windows refusal. Native receipt/spool tests and
Rust/Node interoperability are required before claiming this candidate on a
platform; installer and real issuer/harness qualification remain separate.

### Amendment 1 (2026-09-21): native Windows credential storage

Qualify the credential boundary before opening the other private-state paths.
The existing CLI remains the only credential authority. Windows uses safe,
Windows-only `windows-permissions` and `winapi-util` wrappers for process SID,
handle ACLs and file identity, and `winsafe` for local fixed-drive admission.
These MIT dependencies do not relax the product's unsafe-code prohibition.

Walk local drive paths with no-follow directory handles held against deletion.
Ancestors must have trusted owners and refuse unprivileged data/attribute,
delete or ACL mutation; public traversal and creating a new subdirectory are
allowed. This prevents inherited ACL changes from another account above the
private directory. Standard root ownership by TrustedInstaller is admitted.
Refuse reparse points, alternate streams, device names, ambiguous components,
non-disk objects and multiply linked credential files. Existing private
directories/files must belong to the process user and grant access only to that
user, LocalSystem and Administrators. Compare numeric SIDs from the native
descriptor; do not infer machine/domain identity from SDDL abbreviations.
Inheritable creator-owner entries are allowed only when they cannot grant
access on the parent. Unknown ACL forms
fail closed; no repair or silent adoption of existing permissions occurs.

Create missing directories only below an already private parent with private
inheritance. Seal new directories and files with a protected, explicit DACL
before writing private bytes. A stable empty lock holds the existing bounded
cross-process credential transaction; handle identity and ACLs are checked
before and after lock acquisition. Reads are bounded. Replacement writes and
flushes an exclusively created sibling, checks the current destination, closes
handles and renames in the same directory without a delete/truncate fallback.
Rename sharing/access refusals receive at most 20 retries with 25 ms delay,
rechecking the destination before each attempt, so brief readers do not prevent
refresh commit.
Failure retains the original credential file. This is interruption resistance,
not a power-loss recovery or hostile same-account/administrator guarantee.

Login checks storage before its issuer round trip. Native filesystem and
mock-gateway refresh tests establish this local storage slice; real issuer,
installer, hook, receipt and spool qualification remain separate. Keep the
non-Unix refusal on those other private-state paths until their own consumers
and native behavior tests use the same protection. No Windows server port is
introduced.

### Original platform boundary

Keep one CLI. Isolate the Unix peer-witness implementation and explicitly refuse
that deployment witness on other platforms. Preserve its no-follow, ownership,
bounded-read and change-detection checks on Unix. Do not introduce a Windows
server/bootstrap implementation or weaken the existing unsafe-code prohibition.

Share an executable path contract between the CLI and hooks. Unix retains
absolute XDG overrides and HOME defaults. Windows resolves config and state
under LOCALAPPDATA/synveda/config and LOCALAPPDATA/synveda/state; explicit XDG
overrides must be fully qualified local drive paths. Relative XDG overrides
are ignored, while missing/relative default roots fail rather than using the
working directory. No automatic copy or migration of credentials is performed.

Until each Windows storage path has ACL, file identity and atomic replacement in a reviewed native
implementation, private credentials, receipts, logs and spools explicitly
refuse access on non-Unix platforms. Refuse login before the issuer round trip
and refuse local-state mutations before creating files. Hooks must not report
durable recording when their private storage is unavailable. Path resolution
and compilation are separate from permission enforcement and support claims.

## Options considered

1. **Explicit platform boundaries** — chosen; preserves the existing client and
   exposes the remaining privacy work without silently granting access.
2. **Remove permission checks to cross-build** — a compiling executable could
   disclose credentials or transcript data through inherited ACLs.
3. **Split out another Windows client** — duplicates credential and public-API
   behavior before there is native qualification evidence.

## Consequences

The path contract is testable on any development host. Native Windows build and
refusal checks alone can run without deployment services; they do not qualify login,
refresh, spooling, vendor loading or an installer. PowerShell installation and
Windows artifact publication remain gated on private storage and native
acceptance for each claimed architecture. Replace the explicit refusal only
with those checks and evidence, never with inherited-permission assumptions.

## Compliance notes

Public APIs retain Cedar, forced RLS, VedaFlow and audit authority. Local paths
and receipts grant no product authority. Existing Unix credential serialization,
observation consent, spool formats and retained deployment data are unchanged.
