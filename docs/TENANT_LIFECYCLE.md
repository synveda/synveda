# Tenant lifecycle operations

Deployment operators can suspend, inspect, resume, archive, restore and erase
one tenant without changing any other tenant. The decision record is
[ADR-0157](adr/adr-0157-tenant-lifecycle-suspension-archive-and-erasure.md).
These commands are deployment administration. A tenant's own
administrators cannot run them, and they never return tenant content.

| Command | Database credential | Changes state |
| --- | --- | --- |
| `synveda tenant inventory` | any role that reads the tenant under forced RLS, normally the gateway role | no |
| `synveda tenant suspend` | the exact migrator role from the deployment role contract | yes |
| `synveda tenant resume` | the exact migrator role, plus the deployment KEK when the tenant has keys | yes |
| `synveda tenant erase` | the exact migrator role | yes, irreversibly |
| `synveda tenant archive export` | a configured database administrator that bypasses row security, plus the KEK | no tenant rows; chains `tenant.archived` |
| `synveda tenant archive verify` | none; the KEK only | no |
| `synveda tenant archive import` | a configured database administrator, plus the KEK; gateway and worker stopped | yes |

Every command except `synveda tenant archive verify` needs `DATABASE_URL` or
`DATABASE_URL_FILE`. Suspension,
resumption and erasure also need `SYNVEDA_DATABASE_ROLES` or
`SYNVEDA_DATABASE_ROLES_FILE`. Both refuse any other principal, target
database or schema head before writing. Resumption reads `SYNVEDA_KMS_KEY` or
`SYNVEDA_KMS_KEY_FILE` and the matching `SYNVEDA_KMS_KEY_REF` the same way as
`synveda tenant key status`.

## Inspect a tenant first

```sh
synveda tenant inventory --tenant "$tenant_id" --json > inventory.json
```

The inventory reads one consistent read-only snapshot. It counts the tenant's
rows in each of the 116 tenant-bearing tables, up to `--cap` rows per table
(default 1,000,000; `capped` marks a table that holds more). Each table has an
archive treatment (`portable`, `derived` or `excluded`) and an erasure treatment.
The inventory also names the deployment tables a tenant can reach and every
copy held outside PostgreSQL: application-owned Git export repositories,
customer-owned source and remote repositories, identity-provider accounts,
delivered client copies, operator-held archives, deployment backups and
telemetry. It works on active and suspended tenants. The report contains
table names and counts only.

## Suspend

```sh
synveda tenant suspend --tenant "$tenant_id" --reason incident
```

`--reason` is one of `incident`, `maintenance`, `offboarding`, `recovery` or
`administrative`. Free text is refused so the audit chain never carries it.

The command takes the tenant's exclusive admission lock. It waits up to about
four seconds for transactions that were admitted earlier, then commits the
new status and chains `tenant.suspended`. The output is:

```json
{"tenant":"…","status":"suspended","changed":true}
```

After it returns:

- every new request, fresh OIDC login, stored console session, refreshed
  token and service bearer for that tenant gets the uniform 401 on every
  gateway replica;
- any later database transaction of a request or worker that started before
  the suspension is refused, so in-flight work cannot commit;
- worker sweeps skip the tenant. A lease that was held expires normally and
  is reclaimed after resumption. At most the attempt in flight is consumed;
- responses committed earlier and context already delivered to agents are not
  recalled;
- each refused request is recorded on the tenant's own audit chain as
  `tenant.resolution.denied`.

Repeating the command on a suspended tenant prints `"changed":false` and
chains nothing. Key custody (`synveda tenant key …`, `synveda kms rewrap`)
and break-glass policy repair (`synveda policy apply|clear`) keep working
while a tenant is suspended.

## Resume

```sh
synveda tenant resume --tenant "$tenant_id" --reason recovery
```

Before changing the row, resumption proves that:

1. every stored policy pack for the tenant compiles with this build, so the
   resumed tenant cannot fail the process-wide policy freshness check and
   withdraw other tenants; and
2. the configured KMS unwraps every stored generation of the tenant's data
   key, so its sealed secrets can still open.

Then it takes the exclusive lock, confirms the key rows are the ones it
checked, flips the status and chains `tenant.resumed` with the pack and key
generation counts.

## Refusals

Every refusal exits non-zero, changes nothing and starts with a stable reason:

