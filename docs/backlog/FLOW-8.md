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
both channels. The fine-grained PAT is sealed under the matching
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

The subsequent same-host recovery drill passes through the unchanged canonical
consumer logical-recovery commands. The paired Synveda/Keycloak archives and
original keys restore into a fresh private project; ordinary tenant verification
checks all 139 restored audit events before runtime convergence, opens the
original tenant key and refuses an unrelated key. The original source remains
stopped while an explicit loopback/provider overlay activates the same pinned
gateway image and executable on the restored project. Old CLI refresh is
refused; four fresh browser/PKCE logins preserve the exact issuer, tenant and
subjects. The recovered sealed GitHub secret remains at its original reference
and value revision 1, without reprovisioning. Both channels return the original
exact no-op receipts on first request and replay; independent clones verify
unchanged remote/source heads. The frozen 137-event public audit prefix and all
nineteen prior Git event hashes match, and the public verifier passes a new
147-event snapshot. Four successful export operation/duration samples are
retained. Backup takes 9.065 seconds and canonical restore 236.124 seconds on
macOS/OrbStack; these are local ceremony durations, not RPO/RTO objectives.
Both isolated projects are stopped with the source backup and restored volumes
retained. Private content-free reports remain outside tracked project files.
The required [CI run](https://github.com/synveda/synveda/actions/runs/37895821790)
passes at `65fdc6516d140f1e988d35fb9167889179d54bc9`.

The subsequent credential drill on that retained restored project passes a
genuine rotation between two distinct fine-grained PATs. Both resolve
the exact private repository before the transition. Four fresh real PKCE logins
preserve issuer, tenant and subjects. The stable secret reference, destination
digest and original exact no-op receipts survive value revision 1 to 2. A third
curator-reviewed synthetic publication then advances both channels with
first-request `completed`, exact no-op replays and independent clone verification
of six source commits and three objects per channel. Local custody revocation
at revision 3 removes the envelope key version and refuses both public exports
without changing source or GitHub heads. Resealing the same replacement at
revision 4 restores the exact receipts and verified clones; this is reactivation,
not another distinct PAT rotation. Public custody audit records identify stored
revisions 1, 2 and 4 and cleared revision 3. All 27 baseline Git event hashes
remain intact; the public verifier passes a 210-event snapshot containing 41 Git
export events. Sixteen successful and two expected rejected operation/duration
samples are retained. A new canonical paired backup retains the recovered
replacement and passes verification. The owned fixture is stopped with all
source, restored and backup volumes retained. The operator guide now separates
distinct rotation, local custody revocation and provider revocation. Required
[CI](https://github.com/synveda/synveda/actions/runs/37903182003) passes at
`aef89e78af9ea12a308bbf1eb4426c787bfd3b3b` after rerunning the native ARM64 job
whose pinned Syft download initially failed; binary inspection remains required.

Provider revocation passes after owner deletion of the superseded PAT.
Independent verified HTTPS checks observe old-credential HTTP 401 and
replacement HTTP 200 for the same private
repository. The restarted fixture passes ordinary readiness, running artifact
binding and four new PKCE logins with unchanged identities. Its revision-4
replacement returns the retained exact no-op receipts and verified clones.
Sealing the provider-revoked PAT under that same reference at active revision 5
then refuses both public exports with the uniform credential-unavailable error;
source and GitHub refs remain unchanged despite a usable independent clone
credential. Restoring the replacement at revision 6 recovers all four exact
no-op receipts, the unchanged destination digest and both independently verified
clones. Public custody audit records identify revisions 5 and 6. The frozen
210-event public prefix and all 41 prior Git event hashes match; the verifier
passes a 235-event snapshot containing 49 Git export events. Eight successful
and two expected rejected operation/duration samples are retained. A new
canonical paired backup of revision 6 passes verification; both owned projects
are stopped with all eight volumes retained. Private content-free credential
reports remain separate from the export/replay runner's evidence tier. Required
[CI](https://github.com/synveda/synveda/actions/runs/37914720349) passes at
`4a015e1ae6ecade7e3635dead83b4d2c3fe2bab8`.

The controlled outage/retry drill now passes on the same source gateway and
retained restored fixture, with unchanged active credential revision 6. Fourth
curator-reviewed synthetic revisions advance both source channels. Disconnecting
only the gateway's owned `application-egress` interface makes both GitHub hosts
unreachable while its four internal networks remain unchanged; sampled public
readiness and authenticated API checks pass between attempts. Three Prompt and
two ContextPack requests fail with the public generic internal error. Public
`prepared`, `transport_failed` and `resume_prepared` events retain each exact
source head, Git head, mapping and destination digest. The first Prompt failure
preceded correction of an operator error-message assertion and remains counted.
A gateway restart during the outage preserves that intent and leaves egress
disconnected. The restarted image/executable still match their original hashes.
Reconnecting the original address, aliases and gateway priority restores verified
GitHub HTTPS. Both first successful requests return `resumed` for the frozen
intent; exact no-op replays and independent clones verify eight source commits
and four objects per channel, with no duplicate source history.

The frozen 235-event public prefix and all 49 prior Git event hashes match; a
295-event chain verifies with 69 Git export events. Metrics collected before
and after the process restart respectively record four successes/three errors
and four successes/two errors, with matching duration samples; the restart
resets these process-local counters. A new canonical paired backup of the
recovered state verifies (23.575 seconds for backup, 0.647 for verification on
macOS/OrbStack). Both owned projects are stopped with all eight volumes retained.
Private content-free outage evidence remains separate from the export/replay
runner's tier. Operator preflight corrections are retained with the drill.

Branch-protection preflight now independently receives HTTP 403 for both private
export branches: GitHub requires an eligible plan. No billing or repository
visibility changes are made, and live protection remains unqualified. The
retained fixture passes another four exact no-op receipts, two verified clones
and a 304-event baseline audit snapshot. Preparation then verifies a 331-event
public snapshot; the prior 295-event prefix and all 69 Git event hashes remain
intact. Fifth synthetic Prompt/ContextPack revisions pass ordinary authoring,
distinct curator review and publication. A separate
private recovery repository, ID `1412116922`, provides a new immutable destination
for `github-canary-recovery`.

Preflight discovered that both dedicated PATs covered all owner repositories;
earlier prose overstated the working PAT's repository restriction. With explicit
owner approval, GitHub now restricts the working token to
`sujitn/synveda-git-export-canary` and the distinct recovery token to
`sujitn/synveda-git-export-canary-recovery`. Saved settings and independent
verified-HTTPS checks confirm each destination and refuse cross-canary Git
content access with HTTP 404. Token values and expiry dates are unchanged.
Original sealed custody remains active at revision 6; the recovery credential
is separately sealed at the exact tenant/scope under its own reference/revision 1.

The live append-only divergence and governed new-target recovery drill passes.
Checkpoint refs retain both verified original projections. The independent
operator fast-forwards each export branch to a single-parent, same-tree foreign
commit, without forcing, deleting or resetting any ref. Four public exports
(first request and retry per channel) return the destination-divergence conflict,
leave source/foreign heads unchanged, and preserve exact prepared intent across
`prepared`, `transport_failed`, `resume_prepared`, `transport_failed` audit phases.
Independent clones verify both retained projections and ancestry; both foreign
tips fail projection verification despite their unchanged files.

The new Configuration version and the existing pinned binding's revision 1-to-2
update each pass distinct administrator review and normal VedaFlow apply.
Publishing the version alone correctly leaves the old pin effective. The reviewed
binding change disables `github-canary` and enables `github-canary-recovery`.
Adding its separate repository/secret descriptor and recreating only the gateway
preserves the pinned product artifact and original mapping. Both new-target
exports complete on first request; exact no-op replays and independent clones
verify ten source commits and five objects per channel. Two old-target requests
refuse at the Configuration boundary, and the retained diverged repository stays
unchanged. No old receipt, destination identity or prepared intent is retargeted.

The 295-event public prefix and all 69 prior Git event hashes match; a 377-event
chain verifies with 87 Git export events. Metrics across the two gateway process
intervals total eight successes, six expected rejections and no errors, with
matching duration samples. A new canonical paired backup of the recovered state
verifies (24.527 seconds for backup, 0.500 for verification on macOS/OrbStack).
Both owned projects are stopped with all eight volumes retained. Private
content-free qualification evidence has a verified 60-file SHA-256 inventory,
including the token-scope, expired-login and pinned-binding preflight corrections.
Required [preparation CI](https://github.com/synveda/synveda/actions/runs/37961537501)
passes at `06e994a76c587b36ffb66c847284ee6c9b5af95c`.

The [operator procedure](../integrations/git-bridge.md#qualify-branch-protection-and-divergence)
keeps the old destination and receipts intact. Live protection remains blocked
on an eligible private fixture. Next use fresh reviewed revisions at the active
recovery target, retain a newly prepared receipt during controlled pre-write
egress interruption, and qualify paired restore/resumption into a separate owned
project using the existing recovery custody and exact frozen projection. The
measured pre-write interruption, process restart and append-only divergence do not
qualify live acknowledgement loss after a provider write, that prepared-receipt
restore window, a live force-push/deletion race, a published deployment or
encrypted off-host recovery.
Owned egress/retention also remains open.
Large-export deadlines and gateway connection pressure remain unmeasured.
Both transports stay disabled by default; real credentials and writes are
enabled only in this explicit synthetic operator fixture.

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
