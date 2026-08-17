---
title: "CPR-2: Fresh schema epoch, startup guard & local reset"
labels:
  - epic:CPR
  - phase:5
size: M
---

# CPR-2: Fresh schema epoch, startup guard & local reset

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** M

## Description

Prompt 2 of the 33-prompt context-platform programme, and the first one that
changes runtime behaviour. It makes ADR-0068 decision 3 — *a fresh schema
epoch, with no old-data migration* — a thing the software enforces rather than
a thing an ADR says.

Four pieces:

- **`schema_metadata`**, a single-row table carrying the schema epoch, the
  migration head reached, when the epoch was created, and the product version
  that created it.
- **A preflight before the migrator**, which refuses to advance a database
  that has a schema but no marker — the shape every pre-cut database has.
- **A startup guard**, which the gateway refuses to boot past, repeated on
  `/readyz` because the gateway is allowed to start without a database.
- **`synveda reset --database --force`**, which drops and recreates the
  application database at the current epoch — the one supported way past a
  refusal, and destruction rather than translation.

Decisions in ADR-0069.

## Why this exists

The base commit has 38 migrations, `sqlx::migrate!()`, and no marker of any
kind. CPR-1's inventory recorded the consequence in one line: *"a database at
any prefix of this sequence is accepted and brought forward."* So a binary
built after the cut, pointed at a database built before it, cannot tell — and
`MIGRATOR.run` will bring it forward. That is the silent acceptance ADR-0068
decision 3 forbids, and it was reachable by running the command
`docs/INSTALL.md` already tells operators to run.

The obvious marker is the wrong one. `_sqlx_migrations` answers "which of
*this binary's* migrations have been applied", which is a question about the
current chain rather than about the model: Prompt 33 squashes that chain to a
single `0001`, and on the day it does every version number in that table stops
being evidence about anything. It also moves on every ordinary release, so a
guard built on it would need editing for reasons unrelated to the model — and
a guard that is edited routinely stops being read.

And the instruction the guard is supposed to print did not name anything.
`scripts/uninstall.sh --purge` destroys the Docker volumes, which is both too
much — Temporal's `temporal` and `temporal_visibility` live in the same
`pg-data` volume — and the wrong shape: it removes the installation rather
than resetting the database. `scripts/db-test.sh` does the right thing for a
scratch database, in shell, inside `docker compose exec`, using a `psql` an
installed release does not have.

## What this prompt deliberately does not do

It does **not squash the migration chain**. That is Prompt 33's, and doing it
here would mean the epoch marker and the whole of the new model landed in one
commit — so the guard would have nothing to be tested against. Keeping the 38
migrations means a pre-cut database is a fixture this feature's tests can
build, refuse and reset, which is the only way "an old database is rejected"
is a claim rather than an assertion.

It adds no audit event. The chain lives in the database `reset` destroys, so
an event recording the destruction would be destroyed by it, and one written
into the fresh database would be a claim about a history nothing can verify.
ADR-0068 already recorded that a fresh epoch starts a fresh chain.

## Acceptance criteria

- `schema_metadata` exists, holds exactly one row, and carries the schema
  epoch, the migration head, the creation timestamp and the product version
  that created the epoch. `created_at` and `created_by_version` are written
  once and never rewritten; re-migrating moves only the head.
- A **fresh empty database** bootstraps to the current epoch and is accepted.
- A **current-epoch database** starts normally, and migrating it again changes
  nothing it should not.
- A database from **before the cut** is refused at gateway startup (the
  process exits non-zero), by `/readyz` (503), by every store-level CLI
  command, and by the migrator — which **writes nothing**: the rows it refused
  are still exactly there and no marker was created.
- **Missing** metadata (no table, or no row) and **malformed** metadata (a
  table of another shape, or a blank provenance) are both refused.
- Every refusal prints `synveda reset --database --force` **verbatim**. The
  one refusal that must not is a database from a *newer* build, which says to
  upgrade the installation instead and names the command only to say not to
  run it. An unreachable database is a don't-know rather than a verdict: the
  gateway boots through it and readiness decides.
- `synveda reset --database --force` requires **both** flags, refuses a
  database that is not on this machine, stops the host gateway first, drops
  and recreates the database, installs the extensions, migrates to the current
  epoch, removes the derived search sidecar, and is **idempotent**. It
  preserves `kms.key`, the compose profile, the console bundle, stored logins,
  the Docker volumes and every other database on the server. It never prints
  the password `DATABASE_URL` carries.
- **No old-to-new data migrator exists**, asserted rather than asserted-to: a
  reset carries zero rows across, a refused migration writes nothing, the
  epoch migration is pure DDL (no `select`/`insert`/`update`/`delete`/`copy`/
  `with` statement in it), and there is no `.down.sql` anywhere in the chain.
- Demonstrated end to end by `demos/cpr-2-schema-epoch.sh`, which drives a
  real gateway binary against a real pre-cut database — the boot refusal is
  in `main` and no in-process test can reach it.
