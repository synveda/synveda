# Governed Git export (FLOW-8)

The bridge exports the complete bounded ancestry of a Prompt or ContextPack
channel into an operator-owned bare Git repository or pinned private GitHub.com
repository. PostgreSQL and
VedaFlow remain authoritative. The public endpoint is
`POST /v1/channels/{scope_id}/git-export`; the CLI uses that same endpoint.
Import and forge reviews are outside this export contract.
[ADR-0138](../adr/adr-0138-governed-git-export.md) fixes its evidence and recovery
contract; [ADR-0139](../adr/adr-0139-private-github-git-export.md) adds the private
GitHub transport. Credentialed provider/deployment qualification remains tracked
in [FLOW-8](../backlog/FLOW-8.md).

## Enable a local destination

Run the gateway on a Unix host with installed Git and a retained directory owned
by the gateway's operating-system user, mode `0700`. Use an absolute physical
path with no symlink components. Set `SYNVEDA_GIT_EXPORT_ROOT` to that directory
before gateway startup. An unset variable disables local export; an invalid root
refuses startup. The current bundled product image does not provide Git or an
export volume, so this rollout requires an operator-prepared native gateway.

Publish a complete governed Configuration document containing a sorted, unique
`git_export_targets` array, for example `["review"]`, through the existing
Configuration proposal/review/apply lifecycle, then bind it at the channel
scope. The array defaults to empty and allows at most sixteen identifiers.
Identifiers are 1–48 lowercase ASCII letters, digits or hyphens, beginning
with a letter or digit. They contain no paths, URLs, credentials or forge
settings. Inspect the selection with `synveda configuration effective SCOPE
--json`. Configuration enables a destination but grants no access.

Built-in packs require a scoped administrator's `channel.export` permission,
`channel.read`, and sensitivity-aware read permission for every historical
object at its original scope. A private or currently denied ancestor refuses
the entire export, even if the head alone is readable. Custom packs must
explicitly permit `Synveda::Action::"ChannelExport"` with its scope context.

```sh
synveda git-bridge export SCOPE --target review --channel prompt/published
synveda git-bridge export SCOPE --target review --channel context-pack/published
```

`prompt/staged` and `context-pack/staged` are also supported. Each destination
lives at `ROOT/TENANT/SCOPE/review.git`; the response supplies the exact
`git_ref`, `git_head`, source head/pin and mapping digest without filesystem
paths or source content. A successful retry returns `no_op` or `resumed`.
Export is explicitly requested; no background synchronizer is installed.

## Enable a private GitHub destination

Use a dedicated, existing, non-fork private GitHub.com repository. Record its
immutable numeric repository ID independently. Its owner must restrict changes
to visibility, access and retention. Metadata checks establish observed privacy;
GitHub's settings API and Git push cannot lock visibility together. Protect
export branches as appropriate; provider protections are respected and never
bypassed. GitHub Enterprise, SSH, repository creation, proxy and private-CA
configuration are outside this transport's contract.

Provision a fine-grained PAT restricted to that repository, with Metadata read
and Contents read/write permissions and an operator-managed expiry/rotation.
Keep this credential document in a private file or provide it on stdin:

```json
{
  "format": "synveda-github-export-credential-v1",
  "repository_id": 123456789,
  "token": "github_pat_REPLACE_WITH_THE_REPOSITORY_TOKEN"
}
```

Store it through the existing operator secret boundary, using the deployment's
configured KMS and exact tenant/scope. The command prints only metadata, including
the stable `synveda-secret://` reference. No credential is accepted in argv.

```sh
synveda tenant secret put --tenant TENANT --scope SCOPE --kind import_export \
  --label git.review --provider github --from /private/github-export.json
```

Set `SYNVEDA_GIT_EXPORT_REMOTES_FILE` to a deployment-owned JSON allowlist before
gateway startup. This file contains descriptors, never token values:

```json
{
  "format": "synveda-github-export-targets-v1",
  "targets": [
    {
      "tenant_id": "00000000-0000-0000-0000-000000000001",
      "scope_id": "00000000-0000-0000-0000-000000000002",
      "target": "github-review",
      "owner": "example-owner",
      "repository": "context-export",
      "repository_id": 123456789,
      "secret_reference": "synveda-secret://00000000-0000-0000-0000-000000000003"
    }
  ]
}
```

