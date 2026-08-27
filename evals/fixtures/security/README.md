# The security corpus (EVAL-5, ADR-0048)

One corpus of governed Knowledge with exhaustively declared visibility
boundaries, asked back under generated phrasings through the current
session-scoped surfaces. `security_runner.rs` drives the pull-request slice
against `evals/baseline.json` and the larger nightly against
`evals/baseline-security.json`.

## What a file says

Each proposed Knowledge item declares every corpus reader in exactly one of
`readable_by` or `forbidden_to`. The loader rejects omissions and duplicates:
an undeclared `(Knowledge item, reader)` pair would be an unmeasured boundary
that could still produce a deceptively green zero-leak report.

The file does not declare which boundary separates the pair. The runner derives
that from the admitted tenant, governing scope and sensitivity so a fixture
cannot file a real disclosure under the wrong axis.

## The premise is governed

Material enters through `POST /v1/sessions/{id}/events`, becomes a capture
candidate, and is accepted through the public candidate action. Acceptance
always invokes the Knowledge command layer and VedaFlow. `publish_scope`
selects an environment-provided current scope alias; `sensitivity` selects the
revision sensitivity. No fixture writes an application table directly.

The principal cases are:

- policy visibility and sensitivity, including an item even its author cannot
  read once governed policy denies the sensitivity;
- cross-tenant isolation, where tenant identity comes only from the bearer;
- direct diagnostic ID probes, proving denial by exact item identity rather
  than relying on ranking;
- prompt-injection payloads that try to forge the current JSON entry or
  `[Synveda Knowledge: …]` address footer.

The line invariant treats every non-empty rendered line as fixed furniture or
one complete JSON payload. Supplied Markdown remains JSON-escaped on its
attributed line, so it cannot manufacture a second entry or footer.

## Growing the corpus

Add Knowledge items before adding readers. Every new item must classify every
reader. Keep each actor's diagnostic enumeration below its explicit bound; grow
coverage with additional actors instead of raising a limit until completeness
can no longer be demonstrated.
