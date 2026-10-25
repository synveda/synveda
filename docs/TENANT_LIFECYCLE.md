# Tenant lifecycle operations

Deployment operators can suspend, inspect, resume, archive and restore one
tenant without changing any other tenant. The decision record is
[ADR-0157](adr/adr-0157-tenant-lifecycle-suspension-archive-and-erasure.md).
These commands are deployment administration. A tenant's own
administrators cannot run them, and they never return tenant content.

| Command | Database credential | Changes state |
| --- | --- | --- |
| `synveda tenant inventory` | any role that reads the tenant under forced RLS, normally the gateway role | no |
| `synveda tenant suspend` | the exact migrator role from the deployment role contract | yes |
| `synveda tenant resume` | the exact migrator role, plus the deployment KEK when the tenant has keys | yes |
| `synveda tenant archive export` | a configured database administrator that bypasses row security, plus the KEK | no tenant rows; chains `tenant.archived` |
| `synveda tenant archive verify` | none; the KEK only | no |
| `synveda tenant archive import` | a configured database administrator, plus the KEK; gateway and worker stopped | yes |

Every command needs `DATABASE_URL` or `DATABASE_URL_FILE`. Suspension and
resumption also need `SYNVEDA_DATABASE_ROLES` or
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

- be at the archive's schema epoch, baseline revision and migration head (on
  either lineage and any admitted PostgreSQL major);
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

## Not yet available

Tenant erasure is not shipped. The earlier `synveda tenant export` command
still produces the sealed Knowledge and audit export, which has no import.
Compose and Helm one-shot wrappers for these commands are not shipped; run the
native CLI where it can reach the database with the stated credential.
