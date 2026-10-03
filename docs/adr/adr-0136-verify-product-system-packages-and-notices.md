# ADR-0136: Verify product system packages and notices

- **Status**: Accepted
- **Date**: 2026-10-01
- **Feature(s)**: OPS-12
- **Deciders**: Synveda maintainers

## Context

The [retained Syft probe](../../demos/evidence/ops12-native-package-notice-probe.json)
identifies 106 installed Debian packages and 119 complete copyright/common-licence
files in the product. Parsed licence expressions do not replace those raw files.
Rust's `openssl-sys` version also does not identify the native OpenSSL library.
The existing pinned BuildKit scanner's actual product SPDX includes the package
identities, supporting notice paths, package database and raw-file SHA-256 values.

## Decision

Extend the existing product OCI SBOM and image reports with the installed Debian
inventory and complete notice hashes. Reuse the pinned Syft SPDX, its existing
hashed native-image/attestation reader and bounded stopped-container file reads;
add no scanner, formatter, service, workflow or release sidecar.

Before retaining the product candidate, require unambiguous Debian name/version,
architecture and package URL identities, a supporting copyright for every
installed package, complete copyright/common-licence file hashes, the package
database hash and the actual OpenSSL, libc and C++ library file hashes and owners.
Bind this inventory to the same native image and source as the Cargo check.
Missing, ambiguous, foreign-platform or placeholder evidence refuses.

Independently read the package database, every selected complete notice and the
reviewed native library files from an invocation-owned stopped product container.
Compare installed package identities to the database, native ELF headers to the
target and actual bytes to the original SPDX hashes. Keep the raw notices in the
image; retain their sizes and hashes in the existing image report. Do not infer
licence conclusions from mixed
Debian source copyright sections or execute the product to discover packages.

The existing candidate report supplies the native-image-bound expected inventory
to local and both-registry inspection. Public verification already downloads and
checksums that report before inspection. Assembly requires matching candidate
and image evidence for each architecture. Bound files, total copied bytes and
inspection time; always attempt owned cleanup and preserve the original cause.
Keep all Cargo, console, Node, original 51-payload and publication gates.

## Options considered

1. **Existing SPDX and independent file reads**: the actual scanner already
   supplies the required hashes; no new distribution format or tool is needed.
2. **Another full Syft scan/report**: the mechanism probe supports it, but it
   duplicates the current OCI attestation and adds scanner provisioning.
3. **Licence expressions or package counts alone**: neither proves complete
   notice carriage, exact installed versions or native-library ownership.

## Consequences

This first increment covers the Debian product image and the reviewed native
library files. Other images, static Node/V8 components, import-resolution
coverage, plugins/charts, vulnerability policy and publisher incident handling
remain in OPS-12. Native Rust's Windows scanner blocker remains separate.
The [local ARM candidate](../../demos/evidence/ops12-product-package-inventory-candidate.json)
passes actual file/database checks and two real substitution refusals; clean
hosted and both-architecture qualification remain pending.
Changing the base distribution or scanner output requires reviewed artifact
probes before changing this contract. Source candidates do not establish clean
hosted, registry, deployment or complete third-party qualification.

## Compliance notes

Release tooling only. No Rust, SQL, public API, tenancy, policy, audit or runtime
dependency changes. Cedar, forced RLS, VedaFlow and content-free audit remain
mandatory. Reports retain public package identities and artifact hashes, not
scanner host paths, credentials, keys or tenant content.
