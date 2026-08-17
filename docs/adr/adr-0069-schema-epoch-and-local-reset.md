# ADR-0069: an authoritative schema epoch, a startup guard that refuses everything else, and a local reset that destroys rather than translates

- **Status**: Accepted
- **Date**: 2026-08-17
- **Feature(s)**: CPR-2
- **Deciders**: sujitn

## Context

ADR-0068 decision 3 committed to a fresh schema epoch with no old-data
migration, and said what must follow from it: *"A database carrying the old
epoch is rejected at startup with a reset instruction, rather than upgraded,
half-read, or silently accepted."* This ADR is the mechanism, and it exists
because the decision as stated has three holes an implementation has to close
before the rest of the programme can lean on it.

**Nothing in the database says which model its rows are in.** The base commit
has 38 migrations, `sqlx::migrate!()`, and no marker of any kind
(`docs/implementation/synveda-context-platform.md` §1.2: *"There is no
down-migration, no reset guard, and no epoch marker: a database at any prefix
of this sequence is accepted and brought forward"*). A binary pointed at an
old database cannot tell it from a new one, and `MIGRATOR.run` will happily
advance it — which is the silent acceptance the decision forbids, available
today by running the command the documentation already tells operators to
run.

**`_sqlx_migrations` is not that marker, and using it would be worse than
having none.** It answers "which of *this binary's* migrations have been
applied", which is a question about the current chain. Prompt 33 squashes the
chain to a single `0001`; on the day it does, every version number in that
table becomes meaningless as evidence about the model. It also moves on every
ordinary release, so a guard built on it would have to be edited for reasons
that have nothing to do with the model changing — and a guard that is edited
routinely stops being read.

**There is no reset.** `scripts/uninstall.sh --purge` destroys the Docker
volumes, which is both too much (it takes Temporal's `temporal` and
`temporal_visibility` databases, which live in the same `pg-data` volume) and
the wrong shape — it removes the installation rather than resetting the
database. `scripts/db-test.sh` does exactly the right thing for a scratch
database and does it in shell, inside `docker compose exec`, using `psql`,
which an installed release does not have. So the instruction the guard is
supposed to print does not currently name anything.

The forces:

- **Pre-1.0, once.** ADR-0068 accepted "every existing database is a reset"
  and asked that it be *loud rather than silent*. Loud means the process
  stops, names the reason, and names the fix — not a warning in a log.
- **Seed §2.2 — policy is enforced, never advisory.** A guard that can be
  reached around is the same shape as a PDP that can be bypassed. There must
  be no path from a binary to an old database's rows.
- **A migrator is refused, and refusing it is a claim that has to be
  checkable.** "There is no old-to-new data migrator" is easy to assert and
  easy to violate by accident — one `insert ... select` in the migration that
  introduces the epoch and it is gone.
- **The gateway is allowed to boot without a database.** ADR-0007's readiness
  design is deliberate: `connect_lazy`, so an outage is reported by `/readyz`
  instead of a crash loop. A startup guard must not break that, and must not
  leave a hole where a database that comes up *after* boot slips past a check
  that already ran.
- **CLAUDE.md: sqlx compile-time checked queries only, no string-built SQL,
  ever.** `DROP DATABASE` takes an identifier, and Postgres has no
  parameterised form of it.

## Decision

**1. The epoch is its own number, in its own table, and is not derived from
the migration chain.** `schema_metadata` is a single-row table
(`epoch`, `migration_head`, `created_at`, `created_by_version`, `updated_at`)
created by migration `0039_schema_epoch.sql`. `epoch` changes when the model
underneath changes incompatibly and for no other reason; adding a migration
never touches it. `migration_head` is diagnostic — it tells two databases at
the same epoch apart and is quoted back in refusals — and is explicitly not
what the guard decides on. The current epoch is **1**: the context platform.
Everything before it carries no marker at all, which is how a pre-cut database
presents.

**2. The marker is written by Rust after a successful migration, not by the
migration.** Two of its four facts are only available to the running binary:
the product version that created the epoch, and the head actually reached.
`created_at` and `created_by_version` are written once and never rewritten —
"which release minted this database" is a fact about the past, and a column
that tracked the current binary would answer a question nobody asked.

**3. The refusal of a pre-cut database is one implementation, in Rust, before
the migrator runs.** `synveda_store::epoch::preflight` refuses any database
that has a schema but no marker, and `synveda_store::migrate` calls it first —
so a refused database is left byte for byte as it was found rather than
half-advanced. The test is "any table in `public` that is neither the marker
nor `_sqlx_migrations`", not a list of table names, because every table name
this programme could sentinel on is one a later prompt deletes.

**4. The startup guard runs in two places, and the second one is not
redundant.** The gateway calls `epoch::verify` before it serves anything and
**refuses to start** on a verdict; `/readyz` asks the same question on every
probe. The second exists because the first cannot cover the case the boot
contract creates: the gateway may start without a database, so a database that
arrives afterwards would otherwise be behind a check that had already run.
Every store-level CLI command goes through `connect_current_epoch`; the two
that do not are the two that cannot — `db migrate`, which creates the epoch,
and `reset`, which is what a refusal tells you to run.

**5. An outage is not a verdict.** `SchemaEpochError::Unreachable` is
distinguished from every other variant and is the only one the gateway boots
through. Reporting "cannot reach Postgres" as "your database is the wrong
epoch" would send an operator to destroy a database over a network problem.

**6. A newer epoch is refused differently from an older one.** `Older` and
`Missing` and `Malformed` print the reset command. `Newer` prints "upgrade
this installation" and names the reset command only to say **not** to run it:
a database from a later build holds data this one cannot read, and telling its
operator to destroy it would be the worst advice this guard could give.

**7. `synveda reset --database --force` drops and recreates the application
database — not the volume, not the installation.** Both flags are required and
each refuses separately: `--database` because the command names what it
destroys rather than defaulting to everything there is, `--force` because
nothing is destroyed by omission. It stops the host gateway first (a gateway
evicted by `WITH (FORCE)` stays alive and keeps serving from in-process
caches), drops and recreates the database, installs the extensions the compose
`initdb` script installs, migrates to the current epoch, and removes the
Tantivy sidecar. It preserves `kms.key`, the compose profile, the console
bundle, stored logins, the Docker volumes, and every other database on the
server.

**8. It refuses a database that is not on this machine.** `--force` says "yes,
destroy it"; it does not say "and I checked which server I am pointed at".
A non-loopback host is refused with the exact two statements to run by hand —
an escape hatch that stays deliberate rather than a flag that ends up in a
runbook.

**9. The one place SQL is built from a string is quarantined and validated.**
`DROP DATABASE` cannot be parameterised, so the database name is checked
against a grammar narrower than Postgres's own (ASCII letters, digits and
underscores; leading letter or underscore; ≤63 bytes), then double-quoted. The
validator is the placeholder this statement cannot have, and it is unit-tested
as one — against `synveda"; drop database temporal; --` among others. Anything
outside the grammar is refused rather than escaped.