| Reason | Meaning | Next action |
| --- | --- | --- |
| `tenant_busy` | admitted work held the tenant past the bounded wait | retry; the tenant was not changed |
| `tenant_state_changed` | another operator changed the tenant meanwhile | rerun the inventory and decide again |
| `tenant_not_found` | no tenant has that ID | check the ID |
| `policy_invalid` | a stored pack does not compile with this build | repair it with `synveda policy apply` or remove it with `synveda policy clear`, then resume |
| `key_unavailable` | the configured KMS cannot open a stored tenant key generation | supply the KEK custody set that wraps it, then resume |
| `tenant_erasing` | erasure has started | only `synveda tenant erase` may continue it |
| `tenant_erased` | the tenant is erased | nothing; erasure is one-way |

A refusal from the migrator authority proof names the principal, target or
schema mismatch instead. Fix the credential or migrate first; do not grant
the runtime roles write access to `tenants`.

## Archive and restore one tenant

The archive (`synveda.tenant-archive.v1`) is portable tenant recovery. It is
not deployment recovery: keep using the paired backup and PITR procedures in
the [Compose guide](../deploy/compose/README.md) for whole deployments.

Archive export and import use a database administrator named in the role
contract's `administrators` list, connected directly rather than through
`SET ROLE`, that bypasses row security. Thirteen tables hide withdrawn,
suppressed or erased rows from every product role. Those rows are retained
evidence, so only such a reader can archive the whole tenant. Every statement
still names the tenant explicitly. Import also suspends and restores table
triggers and writes every table, so its administrator must be a superuser or
own the tables. Only the bundled Compose superuser has been exercised; an
external-PostgreSQL administrator without those rights is untested and would
fail before committing anything.

```sh
synveda tenant archive export --tenant "$tenant_id" --out tenant.svarc
synveda tenant archive verify --archive tenant.svarc
```

Export reads one consistent snapshot, which may be of an active or a suspended
tenant. Suspend the tenant first when the restored copy must be its final state.
It needs the KEK that wraps the tenant's current key, which must be
provisioned. It writes a private `.tenant.svarc.<archive-id>.partial` file next
to `--out` and publishes it with a rename that never replaces an existing file.
An interrupted or refused export leaves no archive at `--out`; delete a
leftover `.partial` file and rerun. The receipt names the archive ID, manifest
digest, table, row and frame counts, bytes, frozen audit head and key
generation. `tenant.archived` is then chained on the source tenant.

Bounds: `--max-bytes` (default 8 GiB, at most 64 GiB) and `--timeout-seconds`
(default 3600) apply to the whole run and to each statement. One frame, and
therefore one row, holds at most 32 MiB of COPY text, which bounds memory.

`verify` needs no database. It authenticates every frame and checks every
table digest, the row counts and the manifest.

The archive contains every portable tenant table exactly. Knowledge revision
embeddings are excluded because the index sweep rebuilds them after
resumption. Console session index rows are excluded because console sessions
are deployment-sealed and users sign in again. The cleartext header names only
the archive and tenant IDs, the schema identity, the product version, the
snapshot time and the KEK-wrapped tenant key generation; slugs, names and
rows are sealed.

### Restore

```sh
synveda tenant archive import --archive tenant.svarc
synveda tenant inventory --tenant "$tenant_id" --json
synveda tenant resume --tenant "$tenant_id" --reason recovery
```

The target must:

- be at the archive's schema epoch, baseline revision and migration head.
  Restore onto the same PostgreSQL major and lineage is tested; another
  admitted major or the other lineage is accepted but not yet qualified;
