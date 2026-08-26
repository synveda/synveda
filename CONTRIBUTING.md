# Contributing

Synveda's trust boundaries are part of its product contract. Read
[AGENTS.md](AGENTS.md), the Seed invariants and the relevant accepted ADRs
before changing code.

## Workflow

1. Map the change to the [feature inventory](docs/backlog/STATUS.md). For new
   work, add an open entry and its implementation brief together.
2. Use `feat/<ID>` for ordinary feature work and include the feature ID in each
   commit subject.
3. Record architectural decisions before implementation using
   [the ADR template](docs/adr/adr-0000-template.md).
4. Add behaviour-level tests and runnable acceptance under `demos/` where the
   feature has an executable path.
5. Keep generated OpenAPI, console types and SQLx metadata derived from their
   sources; never hand-edit them.
6. Update the brief and delivered/open state with the same change. When work is
   delivered, retain its contract in tests/ADRs/docs and delete the brief; git
   is the implementation archive.

Do not add a test-only path around Cedar or tenant RLS. Use a test policy pack
and the ordinary tenant transaction boundary.

## Local checks

Run focused tests while working. Before review, run the gates appropriate to
the change and record prerequisites that were unavailable:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
make ci
make db-test
```

Database, live-model and proprietary-client gates require their documented
services or credentials. Missing prerequisites are not passing results.

## Review expectations

A review should be able to identify:

- the acceptance criterion and test that demonstrate the change;
- the PDP, RLS, VedaFlow and audit effects, including an explicit “unchanged”;
- all new resource bounds, timeouts and retry/idempotency behaviour;
- generated-contract or schema effects;
- operational rollout, rollback and compatibility consequences;
- any remaining production-readiness gap.

Keep commits small enough to review independently. Do not combine semantic
changes with bulk file movement, generated churn or historical-document cleanup.

## Security

This checkout does not yet publish a vulnerability-reporting channel or
response SLA; that is a production-readiness gap. Do not put vulnerability
details, secrets, tenant content or unredacted diagnostics in public issues,
logs, fixtures or audit evidence. The repository owner must publish a private
reporting route before accepting external distribution or contributions.
