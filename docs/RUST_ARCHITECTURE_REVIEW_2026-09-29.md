# Rust architecture review — 2026-09-29

This is a source review for the small-team production roadmap, not a runtime
certification. It screens all 14 workspace crates with dependency, formatting,
strict Clippy and source-pattern checks, then examines the database/authority,
identity, gateway and local-input seams manually. The workspace contains about
255,000 Rust lines including tests; this is not a line-by-line proof of every
path. The [NASA Software Engineering Handbook](https://swehb.nasa.gov/spaces/SWEHBVC/pages/85426344/9.16%2BThread%2BSafety)
references the Power of Ten rules for simple control flow, bounded loops and
resource predictability. Here they are applied to a Rust async service as
bounded work, small reviewable authority paths, explicit error handling and
strong static checks. A literal ban on heap allocation after startup would
not fit this product.

## Findings and disposition

| Priority | Finding and evidence | Disposition |
| --- | --- | --- |
| P1, corrected | The local OKF directory reader recursively traversed empty directories without counting them toward its 2,000-entry limit, collected a whole directory before checking that limit, and used an unbounded `fs::read` after a metadata size check. A changing local file could therefore exceed its declared read bound. The CLI archive reader had the same metadata/read gap. See `synveda-okf/src/archive.rs` and `synveda-cli/src/okf.rs`. | CPR-27/CPR-28: iterative depth-first traversal now counts every non-`.git` entry before queuing, bounds each read and the 4 MB expanded total, and refuses a symlink root. The CLI caps the actual archive read at 1.5 MB plus one byte. Regression tests cover empty directories, aggregate bytes and root symlinks. |
| P2, corrected | The gateway's principal-scope authorisation path said a missing scope would fail closed but called `unreachable!()` instead. An inconsistent catalogue or future store change could panic a request. See `synveda-gateway/src/authz.rs`. | CPR-6: a missing scope now returns a storage error and refuses the decision. The foreign-key invariant makes the path exceptional, not a reason to panic at the authority boundary. |
| P2, open | Forty-four production SQLx query calls are in `synveda-audit` (13) and `synveda-vedaflow` (31), outside the `synveda-store` SQL boundary required by `AGENTS.md`. They are static and SQLx checked; no PDP/RLS bypass was found in this review. `make check-deps` checks crate direction, not SQL placement. Apalis transport SQL is the documented exception. | Move these queries behind narrow store APIs when the affected audit/flow seams are next changed, with database tests preserving transaction and RLS behavior. Add a source gate against **new** product SQL outside store before attempting a broad migration. This is an architectural maintenance debt, not evidence that existing queries are unsafe. |
| P2, open | The local OKF importer still performs path metadata/canonicalization checks separately from file opening. A concurrently replaced filesystem path could race those checks. The current workflow reads a path selected by the local CLI user; it does not prove safe ingestion of a tree actively mutated by another principal. | For an adversarial shared checkout, pin directory/file descriptors and use no-follow opens on each platform, or snapshot into an owner-private directory before reading. Include a concurrent replacement test. Until then, document the local source as stable during import. |
| P2, open | Authority-sensitive functions are difficult to audit in one pass: `plan_context_run` spans about 680 lines in `synveda-gateway/src/context_api.rs`, and `apply_loaded` spans about 776 lines in `synveda-gateway/src/knowledge.rs`. Their behavior is real product scope, but policy checks, selection, mutation and trace bookkeeping share long control flows. | At the next functional change, extract typed stages with explicit inputs and results, keeping one transaction/decision boundary. Preserve existing endpoint, audit and RLS tests; avoid introducing a generic workflow framework just to shorten files. |
| P3, assessed | The reserved `synveda init` CLI command intentionally returns a refusal, while `PromptVariableSchema` is an OpenAPI-only type marked `allow(dead_code)`. Neither is evidence of an accidentally live or unused authority path. Searches and strict Clippy found no confirmed production-only dead branch to delete safely. | Keep the refusal while installation guidance relies on it; reconsider the command at a deliberate CLI contract change. Use coverage and public-API reachability evidence before deleting exported code. [NASA's dead-code guidance](https://swehb.nasa.gov/spaces/SWEHBVC/pages/85426324/9.06%2BDead%2BCode%2BExclusion) calls for analysis of retained unreachable code, not deletion by name alone. |

## Architecture assessment

The crate dependency rule passes for all 14 crates. Core libraries forbid
`unsafe`; the CLI permits edition-2024 environment mutation only in tests.
The reviewed OIDC response/discovery readers and pending-login maps have byte,
time or count caps. The sampled gateway authority paths use Cedar decisions and
the store's forced-RLS transaction model; the review found no competing
authority plane in those paths. SQL placement and the two long authority
functions are the clearest structure costs. Comments that claimed behavior the
code did not provide were corrected rather than treated as implementation
evidence.

"AI slop" is not a reliable provenance diagnosis from source text. The
actionable cases here were a comment contradicting an `unreachable!()` in
authorisation and documented input limits that the directory reader did not
enforce. Both are corrected. The long gateway functions deserve smaller,
testable stages; deleting descriptive comments or adding abstraction layers
without changing those risks would make the code harder to audit.

The corrected OKF path is bounded for a stable local directory, but concurrent
filesystem replacement remains open as above. Static checks do not establish
multi-process availability, live restore, enterprise isolation, or the hosted
deployment and release evidence tracked in [production readiness](PRODUCTION_READINESS.md).

## Checks

- `make check-deps`: pass, 14 crates.
- `cargo fmt --all --check`: pass.
- `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`:
  pass after the Rust edits.
- `cargo test -p synveda-okf`: pass, including the new local-directory bounds
  tests.
- `cargo test -p synveda-cli okf`: pass, four focused CLI tests.
- `cargo test -p synveda-gateway --lib`: pass, 118 tests with local loopback
  permission for one fixture.
- `make check-fast`: pass, including docs, backlog, ADR, API, adapter, security,
  hard-cut and product-evaluation checks.
- `cargo deny --offline check bans licenses sources`: pass. The advisory check
  could not use its read-only cache in this sandbox, so this review makes no
  fresh advisory claim.

The source review is a prioritized engineering assessment. A full proof of
unreachable code, resource ceilings under adversarial concurrency and all
deployment states would require runtime coverage, stress testing and the live
acceptance gates in the roadmap.
