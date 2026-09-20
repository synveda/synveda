---
title: "CPR-45: Docker-first portable reference deployment"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-45: Docker-first portable reference deployment

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Problem and evidence

Synveda has one application runtime, schema and public API, but complete live
evidence for a portable single-host installation is still missing. The
canonical Compose graph, digest-bound reference archive, logical recovery,
same-schema upgrade and experimental Apalis canary are implemented and covered
by deterministic contracts. Development acceptance has run from a clean volume
on one macOS/OrbStack host. The v0.4.0 artifacts now pass native Linux
AMD64/ARM64 qualification; Docker Desktop remains unqualified.

Static configuration cannot prove that an operator can pull the artifacts,
sign in through Keycloak, use the product, restart it, recover it and upgrade
it. Local and published 0.4.0 installation/recovery evidence is recorded below;
the broader supported-host matrix and N-1 upgrade remain pending.

[ADR-0102](../adr/adr-0102-portable-reference-deployment.md) makes Compose the
canonical single-host deployment contract. Helm implements the same contract
with Kubernetes primitives; it is not generated from Compose.
[ADR-0105](../adr/adr-0105-direct-compose-acceptance.md) requires direct
acceptance against an operator-supplied supported Docker engine rather than a
non-executing engine simulation.

The reference topology is Caddy, PostgreSQL 17, optimized production-mode
Keycloak, separate gateway and worker commands from one product image, and a
private OpenTelemetry Collector. Synveda and Keycloak use separate databases
and roles. Keycloak stays behind the generic OIDC/OAuth2 + PKCE boundary and is
the only bundled identity provider. Temporal is absent.

## Scope

### Installation mission — 2026-09-20

