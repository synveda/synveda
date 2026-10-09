# FLOW-8: Git bridge — export

## Problem and evidence

VedaFlow provides immutable content-addressed objects, commits, signatures when present, and governed channels for Prompt and ContextPack assets. Reviewers need inspectable Git history without losing source evidence or treating VedaFlow signatures as Git-native signatures. Local bare-repository and pinned private GitHub transports are implemented; credentialed provider/deployment qualification remains open.

### Current implementation and next action

[ADR-0138](../adr/adr-0138-governed-git-export.md) is Accepted. The local leaf,
public export endpoint, CLI export/offline verification, governed destination
allowlist, fresh channel/asset authorization, bounded source graph, recoverable
VedaFlow receipts, content-free audit and telemetry are implemented. See
[the operator contract](../integrations/git-bridge.md) and
`demos/flow-8-git-export.sh` for runnable local acceptance.

On 2026-10-04, four exact-role gateway acceptance tests pass on a fresh current
database, with no skips, including real clones and source-history comparison.
Five native Git integration tests plus two subprocess timeout/cancellation
tests pass. Generated API/client artifacts and the fresh-database SQLx
prepare/check/migration gate are current. These are source/local results,
not remote or published-package qualification.

[ADR-0139](../adr/adr-0139-private-github-git-export.md) selects GitHub.com with
an exact deployment tenant/scope/target mapping, immutable repository ID,
same-scope encrypted `import_export` secret, `github` Configuration provider
narrowing and verified HTTPS. Source transport now sends deterministic packs
with atomic old-head preconditions, explicit unpack/ref reports and exact-head
rediscovery. Receipt identity digests prevent silent destination/custody
retargeting; token rotation preserves the stable secret reference.

On 2026-10-05 the complete runnable demo passes: six exact-role API acceptance
tests and eleven Git projection/transport tests, no skips. Controlled verified
HTTPS with the real native Git HTTP backend covers both families, no-op/retry,
lost or malformed acknowledgement, old-head races, remote force-push/deletion,
privacy/repository-ID refusal, TLS trust, redirects, outages and response bounds.
API tests add provider narrowing, absent/revoked/corrupt/wrong-kind/wrong-scope/
wrong-provider/foreign/repository-mismatched credentials and receipt retarget
refusal. These use synthetic credentials and no real GitHub traffic.

On 2026-10-06, `scripts/run-git-export-canary.mjs` prepares the public-CLI live
export/replay and independent-clone gate. It pins both synthetic source heads,
tenant/scope/target, repository ID and actual CLI bytes, and records only checked
content-free receipts with explicit incomplete/provider evidence tiers. Thirteen
controller tests pass without live traffic, including pre-effect privacy,
identity, Configuration and source-state refusal, replay/destination drift,
failed verification, echoed-secret suppression and temporary-clone cleanup.
The existing GitHub CLI login is usable for independent reads/clones, but its
broad token is not an admissible repository-restricted export credential. No
canary repository, restricted PAT file or deployment was selected; no external
Git writes were performed. See the operator guide's live canary contract.

On 2026-10-08, pre-commit validation passes again: 290 focused core unit tests,
all seventeen Git/API acceptance cases on a fresh disposable exact-role
database, six OpenAPI peer tests and twenty demo/canary controller tests.
Formatting, strict Clippy for the changed Rust crates, workspace Rustdoc with
warnings denied, and the fast/dependency/demo repository gates pass. The fixture
is removed after success. This adds current source evidence only; live provider
and deployment qualification still requires the inputs below.

