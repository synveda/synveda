# Tenant lifecycle operations

Deployment operators can suspend, inspect and resume one tenant without
changing any other tenant. The decision record is
[ADR-0157](adr/adr-0157-tenant-lifecycle-suspension-archive-and-erasure.md).
These commands are deployment administration. A tenant's own
administrators cannot run them, and they never return tenant content.

| Command | Database credential | Changes state |
| --- | --- | --- |
| `synveda tenant inventory` | any role that reads the tenant under forced RLS, normally the gateway role | no |
| `synveda tenant suspend` | the exact migrator role from the deployment role contract | yes |
| `synveda tenant resume` | the exact migrator role, plus the deployment KEK when the tenant has keys | yes |

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

## Not yet available

Portable archive export and import, and tenant erasure, are not shipped yet.
Until they are, `synveda tenant export` produces the earlier sealed Knowledge
and audit export, which has no import. Deployment recovery remains the paired
backup and PITR procedures in the [Compose guide](../deploy/compose/README.md).
