# Contributing to Synveda

Synveda welcomes code, tests, documentation, accessibility improvements and
reproducible bug reports. You do not need an AI assistant, a cloud account or
paid model access to contribute. To try the product first, use the
[Docker installation](deploy/compose/PREBUILT.md); to change its source, follow
[the developer guide](docs/DEVELOPMENT.md).

<a id="local-deployment"></a>

## First contribution

1. Find a small change in the [code map](docs/DEVELOPMENT.md#code-map) or browse
   [open issues](https://github.com/synveda/synveda/issues). A typo, test or
   focused fix can go straight to a pull request. For a larger design or public
   contract change, start a conversation in an issue before implementation;
   GitHub Discussions is not enabled.
2. Fork the repository on GitHub, clone your fork and create a working branch:

   ```sh
   git clone https://github.com/YOUR-USERNAME/synveda.git
   cd synveda
   git switch -c fix/describe-your-change
   ```

3. Follow [source setup and the fast check](docs/DEVELOPMENT.md#source-setup).
   Make one focused change and run the checks for that area. Explain any check
   you could not run and its missing prerequisite.
4. Push your branch to your fork and open a PR against `synveda/synveda:main`.
   Explain the problem, resulting behavior and validation. Screenshots help
   for UI changes; use fictional data and exclude credentials or tenant content.
   Documentation-only changes are welcome through the same short PR template.

Use an existing ID from the [feature inventory](docs/backlog/STATUS.md) in the
PR and commit subject, for example `FND-1: clarify contributor setup`. A small
correction does not need a new feature, planning brief or ADR. A new feature
needs an inventory entry and a brief while it is open. Record a new or changed
architectural choice using the [ADR template](docs/adr/adr-0000-template.md).
Delivered work retains its contract in tests, accepted decisions and operator
docs; remove the completed brief. Git retains implementation history.

## Checks and coding expectations

[The developer guide](docs/DEVELOPMENT.md#validation) owns the exact commands
and prerequisites. Start with `make check-fast`, then focused tests. Rust
changes require formatting and strict Clippy for the affected crates. Run the
fresh database fixture for persistence or authorization changes; ordinary
workspace tests can skip database cases when no database is supplied. Report
those skips honestly. Hosted CI runs additional integration jobs.

Preserve the [product invariants](docs/SYNVEDA_SEED.md#2-product-principles-non-negotiable):
Cedar decides reads and writes, forced PostgreSQL RLS backstops tenant isolation,
and governed changes use VedaFlow with content-free audit evidence. Tests use
ordinary tenant transactions and test policy packs. Configuration cannot bypass
these boundaries. Read the relevant [current ADRs](docs/adr/README.md) when
working on them.

Keep dependency direction, explicit control flow and bounded input/work intact.
Prefer private items and preserve causal errors without leaking secrets or
resource existence. Avoid unjustified panics and unchecked unwraps in production.
Keep product SQL static, SQLx checked and in `synveda-store`. Do not introduce
pre-1.0 compatibility paths or unrelated dependency upgrades. Add tests that
prove changed behavior and relevant refusal cases; a prose-only correction
usually needs the documentation checks, not a new test.

Generated OpenAPI, console types, SDK contracts and SQLx metadata must be
regenerated from their sources. Use [the documented commands](docs/DEVELOPMENT.md#generated-contracts),
review the diff, and avoid unrelated generated changes. Preserve the repository's
[Apache-2.0 licence](LICENSE), [notices](NOTICE) and third-party attribution;
do not copy code or assets without checking their terms.

## Documentation and review

Keep setup commands in their canonical guide and link to them elsewhere.
Distinguish implemented behavior, deterministic replay, live verification and
open work. Documentation links and site assets have local checks; external
network availability is not a routine PR requirement. Keep credentials, logs,
local inventories and session handoffs out of the change.

Maintainers decide scope and merge readiness in the PR. Reviewers should be
able to understand the change without reading an earlier agent conversation.
Explain compatibility, security, resource bounds and rollout effects when they
change; ordinary corrections need no large checklist. Keep unrelated cleanup
out of a behavior change. Support and review are best effort; no response time
or supported-version window is promised. Be respectful, discuss the work, and
do not harass people or disclose their private information.

AI assistance is optional. Contributors remain responsible for understanding
the diff, checking correctness and licensing, and reporting tests accurately.
The same review standards apply to all changes. Do not submit bulk generated
code or documentation that you have not reviewed.

## Security reports

Use [private vulnerability reporting](SECURITY.md), not public issues, for
suspected security problems. Ordinary bug reports should use minimal synthetic
reproductions and redacted diagnostics.