The current review follow-up corrects the product image's cached manifest graph,
advertised symbolic-destination refusal and the transport-specific stop guidance.
Eighteen Git/API acceptance cases now pass, including the symbolic-ref regression
that permits a direct branch's ordinary HEAD alias. All thirteen canary-controller
cases pass in the pinned Linux Node 22 runtime without network access. Native
macOS controller and broader deployment fixture reruns hit subprocess deadlines;
those reruns are not passing validation. The complete required
[CI run](https://github.com/synveda/synveda/actions/runs/37844684400) passes at
`98241b902a4e3773e9321c54eb5454169577d720`, including all six native CLI targets
and all three Helm deployment jobs. Required checks on
[PR #70](https://github.com/synveda/synveda/pull/70) must pass before merging.
Provider/deployment qualification must also measure large-export upload deadlines
and connection pressure: transport currently retains a gateway tenant transaction
until completion. The initial synthetic canary should run serially.

On 2026-10-09, the dedicated
[sujitn/synveda-git-export-canary](https://github.com/sujitn/synveda-git-export-canary)
repository was created and independently checked as private, empty, non-fork,
unarchived and enabled, with immutable repository ID `1411339644`. The isolated
loopback Docker fixture now passes its ordinary database, issuer and runtime
readiness gates. Four synthetic users sign in through real OIDC/PKCE; public
Configuration proposals and scope bindings enable only `github-canary` for the
dedicated scope, and curator-reviewed Prompt/ContextPack publications supply
both channels. The repository-restricted PAT is sealed under the matching
tenant key; its deployment descriptor contains only the stable secret reference.
The existing development hostname marker remains untouched.

Initial live writes exposed a GitHub framing incompatibility: successful
receive-pack status reports end with two flush packets. The strict parser
rejected the second terminator, leaving prepared receipts despite the remote
write. A same-head, empty-pack probe confirms that framing without changing
the ref or retaining provider text. Retrying the Prompt export resumes its
prepared head, and an independent clone verifies its retained evidence.
Two regression cases reproduce the failure before the parser change, then
pass with the bounded one-or-two-flush grammar; all fourteen Git adapter tests
pass, including real native Git/TLS interoperability and refusal/recovery cases.

The corrected source at `29566928bb3cbaaa4fccc08d9c5aecf3d477b7ce` now passes
the complete live canary. Its locally built Linux/ARM64 gateway is independently
bound to OCI digest
`sha256:128b38a9c6177ac8f30fd751a504676faca2945b3122cf223e27ac325fe79d82`
and the running executable hash. The explicit operator gateway/provider
override preserves the original consumer installation, database and keys;
this is source-candidate evidence, not published consumer upgrade qualification.
The initial runner recovers the remaining prepared ContextPack receipt and
verifies both clones. After a second governed synthetic publication, both fresh
exports return `completed` on their first request, both exact replays return
`no_op`, and independent clones verify four source commits and two objects per
channel. Repository identity/privacy and quiescent source heads are rechecked.
The public audit verifier passes a 137-event snapshot; nineteen content-free
Git export events include prepared, failed, resumed, completed and no-op phases.
The corrected gateway records eight successful operations and duration samples
with no error counter. Private reports, artifact bindings and audit/metric
evidence are retained outside tracked project files. The isolated Docker
project is stopped with its paired installation/database/browser volumes retained.

Next provision a second restricted PAT for a genuine same-reference rotation
and revocation drill. Remaining qualification also needs owned egress/retention
and isolated recovery controls, with separately retained outage/retry,
branch protection/divergence and paired database/key recovery against the chosen
deployment artifact. The basic serial canary does not qualify provider failure
recovery, a published deployment, external retention or joint recovery. Both transports stay
disabled by default; real credentials and writes are enabled only in this
explicit synthetic operator fixture.

## Scope

- Export authorized Prompt and ContextPack channel history to a configured private Git repository with deterministic paths, byte-stable files, commit ordering, authorship labels, and timestamps.
- Persist a manifest mapping each Git object/commit to the exact VedaFlow asset, object hash, commit hash, parent set, channel/ref state, policy snapshot hash, and signature/key identifier when present.
- Preserve VedaFlow signatures as verifiable evidence files; create a Git signature only when an independently configured Git signing identity actually signs the Git commit.
- Make repeated and resumed exports idempotent, detect remote divergence/force-push, and refuse destructive reconciliation without an explicit governed reset.
- Reauthorize every exported asset/ref and record the external disclosure destination and result in content-free audit evidence.

## Non-goals

- Git-to-Synveda import, bidirectional round-trip, Git as authority, or pull-request approval as a VedaFlow approval.
- Exporting Knowledge, Skills, Tools, Policy, or Configuration as if they used VedaFlow channels; Knowledge exchange remains OKF.
- Embedding remote credentials in commits, manifests, configuration documents, logs, or audit metadata.
- Claiming a VedaFlow Ed25519 signature is a native Git commit/tag signature.

## Architecture seam

Add an export application service above `synveda-vedaflow` and a replaceable Git transport adapter outside core domain crates. The service reads exact authorized channel/commit/object history in ordinary tenant transactions, renders a canonical projection, then performs the external write with a secret-plane credential. Mapping and export cursors are tenant-bound governed state; SQL remains in `synveda-store`.

## Acceptance criteria

- A published Prompt and ContextPack history exports to a real Git repository with deterministic trees, parent topology, refs, and a complete hash mapping.
- A verifier can independently validate every retained VedaFlow object/commit hash and signature from the export without mistaking it for Git-native signing.
- Repeating or resuming the same export produces no extra commits; changed history advances only the intended ref.
- Revoked authorization, tenant mismatch, missing secret, non-fast-forward remote, size bound, and network failure stop safely without credential/content leakage or corrupted refs.
- The audit chain identifies actor, tenant-bound export target identifier, asset/channel, source head, outcome, and mapping digest, but no exported content or secret.

## Required tests

- Canonical projection and hash/signature verification fixtures for Prompt and ContextPack histories, merges, pins, and rollback commits.
- Local real-Git end-to-end tests for first export, no-op replay, resume, divergence, force-push refusal, and credential failure.
- Cedar allow/deny/revoke and forced-RLS cross-tenant tests at asset and channel boundaries.
- Size/path/encoding, malicious name, secret-redaction, cancellation, timeout, and bounded-work tests.
- Runnable demo that clones the result and verifies the mapping against source VedaFlow history.

## Rollout and rollback

Ship local bare-repository export first, then one private remote provider behind an allowlist and disabled-by-default configuration. Canary with non-sensitive assets. Rollback disables outbound writes and freezes the last mapping cursor; it does not delete or rewrite the remote repository, and VedaFlow remains authoritative.

## Dependencies

An accepted ADR must fix canonical projection, merge/ref mapping, disclosure authorization, signature semantics, and divergence recovery. The owner must choose Git implementation/licence, remote provider, tenant-safe repository/branch naming, credential custody, egress policy, retention, and who may enable or reset an export.
