# Install and use Synveda

Synveda keeps knowledge from agent sessions in a project or personal scope.
People can inspect its source, review proposed changes and choose what later
sessions may use. The web console handles everyday work; the CLI and public API
support automation and advanced administration.

<!-- installation-version: 0.4.7; publication: unreleased -->
The current published release is **[v0.4.6](https://github.com/synveda/synveda/releases/tag/v0.4.6)**,
for controlled evaluation; it is not production-ready. This guide describes
v0.4.6 unless a section says it covers the unreleased 0.4.7 source. Choose the
route that matches your infrastructure:

| Route | Guide | When to use it |
| --- | --- | --- |
| Prebuilt Docker bundle | [Download and verify](../deploy/compose/PREBUILT.md#download-and-verify) | First local evaluation with bundled PostgreSQL and Keycloak. |
| Kubernetes chart | [Chart installation](../deploy/helm/synveda/README.md) | A cluster with persistent storage and an operator who can manage its namespace. |
| Existing PostgreSQL or OIDC | [Compose reference](../deploy/compose/PREBUILT.md#use-existing-infrastructure) or [chart configuration](../deploy/helm/synveda/CONFIGURATION.md) | You already operate the database, identity provider or both. |
| Source checkout | [Development setup](DEVELOPMENT.md#running-your-changes) | You are changing Synveda or running its tests. |

## First run with Docker

Use the [README download commands](../README.md#install-for-local-evaluation)
to verify and extract the prebuilt bundle on a native Linux AMD64 or ARM64
host. It pulls fixed container images and does not compile source on the host.

### 1. Set local options before startup

Open `evaluation.json` in the extracted bundle. Its keys are:

| Key | Default | Change it when |
| --- | --- | --- |
| `port` | `8080` | That loopback port is already in use. |
| `subnet` | Private Docker `/24` | It overlaps a local or VPN network. |
| `demoAccounts` | `true` | Set `false` to use identities you provision yourself. |

The selected port, subnet and account mode are recorded in private installation
state. Keep that state, the database volumes and encryption keys together. The
launcher uses the fixed `synveda-evaluation` Compose project, even from another
directory. For exact state and recovery rules, see the
[Docker lifecycle guide](../deploy/compose/PREBUILT.md#state-and-ordinary-lifecycle).

### 2. Start

```sh
./synveda-compose up
```

The first start downloads the images and can take several minutes. It returns
once every service is healthy and prints `http://localhost:8080/console/`, with
your configured port.

### 3. Choose the sample or an empty workspace

Pick one route per installation:

- **Fictional sample (recommended for a first look).** Run
  `./synveda-compose sample` *before* using **Getting started**. It creates
  **Northstar Delivery → Ingestion API**, a source session, a proposed retry
  learning and a proposed Skill version through the public API, using the four
  generated demo accounts. It approves nothing and is safe to repeat.
- **Your own empty workspace.** Sign in and use **Getting started**. A
  workspace is a place for a person or team; a project is where related
  sessions and knowledge live. You can attach a repository and choose an agent
  client afterwards.

The order matters because the first workspace settings in an installation
apply directly, while later ones follow the review rules. If **Getting
started** runs first, the sample stops with `compose-product: product-demo failed`
and its settings wait for two administrators other than the author; if the
sample runs first, a later **Getting started** workspace queues its settings in
**Advanced → Reviews**. The
[Configuration review steps](../deploy/compose/PREBUILT.md#pending-sample-configuration)
cover recovery.

### 4. Sign in and review the sample learning

```sh
./synveda-compose credential author
```

Open the console URL and sign in as `synveda-demo-admin` (Avery Author) with
the printed password. It is generated for this installation; the command needs
an interactive terminal. If you disabled demo accounts, sign in with an
identity from your configured OIDC provider; the
[provider guide](../deploy/compose/PREBUILT.md#use-existing-infrastructure)
covers that setup.

In **Sessions**, inspect the source session. In **New Learnings**, open the
proposed ingestion retry finding; its banner names the change that waits for
review. The sample requires approval by `synveda-demo-member` (Riley
Reviewer):

1. In a private browser window, so the identity provider does not reuse the
   first sign-in, sign in as the reviewer with the password from
   `./synveda-compose credential reviewer`.
2. Open `/console/advanced/reviews/CHANGE_ID` with the change ID from the
   banner. The reviewer's workspace grants do not include the tenant-wide
   **Reviews** list, so open the review page directly.
3. Inspect the proposal, then select **Approve** and **Apply approved change**.
   The state becomes `applied`.

Then, signed in as `synveda-demo-admin`, open **Context**, keep the sample
session selected and request context about ingestion retries. The result lists
the approved revision with its reason codes; **Open selected Knowledge
revision** shows its text and origin. The sample's team settings keep a
redacted trace, and this bundle's lexical-only search reports *Degraded
retrieval: embedder*; both notices are expected. The
[step-by-step walkthrough](../deploy/compose/PREBUILT.md#first-workspace-and-sample)
also covers the sample Skill proposal.

## Configure access and behavior

### People and scopes

Use **People** to invite teammates and grant access to a workspace or project.
A workspace grant reaches its projects; a project grant stays in that project.
Each person's private scope is separate. The initial administrator comes from
the configured first-login identity group; later grants are made through
Synveda. Access is checked again for every operation.

**Before inviting**, make sure the colleague can sign in: the identity provider
must already have an account for them. Signing in proves who someone is; it
does not give them a workspace. An invitation adds one Synveda grant to the
account that accepts it and never creates an identity-provider account. Joining
the first-login administrator group after the first administrator exists grants
nothing.

**In v0.4.6, a browser cannot accept an invitation link**: the link opens an
API address that only accepts `POST`. Ask the colleague to sign in once, then
grant a project role directly in **People** with **Grant at this project**,
using their token subject. A workspace invitation still works if the recipient
redeems it with `POST` to its `accept_url` using their own credential. The
browser invitation flow below belongs to the unreleased 0.4.7 source.

**To invite** (0.4.7 source), select the workspace in **People**, choose the exact role and,
optionally, an email label (nothing is emailed), then create the invitation.
Copy the link at once: it is shown only in that response. It opens this
installation's `/console/invite` page and carries the one-time token after the
`#`, which a browser does not send to the server, the proxy or another site.
Share it privately. It works once, expires (seven days by default, at most
thirty) and can be withdrawn from the same page while pending.

**To accept**, the colleague opens the link, signs in if asked, checks the
account the page names and selects **Accept invitation**. Opening, reloading or
previewing the link accepts nothing, and the tab forgets it after 20 minutes,
on **Not now** or on sign-out. Signing out of Synveda does not sign out of the
identity provider, which may sign the same account straight back in; to accept
as a different person, open the link in a private window. Expired, withdrawn and already-used invitations
say so and grant nothing. A retried acceptance by the same account is safe; no
other account can reuse an accepted link. Links shared by earlier versions in
the `/v1/invites/<token>/accept` form still open the same page. API clients
redeem with `POST` to the response's `accept_url` using the recipient's own
credential.

**Someone awaiting access** (0.4.7 source) who signs in before being invited sees why they
cannot work yet, the account they are signed in as and how to continue; they are
not offered a workspace they would be refused. Their private scope stays usable
where policy allows. After you invite them or grant access in **People**, they
select **Refresh access** — signing in again is not needed. A person with a
project but not its workspace finds it under **Projects shared with you**.

### Governed runtime configuration

Use **Advanced → Configuration** to select a policy pack and set capture,
context, Skill and Tool behavior for a scope. A configuration is versioned and
governed: publishing or binding a change may apply immediately or wait for
review, according to the active policy. A change cannot bypass Cedar access
checks, PostgreSQL tenant isolation or audit.

The console is the simplest route. With an installed and authenticated native
CLI, copy the target scope UUID and use the corresponding commands:

```sh
scope_id="PASTE_SCOPE_UUID_HERE"
synveda configuration templates
synveda configuration effective "$scope_id"
synveda configuration create --scope "$scope_id" --name project-runtime --template team
configuration_id="PASTE_ID_RETURNED_BY_CREATE"
synveda configuration show "$configuration_id"
synveda configuration bind --scope "$scope_id" --artifact "$configuration_id"
```

Start with `effective` to see what the project currently uses. Create a
configuration from a template, inspect it, then bind it to the intended scope.
Check the returned state before expecting a pending change to affect sessions.
The [policy guide](../policies/README.md) explains the packs, and the
[OpenAPI contract](api/openapi.json) defines the exact request fields.

### Existing identity and data services

Synveda uses OIDC authorization code with PKCE. The issuer in discovery,
tokens and the deployment configuration must match. The first eligible member
of the configured `synveda-admins` group can claim the initial tenant
administrator grant; subsequent access is governed within Synveda. Directory
synchronization through SCIM is a separate setup. Follow the
[Compose OIDC contract](../deploy/compose/README.md#external-oidc) or
[Kubernetes provider contract](../deploy/helm/synveda/CONFIGURATION.md)
for client, audience, certificate and database settings.

## Connect an agent client

On the machine where you run Claude Code, Codex CLI or GitHub Copilot CLI,
follow [Connect an AI coding agent](AGENT_SETUP.md). In short (0.4.7 source):

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
synveda login --gateway http://localhost:8080
synveda setup                                        # lists the projects you can use
synveda setup --project PROJECT_ID --observation on
synveda adapter install --client claude-code         # or codex, copilot-cli
```

Run the `setup` and `adapter install` commands from the root of the Git
repository you work in. Use your server's actual URL; plain HTTP is accepted
only for `localhost`. Hooks record that repository's sessions only after
`--observation on`; the default is `off`. Other MCP clients use
`synveda adapter install --client NAME`; the [support matrix](CLIENT_SUPPORT.md)
lists every client, the tested versions and their capabilities.
Python and TypeScript SDKs have an initial API slice; their
[guide](../sdks/README.md) states the publication boundary.

## Working with knowledge

A Session records agent activity. Capture turns eligible evidence into a
reviewable candidate. **New Learnings** lets you accept, edit, merge, replace
or dismiss it. A governed change creates an immutable Knowledge revision only
when it applies. **Knowledge** shows the current revision and source; **Context**
shows which revisions a session received and why.

Saved ContextRun replay repeats current authority and source checks. A `409`
stating that saved delivery is unavailable means that its original receipt
remains recorded but cannot be delivered again. Request current context with a
new idempotency key. An archived, corrected, stale or removed revision is never
silently replaced under the old key. New deliveries retain private source proof
in every trace retention mode, including authored context and Skill advertisements.
Older nonempty receipts without this proof refuse replay; fresh composition remains
available under current policy. Exact-revision
historical inspection retains its own authorization rules.
Saved replay reads exact stored policy sources into an immutable request-local
evaluator, so it can observe a committed policy change before the background
cache converges. It refuses an unavailable or oversized policy snapshot. The
atomic boundary for concurrent source removal remains open.

Every other governed request also evaluates the tenant's current stored policy.
A `409` stating that current tenant policy cannot be evaluated means the request
was refused without valid current policy. Briefly, it can mean that bounded
snapshot construction was saturated for more than five seconds; a later retry
may then succeed. When it persists, one of the tenant's stored packs does not
compile or exceeds its bounds, and retries or new idempotency keys cannot help.
The gateway log names the failing pack. A storage failure is a separate `503`. An operator replaces it with a compile-checked
`synveda policy apply --tenant <tenant-id> --name <pack> <file>` or removes a
pack that no Configuration references with `synveda policy clear`. Both record
break-glass audit events. Until the repair, failed policy sweeps cannot renew
the gateway and worker freshness lease, so after 30 seconds they withdraw
readiness for every tenant; the next successful sweep restores admission.

The Context Inspector's **Context health** shows separate provenance,
freshness, conflict, budget, outcome and review findings. Each observed concern
links to an exact permitted revision, the learning review, the Session timeline
or a fresh context request. It never counts denied evidence. Its pipeline
observations separate eligibility, retrieval and assembly concerns; capture
coverage, source synchronization, physical cleanup and actual model use remain
unknown unless evidence establishes them. Feedback reports what an authenticated
user said, without claiming independently verified task success.

### Bounded claim declarations

API clients may attach explicit claims inside a revision's `metadata` to improve
conflict review. For `body_markdown: "Retry cap 3."`, this projection binds the
whole twelve-byte statement:

```json
{
  "synveda_claims": {
    "schema_version": 1,
    "claims": [{
      "subject": {"kind": "named", "key": "payment-worker"},
      "predicate": "retry_cap",
      "assertion_status": "asserted",
      "applicability": {"environment": "production"},
      "unit": "count",
      "constraint": {
        "operator": "eq",
        "value": {"kind": "decimal", "coefficient": 3, "scale": 0}
      },
      "evidence": {"start_byte": 0, "end_byte": 12}
    }]
  }
}
```

The revision's existing provenance links name its sources; this metadata does
not fetch or authorize any resource. `asserted` declares what the author says;
`proposed` preserves a proposal. Neither means approved or verified. Unknown
fields and fabricated statuses such as `approved` are refused.

The envelope allows one to eight claims and up to eight applicability conditions
per claim. Subject keys, predicates and condition keys/values preserve case.
Values are fixed-point decimals (signed integer coefficient, zero to nine
decimal places), text, booleans or version release triples. Decimal constraints
require a unit; other values forbid one. Ordered comparisons/ranges accept only
decimals and release triples, with nonempty ordered ranges. Text/boolean values
support equality and inequality. Version `1.9` is explicitly represented as
`1.9.0`, which differs from `1.10.0`; prerelease/build fields are unsupported.

Supported numeric units are `ns`, `us`, `ms`, `s`, `byte`, `bytes`, `KiB`, `MiB`,
`count` and `attempt`. Their finite conversions use exact checked integers.
Unsupported units or incompletely comparable claims remain review hints.
Disjoint values with overlapping subject, predicate, applicability and current
valid time produce contradiction evidence; distinct environments/predicates or
disjoint valid intervals do not become the same fact. Valid time comes from the
revision. Every evidence span must be a nonempty UTF-8 byte slice of that body.

Without annotations, lexical proximity suggests review and preserves the exact
proposition; only an equal canonical content hash is a duplicate hint. Changed
unparsed prose cannot be auto-resolved as support. Use the existing conflict
review and VedaFlow resolution paths to decide the applicable current revision.
The [claim decision](adr/adr-0142-bounded-revision-claims.md) records the limits.

### Other governed artifacts

The console also provides **Skills** for versioned Skill bundles, **Tools** for
reviewed MCP server definitions, and **Import / Export** for bounded OKF v0.2
project knowledge exchange. The gateway records external Tool metadata and
bindings; it does not execute those tools. For API clients, use the generated
[OpenAPI reference](api/openapi.json). For local OKF files, the CLI offers:

```sh
project_id="PASTE_PROJECT_UUID_HERE"
synveda okf validate ./knowledge-bundle
synveda okf import ./knowledge-bundle --project "$project_id" --dry-run
synveda okf import ./knowledge-bundle --project "$project_id"
```

Import first reports a plan; materialized items appear as reviewable
candidates, not active Knowledge. Review them in **New Learnings**.

## Advanced operator reference

### MCP connections

`synveda mcp` is a public-API client over stdio. Inspect a connection entry
with `synveda mcp install --print`, or register a supported client with, for
example, `synveda mcp install --client claude-desktop`. A shared MCP process
needs the Synveda Session ID on each tool call; an MCP connection alone does
not record the full agent session. Use the [client matrix](CLIENT_SUPPORT.md)
for each host's capabilities. Claude Code's plugin already includes
its own MCP entry.

### Directory synchronization

OIDC sign-in and SCIM provisioning are separate. Once the external issuer is
configured, issue a SCIM credential with
`synveda scim token issue --label directory` and supply the printed token once
to the directory provider. The
tenant endpoint is `https://YOUR_HOST/scim/v2`. Directory groups and members
do not grant scope access on their own; an authorized operator creates a
directory access assignment through the public API. For Microsoft Entra ID,
set the issuer's `external_id_claim` to `oid` so login and provisioning use the
same stable identity. See the [OpenAPI contract](api/openapi.json) for the
assignment request.

### Audit, keys and export

An administrator with tenant-wide `audit.read` can inspect and export the
content-free audit chain:

```sh
synveda audit tail --limit 20
synveda audit verify
synveda audit export --output audit-chain.json
synveda audit verify-export audit-chain.json
```

Keep the deployment encryption key with database recovery material. The
native CLI can inspect or rotate a tenant data key and create a sealed export:

```sh
tenant_id="PASTE_TENANT_UUID_HERE"
synveda tenant key status --tenant "$tenant_id"
synveda tenant key rotate --tenant "$tenant_id"
synveda tenant export --tenant "$tenant_id" --out tenant.svexp
```

The tenant export contains Knowledge history and audit evidence. It has no
matching import or complete tenant-erasure operation. Use the platform's
paired database and key backup for deployment recovery.

Source and reference Compose deployments also have an encrypted off-host PITR
path with a sealed custody bundle and a KEK replacement ceremony
(`synveda kms rewrap`). Both are described in the
[Compose guide](../deploy/compose/README.md#encrypted-off-host-pitr-recovery).
They are rehearsed locally but not yet qualified against an off-host store.

## Stop, recover and upgrade

```sh
./synveda-compose status
./synveda-compose down
./synveda-compose up
```

`down` removes containers and networks but preserves volumes and keys.
The [Docker guide](../deploy/compose/PREBUILT.md#backup-and-restore) has the
backup and restore procedure. Keep both the Synveda and Keycloak databases
with the matching encryption and identity material. For Kubernetes, use its
[operations guide](../deploy/helm/synveda/OPERATIONS.md).

Published v0.4.6 keeps the exact v0.4.3 schema epoch 3 baseline and applies
forward migrations `0002` to `0010`. No upgrade between published releases or
joint recovery drill is qualified yet, including v0.4.5-to-v0.4.6; back up the
database pair and keys, and keep retained data until an upgrade for the pair
you need is qualified. Older epochs and v0.4.0's different baseline checksum are refused. A
reset deletes data and requires the exact confirmation in the Docker guide.
Application rollback does not reverse a database migration. The
[production readiness assessment](PRODUCTION_READINESS.md) states current
platform, recovery and availability limits.

The unreleased 0.4.7 source adds `0011` for operator-managed PostgreSQL 14–18
(ADR-0152). Stop application writers and use the normal migration command before
starting a source binary; disposable local acceptance is not a published
upgrade or production recovery qualification.