Replace every example ID/name with the exact provisioned descriptor. Owner and
repository names are lowercase ASCII; repository names omit `.git`. At most 128
mappings and 64 KiB are accepted. Duplicate tenant/scope/target mappings, shared
repository IDs/names, external secret references and caller-selected URLs refuse
startup. A remote target reserved for another tenant/scope never falls back to
local export. Changing an existing receipt's destination or stable credential ID
refuses; create a new governed target for different custody.

Publish/bind a complete governed Configuration allowing the target in
`git_export_targets` and `github` in `allowed_external_providers`. Both defaults
exclude outbound GitHub disclosure. The same scoped administrator, channel and
every historical asset read permissions described above are required. The
secret must be active, of kind `import_export`, provider `github`, and owned by
the exact tenant/scope; its sealed document must bind the pinned repository ID.
Missing/revoked/corrupt/foreign references share one non-oracular error. There is
no fallback token.

```sh
synveda git-bridge export SCOPE --target github-review --channel prompt/published
git clone --branch synveda/prompt/published https://github.com/example-owner/context-export.git review
synveda git-bridge verify review --channel prompt/published
```

Use your own Git credential mechanism to clone; Synveda never returns its token.
The export response adds `provider`, `destination_digest` and `repository_id`.
On each request the gateway opens current custody, verifies private repository
identity, discovers the branch and sends exactly one old-head-bound Git
receive-pack update. An explicit successful unpack/ref report and rediscovery
confirm completion. A lost/malformed acknowledgement retains prepared intent;
retry accepts the exact prepared remote head without another commit or push.
No force or deletion command exists.

Runtime egress is restricted to constructed HTTPS requests to `api.github.com`
and `github.com`, port 443, with public-PKI hostname verification. Redirects and
environment proxies are disabled. Apply those same destinations to the
deployment's network policy/firewall. Responses are bounded to 256 KiB, Git
advertisements to 2,048 refs; connections have a five-second deadline and each
HTTP request fifteen seconds, within the existing sixty-second export deadline.
During export, tokens remain in memory, are redacted/wiped, and never reach native
subprocess arguments, environment, transport files, exported history or audit.
Remote export needs
no installed Git or local root in the gateway; offline verification still uses
installed Git. Source tests do not qualify a published image or deployment.

An export currently retains one pooled gateway database connection during
transport. Run the initial canary serially and measure slow-export connection
pressure before enabling concurrent use on the selected deployment. The
request deadline bounds an export, but does not reserve capacity for other API
requests. Upload throughput within the fifteen-second HTTP deadline also needs
qualification with that deployment's expected export sizes.

Rotate by repeating the same `tenant secret put` command and label, preserving
the reference; revoke with `synveda tenant secret revoke --tenant TENANT SECRET_ID`.
To stop writes, remove the target/provider from governed Configuration or remove
its deployment mapping and restart. Retain remote history, database receipts and
tenant keys together. External retention, deletion and joint recovery remain
operator responsibilities and require a provider/deployment drill.

## Qualify credential rotation and revocation

Use the dedicated synthetic fixture and retain the exact destination descriptor,
running gateway/CLI hashes, source heads and completed receipts. Obtain a second,
distinct fine-grained PAT restricted to the same repository and permissions.
An unchanged export alone does not prove that the replacement can write.

Repeat `tenant secret put` with the same tenant, scope, kind, label and provider,
reading the replacement document from a private file or stdin. Check that the
returned secret ID/reference stays fixed and its value revision increases.
The descriptor and destination digest must remain unchanged. Both first exports
and replays of the existing heads must return the original exact `no_op`
receipts, and independent clones must still verify. Then author, review and
publish fresh synthetic Prompt and ContextPack revisions through their ordinary
public workflows. Require both exports to complete, exact no-op replays and
independent verification of the expanded source history. Git export audit
events must identify the replacement's value revision without its value.