Starting checkout: `736c729c681c9973e271f02e36454e4e4acbb694`, branch `main`,
only untracked `design/` (preserved). Coordinates OPS-11, OPS-8 and FND-7 under
[ADR-0115](../adr/adr-0115-prebuilt-container-release-verification.md). The owner
started v0.3.0 before this increment; these additive installation capabilities
are published as 0.4.0. Source commit
`e59284619567d6a13b70ce3f3b3e81121b7621e6` and tag `v0.4.0` are pushed.
All 13 [hosted CI jobs](https://github.com/synveda/synveda/actions/runs/35522813753)
and the [packaging dry run](https://github.com/synveda/synveda/actions/runs/35522831120)
passed. The [tagged release workflow](https://github.com/synveda/synveda/actions/runs/35525132082)
passed builds, assembly and every required native AMD64/ARM64 image, Docker and
Kubernetes qualification. Only publication failed: `assets/*` also selected the
checkout's `assets/brand` directory. Recovery published the original artifacts
without rebuilding or changing the tag/image digests. All 15
[release downloads](https://github.com/synveda/synveda/releases/tag/v0.4.0)
were anonymously retrieved and matched the qualified bytes; all 14 checksum
entries passed. The anonymously pulled OCI chart was byte-identical at digest
`sha256:a711bcc1593b78e0b8db6bf76d9ce4cbc67621f14a94c6e55a0be86f6b0f628b`.
The failed workflow remains visible; its native reports establish qualification.
The workflow fix excludes checkout directories and checks the exact 15 regular
files before publication. Regression tests cover missing files, directories,
symlinks and a failed GitHub upload. Pages previously deployed the candidate
copy with matching HTML/screenshot bytes; the shared manifest now selects
published copy on the next deployment.

Verified starting inventory: the README led to source builds or the host
Node/OpenSSL, DNS/TLS reference launcher. `scripts/package-release.sh` already
packaged six image identities and the Compose closure. The chart already
supported independent external/CNPG and external/Keycloak choices with real
Kind evidence; its persistent starter required a preinstalled CNPG operator.
The governed demo existed, but its staged review tour required four CLI
profiles. These were implementation/usability gaps, not absent deployment code.
Pages settings confirm `https://synveda.github.io/synveda/`, built from
`website/`; the About description, homepage and topics were empty. The public
release API reports v0.3.0 with an empty asset list (2026-09-20); its workflow
passed builds and isolated image checks but failed publication because that
release page already existed. Anonymous
requests resolve all six v0.3.0 image indexes with Linux AMD64/ARM64 descriptors;
the OCI chart downloads anonymously at digest
`sha256:2ebf89156802cb4024f01d458c275fabad7fbd0f16a59608bc68d00a15e15c7d`.
These are metadata/chart retrieval checks, not empty-cache image-layer pulls
or full installation qualification, and they do not include this increment.

Implemented in the existing packaging/chart mechanisms:

- The release launcher defaults to a loopback evaluation graph. A bounded,
  network-disabled utility prepares private files without a Docker socket,
  host Node/OpenSSL, Git, Make or a compiler. Exact issuer/audience/PKCE checks
  remain intact with an explicitly configured private discovery backchannel.
  Repeated starts preserve keys; incomplete credentials refuse. State locks
  serialize operations and remote Docker contexts refuse.
- The existing chart adds persistent namespaced PostgreSQL, with independently
  selected existing/bundled identity and no operator installation. CNPG remains
  explicit. Existing Secret references, TLS, ordinary runtime roles, the
  advisory-locked migration Job and restricted contexts are retained. A shipped
  preparation example and loopback port-forward recipe provide real login.
- One optional `sample` command uses the packaged CLI/browser and normal public
  APIs with distinct fictional accounts. It resumes its receipt and stops at
  proposed learning; reviewer/approver actions remain deliberate. No approvals
  are represented as human actions by automated acceptance.
- The Docker lifecycle adds paired database/private-key backup and fresh-volume
  restore through the existing helpers. The live drill exposed an incorrect
  ownership query in logical restore; it now joins `pg_database` to the activity
  statistics instead of reading `datdba` from `pg_stat_database`.
- README, installation selection, Docker and chart guides and the existing static
  site share checked 0.4.0/published metadata. Contributor details moved to
  CONTRIBUTING. Existing anchors and STARTER.md remain replacement pointers;
  current ADRs, trust contracts, attribution and acceptance evidence are retained.
  The approved branding is unchanged. `OPERATIONS.md` is now size-neutral.
- Release announcement requires anonymous image/chart pulls and full candidate
  Docker/Kubernetes qualification on native AMD64 and ARM64. Candidates are
  built once; qualifiers consume digest-bound images and the packaged chart.
  Third-party workflow actions are pinned to recorded commit SHAs. Local
  synthetic/cached image tests never count as anonymous-public evidence.

Local qualification completed on macOS 26.6.2 arm64, OrbStack Engine 29.4.0,
Compose 5.1.2, Node 24.18.0 and Playwright 1.62.1. The native browser was Brave
153.1.95.104; container acceptance used the pinned browser fixture. The engine
exposed 18 CPUs and 16.8 GB RAM. Local image digests and `sourceDirty: true` are
explicit in the [Docker report](../../demos/evidence/cpr45-evaluation.json) and
[Kubernetes report](../../demos/evidence/ops11-release-candidate.json).

| Scope | Result and boundary |
|---|---|
| Docker archive, bundled services | PASS: concurrent/repeated preparation, missing-key refusal/recovery, real empty-console login/logout, public-API sample and repeat, retained recreation, paired backup/reset/fresh restore and usable access |
| Docker deliberate faults | PASS: wrong issuer, unreachable backchannel, missing credential, confirmed full disposable tmpfs and occupied port refuse; documented recreation restores the actual host URL |
| Kubernetes database/identity ownership | PASS: bundled/bundled, external/bundled, bundled/external and external/external; providers provisioned separately in owned fixture namespaces |
| Kubernetes operations | PASS: real PKCE, ordinary runtime roles, tenant/workspace denial, service/MCP access and revocation, pod recreation, same-release reapply and retained reinstall; joint restore and interruption/migration drills on bundled/bundled and external/external |
| Packaged local Kubernetes recipe | PASS: actual browser login/logout through loopback forwards, restricted admission, UID 1000900000 for gateway/worker/install/Keycloak/PostgreSQL, failed runtime-role migration and successful subsequent reapply |
| Published 0.4.0 / native Linux | PASS: anonymous retrieval; both native AMD64/ARM64 image smoke, full Docker lifecycle/fault/recovery, four-mode chart operations and real browser port-forward login |
| Docker Desktop / Windows/WSL2 | NOT RUN: no target supplied |
| Real OpenShift / cloud / N-1 | NOT RUN: no authorised target, credentials or qualified published N-1 pair; generic Kind does not establish these claims |

The Kubernetes run used Kind 0.32.0, Kubernetes **and kubectl 1.36.1**, Helm
4.2.3, PostgreSQL 17.11/vector 0.8.6 and Keycloak 26.7.2. An earlier installed
kubectl 1.33.9 was outside supported skew; the final run used a checksum-verified
temporary matching client. Two subsequent chart documentation corrections were
checked to leave every runtime/dependency file byte-identical; the packaged
browser/migration recipe passed again. The report records both archive hashes.

Docker preparation took **0.403 s**, cached-image cold startup **278.494 s** and
warm recreation **223.088 s**. Image download time was not measured. A
post-restore snapshot totalled approximately **1.02 GiB** across running
deployment services; it excludes browser/one-shot utilities and engine overhead
and is not peak usage or a capacity guarantee. After extraction, the guide has
three operator steps: `up`, deliberate credential retrieval, browser sign-in;
optional `sample` is a fourth. Local Helm install-to-ready took **30.637 s**,
excluding cluster creation, image import and private-file preparation. Joint
Kind restores took **35.882 s** bundled and **35.729 s** external; these small
synthetic same-host measurements are not production RTOs.

The published native reports are attached as `release-images-<arch>.json`,
`release-docker-<arch>.json` and `release-kubernetes-<arch>.json`. Both runners
used Docker 28.0.4, Compose 2.38.2, Kind 0.32.0, Kubernetes/kubectl 1.36.1 and
Helm 4.2.3 with 4 CPUs and approximately 16.7 GB engine RAM. All report sources
equal `e59284619567d6a13b70ce3f3b3e81121b7621e6`; deployment reports record
`sourceDirty: false` and match every original manifest image digest. Preparation,
cached-image startup and warm recreation took 1.333/537.730/416.594 seconds on
AMD64 and 0.833/573.754/435.672 seconds on ARM64. Downloads happened in the
preceding image job and were not timed by the deployment report. These hosted
fixture measurements are not platform-wide performance guarantees.

Exact candidate qualification commands, from this checkout (the temporary
paths identify this local run, not downloadable release coordinates):

```sh
candidate=/private/tmp/synveda-candidate-0.4.0-qualified
node scripts/qualify-release.mjs "$candidate/synveda-reference-0.4.0" demos/evidence/cpr45-evaluation.json
PATH=/private/tmp/synveda-qualification-tools:$PATH \
  SYNVEDA_TEST_BROWSER_EXECUTABLE='/Applications/Brave Browser.app/Contents/MacOS/Brave Browser' \
  node scripts/qualify-kubernetes-release.mjs \
  "$candidate/synveda-reference-0.4.0" "$candidate/synveda-0.4.0.tgz" \
  demos/evidence/ops11-release-candidate.json
```

The Docker qualifier runs the extracted `synveda-compose` preparation, startup,
sample twice, down/up, backup and explicitly confirmed reset/restore commands.
It used **port 18080**, preserving the existing development port; the published
default remains 8080. No source CLI or runtime image build occurs in either
qualifier. Public download/OCI commands for 0.4.0 remain **pending publication**.
The immutable archive guide now refers to published qualification reports
instead of freezing main's temporary publication status into the release.
Only `INSTALL.md` changed in that final documentation refresh; all executable
and configuration files were verified byte-identical, and the packaging
acceptance gate passed again. The Docker report records both archive hashes.

Formatting, strict workspace Clippy, 1,662 workspace Rust tests, the final
identity library/integration tests, licence/API/SDK/adapter/security/evaluation
and TypeScript gates passed. `make check-deploy` passed **364 tests**, the
Compose render matrix and deployment convergence. The whole `make ci` command
was not rerun as one invocation; its component gates were executed separately.
The fresh complete `make db-test` and live proprietary-client suites were not
rerun for this increment. The hard-cut checker now distinguishes the exact
Kubernetes RBAC `RoleBinding` from the retired product DTO, with adversarial
regressions retaining the product ban.

The site build passed with 13 public files (668,360 bytes), unchanged approved
brand exports, the genuine fictional-sample screenshot, valid links/metadata
and no JavaScript/analytics. Browser/keyboard/overflow/fragment checks passed
at 1440/768/390/320 px. axe-core 4.11.0 found zero WCAG 2 A/AA or 2.1 AA
violations; decorative-arrow incomplete items were inspected manually.

The live drills also fixed PostgreSQL readiness racing the entrypoint's
temporary socket-only server, and allowed bounded identity reconciliation
inside the existing startup deadline. The occupied-port drill exposed an
engine retaining an unbound proxy endpoint after allocation failure; the
documented down/up recovery is tested against the host URL. Storage exhaustion
correctly returned 78; its test verifies a full filesystem because shell
`printf` reports ENOSPC as a generic I/O error on this image.

The existing `synveda-development-acceptance-interop` remains healthy and
untouched. All owned evaluation containers and Kind clusters are stopped or
removed. Only labelled disposable fixture volumes were reset; final private
state, data and backups are retained. No cluster-wide operator was installed.

Remaining platform boundary: macOS Docker Desktop, Windows/WSL2, real OpenShift
and a supported published N-1 pair require their named infrastructure/artifacts.
The prepared tests must run there; native Linux/Kind qualification does not
establish these claims. Off-host custody, PITR and production DR remain separate
readiness gaps. Next: supply the remaining disposable host/platform targets and
declare a compatible N-1 pair, then run the corresponding installation/upgrade
drills. Keep the existing tag and image digests immutable.
See [release operations](../RELEASING.md).

### Continuing deployment scope

- Keep one provider-neutral configuration, image, command, schema, health,
  public API, OIDC, OTLP, object-store and recovery contract.
- Keep only the reverse proxy public; database, identity management, worker,
  metrics, OTLP and recovery services stay private.
- Use mounted files for secrets and distinct non-owner gateway/worker database
  roles under the existing Cedar, forced-RLS, VedaFlow and audit invariants.
- Support bundled or external OIDC and bundled or external PostgreSQL with the
  same product image; external dependencies remain operator-owned.
- Exercise a clean four-principal public-API review scenario across the fixed
  restart matrix, including a denied restricted-viewer action.
- Back up both product databases with separately held KMS/identity recovery
  material and restore only into a fresh isolated target.
- Keep `skill_validation@1` as the sole optional Apalis 0.7.4 canary behind the
  provider-neutral operation/attempt/outbox model and native-worker rollback.
- Keep OpenTelemetry as the application telemetry interface, with one bounded
  optional local metrics view and a customer-safe Operations route.
- Package the reference graph with immutable image identities and a source-SHA
  environment manifest.

## Non-goals

- No HA, host-loss tolerance, multi-region operation or zero-downtime upgrade.
- No production SaaS, disaster-recovery, compliance or signed-provenance claim.
- No complete Helm promotion, scheduler product or Apalis board.
- No migration of Sessions, Capture, Knowledge mutation, PDP, VedaFlow or audit
  authority into a queue provider.
- No Keycloak-specific Synveda principal, role, grant, tenant or policy model.
- No automatic container-engine installation or VM/provider receipt layer.

## Architecture seam

The packaged evaluation launcher selects a fixed bundled loopback graph.
`deploy/compose/scripts/compose.sh` retains the closed development/reference,
PostgreSQL, OIDC and OTLP matrix. Core services are not profile-gated. Optional
profiles are demo/browser acceptance, bounded local observability and the
experimental Apalis canary. Reference mode requires real DNS, HTTPS certificate
files, an explicit private network and immutable images; development is
explicit loopback HTTP with managed `.test` names.

The release workflow builds five deployment images—product, single-host
PostgreSQL, optimized Keycloak, proxy and CloudNativePG-compatible
PostgreSQL—plus the browser-acceptance fixture. The reference archive contains
only the reference runtime closure and records every image identity in
`environment.json`. The installer stores immutable releases
under `reference/releases`, repoints `reference/current`, and preserves
separate state and backup roots. Automatic artifact removal remains fail-closed
until OPS-10 adds a strict installer ownership receipt.

The operation ledger and outbox are authoritative tenant data under forced
RLS. Apalis receives only an untrusted tenant routing identifier, Synveda
operation ID and version; its worker resolves and reauthorises product state in
a tenant-scoped transaction. The separate queue database is disposable
transport state and is excluded from recovery.

Logical recovery pauses canonical writers, creates separate Synveda and
Keycloak PostgreSQL archives, links them to a separate KMS/identity recovery
set and restores only into a confirmed fresh project. It proves the audit chain,
tenant key and wrong-key refusal before normal convergence. This is same-host
planned-interruption validation, not PITR or disaster recovery.

The documentation audit consolidates source-checkout operation in
`deploy/compose/README.md`, keeps packaged installation distinct, and routes
beginner navigation through the root documentation index. Runtime evidence and
remaining acceptance boundaries are recorded separately above.

The current local fixture uses the existing public API, deterministic Session
Capture, VedaFlow review/apply, Knowledge provenance, Context and Skill
binding surfaces for a synthetic ingestion-retry learning. Its stable seed
stops before Capture, uses Avery Author, Riley Reviewer, Morgan Approver and
Vera Restricted Viewer with existing role keys, refuses an unintended gateway or conflicting
curator file, and never resets product data.

The console walkthrough now exposes those same governed surfaces as a coherent
work journey: readable Knowledge and Session evidence, routed review state, a
small Context request workbench with exact revision links, and Skill placement
resolution. It reuses the generated public client and existing capability
forecasts; no browser-side authority or local-only product model was added.
Focused console tests, production type/build checks and deterministic API and
security-contract checks pass. The administrative increment now discloses
governed person and group names beside exact subjects, scopes and grant ids;
uses exact-grant revocation; explains pending and settled invitations; limits
normal configuration editing to the demo's bounded Capture and Context fields;
and labels Skill, Tool and Operations state only from existing evidence. It
retains the generated public client, Cedar decisions, RLS, VedaFlow review and
the Keycloak administration boundary.

On 2026-09-11 the current console was rebuilt into the canonical `demo`
profile and `make compose-smoke` passed with the development `.test` mapping
installed. The fresh exact-role `access_api` suite also passed all 18 tests,
including exact grant revocation, invitation lifecycle, policy denial,
unauthenticated refusal, tenant non-disclosure and token-free audit evidence.
The connected in-app Browser then exercised the access-action acceptance
sequence against that stack. Native email validation refused malformed input;
Avery Author created a labelled workspace-viewer invitation and saw its exact
pending state without retaining the one-time link; the API created and Vera
Restricted Viewer accepted a second labelled invitation; and Vera's repeated
invitation attempt was refused by both UI and API with HTTP 403, the stable
`policy_denied` kind, action, resource and safe policy reason. The temporary
viewer grant was revoked and the pending UI invitation was withdrawn through
the public API, with the final `revoked` state and absence of the temporary
grant verified.

That execution exposed and fixed two console-only failures: custom HTTP `.test`
origins lacked `crypto.randomUUID()`, so idempotent mutations now use a Web
Crypto random-byte fallback; and structured gateway denials without a
`message` lost their actionable reason, so the console now renders the safe
reason, action and resource. Focused tests cover both paths. No invitation
secret, browser credential or Keycloak administrator capability was recorded
or added. A local walkthrough does not change the production-readiness
boundary.

### Current local demonstration validation

A subsequent interoperability run on 2026-09-12 completed the owned hosts handoff
and fresh `acceptance-interop` acceptance at `0a6dfb5` plus the working-tree client
increment. Browser seed/login, all six service restarts and final verification
passed. After the response-trace correction, canonical down/up with retained
PostgreSQL and secrets and the `demo` profile passed smoke. Native Codex and both
SDKs then passed the shared ordinary Keycloak workflow through the public proxy,
including Capture, explicit task end and a valid audit chain. The current local
project is `synveda-development-acceptance-interop`, pool `10.231.46.0/24`; the
earlier `acceptance-e2e` product data and secrets remain retained. Details and
remaining client limits are in [the client support matrix](../CLIENT_SUPPORT.md). This adds no new
platform, reference-HTTPS, HA or SaaS-readiness claim.

The 2026-09-12 run uses `feat/CPR-45` at starting commit
`d1486929d73dc459ed6dc0bf029c5c825d3a1bb6` plus this working-tree increment.
The host is macOS 26.6.2 arm64, OrbStack Docker Engine 29.4.0, Compose 5.1.2,
Node.js 24.18.0, Rust/Cargo 1.96.0 and Playwright 1.62.1. The isolated project
is `synveda-development-acceptance-e2e` on `10.231.47.0/24`; the normal
`synveda-development` data volume is never a reset target.

Live execution exposed stale workspace/Session receipt observations and a
tenant-plane console guard that incorrectly blocked a workspace-authorised
review deep link. The receipt now records the post-mutation public reads, and
the detail page uses its existing per-proposal read and target-scope control
forecast. Redacted Context selection links also retain their exact immutable
revision address. The four-person fixture supplies the second distinct Skill
approver through workspace-scoped grants; it does not lower approval policy.
Chromium's private temporary storage and Keycloak's convergence health window
are bounded to accommodate the observed real execution.

The full deployment gate passes, including all 169 combined lifecycle/recovery
tests and the Compose render matrix. All 251 console tests, the production
build, all 189 CLI tests, strict CLI Clippy, all 77 policy tests, six OpenAPI
tests, the fresh exact-role 18-test access API suite, dependency/API/security
contracts and documentation gates pass. The final clean-volume
`make compose-acceptance` passes: the seed replay preserves a real browser edit;
direct raw-Session-content and approval requests are denied to the restricted
viewer; worker-completed Capture leads through separate review and apply to
persisted Knowledge, exact provenance, redacted Context revision links and an
available pinned Skill. Skill install and binding retain their two-person
approval requirement. Open and approved-but-not-applied changes are checked
before publication. All six native service restarts, full smoke after each,
repeat browser login and final live receipt verification pass.

Safe Knowledge and People desktop screenshots and a 1024-pixel-wide Context
screen were inspected; no credentials, headers, login screens or one-time
invitation links were captured. Keyboard focus/navigation, native labels,
invalid email validation, exact-grant confirmation dismissal, empty/pending
states and no horizontal overflow are covered by the existing browser suite.
Context honestly displays the unconfigured embedder fallback and redacted
content, not a claim that every retrieval leg ran.

Canonical `compose-down`, `compose-up` and `compose-smoke` also pass, reusing the
exact isolated product volume. Down removes the disposable browser
credential/receipt volume; a fresh browser login then reopens the same
persisted Knowledge, Context and grant addresses without restored auth state.
Byte comparisons across the separately confirmed isolated resets, acceptance
and down/up prove both projects' complete secret sets and issuers are
unchanged; the normal product volume retains its identity. No normal-project
reset was performed.

The isolated demo is left running for inspection, with its hosts block still
installed. The normal stack remains stopped. No manual host action is needed
to inspect the demo; returning to normal development requires canonical down
and the confirmed hosts removal/install plus resolver-cache steps in the
[Compose guide](../../deploy/compose/README.md#governed-ingestion-retry-walkthrough),
using the exact `acceptance-e2e` suffix for this deployment. Local demo phase
acceptance has no remaining blocker. The next wider-feature action is the
supported-host/reference repetition below. This is one-host local evidence,
not Docker Desktop/Linux, reference HTTPS, recovery, upgrade, Apalis,
installed-release or production evidence.

### Deterministic gate progress (2026-09-12)

The apparent interrupted-build stall recorded during archive validation was
delayed test reporting. At `f06926a`, the unchanged 89-test lifecycle suite
passed in 360 seconds on macOS arm64 Node 24.18.0 with a 30-second per-test
deadline and zero skips. The interrupted-build case completed in 2.5 seconds;
later backup, restore and upgrade fixtures kept running while its result was
buffered. A process sample showed synchronous fixture execution nested inside
the child-exit callback. The two earlier terminated full gates remain incomplete
runs, not passes.

`scripts/compose-lifecycle.test.mjs` now yields to the event loop after each
case so pending report I/O can flush. Assertions, test selection, production
deadlines, signal cleanup and exact-project lock retention are unchanged.
This is a test scheduling correction under ADR-0105's deterministic evidence
class, with no architecture or deployment-contract change.

The complete unfiltered `make check-deploy` passes all 342 tests with zero
failures, cancellations or skips, including the 169 combined script checks,
the Compose render matrix and final deployment convergence. Results now flush
past the interrupted-build case while later fixtures are still running. Four
focused build/failure/interruption checks also pass on pinned Node 22.23.2
Linux arm64 Docker as the ordinary `node` user. Formatting and backlog/ADR/docs
checks pass; no Rust changed, so strict Clippy is not applicable to this fix.
Full CI, database and live deployment acceptance were not rerun. This closes
the deterministic-gate checkpoint without changing the remaining live criteria.

### Remaining live acceptance

1. Repeat clean development acceptance on Linux and Docker Desktop, and run
   reference HTTPS acceptance on both host classes, including exact
   issuer/browser login and the restart matrix.
2. Run logical backup and isolated restore with the recovered KMS key and
   post-restore Keycloak login on both supported host classes.
3. Run the native and Apalis Skill-validation paths through duplicate,
   cancellation, restart and rollback cases on the live reference.
4. Run the same-schema two-image upgrade/rollback smoke using published digest
   references.
5. Run deterministic external-dependency modes and at least one live external
   OIDC/PostgreSQL/OTLP combination when credentials exist.
6. Publish one candidate archive and manifest-bound image set, then install and
   pull it from an empty supported host.

## Acceptance criteria

- Every supported selector passes Compose configuration without rendering a
  secret; fresh bundled mode converges PostgreSQL, Keycloak, gateway, worker,
  proxy and Collector.
- Browser authorization-code + PKCE S256 login proves exact issuer, JWKS,
  audience, algorithm, callback and administrator-group handling; negative
  token cases fail.
- Separate database roles cannot cross product boundaries and forced RLS holds
  for runtime work.
- The public-API scenario creates a workspace/project, maps four distinct
  identities to existing grants, records and captures a synthetic Session,
  denies the viewer's review attempt, separately reviews/applies Knowledge and
  a versioned Skill binding, then reuses the exact Knowledge revision with
  provenance after every service restart.
- Backup/restore binds both databases and recovery keys, verifies audit/key
  evidence and refuses a wrong key without overwriting the source.
- Native and optional Apalis delivery produce one authorised canary effect
  under duplicate/restart/cancellation cases; queue payloads stay opaque.
- Operations and telemetry disclose no content, secret, cross-tenant data or
  provider-internal task identity.
- A digest-bound current-schema image upgrade preserves product evidence,
  restores the last verified image on ordinary failure and refuses an unsafe
  candidate before transition.
- Confirmed reset removes only the exact Compose project's owned resources.
- No active Rauthy, Temporal or second Compose lifecycle remains.

## Required tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
make ci
make db-test
make claude-acceptance
make check-deploy
make compose-config
make compose-acceptance
make compose-backup
make compose-restore-smoke
make compose-upgrade-smoke
```

Unavailable live services or credentials are reported as unavailable, never
relabeled as pass.

## Rollout and rollback

The identity and release hard cuts are complete; there is no retired-provider
compatibility mode. Rollback uses a previous complete environment manifest,
not mixed current/old images. The Apalis fragment is disabled by default and
the native PostgreSQL worker is its rollback. Restore always targets a fresh,
confirmed project.

## Dependencies

The contributor walkthrough needs unoccupied deployment resources. On the
current development host, retained `synveda-evaluation_postgres-data` and the
managed hosts block for `synveda-development-acceptance-interop` prevent a
separate clean install: the launchers refuse mismatched state or hosts
ownership. Next: repeat the README and source-guide walkthrough on a disposable
Docker host, or arrange an explicit operator handoff with the original state.
Do not reset retained volumes or replace another deployment's hosts entries.

Completion needs a supported Docker Engine, Linux and Docker Desktop hosts,
browser trust for the selected issuer, and one published candidate. Production
S3/WAL-PITR, encrypted off-host retention and recurring recovery drills remain
OPS-5. Signing/provenance, HA, hosted SaaS and broader on-prem/Helm promotion
remain separate readiness work.
