# ADR-0108: Run each CI contract once and cache Kind image builds

- **Status**: Accepted
- **Date**: 2026-09-19
- **Feature(s)**: FND-1
- **Deciders**: Synveda CI maintenance

## Context

The successful [CI run 35447119586](https://github.com/synveda/synveda/actions/runs/35447119586)
spent 29m35s in Kind acceptance. Its Docker builds start cold on each hosted
runner. The beta demo also spent 6m54s building release binaries that its
debug-profile Cargo test never uses. Deployment contracts ran in both
`layering` and `release-install`.

The release-install demo wraps those static contracts in a database fixture,
then repeats OpenAPI tests already owned by the Rust job. Its epoch tests do
not receive the dedicated lifecycle credentials and skip their database cases;
the complete epoch acceptance remains owned by `make db-test`.

## Decision

1. Preserve CI job identities and all distinct acceptance boundaries. Let
   `release-install` own `make check-deploy`, including installer, chart-image,
   reference-package and Compose checks. `layering` keeps dependency, generated
   API, documentation, chart-render and benchmark checks. The standalone
   release-install demo remains available outside CI.
2. Remove beta-demo's unused frontend installation/build and release-profile
   Rust build. Its unchanged demo still creates the exact-role database and
   executes the governed team-loop acceptance test.
3. Keep the workspace production build after workspace tests. Gateway tests
   enable a development-only feature through their support crate, so those
   builds do not prove the same feature configuration. Keep both source and
   installed SDK tests and both SDK runtime pairs for the same reason.
   Run SDK tests only in that matrix; reuse adapter test compilation for
   archive replay and retain the console's separate production bundle build.
4. Use an optional GitHub Actions v2 BuildKit cache for Kind's three images,
   with a separate scope per image, distinct from release caches. Build every
   image from the existing Dockerfile and load it locally; never substitute a
   previously published image or skip acceptance. Local demo execution keeps
   ordinary Docker builds. Only cache-export errors may be ignored; image
   build failures remain fatal. Prune local builder storage after loading the
   images into Kind, once cache export has finished.
   Follow Docker's [GitHub Actions cache contract](https://docs.docker.com/build/cache/backends/gha/)
   for v2 runtime authentication, intermediate layers and per-image scopes.
5. Cancel superseded CI runs for the same pull request or branch. Keep main
   and pull-request validation, and leave release publication and the distinct
   nightly deterministic, semantic and full security gates unchanged.
6. Reuse one hash-verified Python wheel download for development installation
   and offline package validation. Cache pnpm's store in jobs that install it.

## Options considered

1. **Remove proven duplication and cache Docker layers (chosen).** Reduces
   runner work and accelerates the longest job without moving a gate off PRs.
2. **Move Kind to nightly or filter jobs by changed paths.** Rejected for this
   change: either can omit acceptance that currently blocks a merge.
3. **Share all Rust build trees between jobs.** Deferred: jobs build different
   feature/profile sets, and cache transfer and invalidation need measurement.
4. **Keep the workflow unchanged.** Retains measured unused builds and repeat
   static suites while every Kind run recompiles dependencies.

## Consequences

- Positive: each deployment contract has one CI owner; builds and package
  downloads are reused without changing product tests or baselines.
- Negative / accepted trade-offs: GitHub cache transfer consumes storage and
  time, a cold Kind run still builds everything, and hosted before/after runs
  are required before claiming a wall-clock improvement. The Kind timeout
  includes cold builds plus cache export.
- Reversal trigger: cache transfer costs more than the compilation it saves,
  persistent eviction prevents reuse, or a removed preparation step gains an
  actual consumer. Restore only the preparation justified by that evidence.

## Compliance notes

Cedar, forced RLS, VedaFlow and audit paths are unchanged. Database acceptance
continues through ordinary tenant transactions and the existing exact-role
fixture. Cache credentials are supplied through GitHub's runtime environment,
never command arguments or repository files. Cached layers contain build
inputs and outputs, not acceptance databases or generated runtime secrets.
No image is published by CI and no production-readiness claim changes.

## Amendment: contributor validation and fork isolation (2026-09-20)

`make check-fast` composes the existing local documentation, generated-contract
and static boundary checks. It needs Git, Make and Node, without a database,
model account or Python environment. It is a first feedback loop, not a
replacement for focused tests or the deeper CI jobs. `make ci` is a local
aggregate; hosted CI additionally owns database-backed and Kind acceptance.

PR validation uses GitHub-hosted runners, a read-only repository token and
checkouts without persisted credentials. Only main may save reusable Rust or
Kind build caches; PRs may restore them. Release publication stays in the
separate release workflow. Existing check names and all-PR triggers remain,
including documentation-only PRs. No privileged PR event or maintainer runner
is needed for a first contribution.

Contributor instructions live in CONTRIBUTING and the source-development
guide. Small fixes use an existing feature ID and need neither an advance issue
nor an ADR unless they change an architectural decision.