Revoke that stable secret through `tenant secret revoke`, then confirm that both
public exports refuse unavailable custody and source/remote heads remain fixed.
The operator metadata must report a revoked state and no envelope key version.
Put the working replacement document under the same label to recover, and
require the original receipts and clones to verify again. Reusing that same PAT
is reactivation; its increased value revision is not another distinct token
rotation.

Separately, the GitHub owner must revoke the superseded PAT after replacement
writes have passed. Require old-credential HTTP 401 and replacement HTTP 200 for
the exact private repository. In the isolated synthetic fixture, seal the
provider-revoked document under the stable reference
and check that `tenant key status --tenant TENANT` still reports active custody
at the expected new value revision. Then confirm that both public exports refuse
with the uniform credential-unavailable error without moving either ref. An
already-revoked Synveda secret would exercise only the local custody check.
Restore the working replacement and verify exact receipt/clone recovery.
Synveda secret revocation and GitHub PAT revocation are separate checks; neither
substitutes for the other. The gateway must never fall back to the independent clone user's
GitHub CLI credential.

Retain content-free custody metadata, receipts, Git event hashes, public audit
verification and operation/duration metrics in an enclosing credential report.
The export/replay runner's report does not itself measure these transitions.
Preserve the recovered replacement in a new verified paired backup, retaining
the operator destination descriptor separately. A historical backup can contain
a superseded PAT; restoring it does not restore that PAT's GitHub authority.

## Qualify controlled outage and retry

Use the dedicated synthetic fixture, bind the running gateway/CLI artifacts and
retain completed receipts, independent clone results and credential metadata.
Author, distinctly review and publish new Prompt and ContextPack revisions
through their ordinary public workflows, then quiesce writers and record both
heads. An unchanged completed export exercises no-op failure; it cannot prove
that a new prepared receipt survives interruption.

On the canonical Compose graph, the gateway's `application-egress` network is
separate from its internal database, identity, proxy and telemetry networks.
Record the owned gateway container ID and every network attachment, including
the egress address, aliases and gateway priority. Arrange reconnection in the
operator's cleanup before disconnecting only that container from the egress
network. Verify the internal attachments stay unchanged and bounded probes to
both `api.github.com` and `github.com` fail. Sample public readiness and an
authenticated API between export attempts; independently observe GitHub heads
through the separate clone credential.

Request both exports through `synveda git-bridge export`, which already emits
JSON and accepts no `--json` flag. Require failure without changed source/remote
heads, plus public `prepared` and `transport_failed` audit events binding the
exact source head, Git head, mapping, destination and active secret revision.
Internal transport errors expose the generic public internal error; the audit
phase identifies the transport failure. Restart the owned gateway while egress
remains disconnected, recheck artifact binding, topology and ordinary readiness,
then repeat both exports. Require `resume_prepared` and `transport_failed` with
the same frozen evidence. Capture metrics before restart because process-local
counters reset; retain and verify operation/duration counts for both intervals.

Reconnect using the recorded address, every alias and original gateway priority.
Verify the restored topology and ordinary verified GitHub HTTPS, then run the
public-CLI canary against the recorded new heads. Both first successful requests
must return `resumed` matching the prepared evidence, both replays must return
the exact `no_op`, and independent clones must verify the expected expanded
history without duplicate source commits. Verify the public audit chain and
prior event hashes, preserve a new verified paired backup, and stop the owned
fixture with its volumes retained.

Keep this enclosing outage report separate from `live_github_export_replay`.
Blocking egress before a write proves that interruption case; live lost
acknowledgements after a write, remote divergence/protection and a paired restore
with prepared intent require separate drills. This fault injection does not
qualify an owned production egress policy, retention, RPO/RTO or other hosts.

## Qualify branch protection and divergence

Start with the dedicated synthetic fixture, pinned gateway/CLI artifacts,
current credential metadata, exact receipts, source heads and independently
verified clones. Inspect the repository's actual protection capability and
existing rules before changing them. GitHub requires an eligible paid plan for
[protected private branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches).
A provider capability refusal is an unqualified gate; retain private visibility
and select a protection-capable private fixture before claiming that case.

