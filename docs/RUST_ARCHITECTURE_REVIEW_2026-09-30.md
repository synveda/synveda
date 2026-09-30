# Rust architecture and reliability review — 2026-09-30

This review covers the current `codex/synveda-production-roadmap` source at
`db74007`, including the OPS-5/OPS-7/OPS-6 and AUTH-6 changes after the
[29 September review](RUST_ARCHITECTURE_REVIEW_2026-09-29.md). It is a
risk-based source and tool review of the 14 workspace crates, with closer
inspection of Cedar convergence, gateway/worker supervision, one-time login,
credential cleanup, and local importers. It is not a line-by-line proof of the
roughly 182,000 lines under `crates/*/src`, a production load qualification, or
a NASA certification.

The [JPL Power of Ten](https://spinroot.com/p10/) was written for
safety-critical C. Here its useful tests are simple control flow, bounded work
and memory, small authority functions, narrow scope, explicit error handling,
and warning-clean static analysis. A literal ban on allocation after startup
does not fit an async HTTP service. Long-running server loops are expected;
each iteration still needs a known cost and a cancellation path. NASA's
[dead-code guidance](https://swehb.nasa.gov/spaces/SWEHBVC/pages/85426324/9.06%2BDead%2BCode%2BExclusion)
also calls for reachability analysis before removing apparent unused code.

## Findings, ordered by operational impact

### P1 — OPS-7: one tenant's policy sweep can close every tenant's gateway

[`tenants::active`](../crates/synveda-store/src/tenants.rs) lines 88–100
fetches every active tenant into a vector without a page size or configured
tenant ceiling. [`converge_packs_once` and `refresh_packs_once`](../crates/synveda-gateway/src/authz.rs)
lines 847–886 process that vector serially; lines 997–1015 allow five seconds
for the **whole** refresh. Only a complete successful sweep renews the
30-second process lease. One slow or invalid tenant pack, or enough healthy
tenants to exceed five seconds, can therefore make every gateway and core
worker stop serving unrelated tenants. The invalid-pack fail-closed behavior
is intentional and tested under ADR-0127/0128; the deployment-wide blast
radius and tenant-count limit are not yet an enterprise contract. The comment
that this is fine at “admissible tenant counts” has no enforced count behind it.

Before enterprise admission, set a measured tenant/pack work envelope and
test it at the accepted maximum. Then make convergence incremental or isolate
failed tenant generations while preserving fail-closed decisions for that
tenant. Any different authority boundary needs an ADR and adversarial tests;
simply extending the lease or timeout would weaken the current guarantee.

### P2 — CPR-23: Skill directory import has an unbounded traversal and read

[`skill::collect`](../crates/synveda-cli/src/skill.rs) lines 451–502
recurses into every directory but counts only files against `MAX_SKILL_FILES`.
A tree of empty directories can consume unbounded traversal work and stack
depth. A file is loaded with `read_to_string` before its 65,536-character
limit is checked, so a large or growing file can allocate far beyond the
declared bound. The sibling OKF directory importer was fixed for precisely
these patterns in the prior review; this CLI path remains. The source is
operator-selected local input, so this is a CLI availability and importer
robustness defect, not a demonstrated gateway exploit. Use iterative traversal,
count directories and entries, cap actual bytes before UTF-8 decoding, and
retain the bundle-character limit. Include empty-tree, deep-tree, oversized
file and changing-file cases.

Follow-up: CPR-23 now walks directories with a bounded worklist, counts empty
entries within the maximum implied by 64 files and four path segments, and
caps each read at one byte beyond the largest valid UTF-8 encoding before
decoding. Focused tests cover flat empty trees, deep trees, oversized files,
growth after metadata inspection and four-byte Unicode scalars at the
character limit. This closes the local importer availability finding; it does
not claim protection from an adversary mutating the directory between path
inspection and open.

### P2 — OPS-7: transient deployment-key provisioning is not retried

The gateway marks policy readiness at
[`main.rs`](../crates/synveda-gateway/src/main.rs) line 474, then makes one
bounded `keys.provision` attempt at lines 480–513. An error or timeout logs a
warning and enters the policy-refresh loop without another provisioning
attempt for that authority generation. [`/readyz`](../crates/synveda-gateway/src/app.rs)
lines 672–685 sees only admission, database authority and policy freshness.
On a fresh OIDC installation, a transient failure can leave a ready gateway
whose `/auth/login` cannot seal pending state; `KeyRing::sealing_key` explicitly
does not provision a missing key. Retry provisioning with bounded backoff and
give the configured login plane an observable readiness result. Preserve
bearer-only behavior when the KMS is intentionally disabled.

Follow-up: OPS-7 now retries bounded provision-and-unwrap attempts before
admission, reports key readiness and attempt outcomes, and passes an exact-role
timeout/recovery plus wrong-KEK process test. Live OIDC and cross-pod key
rotation remain open.

### P2 — AUTH-6: credential cleanup has a fixed service rate without backlog evidence

[`console_sessions::purge_expired`](../crates/synveda-store/src/console_sessions.rs)
lines 230–253 correctly bounds and locks one deletion to 256 rows. The
combined worker invokes one batch every 60 seconds at
[`worker.rs`](../crates/synveda-gateway/src/worker.rs) lines 733–769: at most
4.27 expired rows per second in steady state. Console-session creation has
no matching global cap, and the metrics count sweeps and removals but not
oldest expired age or remaining rows. Sustained expiry above that rate grows
retained credential ciphertext while all current checks can remain green.
Measure backlog and age, then drain bounded batches up to a time/work budget
per tick or set an admission and retention envelope. Keep skip-locked
coordination and database-time expiry.

Follow-up: AUTH-6 now exposes the indexed oldest-expired age after successful
sweeps. The fixed service rate and unproven retention bound remain open.

### P2 — architecture: product SQL remains outside `synveda-store`

The prior review reported 44 production SQLx calls in `synveda-audit` and
`synveda-vedaflow`; a complete recount found 53 macros (15 audit, 38
VedaFlow). This branch adds no new calls there, but the boundary still
contradicts `AGENTS.md`'s store-only product SQL rule. `make check-deps`
verifies crate direction, not SQL placement. Move queries behind narrow store
APIs when those transactions are next changed; first add a source gate against
new outside-store product SQL. Preserve tenant transaction, RLS and audit
atomicity rather than moving text mechanically.

Follow-up: CPR-44 adds a `make check-deps` source gate that pins the path,
exact SQL literal and multiplicity of the 53 existing production macros. It
rejects added or altered calls, dynamic SQLx calls and query imports outside
`synveda-store`, with the documented Apalis transport and test-only CLI
deployment fixture excluded. The gate does not relocate the 53 queries or
prove SQL placement against deliberately obscured Rust syntax. Their safe
migration remains architectural debt.

### P2 — structure: two authority-heavy functions remain too large to audit locally

[`plan_context_run`](../crates/synveda-gateway/src/context_api.rs) spans
roughly lines 2484–3163, and
[`apply_loaded`](../crates/synveda-gateway/src/knowledge.rs) roughly
1094–1869. They combine policy decisions, data retrieval or mutation, trace
bookkeeping, and failure handling in one control flow. This was open in the
prior review and is still the clearest Power-of-Ten function-size departure.
At their next functional edit, extract typed stages with explicit inputs and
outcomes while retaining one coherent transaction/decision boundary. A generic
workflow framework would increase the audit surface.

### P3 — current comments and public Rustdoc retain removed architecture

`RUSTDOCFLAGS='-D warnings' SQLX_OFFLINE=true cargo doc --workspace --no-deps`
fails. Broken links still name removed `Role`, `records` and `MemoryAsset`
types in `synveda-types`, `synveda-store`, `synveda-audit` and
`synveda-vedaflow`; other public comments link to private items. The
[`EntityStore` header](../crates/synveda-policy/src/entity_store.rs) lines
5–19 says a scope-chain cache is flushed after hierarchy changes, although
the current code resolves chains per request. Several comments cite old
numbered migrations after the epoch-3 baseline cut, and the
[`gateway main` header](../crates/synveda-gateway/src/main.rs) simultaneously
says “embed-or-fail is unconditional” and documents lexical degradation.
These are not evidence of an AI author, but they are misleading architecture
claims. Replace them with present-tense invariants and run strict Rustdoc in
the docs gate after the links are repaired.

## Positive controls and unused-code assessment

The dependency direction holds for all 14 crates. Product libraries forbid
`unsafe`; the CLI's edition-2024 environment mutation appears in tests.
The inspected one-time login selectors are hashed, payloads are sealed under
the deployment key, and consumption is atomic in PostgreSQL. The worker's
Capture profile excludes the known unfenced maintenance loops. The reviewed
gateway request path still gates on database authority and Cedar convergence;
this pass found no confirmed Cedar, forced-RLS or audit bypass.

No accidentally live, removable production path was established by this
source pass. The in-memory login ledger is test-gated, the reserved `synveda
init` command intentionally refuses, and the OpenAPI-only prompt projection
is consciously retained. Strict Clippy is useful for private dead code but
cannot prove reachability of exported API or deployment-only branches. A
release-binary reachability/coverage exercise should precede deleting any of
those paths.

## Validation and limits

- `make check-deps`, `cargo fmt --all --check`, `make check-fast` and
  `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`:
  passed.
- `cargo deny --offline check bans licenses sources`: passed with existing
  duplicate-dependency and unmatched-license-allowance warnings; no new
  licence conclusion beyond those three checks.
- Strict Rustdoc: failed on broken/private intra-doc links as described above.
- No database, load or destructive deployment test was run for this
  review-only change. Prior focused exact-role and Kind results remain
  recorded in the feature briefs and production-readiness inventory.
