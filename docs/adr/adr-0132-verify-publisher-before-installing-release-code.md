# ADR-0132: Verify publisher before installing release code

- **Status**: Accepted; amended once. ADR-0155 also amends the expected-commit
  requirement.
- **Date**: 2026-09-30
- **Feature(s)**: Consumer installation and harness setup, Release & distribution
- **Deciders**: Synveda maintainers

## Amendment 1 (2026-10-29): installer-provisioned GitHub CLI on Windows

Managed Windows machines often have no `gh`, no package manager the user may
run and no GitHub account. On such a machine the PowerShell installer cannot
satisfy the "independently installed" prerequisite above and stops before
verification.

`install.ps1` chooses its verifier in this order:
1. `-GhPath`/`SYNVEDA_GH`, used as given. An explicit operator choice is never
   silently replaced.
2. `gh` on PATH, if `gh --version` reports at least the pinned version. Older
   releases may lack the pinned verification flags, or ask for `gh auth login`
   even with `--bundle`.
3. Otherwise, the pinned GitHub CLI Windows ZIP for the native target. It comes
   from the GitHub CLI release, or from `-BaseUrl` when a mirror is used.

The pinned ZIP must match the SHA-256 written in the inspected script before
anything is extracted. Only `bin/gh.exe` is extracted, and only after archive
bounds are checked; its PE architecture must match the host. It runs from the
invocation's private download directory and is deleted with it. Its
configuration, cache, state and data directories also stay in that directory,
and update notices, prompts and telemetry are disabled. The pinned arguments,
the absence of a skip option and the refusal of a verifier supplied by the
archive being verified are unchanged.

This differs from option 2 below in each reason it was rejected. The verifier
is not part of a Synveda distribution. Its bytes are trusted exactly as far as
the script that pins them, which the operator already had to inspect. It
persists nothing, so it has no lifecycle of its own. A pin change is reviewed
like any installer change. The release checksum file must list the new ZIPs,
the GitHub CLI release workflow's provenance must verify against the tag
commit, and a run with no sign-in must pass against a published Synveda
release.

`install.sh` is unchanged: Unix hosts can install `gh` without elevation. The
verifier still needs HTTPS access to the Sigstore and GitHub trusted-root
services unless the offline trusted-root ceremony is used. Revisit this choice
if pin updates fall behind GitHub CLI security releases.

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