**10. "No old-to-new data migrator exists" is asserted, not asserted-to.**
Three checks, two behavioural and one structural: a refused migration writes
nothing and leaves the rows it refused (tested); a reset carries **zero** rows
across (tested); and the epoch migration is pure DDL — every statement in it is
checked to be neither `select`, `insert`, `update`, `delete`, `copy` nor
`with`, because that file is the one place a translator would live. The same
test refuses any `.down.sql`, which would both make the epoch look reversible
and be the other place a translation hides.

## Options considered

**1. Derive the epoch from `_sqlx_migrations`.** No new table, and the data is
already there. Rejected in decision 1: it answers a question about the current
chain rather than about the model, it becomes meaningless the moment Prompt 33
squashes the chain, and it moves on every release — so the guard would have to
be edited routinely, which is how guards stop being read.

**2. Put the refusal in the migration, as a `RAISE EXCEPTION`.** Tempting: it
would catch a database migrated by `sqlx migrate` directly, not only one going
through `synveda_store::migrate`. Rejected because the refusal's whole value is
its message, which names a CLI command — and a copy of that message in SQL is a
second thing to keep in step with the CLI, in a language where nothing checks
that it still names a verb the binary has. The Rust seam is the one every path
in this repository goes through, and the tests pin that.

**3. Make the guard a warning and let the process start.** Rejected outright.
The failure mode is a gateway serving reads over rows in a model it does not
implement, which does not look like an error at any layer — it looks like data
that is missing, or a policy that decided oddly. ADR-0068 asked for loud.

**4. Reset by removing the Docker volume, or by wrapping
`uninstall.sh --purge`.** Simplest to write, and wrong twice. `pg-data` holds
Temporal's two databases as well as ours, so it destroys more than it was asked
to; and it is not available at all when `DATABASE_URL` points somewhere that is
not the bundled compose Postgres. `DROP DATABASE` is the smallest thing that
leaves nothing of the old epoch and touches nothing that was never ours.

