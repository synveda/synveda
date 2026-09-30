# ADR-0133: Embed Rust dependency inventory in release binaries

- **Status**: Proposed
- **Date**: 2026-09-30
- **Feature(s)**: OPS-12
- **Deciders**: Synveda maintainers

## Context

The published v0.4.3 product image has valid SPDX documents but no identified
Cargo, Cedar or SQLx packages on either Linux architecture. The
[inspection](../../demos/evidence/ops12-published-sbom.json) binds that finding
to its signed inventory and immutable image identity. Plain compiled binaries
do not provide the dependency metadata needed by the existing final-stage Syft
scanner. Attestation presence alone therefore cannot qualify Rust coverage.

An [isolated mechanism probe](../../demos/evidence/ops12-rust-sbom-probe.json)
built the unchanged CLI with Rust 1.96.0 and `cargo-auditable` 0.7.6. Syft 1.51.0
identified 295 Cargo packages, including locked Cedar and SQLx versions, before
and after the same macOS strip command used by server packaging. Ordinary
synthetic binaries produced valid SPDX with no Cargo packages. This is local
macOS ARM evidence, not native release or container qualification.

## Decision

Build release Rust executables with pinned `cargo-auditable` through the existing
Cargo build commands and locked dependency graph. Retain its standard embedded
dependency metadata through final packaging and stripping, then require actual
Rust dependency content in the artifact's SBOM before qualification succeeds.
The [upstream format](https://github.com/rust-secure-code/cargo-auditable/tree/v0.7.6)
and [Syft cataloger](https://oss.anchore.com/docs/capabilities/rust/) support this
existing binary-analysis boundary; native target evidence remains mandatory.

The proposed initial pins are `cargo-auditable` 0.7.6 (official Cargo source
checksum `26d8fb7a4422b2aea452c301a30bac8528ff54ad89ef300d6ad80dcfb43aef6a`)
and Syft 1.51.0. For OCI builds, select the existing BuildKit Syft generator by
immutable digest:
`docker.io/docker/buildkit-syft-scanner:1.12.0@sha256:ae4f3b554449e7e25548e7d8ccc029d17357348e30c6e3df01b92bc93654d6a9`.
That index has native Linux AMD64 and ARM64 manifests and uses Syft 1.51.0.
Native scanner downloads need reviewed release hashes before execution.

Validate the packaged binary or final image rather than a source `Cargo.lock`
copied into the distribution. Bind each SBOM/report to its exact artifact,
source and platform identity. Require expected first-party roots and critical
runtime packages with versions derived from the locked build; empty Rust
inventory, absent or wrong-version Cedar/SQLx and foreign artifact subjects must
fail. Apply this to native client/server archives and Rust-bearing images with
their existing independent platform reports. Keep the existing compiler,
installer, image-copy, publisher-verification and deployment gates.

The previous clean source passed its nonpublishing qualification, recorded in
[OPS-12 evidence](../../demos/evidence/ops12-source-qualification.json).
Implementation has not started. Next add build metadata and content validation
with negative controls before qualifying the changed artifacts. Update this
ADR to Accepted when that source implementation and its required evidence land.

## Options considered

1. **Embedded inventory with maintained scanners**: standard ELF/Mach-O/PE data,
   existing Syft integration and no runtime service or product dependency.
2. **Copy the full source lockfile into artifacts**: inventories dependencies
   without proving they belong to the packaged executable or target.
3. **Nightly Cargo SBOM precursors**: useful future compiler integration, but
   requires replacing the pinned stable toolchain contract.
4. **Accept an SPDX envelope or image descriptor alone**: the published-image
   inspection and ordinary-binary controls demonstrate incomplete coverage.

## Consequences

Release builds gain a pinned build tool and scanner update responsibility;
ordinary development builds can retain their current commands. Existing build
caches must not reuse a plain executable when embedded inventory is required.
Final-package checks must catch metadata lost by stripping or copying.

Embedded Rust inventory does not establish statically linked C-library,
bundled JavaScript, Node runtime, chart or third-party notice completeness.
Those need separate artifact coverage. Vulnerability findings, exception expiry,
publisher incidents, OS signing and support policy remain distinct release gates.
Revisit the wrapper when stable Cargo provides equivalent metadata or a required
native target cannot preserve/read it under the actual packaging pipeline.

## Compliance notes

This is build and distribution tooling, with no core dependency, public API,
configuration, product SQL or authority change. Cedar, forced RLS, VedaFlow,
content-free audit and Apalis's deployment-leaf boundary remain mandatory.
Inventory contains dependency identities; it must not contain tenant content,
credentials or deployment keys. It does not replace ADR-0132's publisher check.