For protection, use a temporary requirement on the exact synthetic export
branches that the export credential cannot bypass, including administrators.
Record the original rule and an unsatisfied required check or review. Publish
fresh synthetic revisions through ordinary authoring and distinct review, then
require public export refusal without source/remote movement. Retain matching
`prepared`, `transport_failed` and retry audit phases. Restore only the temporary
rule or satisfy its requirement, then require recovery of the same prepared
intent, exact no-op replay and independent clone verification. Inspect the
provider result; a transport error alone does not prove protection enforcement.

For divergence, first prepare a separate private repository, immutable repository
ID, new target identifier and distinct repository-restricted credential. Verify
the token's selected repositories and relevant Contents permission; metadata
visibility alone does not establish its content restriction. Keep the new target
inactive until refusal is measured. Retain checkpoint refs at the original
verified tips. Through the independent fixture operator, fast-forward each
synthetic branch with a single-parent, same-tree commit whose parent is that
original tip. Record the exact parent/tree/head and `force: false`. This preserves
all source history while presenting an unexpected head to the exporter.

With newly reviewed source revisions ready, repeat both public exports. Require
the destination-divergence conflict, unchanged source and foreign remote heads,
and exact prepared intent across `prepared`, `transport_failed`,
`resume_prepared`, `transport_failed` events. Independently clone the retained
checkpoints and verify their original evidence. The same-tree foreign tips must
fail projection verification: matching files do not make an arbitrary commit a
valid export state. The verifier uses canonical channel refs; a checkpoint can
be verified by creating an absent canonical alias in a fresh independent clone,
without rewriting any retained or remote ref.

Recover through the normal reviewed Configuration lifecycle: disable the old
target and enable the separate new identifier. Inspect the existing scope binding:
if it pins the old version, publishing the new document does not change effective
Configuration. Submit an exact-revision binding update pinning the approved new
version, obtain its required distinct review, and apply it through VedaFlow.
Verify the effective version, target list and binding revision before exporting.
Seal the new credential through
the exact tenant/scope secret boundary, add its distinct repository mapping and
restart the gateway using the same artifact. Require two first-request
`completed` exports, exact no-op replays and independently verified full source
history at the new destination. Confirm old-target requests refuse at the
Configuration boundary and the retained diverged destination stays unchanged.
Neither an old receipt nor its repository identity is retargeted or reset.

Retain both custody references, public audit continuity and metrics from each
process interval when descriptor activation recreates the gateway. Preserve the
completed recovery state in a new verified paired backup, retaining destination
descriptors separately. Report protection capability, append-only divergence
and new-target recovery separately. This drill does not measure a live
force-push/deletion race, prepared-receipt paired restore, post-write
acknowledgement loss or off-host recovery.

## Qualify recovery on the selected deployment

