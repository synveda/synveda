# Governed Git source refresh

Git source refresh uses ordinary ImportArtifacts and Capture review. A refresh observes source
files and may propose changes; the existing approval/apply lifecycle publishes
Knowledge. [ADR-0148](../adr/adr-0148-inbound-git-source-reconciliation.md)
defines the bounded contract. Local real-Git acceptance is separate from live
private GitHub qualification.

## Configure a local source

Run an operator-prepared Unix gateway and core worker with installed Git. Set
`SYNVEDA_GIT_SOURCE_ROOT` on both processes to the same retained absolute physical
directory, owned by their operating-system user with mode `0700`. The default
bundled image does not install Git or mount this directory. An unset setting
disables the reader; invalid custody refuses startup.

Provision a bare repository at `ROOT/TENANT/PROJECT_SCOPE/TARGET.git`. The root,
tenant directory, scope directory and bare repository must be private, owned
directories without symlinks. Provisioning and fetching that repository are
operator actions. Synveda reads immutable objects and never checks out files,
executes scripts/hooks/filters, follows submodules or lazily fetches missing
objects.

Attach the repository to the project through the existing repository API/CLI.
Publish a complete governed Configuration document at the actual project scope,
with sorted unique source IDs and paths, for example:

```json
{
  "git_sources": [{
    "source_id": "iris",
    "repository_id": "<actual attached repository UUID>",
    "read_target": "fixture",
    "git_ref": "refs/heads/main",
    "allowed_paths": ["conventions/trace.md", "procedures/retries.md"],
    "enabled": true
  }]
}
```

This fragment belongs in the complete document, not a standalone Configuration.
Use the console Configuration source editor or the normal Configuration CLI
publish/bind flow. Configuration review and current scope authority still apply.
Custom policy packs must explicitly permit scope-only `SourceRead`; refresh also
requires `KnowledgeWrite`. Configuration grants neither permission.

## Configure private GitHub reads

Use a private, active, nonfork repository on GitHub.com. Independently obtain its
numeric repository ID. Provision a fine-grained PAT restricted to that repository
with Metadata read and Contents read permissions. Seal this distinct document
through the existing tenant secret boundary:

```json
{
  "format": "synveda-github-source-credential-v1",
  "repository_id": 123456789,
  "token": "github_pat_REPLACE_WITH_THE_READ_TOKEN"
}
```

```sh
synveda tenant secret put --tenant TENANT --scope PROJECT_SCOPE --kind source_read \
  --label source.iris --provider github --from /private/github-source.json
```

Set `SYNVEDA_GIT_SOURCE_REMOTES_FILE` on gateway and core worker to the same
operator-owned allowlist. It contains references and identity assertions only:

```json
{
  "format": "synveda-github-source-targets-v1",
  "targets": [{
    "tenant_id": "<actual tenant UUID>",
    "scope_id": "<actual project scope UUID>",
    "target": "private-iris",
    "owner": "example-owner",
    "repository": "iris-context",
    "repository_id": 123456789,
    "secret_reference": "synveda-secret://<actual secret UUID>"
  }]
}
```

Owner/repository names are canonical lowercase ASCII, without `.git`. The
allowlist permits at most 128 mappings/64 KiB. Its exact tenant/project scope,
repository URI and secret scope must agree. Configure the attached repository in
`git_sources`, use `read_target: "private-iris"`, and include `github` in the
complete document's `allowed_external_providers`. SourceRead and KnowledgeWrite
remain independent requirements. An export credential or a remote mapping at
another scope cannot supply inbound authority or a local fallback.

Synveda sends GETs only to `https://api.github.com`, validates repository ID/name
and privacy before reading, ignores response URLs, and disables redirects and
proxies. It verifies complete nonrecursive tree objects and decoded blob hashes.
JSON framing also counts against the remote byte allowance. Failed/truncated
reads preserve the last complete observation. Key opening and HTTPS waits hold
no tenant transaction. Rotation/revocation during fetch blocks final commit;
inspect its operation and request a new refresh after custody is restored.

Rotate using the same secret put label/reference; revoke through `synveda tenant
secret revoke --tenant TENANT SECRET_ID`. For a repository rename, update the
operator mapping and attach/configure its new canonical URI with the same pinned
numeric ID. The permanent root preserves old suppression and pending review
identity. A different ID at an old URI or ambiguous retained roots refuses
rebinding; retained data is preserved. Disconnect remains the Configuration
`enabled: false` operation described below.

## Inspect, refresh and review

```sh
synveda sources list PROJECT
synveda sources status PROJECT iris
synveda sources refresh PROJECT iris --idempotency-key iris-refresh-001
synveda okf inspect IMPORT_JOB
```

The console Imports page lists authorized configured sources and retains the
operation receipt. Source status returns the latest receipt; the public
`GET /v1/operations/{id}` and cancellation API address the exact operation. Refresh queues worker work and performs no transport on the
request path. Retry the same key after an uncertain response; a new key requests
a new observation. An active refresh must finish or be cancelled before another
refresh for that source. Use the receipt's actual import job to inspect and
review candidates through Capture. Unchanged or deletion-only observations have
no import job.

Each observation binds a once-pinned commit/tree and every exact configured path.
Limits are 16 descriptors, 32 paths per source, depth 16, 128 KiB per blob,
2 MiB cumulative object bytes, 128 object reads and 30 seconds per execution.
Missing objects, unsupported entries, bounds and transport failures keep the
successful cursor unchanged. The operation records a safe failure; that failure
cannot establish deletion. Three bounded attempts precede terminal exhaustion.

Freshness compares each retained dependency's path/hash with the latest complete
observation. A relevant change or deletion requires targeted review; an unrelated
commit does not stale every item. Status distinguishes the last complete
observation from failed work. Historical source evidence remains independently
readable under current policy until source controls remove it.

Setting `enabled` to false disconnects future synchronization and preserves
approved Knowledge. A source ID cannot be redirected to a different repository.
Withdraw/Forget the exact inspected ImportArtifact through the existing source
controls to suppress its permanent repository/path/byte alias. Reconnection,
re-import and branch reversion preserve that suppression. Different bytes can
still produce a review. Familiar bytes returning after a different approved
revision create a review against that exact current head; repeating the same
comparison reuses its review.

## Run local acceptance

```sh
bash scripts/db-test.sh -p synveda-gateway --test source_reconciliation -- --test-threads=1
SQLX_OFFLINE=true cargo test -p synveda-git --lib sources::tests
```

These are real local Git and ordinary-role PostgreSQL/protocol tests. They do not
establish live GitHub access, native harness task success or model-quality gains.

The [Iris domain walkthrough](iris-domain.md) composes actual Git refresh, separate review, exact context/handoff, fixed Procedure qualification and source forgetting.
