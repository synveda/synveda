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
on one macOS/OrbStack host; the complete set has not run from an empty published
installation on Linux and Docker Desktop.

Static configuration cannot prove that an operator can pull the artifacts,
sign in through Keycloak, use the product, restart it, recover it and upgrade
it. Until those gates pass, the verdict is “Docker reference implemented; live
validation pending.”

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

`deploy/compose/scripts/compose.sh` selects the closed development/reference,
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
beginner navigation through the root documentation index. It does not add live
deployment evidence or change the remaining acceptance boundary.

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
remaining client limits are in `docs/INTEROPERABILITY_PLAN.md`. This adds no new
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

### Current deterministic gate blocker (2026-09-12)

During OPS-8/CPR-39 archive validation from `d17bd4d`, `make check-deploy`
stalled in `scripts/compose-lifecycle.test.mjs` at `interrupted build retains
its exact lock and removes private Buildx state`, after the preceding timeout
case passed. This occurred both inside and outside the restricted tool sandbox
on macOS arm64 Node 24.18.0. Both full runs were terminated and are not passes.
The interruption case alone passed in 2.6 seconds; selecting it with the preceding
timeout case passed both in 6.0 seconds. No lifecycle implementation/test or
live Compose project was changed by that packaging batch.

Next action: reproduce the full-suite interaction with bounded test deadlines,
isolate the preceding lifecycle cases, and inspect child/pipe/signal cleanup
before changing the existing teardown contract. Keep the full gate blocked until
an unchanged invocation finishes; do not substitute a filtered pass. The focused
probe is:

```sh
node --test --test-name-pattern='^(timed-out|interrupted) build retains its exact lock and removes private Buildx state$' --test-timeout=30000 scripts/compose-lifecycle.test.mjs
```

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

Completion needs a supported Docker Engine, Linux and Docker Desktop hosts,
browser trust for the selected issuer, and one published candidate. Production
S3/WAL-PITR, encrypted off-host retention and recurring recovery drills remain
OPS-5. Signing/provenance, HA, hosted SaaS and broader on-prem/Helm promotion
remain separate readiness work.