Use the deployment's existing paired database/key recovery ceremony. The
[Compose logical recovery guide](../../deploy/compose/README.md#logical-backup-and-isolated-restore)
owns the source lifecycle prerequisites; the
[consumer named-volume command](../../deploy/compose/scripts/consumer-recovery.sh)
restores only into a different empty project with an exact source/backup/target
confirmation. Retain the running gateway artifact and deployment-owned GitHub
destination descriptors separately from the database/key set. An operator
provider override is not part of the canonical consumer configuration backup.

Before backup, quiesce synthetic source writers and retain the exact source
heads, completed export receipts, independently verified clones and a frozen
public audit prefix. After restore, verify the original tenant key and the
expected unrelated-key refusal through the ordinary tenant recovery command.
Reapply the exact destination descriptors and qualified gateway artifact, then
independently bind the running image and executable to their original hashes.
Keep the source stopped while using its original loopback issuer in the restored
fixture. If CLI refresh is refused, repeat real OIDC/PKCE login and check that
issuer, tenant and subject identities are preserved.

Run the public-CLI canary against the unchanged source heads using the restored
sealed secret under its original reference. Both first requests and replays
must return the original exact `no_op` receipts, and independent clones must
verify unchanged Git heads and evidence. Verify the restored public audit chain,
compare the frozen prefix and prior Git event hashes, and retain content-free
metrics. Keep this enclosing recovery report separate from the canary runner's
`live_github_export_replay` report, which does not itself measure recovery.

A completed-receipt replay proves only that recovery case. Qualify a prepared
receipt separately in the procedure below, requiring the same frozen projection
to resume without extra commits. Same-host logical recovery does not measure
encrypted off-host custody, PITR, production RPO/RTO or a published deployment.
Stop only the owned fixture projects and retain their paired state and verified
backup until the recovery work is complete.

### Qualify prepared intent across paired restore

Start with exact no-op receipts and independently verified clones at the active
governed target. Record its Configuration version/hash, sealed-secret reference/
revision/key, source and remote heads, gateway/CLI/descriptor hashes and frozen
public audit prefix. Author, distinctly review and publish fresh synthetic
revisions for both channels, then quiesce source writers.

For the pre-write case, use the controlled egress procedure above to interrupt
only the owned gateway's GitHub access while keeping public authentication and
internal dependencies available. First requests and retries must fail without
moving source or remote heads. Require `prepared`, `transport_failed`,
`resume_prepared`, `transport_failed` events per channel, with identical source,
Git head, mapping, destination, Configuration and secret revision. Export and
independently verify the entire frozen public audit prefix. Save metrics before
the source process stops.

Restore the exact network attachment and verify ordinary HTTPS, then make no
further export request on the source. Create and verify a new immutable paired
backup through the canonical ceremony. Keep that source stopped and restore
into a different empty owned project with exact source/backup/target confirmation.
Require ordinary tenant audit/key verification and unrelated-key refusal.
Apply the retained provider descriptor and qualified gateway artifact separately
from the canonical backup, and bind the actual running image, executable and CLI
to the original hashes. Fresh PKCE logins must preserve issuer, tenant and
subjects. Before resumption, verify unchanged source/remote heads, exact effective
Configuration and original secret metadata without resealing. Compare every
event of the frozen public prefix and every prepared Git event hash.

The restored first requests must return `resumed` with the prepared source,
Git head, mapping and destination digest; exact replays must return `no_op`.
Independent clones must verify all expected history without duplicate commits.
Require `resume_prepared`, `resumed`, `no_op` audit phases with the original
identity, verify the complete final audit chain offline and retain metrics from
both process intervals. Preserve a separate completed-state paired backup and
stop only the owned fixtures with their volumes retained.

Report this pre-write case separately from a prepared receipt after a provider
write whose acknowledgement was lost. That case additionally requires observed
remote advancement before backup and exact frozen-projection recovery without
another commit. Keep the enclosing recovery report separate from the runner's
export/replay evidence tier, and retain the same-host/published-artifact/off-host
qualification limits above.

### Qualify post-write response loss across a gateway restart

After distinctly reviewed synthetic publication, use a temporary fault confined
to the owned gateway to drop GitHub's response after forwarding the export pack.
Keep public-PKI hostname verification enabled; an opaque TCP relay can inject
loss without decrypting TLS. Both public exports must fail within their deadline,
retain identical `prepared`/`transport_failed` intent, and leave source heads
unchanged. Independently read GitHub to prove each remote head is already the
exact prepared result, and retain content-free proof that the responses were
dropped. Sample ordinary public readiness/authentication and save metrics.

Remove the fault and restart only the owned gateway with its normal configuration.
Check its artifact/descriptor hashes, effective Configuration and original sealed
custody. Both retries must return `resumed`, exact replays must return `no_op`,
and independent clones must verify all expected history. GitHub heads must stay
unchanged during recovery. Verify the complete public audit prefix and exact
`resume_prepared`/`resumed`/`no_op` evidence, retain metrics from both process
intervals, then preserve a canonical paired backup and stop the owned fixture.
This restart case is separate from a post-write paired restore.

## Live GitHub canary

`scripts/run-git-export-canary.mjs` runs the first live provider check on Linux
or macOS with Node 22+, Git, an authenticated GitHub CLI and the current Synveda
CLI. Provision the repository-restricted export credential and governed
target/provider using the commands above. The runner uses the selected Synveda
credential profile for public API calls. Independent metadata reads and clones
use GitHub CLI authentication; that login supplies no gateway export credential.

Use a dedicated canary tenant/scope with only non-sensitive synthetic Prompt and
ContextPack history. Publish both channels through the existing governed review
flow, quiesce all source writers, and record their exact heads with
`synveda channel status SCOPE --profile git-canary --json`. Both published
channels must be unpinned. The fixture must remain quiescent for the whole run;
the export API freezes current source state and has no caller-supplied source
head precondition.

Build/select the CLI to test and independently record the running gateway's
artifact SHA-256 through the selected deployment's diagnostics. A native binary
hash or OCI digest is accepted as the operator pin. The report labels this pin
as operator supplied; bind it independently to the running gateway before
claiming deployment qualification.

```sh
SQLX_OFFLINE=true cargo build -p synveda-cli --bin synveda
shasum -a 256 target/debug/synveda
```

Prepare a local JSON file with this contract. Replace every example identifier,
digest and source head with the selected fixture's recorded values. The
`synthetic_scope` field declares the fixture boundary; it grants no authority.
Token values and unknown fields refuse.

```json
{
  "format": "synveda-github-canary-v1",
  "profile": "git-canary",
  "tenant_id": "00000000-0000-0000-0000-000000000001",
  "scope_id": "00000000-0000-0000-0000-000000000002",
  "target": "github-canary",
  "repository": "example-owner/context-export-canary",
  "repository_id": 123456789,
  "deployment_artifact": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
  "cli_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "source_heads": {
    "prompt/published": "1111111111111111111111111111111111111111111111111111111111111111",
    "context-pack/published": "2222222222222222222222222222222222222222222222222222222222222222"
  },
  "synthetic_scope": true
}
```

For signed source history, add an absolute `trusted_keys_file` path containing
the independently trusted public-key mapping described below. Set `SYNVEDA_BIN`
to the selected CLI's absolute path when it is outside `target/debug/synveda`.
Use a new evidence directory beneath an existing private operator directory:

```sh
node scripts/run-git-export-canary.mjs /private/flow8-canary.json /private/flow8-evidence
```

Before export, the runner checks actual CLI bytes, caller tenant, effective
Configuration, both recorded source heads, and observed GitHub repository
identity/privacy. It sends one export and one no-op replay per channel through
the public CLI, independently clones both branches, compares the Git heads and
verifies all retained source/mapping evidence. It rechecks repository metadata
afterwards. Commands have a sixty-second deadline and bounded captured output;
GitHub's reported repository size also gates this small canary. That reported
size is a preflight check rather than a transfer or disk quota.

The private `report.json` contains validated IDs, hashes, counts and outcomes.
It records source revision/dirty state and actual CLI SHA-256, plus the operator
gateway pin. Successful output is specifically `live_github_export_replay`.
Failures retain `partial.json` with the last attempted phase; retry the same
fixture to recover its existing prepared receipt. Temporary clones are removed.
The runner never stores source content, provider bodies, bearer values or PATs
in its evidence. Invocation without inputs exits 77 (`PENDING`), which is not
a passing live check.

Complete qualification still needs separately retained rotation/revocation,
outage/uncertain-acknowledgement retry, branch-protection/divergence,
deployment egress/retention and paired database/key recovery evidence. After a
joint restore into an isolated destination, select its authenticated CLI profile
and rerun with the same retained tenant/scope/target and recorded source heads.
Compare the destination digest, Git heads and mapping digests against the
original report; resumption must preserve repository/secret identity. Use the
existing deployment recovery procedures and their ownership safeguards.

## Inspect and independently verify

```sh
git clone --branch synveda/prompt/published /absolute/review.git review
synveda git-bridge verify review --channel prompt/published
```

Files under `assets/` are the exact canonical source JSON bytes. Their filenames
hex-encode UTF-8 source names, segmented into components of at most 120 hex
characters with a final `.json` suffix. No source name becomes a checkout path.
Each mapped source commit contains `.synveda/commit.json` and `tree.json`,
including policy fingerprint, original identity and microsecond timestamp.
Git author labels use that identity ID; Git timestamps retain whole seconds.
Source parent order and merge topology are preserved.

The branch tip is an unsigned export-state commit containing
`.synveda/export.json`: the complete source snapshot, BLAKE3 evidence digest,
source-to-Git blob/tree/commit maps, channel head/pin and previous exported state.
Its first parent retains the preceding export state. This lets a source rollback
or pin change advance Git without rewriting the original source DAG.

For signed source evidence, supply a JSON object mapping independently trusted
key IDs to arrays of exactly 32 public-key bytes:

```sh
synveda git-bridge verify review --channel prompt/published --keys trusted-public-keys.json
```

Verification recomputes every source object/tree/commit hash, every mapped Git
object and the manifest digest. Every signed commit requires a trusted matching
Ed25519 key. Unsigned evidence remains unsigned. These signatures are retained
VedaFlow evidence, never native Git signatures. Verification needs neither
database nor bearer. It validates the selected branch's retained export-state
chain and its mappings;
it does not establish that an independently supplied key is trustworthy or that
the exporter currently holds authority.

## Retry, divergence and rollback

A content-free VedaFlow receipt and audit event commit before any Git content
write. The gateway rechecks current Configuration and Cedar before that write.
A prepared receipt survives a lost response, process cancellation or database
completion failure. Retry the same command: it resumes the recorded exact
source state first, even if the live channel advanced. Request another export
afterwards to publish the newer source state.

Git atomically compares the observed branch with the recorded old head or the
exact prepared result. A changed/deleted ref, force-push, deleted repository,
symbolic local ref, corrupt local object, changed destination custody, or
non-private destination refuses. There is no
destructive reconciliation/reset endpoint. Retain the old repository and
receipt; approve a new target ID through Configuration for a separate export.

To stop future writes, remove the target from effective governed Configuration.
Unsetting `SYNVEDA_GIT_EXPORT_ROOT` and restarting stops local export only. For
private GitHub destinations, remove the governed target or `github` provider,
or remove the deployment mapping and restart. Existing Git content and receipts
are retained; revocation cannot erase previously disclosed bytes. Retain Git
repositories, PostgreSQL receipts and tenant keys together, and verify before
resuming. Automatic pruning and disaster recovery are not qualified here.

Each snapshot admits at most 128 commits (all parents plus pin ancestry), 1,024
distinct objects, 2,048 entries per tree and 8 MiB of serialized source evidence.
Oversized histories refuse before preparing disclosure. Each native subprocess
has bounded input/output and a 15-second timeout; the request has a 60-second
deadline. Cancellation kills its active native child, preserving any prepared
receipt. Git configuration, hooks, replacement objects and network protocols
are isolated or disabled during transport/verification.

Each target admits at most 1,024 retained export states and 64 MiB of cumulative
manifest bytes. Both export and offline verification enforce this bound;
verification checks historical signatures even after source rollback. Overflow
refuses; retain the old target and approve a new one through Configuration.

Audit action `vedaflow.git.exported` records actor, target ID, asset/channel,
source head/pin, Git head, mapping digest, governing Configuration and phase.
It also records provider/destination digest and the stable secret ID/value
revision when remote custody is used.
It excludes messages, source content, paths and credentials. Metrics
`synveda_git_export_operations_total` and `synveda_git_export_operation_seconds`
use only the closed response outcome labels.

## Runnable acceptance

```sh
sh demos/flow-8-git-export.sh
SQLX_OFFLINE=true cargo test -p synveda-git
```

The demo owns a disposable exact-role PostgreSQL fixture. It exports both
families through the authenticated API, clones their branches, compares mapping
evidence with source VedaFlow history, and checks replay/recovery, fresh Cedar,
Configuration revocation, forced RLS and content-free audit. The leaf suite
also checks signed evidence, merge ordering, rollback/pins, native Git integrity,
divergence, path/size/custody, timeout and cancellation.
Its controlled HTTPS/native Git backend checks both artifact families, uncertain
push recovery, old-head races, remote deletion, privacy/repository-ID refusal,
TLS trust, redirects, outages and malformed/oversized responses. API acceptance
checks active exact-scope encrypted credentials, provider narrowing and refusal
to retarget existing receipts. These tests use synthetic values and never call
GitHub; a real private-repository canary is a separate qualification step.