**5. Reset by `DROP SCHEMA public CASCADE`.** Avoids needing a maintenance
connection or `CREATEDB`. Rejected: `pgmq` creates its own schema and holds the
observe queue's tables there, so this would leave a queue full of the previous
epoch's messages behind a schema that claims to be fresh — the exact partial
state the epoch exists to make impossible.

**6. Let `--force` destroy any database, anywhere.** The user asked for it, on
this argument. Rejected in decision 8: the documented deployment modes are all
loopback, the cost of being wrong is somebody else's production database, and
the by-hand path is two statements this command prints for them. A flag that
disables the check would be pasted into a runbook within a release.

**7. Add `--dry-run`, as `init`, `uninstall.sh` and `mcp install` all have.**
Consistent with the house style, and deliberately not done: `reset` has exactly
one effect and describes it in the refusal you get without `--force`, so a
dry-run would print the same paragraph a second time under a different flag.
If the command ever grows a second target, this is the first thing to revisit.

## Consequences

- **Positive.** "A database from before the cut is refused" stops being a
  sentence in an ADR and becomes three enforcement points and a demo. The rest
  of the programme can delete tables without leaving any binary able to read a
  half-cut database.
- **Positive.** The marker records provenance, so `created_by_version` answers
  "which release minted this database" for every deployment from here on —
  including the ones that will be forensically interesting after a
  33-prompt redesign.
- **Negative / accepted.** Every existing database is destroyed rather than
  upgraded, including a contributor's dev database and every deployment of
  v0.2.0. This is ADR-0068's accepted cost, paid here, once.
- **Negative / accepted.** The reset connects to the `postgres` maintenance
  database and requires the role to have `CREATEDB`. True of the bundled
  compose Postgres (its `synveda` role is the container's superuser) and of any
  local development server; a locked-down managed Postgres will refuse, and the
  error says which permission is missing and what to run instead.
- **Negative / accepted.** `AGE` is created best-effort. It is installed by the
  dev image and called by nothing in the product (ADR-0043 built the knowledge
  graph on indexed adjacency), so a server without it runs Synveda perfectly
  and only `crates/synveda-store/tests/graph_spike.rs` notices. `vector` and
  `pgmq` are required and a missing one is a hard error naming the extension.
- **Negative / accepted, and worth stating plainly.** This guard defends
  against pointing a binary at the wrong database. It is not a defence against
  an operator with SQL access, who can write any epoch number they like into
  `schema_metadata` — the same boundary ADR-0009 draws for RLS and ADR-0064
  draws for the KEK.
- **Reversal trigger.** If a prompt in this programme ever needs to read a row
  written before its own epoch — for a benchmark corpus, an evaluation
  fixture, anything — then decision 3 has failed and what is actually wanted is
  an *export/import* at the application layer, not a relaxed guard. The guard
  is not the place to make that possible; the OKF adapter (Prompt 23) is.
  Equally: if `epoch` is ever bumped for a change that was not incompatible,
  decision 1 has failed and the number has started tracking releases.

## Compliance notes

- **Tenancy.** `schema_metadata` carries no `tenant_id` and no RLS, which is
  structural satisfaction rather than an exemption — the same shape
  `console_sessions` (migration 0034) and `deployment_keys` (migration 0038)
  have. It is also structurally *necessary*: the guard reads the marker before
  a tenant is resolved, so a tenant-keyed policy would evaluate false and hide
  the marker from the check that exists to read it. The RLS completeness guard
  in `crates/synveda-store/tests/rls.rs` does not discover it, and the reason
  is recorded there.
- **Policy enforcement.** No new governed surface and no new decision point.
  `reset` is a store-level break-glass command in the family `db migrate` and
  `tenant create` already belong to (ADR-0008), for the same reason: it exists
  precisely for the moment there is no usable gateway. It writes no scope, no
  identity, no role binding and no record.
- **Audit.** Deliberately no audit event. The chain lives in the database this
  command destroys, so an event recording the destruction would be destroyed by
  it — and one written to the *new* database would be a claim about a history
  nothing can verify. A fresh epoch starts a fresh chain, which ADR-0068
  already recorded as the honest outcome. The gateway's refusal is a `tracing`
  error and a stderr block; `synveda init` prints the epoch it produced.
- **Secrets.** `reset` names its target twice before destroying anything and
  never with the password `DATABASE_URL` carries — the URL is re-rendered
  without it, and a unit test pins that. The KEK file survives a reset
  untouched.
