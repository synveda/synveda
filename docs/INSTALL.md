# Install and use Synveda

Synveda keeps knowledge from agent sessions in a project or personal scope.
People can inspect its source, review proposed changes and choose what later
sessions may use. The web console handles everyday work; the CLI and public API
support automation and advanced administration.

<!-- installation-version: 0.4.3; publication: published -->
The current published release is **v0.4.3**. Choose the route that matches your
infrastructure:

| Route | Guide | When to use it |
| --- | --- | --- |
| Prebuilt Docker bundle | [Download and verify](../deploy/compose/PREBUILT.md#download-and-verify) | First local evaluation with bundled PostgreSQL and Keycloak. |
| Kubernetes chart | [Chart installation](../deploy/helm/synveda/README.md) | A cluster with persistent storage and an operator who can manage its namespace. |
| Existing PostgreSQL or OIDC | [Compose reference](../deploy/compose/PREBUILT.md#use-existing-infrastructure) or [chart configuration](../deploy/helm/synveda/CONFIGURATION.md) | You already operate the database, identity provider or both. |
| Source checkout | [Development setup](DEVELOPMENT.md#running-your-changes) | You are changing Synveda or running its tests. |

## First run with Docker

Use the [README download commands](../README.md#install-for-local-evaluation)
to verify and extract the prebuilt bundle. It pulls fixed container images and
does not compile source on the host.

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

### 2. Start and sign in

```sh
./synveda-compose up
# With demoAccounts set to true:
./synveda-compose credential author
```

Open `http://localhost:8080/console/`, replacing `8080` with your configured
port. Sign in as `synveda-demo-admin` with the password printed by the second
command. The password is generated for this installation. If you disabled demo
accounts, sign in with an identity from your configured OIDC provider; the
[provider guide](../deploy/compose/PREBUILT.md#use-existing-infrastructure)
covers that setup.

**Getting started** asks you to create a workspace and project. A workspace is
a place for a person or team; a project is where related sessions and knowledge
live. You can attach a repository and choose an agent client afterward.

### 3. Run the sample and inspect the result

```sh
./synveda-compose sample
```

This creates fictional **Northstar Delivery → Ingestion API** data through
Synveda's public API. In **Sessions**, inspect the source session. In **New
Learnings**, open its proposed ingestion retry finding. The sample does not
approve its own proposal.

To complete the review, sign out and use `./synveda-compose credential reviewer`
to get the password for `synveda-demo-member`. Sign in as that reviewer, open
**Advanced → Reviews**, inspect the proposal, then approve and apply it.
Select **Ingestion API** in **Context** and request context about ingestion
retries. The result should cite the approved knowledge revision and its source.
The [step-by-step walkthrough](../deploy/compose/PREBUILT.md#first-workspace-and-sample)
also covers the sample Skill proposal.

## Configure access and behavior

### People and scopes

Use **People** to invite teammates and grant access to a workspace or project.
A workspace grant reaches its projects; a project grant stays in that project.
Each person's private scope is separate. The initial administrator comes from
the configured first-login identity group; later grants are made through
Synveda. Access is checked again for every operation.

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

Install the [native client package](CONSUMER_CLI.md#release-downloads) on the
machine running your agent, then sign in to the gateway:

```sh
synveda login --gateway http://localhost:8080
synveda whoami
```

Use the actual gateway URL if it differs. Then follow the setup for your
client:

| Client | Setup |
| --- | --- |
| Claude Code | [Plugin and hooks](../adapters/claude-code/README.md) |
| Codex CLI | [Native hooks and MCP](integrations/codex.md) |
| GitHub Copilot CLI | [Native hooks and MCP](integrations/copilot-cli.md) |
| Other MCP clients | `synveda mcp install --print` shows the entry; see the [support matrix](CLIENT_SUPPORT.md) for host capabilities. |

The [support matrix](CLIENT_SUPPORT.md) is generated from the client registry
and lists versions, platforms and capabilities.
Python and TypeScript SDKs have an initial API slice; their
[guide](../sdks/README.md) states the publication boundary.

## Working with knowledge

A Session records agent activity. Capture turns eligible evidence into a
reviewable candidate. **New Learnings** lets you accept, edit, merge, replace
or dismiss it. A governed change creates an immutable Knowledge revision only
when it applies. **Knowledge** shows the current revision and source; **Context**
shows which revisions a session received and why.

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

Schema epoch 3 is the current baseline. Older schemas are refused; they are
not migrated. A reset deletes data and requires the exact confirmation in the
Docker guide. Application rollback does not reverse a database migration.
The [production readiness assessment](PRODUCTION_READINESS.md) states current
platform, recovery and availability limits.
