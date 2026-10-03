# ADR-0132: Verify publisher before installing release code

- **Status**: Accepted
- **Date**: 2026-09-30
- **Feature(s)**: OPS-12, OPS-8
- **Deciders**: Synveda maintainers

## Context

Release CI signs the final `SHA256SUMS` inventory through GitHub artifact
attestations. The Unix and Windows installers currently consume that inventory
without verifying its signer. A replaced archive and checksum can therefore
reach extraction and, in client mode, execute its bundled Node installer.
The manual Docker download ceremony already requires GitHub CLI verification.
ADR-0065's client distribution does not require Docker or a system Node.

## Decision

Require an independently installed, trusted GitHub CLI before a remote install.
Use its [attestation verifier](https://cli.github.com/manual/gh_attestation_verify)
on the downloaded inventory and bundle before downloading or extracting code.
Pin GitHub.com, `synveda/synveda`, `.github/workflows/release.yml`, the GitHub
Actions OIDC issuer and the SLSA v1 provenance predicate. Require the canonical
release tag, an operator-supplied expected 40-character source commit and
GitHub-hosted runners. An absent bundle, unsupported verifier or unsuccessful
verification stops installation. There is no remote checksum-only fallback,
publisher override or skip-verification option. HTTPS mirrors retain this
same publisher contract; HTTP and remote file URLs are refused.

`SYNVEDA_SOURCE_SHA` (Unix) and `-SourceSha` (Windows) select the expected commit
from the reviewed release record. Future release notes render that exact commit
and fetch the inspected bootstrap script by commit rather than a mutable ref.
Bootstrap-script inspection remains a trust prerequisite: the script cannot
authenticate itself after the operator has already executed it.

An absolute local `file:///` asset directory without an expected commit is an
explicit development candidate path. It uses checksum and archive validation,
prints that publisher identity was not verified, and establishes no release
trust claim. Existing native packaging fixtures use this path before signing.
Supplying an expected commit to a local directory instead requires the same
attestation check, permitting a downloaded signed release to be tested locally.
This is not an air-gap qualification: GitHub CLI may refresh its trusted roots.
Offline verification requires the separately documented
[trusted-root ceremony](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/verify-attestations-offline).

Bound inventory/bundle downloads and verifier runtime. Verify the archive's
exact unique checksum entry after publisher verification, then preserve the
existing archive, private-state and retained-installation checks. Do not fetch
or execute a verifier supplied by the archive being verified.

## Options considered

1. **GitHub CLI before code execution**: reuses the release's maintained
   verifier and existing manual ceremony; adds a client-install prerequisite.
2. **A new bundled verifier**: removes the system CLI prerequisite but creates
   a second binary distribution, bootstrap trust and update boundary.
3. **Checksums or manual verification only**: leaves automated remote installs
   accepting an unauthenticated inventory.

## Consequences

Remote client installation now needs GitHub CLI as well as existing host tools;
Unix downloads require curl for HTTPS redirect restrictions.
it still needs no Docker, compiler, system Node or registry credentials. Local
unsigned candidate reports must be labelled separately from publisher proof.
Prior published tag-bound installers retain their historical behavior; this
source candidate changes only a future release's installer contract.

The attestation binds distribution bytes and provenance, not OS signing,
vulnerability policy, reproducible builds or data migration. PR-11's archive
SBOM, vulnerability exceptions and revocation procedure remain open. Revisit
the verifier dependency only when a maintained alternative has an independently
qualified bootstrap and identity policy.

## Compliance notes

This is a distribution boundary. It changes no Cedar, forced-RLS, VedaFlow,
tenant authority, audit chain or credential custody behavior. Installation
does not start a deployment or contact an identity provider.
