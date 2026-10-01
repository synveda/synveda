# ADR-0135: Bind bundled Node runtime inventories to upstream bytes

- **Status**: Accepted
- **Date**: 2026-10-01
- **Feature(s)**: OPS-12
- **Deciders**: Synveda maintainers

## Context

The six private clients copy Node 24.21.0 from reviewed upstream archives. The
product copies Node 22.23.2 from its digest-pinned console builder. Existing
packaging retains the full upstream `LICENSE`, but neither the Cargo inventory
nor a generic Node package identity describes Node's embedded libraries.

The [mechanism probe](../../demos/evidence/ops12-node-runtime-probe.json) checks
all six official client distributions and both product Linux distributions.
Syft 1.51.0 identifies Node on POSIX but does not identify embedded library
versions; its original `node.exe` filename also produces no Node identity.
Actual product ARM bytes match the official upstream executable and licence.
Windows' complete upstream licence uses CRLF; POSIX uses LF. Both upstream
aggregate licences omit the separate `deps/nbytes/LICENSE` MIT notice.

## Decision

Reuse Node's maintained [`process.versions` and `process.config` APIs](https://nodejs.org/docs/v24.21.0/api/process.html#processversions)
in the existing native packaging and execution checks. Pin the actual upstream
executable and complete licence hashes per target, in addition to the existing
distribution archive hashes. Independently check the final client archive and
stopped product-image files against those pins before retaining their existing
source/target-bound reports. No new scanner, runtime service, package manager,
SBOM formatter or release sidecar is introduced.

Keep the observed version map in the existing client manifest and image report.
Check it against reviewed version-specific expectations and the actual native
runtime. Treat `modules`/`napi` as ABI identifiers and `cldr`/`tz`/`unicode` as
data versions; empty optional version values do not identify shipped libraries.
Retain only the reviewed shared-library flags, never the full build configuration
or a diagnostic report containing host paths or environment variables.

Preserve complete upstream `LICENSE` bytes, including their platform line
endings. Also carry the exact upstream nbytes MIT notice and SQLite's upstream
copyright disclaimer, with source references and hashes. Keep upstream notices
intact rather than regenerating or trimming their mixed runtime/build content.
The upstream
[licence builder](https://github.com/nodejs/node/blob/955266bfdd854cd280dffd47548673914484e4c0/tools/license-builder.sh)
owns that aggregate; Synveda only validates its copied bytes and supplies the
identified omissions. Inventory readers remain build/release tools, outside
client installers and hook execution.

For the product follow-up, extend the existing native image verifier with one
invocation-owned stopped container whose fixed entry point is the pinned Node
metadata command. Read the executable and three notice files through bounded
Docker TAR streams and the existing unique-regular-member reader. Only after
byte/platform checks pass, attach to that container's short native metadata
execution. Keep its network disabled, filesystem read-only, privileges dropped,
ambient Node options cleared and resources bounded. Always attempt owned
container/stream cleanup; cleanup failure cannot retain passing evidence and
must preserve the first inspection cause. Require the resulting inventory in
existing local and both-registry image reports through assembly. Keep the
console's separate 34 MiB stream bound unchanged.

This is an inventory of the pinned Node distribution and its reported
dependencies, not a claim of complete transitive native coverage. Unversioned
embedded helpers, V8's internal dependencies, other native libraries and
upstream-image contents remain in the ordered OPS-12 coverage work. On Linux,
non-vendored `openssl-sys` is a Rust binding: its version cannot establish the
linked C OpenSSL version. A locked but unused `libsqlite3-sys` entry must not be
reported as shipped product content.

## Options considered

1. **Upstream runtime metadata plus exact byte checks**: reuses the supported
   Node boundary and existing reports without a new tool dependency. Coverage
   limits remain explicit.
2. **Generic binary scanning alone**: the actual probe misses embedded library
   versions and both Windows Node identities.
3. **A custom native dependency scanner or source-only SBOM**: adds a separate
   discovery mechanism without proving its relation to distributed bytes.

## Consequences

Runtime pin updates require reviewing distribution, executable, notice and
dependency-version changes together. Native execution evidence is required for
each target; inspecting a foreign executable header does not qualify execution.
Final archive/image substitutions and incomplete notices must refuse even when
a producer updates its report coherently. The existing 51-payload checksum,
publisher-verification, Cargo-content and console-content gates remain.

Revisit this small contract if upstream Node supplies a maintained complete
binary-bound dependency inventory, or if its native metadata no longer exposes
the required reported dependencies. The separate Syft Windows resolver blocker
still prevents native Rust source qualification.

## Compliance notes

Distribution metadata only: no Rust, SQL, public API, tenancy, policy or audit
change. Cedar, forced RLS, VedaFlow and content-free audit remain mandatory.
Only public dependency identities and artifact/source hashes are retained;
production keys, credentials, tenant content and local host paths are excluded.
