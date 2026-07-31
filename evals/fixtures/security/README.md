# The security corpus (EVAL-5, ADR-0048)

One corpus of governed material with **exhaustively declared boundaries**,
asked back under thousands of generated phrasings over every read surface
the product has. Read by `crates/synveda-eval/src/security.rs`, run by
`security_runner.rs`, gated by `evals/baseline.json` (the pull-request
slice) and `evals/baseline-security.json` (the nightly's full budget).

## What a file says

Each record declares each of the corpus's readers **`readable_by` or
`forbidden_to`**, and the loader refuses a file where a pair is left out or
declared twice. That guard is the format's reason to exist: an undeclared
pair is a boundary nothing asserts, and it would still report zero leaks
and a complete-looking report.

What a file deliberately does *not* declare is **which** boundary separates
a record from a reader. That is derived per pair — a reader in another
admitted tenant is a `tenant` boundary whatever else is true, a record
above the working tier is a `sensitivity` one, everything else is `scope` —
because a corpus author who mis-declared it would file a real leak under
the wrong axis, and a derived answer cannot be mis-declared.

## The premise is governed, never seeded

Material enters at its author's leaf through `/v1/observe`. Anything above
the working tier gets there through a **classification proposal the author
opens at their own home scope** and two distinct approvers sign, one of them
holding `compliance` — the only mechanism in the product that mints a tier
(ADR-0038 decisions 8 and 9). Anything above a leaf gets there through a
real proposal and a real approval (ADR-0034).

The order is forced, not stylistic: **classify, then climb.** A publication
names a record at its current address and the installed tier is part of that
address, so reclassifying afterwards would move the material out from under
its own channel entry.

A leak suite that wrote its own `restricted` row would be asserting that a
tier no product path produced does not cross a boundary no product path
opened.

## The three halves

- **Policy leak.** `vault-ceremony` is `restricted` and forbidden to
  everybody *including its own author*: `base.cedar`'s forbid has no owner
  carve-out and the one base-layer permit that could lift it carries
  `resource.kind != "user"`, so a restricted record at a personal leaf is
  invisible to everyone and no lapse can change that. `supplier-terms` is
  the sensitivity boundary with the reader **in scope** — a teammate at the
  team it climbed to, who still may not compose it. `bridge-rota` and
  `owner-diary` are scope boundaries, one published to a team and one at a
  personal leaf.
- **Cross-tenant.** `beta-ledger` lives in a second admitted tenant. The
  runner never sends a tenant — the token carries one — which is what makes
  this a real probe rather than a filter test, and the ids form asks the
  product to refuse the record **by name**, with no retrieval in the way.
- **Prompt injection.** The three `forge-*` records carry the block's own
  structural vocabulary in their content: a scope section header, a whole
  entry line, a watermark comment, a trust marker, a recall handle. The
  assertion is not about them specifically — it is about every block the
  suite composes: every non-empty line is the preamble, the data notice, a
  section header, the index legend, the watermark or an entry, and the
  entries number exactly as many as the watermark names.

The threat model needs no attacker inside the tenant. An agent reads a
poisoned page, the tool result is observed, and the memory composes into a
**later session of the same user** — at the nearest end of the gradient,
which is the top of the block (seed §4.4).

## Growing it

Add records, not readers, first. The sweep this suite uses to find where
material landed is capped at 32 records (ADR-0046 decision 3) and a full
page is indistinguishable from a truncated one, so a corpus grows by adding
**actors** once it approaches that — never by raising the limit.

Every new record must declare every reader. That is not paperwork: it is
the difference between a boundary this suite asserts and one it silently
skips.