- hold neither the tenant ID nor its slug;
- have its KMS configured with the KEK named in the archive; to move to a
  different KEK, restore under the original one first, then use the
  [KEK replacement ceremony](../deploy/compose/README.md#encrypted-off-host-pitr-recovery);
- have every gateway and worker process stopped. `--writer-drain-seconds`
  (default 30) bounds the wait.

Import authenticates the whole archive before writing anything, then restores
in one transaction. It recomputes every table's digest, re-verifies the audit
chain to exactly the frozen head and chains `tenant.restored` directly after
it. The tenant arrives suspended. Inspect it, restart the gateway and worker,
then resume it. An interrupted import leaves nothing behind and can be rerun.
Rerunning a completed import reports `already_restored`. The source tenant is
never changed by export or import; keep it until the restored copy is verified.

Archive refusals exit non-zero and change nothing:

| Reason | Meaning |
| --- | --- |
| `archive_corrupt` | a frame, the header or the manifest does not authenticate or match |
| `archive_truncated` | the file ends before its manifest |
| `archive_too_large` | the file exceeds `--max-bytes` |
| `archive_unsupported` | another format version, frame bound or table set |
| `archive_exists` | `--out` already exists |
| `archive_timeout` | `--timeout-seconds` elapsed |
| `key_unavailable` | the configured KMS cannot open the archive's tenant key |
| `schema_incompatible` | the target's schema identity or a table's columns differ |
| `tenant_collision` | the target already holds the tenant or its slug |
| `writers_connected` | gateway or worker sessions stayed connected |
| `restore_refused` | a restored table, constraint or audit chain differs; the transaction rolled back |

## Erase a tenant

Erasure irreversibly deletes a suspended tenant's rows. It follows the interim
fixture retention rules of ADR-0157 and is **not production-qualified**:
retention, legal holds, certificate contents and support authority have no
owner decision yet. The command therefore needs two acknowledgements:

```sh
synveda tenant inventory --tenant "$tenant_id" --json > before.json
synveda tenant erase --tenant "$tenant_id" --reason offboarding \
  --force --fixture-rules > receipt.json
```

What remains afterwards:

- the tenant row as a tombstone: ID, slug and status `erased`, with the
  display name cleared. The slug stays reserved, so importing an archive of
  this tenant later refuses with `tenant_collision`;
- the tenant's content-free audit chain, ending with `tenant.erased`;
- the `tenant_erasure` operation and its attempts. The operation's result is
  the receipt.

Everything else is deleted. That includes rows hidden from every product role
by withdrawal, suppression or unproven lineage, Knowledge erasure tombstones,
and the console session custody named by the tenant's session index.

How a run proceeds:

1. Under the exclusive admission lock, erasure refuses an active tenant. If
   any Knowledge revision carries `legal_hold` or `retention_hold`, it stops
   before deleting anything: it records a blocked operation, chains
   `tenant.erasure.held` and exits with `erasure_held`. The tenant stays
   suspended and can be resumed. Otherwise erasure records the operation,
   moves the tenant to `erasing` and chains `tenant.erasure.started`.
2. Rows are deleted in an order derived from the live foreign-key catalogue,
   so every referencing row goes before the row it references. Each step is
   one migrator transaction that deletes at most `--chunk-rows` rows (default
   2,000, at most 50,000). The step records the measured count under the
   operation's lease in the same transaction. Some tables need a special
   order:
   - self-referencing scopes go leaves first;
   - Configuration, Knowledge, policy relaxation, Skill and Tool parents go
     with their current version, after every other version;
   - the inbound source tables, whose mutual references are all deferrable,
     go in one step bounded at 1,000,000 rows. A larger set refuses with
     `erasure_unit_too_large` before anything is deleted.
3. Finally, erasure re-counts every tenant table, hidden rows included, and
   refuses with `erasure_incomplete` unless only the retained rows remain.
   It then marks the tenant `erased`, stores the receipt and chains
   `tenant.erased` with the receipt digest.

While a tenant is erasing or erased, it refuses requests, key custody, policy
repair, suspension, resumption and archive export. `synveda kms rewrap` skips
it: erasure deletes an erasing tenant's keys without opening them, and an
erased tenant has none.

Tenant convergence refuses an existing tenant whose status or name differs.
Compose requires `SYNVEDA_BOOTSTRAP_TENANT_ID` and converges that tenant as
active on every `compose.sh up`, so while it is suspended or erased `up` stops
at that step; point the setting at another tenant first. On Helm, clear
`install.tenant` before suspending or erasing the tenant it names.

A run continues across interruptions:

- `--timeout-seconds` (default 3600) ends a run after whole steps, hands the
  lease back and exits with `erasure_timeout`. Rerun the same command to
  continue.
- A process that dies holds its 30-second lease until it expires. A rerun
  after that reclaims it as a new attempt. A rerun while another process
  holds the live lease refuses with `erasure_in_progress`.
- Each count is committed with its deletion, so no row is counted twice.
- A rerun on an erased tenant prints the stored receipt with
  `"changed":false`.

The receipt (`synveda.tenant-erasure-receipt.v1`) contains no tenant content.
It records:

- the operation, its attempts, the plan digest, and the start and completion
  times;
- the measured deletions per table and the console custody rows deleted;
- the retained row counts, measured before `tenant.erased` is chained;
- `residual_rows: 0` and a BLAKE3 digest of the receipt.

It also names every copy outside this database and leaves each one
unchanged:

- Git export repositories. Remove the tenant's directory under the gateway's
  Git export root yourself.
- Customer repositories and identity-provider accounts.
- Delivered client copies.
- Operator-held archives, deployment backups and telemetry.

Short-lived CLI login handoffs carry no tenant reference and expire within
minutes; the receipt names them instead of deleting them.

Erasure refusals exit non-zero:

| Reason | Meaning |
| --- | --- |
| `erasure_unacknowledged` | `--force` or `--fixture-rules` is missing |
| `tenant_not_suspended` | suspend the tenant first |
| `erasure_held` | a Knowledge revision is held; nothing was deleted |
| `erasure_in_progress` | another process holds the live lease; retry after it expires |
| `erasure_lease_lost` | this run lost its lease to another process; rerun |
| `erasure_timeout` | `--timeout-seconds` elapsed; the committed steps stand, rerun |
| `erasure_unit_too_large` | the inbound source tables exceed one step's bound; nothing was deleted |
| `erasure_plan_unsupported` | the catalogue has a reference cycle that no unit declares; nothing was deleted |
| `erasure_incomplete` | rows remain after every step; rerun |

## Compose and Helm one-shots

On Compose, `deploy/compose/scripts/compose.sh tenant COMMAND...` runs one
lifecycle command in a one-shot on the private database network. It uses the
same project selection and lock as every other lifecycle action. Arguments
may contain only letters, digits and `. _ : / = -`.

```sh
deploy/compose/scripts/compose.sh tenant suspend --tenant "$tenant_id" --reason incident
deploy/compose/scripts/compose.sh tenant inventory --tenant "$tenant_id" --json
deploy/compose/scripts/compose.sh tenant erase --tenant "$tenant_id" \
  --reason offboarding --force --fixture-rules
```

`suspend`, `resume`, `inventory` and `erase` run in the `tenant-lifecycle`
one-shot, which receives the migrator URL, the role contract and the KEK.
`archive export|verify|import` run in the `tenant-archive` one-shot, which
needs two inputs:

- `SYNVEDA_TENANT_ARCHIVE_DIR`: an absolute private directory (mode 0700,
  owned by the runtime UID:GID) outside the repository and the secrets
  directory. It is mounted at `/var/lib/synveda/archives`; pass archive paths
  under that mount.
- `SYNVEDA_TENANT_ARCHIVE_DATABASE_URL_FILE`: an absolute private file (mode
  0600, runtime UID:GID) holding a configured administrator URL. For bundled
  PostgreSQL that is the `synveda_owner` login,
  `postgres://synveda_owner:<postgres_owner_password>@postgres:5432/synveda`.
  Only this one-shot receives it; create the file for the run and delete it
  afterwards.

`archive import` stops the gateway and worker, imports, then recreates them.
The restored tenant arrives suspended. If an import refuses, run
`compose.sh up` to bring the writers back.

On Helm, set `tenantLifecycle.enabled=true`, `tenantLifecycle.command`
(`suspend`, `resume`, `inventory` or `erase`), `tenantLifecycle.tenant` and
`tenantLifecycle.reason`, then `helm upgrade`. Each revision runs one Job
named `<release>-tenant-<command>-<revision>`. The Job receives the migrator
Secret, the role ConfigMap and the KMS Secret, and runs on the install Job's
placement under its own network policy.

- `erase` also needs `tenantLifecycle.force=true` and
  `tenantLifecycle.fixtureRules=true`; the chart refuses to render without
  them.
- Read the result with `kubectl logs job/<name>`, and set `enabled=false` on
  the next upgrade.
- The Job does not retry (`backoffLimit: 0`). To retry, upgrade again; an
  interrupted erase continues where it stopped.

Archive export and import are not wrapped in Helm, because the chart never
gives a product container an administrator credential. Run them from a host
that reaches the database with that credential.

## Not yet available

- Production-qualified erasure: the owner decisions above are open.
- A Helm wrapper for archive export and import.
- Evidence that the Compose and Helm one-shots run against a live
  deployment. They are covered by rendering and lifecycle-script tests only.
- The tenant one-shot Compose files in the released reference archive. Like
  `kek-rewrap`, `compose.sh tenant` currently needs a source checkout.

The earlier `synveda tenant export` command still produces the sealed
Knowledge and audit export, which has no import.
