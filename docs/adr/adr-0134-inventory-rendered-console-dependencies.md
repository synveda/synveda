# ADR-0134: Inventory rendered console dependencies

- **Status**: Accepted
- **Date**: 2026-10-01
- **Feature(s)**: OPS-12
- **Deciders**: Synveda maintainers

## Context

The console ships minified JavaScript in a standalone archive and the product
image. The [mechanism probe](../../demos/evidence/ops12-console-sbom-probe.json)
found no npm identities with the final-directory Syft JavaScript cataloger,
although the rendered module graph contains React, React DOM and scheduler.
Their complete MIT notices must accompany those bytes. Source manifests alone
also include dependencies whose code never reaches the bundle.

The native Rust archive qualification remains blocked by Syft's upstream
Windows resolver defect. The owner's continuation permits the independent
non-Rust inventory/notices work while retaining that failure and every native
gate. Neither the old OCI qualification nor a console build closes that gate.

## Decision

Use exact build-only `rollup-plugin-sbom` 4.0.0 to emit CycloneDX 1.6 JSON with
complete package licence evidence through the existing Vite build. Independently
check its package identities and licence bytes against the positively rendered
physical module graph and installed package metadata before accepting output.
The [maintained release](https://github.com/janbiasi/rollup-plugin-sbom/releases/tag/v4.0.0)
supports the existing Node 22+/Vite 6 boundary; explicitly select schema 1.6
rather than inheriting its changed default. The frozen pnpm lockfile pins its
closure. Disable timestamps, random serials and duplicate well-known output.
Do not change shipped JavaScript or introduce a runtime service.

Accept three package-specific build-only licence exceptions for this tool's
locked closure: `lru-cache` 11.5.3 uses
[BlueOak-1.0.0](https://blueoakcouncil.org/license/1.0.0),
`spdx-exceptions` 2.5.0 retains the Linux Foundation's
[CC-BY-3.0 attribution](https://github.com/jslicense/spdx-exceptions.json#copyright-and-licensing),
and `spdx-license-ids` 3.0.24 declares CC0-1.0. The two SPDX packages provide
licence identifier data to the build-time formatter; none of these packages is
rendered or installed in the product. Retain their upstream notices in the
build installation. Keep the shipped licence allowlist and the default build
allowlist unchanged; the existing unused-exception gate still applies.
The exact pinned console-builder image runs Node 22.23.2, satisfying the
closure's stricter Node 22.22.2 floor; plugin metadata alone is insufficient
to establish that compatibility.

Emit `sbom.cdx.json`, `THIRD-PARTY-NOTICES.txt` and
`dependency-inventory.json` in the existing console output. The inventory
binds every other output file by relative path, byte size and SHA-256, including
the SBOM and complete notices. Bind the build source when supplied by release
CI, and hash the console manifest, frozen lockfile, Vite config and inventory
implementation. Ordinary local builds may have no release source; assembly
must refuse that case. Release assembly compares input hashes to its own
exact-source checkout and rehashes actual archive members before checksumming.

Keep package discovery bounded and use only positively rendered modules; a
missing, duplicate, foreign or misversioned package or truncated licence fails.
Do not rely on the plugin's envelope alone: its analyser includes non-rendered
modules and ignores virtual modules. Attribute known CommonJS wrappers and
module-preload polyfill code to the exact installed Vite distribution and
retain its full `LICENSE.md`, which includes the bundled CommonJS plugin
notice. Record helper identities separately from runtime npm packages. Unknown
rendered virtual modules fail until their provenance is reviewed. Do not
normalize an arbitrary virtual path into an accepted helper.

Retain Inter's complete upstream OFL bytes and bind the emitted font to the
source font by hash. The standalone console archive and existing full-directory
product-image copy both carry the inventory and notices. The first slice
requires archive content validation and exercises the actual build; native OCI
extraction and complete non-Rust image coverage follow separately. Preserve the
existing first-party notices, archive/member bounds, publisher verification,
Rust inventory and all deployment gates. The console inventory stays inside
the archive and its signed checksum boundary; it adds no release payload.

## Options considered

1. **Maintained bundler plugin with independent checks**: captures the actual
   package graph and full notices with no runtime dependency; requires helper
   attribution and output checks beyond the plugin.
2. **Final-directory scanner alone**: cannot identify the minified runtime
   packages demonstrated by the probe.
3. **Copy the workspace dependency tree**: includes build-only and unused
   packages without proving their relation to distributed bytes.
4. **Write a second SBOM generator**: duplicates a maintained format/library
   boundary and increases update and schema obligations.

## Consequences

The build gains a pinned tool and small bounded validation/emission seam.
Release preparation must supply the exact source identity. Notice text is
deduplicated only by complete byte content; package attribution remains explicit.
Changing runtime packages, helpers, fonts or build tooling requires reviewing
the resulting inventory and refusals. Unknown helpers or differing independently
observed package content require correction, not a widened empty-inventory gate.

This console slice does not establish native Rust archive qualification,
Node/static C library inventories, plugin/chart/upstream-image notice coverage,
vulnerability results, signing, a supported platform window or production
readiness. Revisit the plugin if it stops supporting the pinned build stack or
its emitted package graph diverges from independently observed runtime content.

## Compliance notes

Build and distribution metadata only: no Rust, SQL, generated public API,
runtime authority or tenant-data changes. Cedar, forced RLS, VedaFlow,
content-free audit and Apalis's deployment-leaf boundary remain mandatory.
Only repository-relative input/output paths and public dependency identities
belong in the inventory; local host paths, credentials and tenant content do not.
