# ADR-0152: Admit PostgreSQL 14–18 through a portable baseline and server-major authority profiles

- **Status**: Accepted (extension profiles extended by [ADR-0153](adr-0153-default-to-patched-pgvector-0-8-7.md))
- **Date**: 2026-10-18
- **Feature(s)**: OPS-11, CPR-45
- **Deciders**: Synveda maintainers

## Context

Enterprise customers install Synveda against their own PostgreSQL server.
Until this decision the runtime admitted only PostgreSQL 17. The owner asks
for one application build and one installation workflow that work on
PostgreSQL 14, 15, 16, 17 and 18, with 14 as the minimum, while the bundled
quick start keeps its qualified PostgreSQL 17 image.

An audit of every migration, every static SQLx query (828 at the time) and the
deployment SQL found these actual incompatibilities:

| Location | Requirement | Fails on |
| --- | --- | --- |
| `0001` `SET transaction_timeout = 0` | PostgreSQL 17 setting | 14–16 |
| `0001` `context_pack_chunks_id_v7 CHECK (uuid_extract_version(id) = 7)` | PostgreSQL 17 function | 14–16 |
| `0001` `knowledge_current`, `knowledge_item_versions` `WITH (security_invoker)` | PostgreSQL 15 view option | 14 |
| `0001` `scopes_sibling_slug_unique`, `scope_grants_unique`; `0007` `import_jobs_source_digest_unique` `NULLS NOT DISTINCT` | PostgreSQL 15 uniqueness | 14 |
| Seven authority queries reading `pg_auth_members.inherit_option`/`set_option` | PostgreSQL 16 catalogue | 14–15 |
| Global-ACL proof reading `pg_parameter_acl` | PostgreSQL 15 catalogue | 14 |
| Local reset reading `pg_database.dathasloginevt` | PostgreSQL 17 catalogue | 14–16 |
| Implicit array types of `vector` types are not extension members | catalogue difference | 14–16 |
| `vector` 0.8.6 installs 234, not 237, members | genuine per-major catalogue | 14–16 |

Every other static query prepared on a PostgreSQL 14 schema. The published
`0001` cannot be edited: [ADR-0121](adr-0121-preserve-v0-4-3-through-forward-migration.md)
byte-pins it as the v0.4.3 baseline, and appending a migration cannot repair a
baseline that does not parse. PostgreSQL 14 also has no per-membership INHERIT
or SET option: every member may `SET ROLE`, inheritance follows the member's
`rolinherit`, and `ADMIN OPTION` therefore always confers the role's
privileges. PostgreSQL 14 reaches upstream end of life on 12 November 2026.

## Decision

1. **Finite server admission.** `synveda_store::server_version::PostgresMajor`
   maps `server_version_num` to exactly 14, 15, 16, 17 or 18. The session
   safety query that already starts every product connection and authority
   proof reads it in the same round trip; any other major is refused before a
   catalogue-shaped query is prepared or a migration runs. The major is part
   of `DatabaseIdentity`, so pinned pool targets refuse a connection that
   changes server major. Business requests never probe it. There is no
   calendar-based refusal.
2. **Two schema lineages that converge.** The published lineage keeps every
   released file byte for byte and gains append-only
   `0011_portable_postgres_convergence.sql`, which drops the two views and
   replaces the newer constraint forms with PostgreSQL 14 equivalents. Fresh
   installations instead run `portable-migrations/0011_portable_context_platform.sql`,
   a single baseline recorded at SQLx version 11. It is the `pg_dump` of the
   converged published head on PostgreSQL 17.11 with only the dump header,
   role/extension stanzas and `\restrict` markers removed, and it is
   byte-pinned. The schema dump of a converged published database and of a
   portable installation are identical on PostgreSQL 17, and the application
   ACL, routine, trigger and forced-RLS inventories are identical on 14–18.
   The migrator selects a lineage only from the exact SQLx ledger: an empty
   database takes the portable lineage, a recorded published prefix keeps
   advancing on the published lineage, and a mixed or drifted ledger is
   refused. Every later migration is appended to both directories as a
   byte-identical copy, which the hard-cut gate enforces with a PostgreSQL
   15+ syntax scan of the portable path.
3. **Equivalent SQL.** Knowledge current/as-of reads put the exact former view
   definitions in caller-executed `NOT MATERIALIZED` CTEs over the forced-RLS
   tables; the public schema has no views. `scopes_sibling_slug_unique` is a
   plain unique constraint because `scopes_one_root_per_tenant` already admits
   one parentless scope per tenant. `scope_grants` uses one partial unique
   index per subject kind, matching its shape checks. `import_jobs` uses one
   expression unique index over `COALESCE(…, '')` for the three nullable
   source keys; the existing CHECK constraints forbid the empty string in
   each, so the sentinel cannot collide, and the import upsert names that
   index as its conflict target. The v7 CHECK is rewritten over `uuid_send`
   with the same result for every version/variant nibble.
4. **Authority by actual membership semantics.** PostgreSQL 16+ keeps the
   existing proofs unchanged. PostgreSQL 14 and 15 use static variants that
   require the runtime memberships to carry no `ADMIN OPTION` (inheritance is
   still proved through `rolinherit`), refuse any other membership into a
   protected role, and refuse a declared `administrative_memberships` entry
   with an explanation: ADMIN-only membership cannot exist there, so role
   management belongs to a declared superuser or CREATEROLE administrator,
   which needs no membership. `pg_parameter_acl` is inspected on 15+ only,
   because PostgreSQL 14 has no parameter privileges. The array types of
   trusted extension types are accepted through their internal element
   dependency. The destructive local reset stays on PostgreSQL 17 and 18.
5. **Measured extension profiles by server major.** The shared fingerprint
   keys each expected row by a server-major range: `btree_gin` 1.3 and
   `plpgsql` 1.0 on 14–18; `vector` 0.8.6 on 14–16 and, separately, 17–18;
   `vector` 0.8.2–0.8.5 on 17 only. Fixtures use genuine PGDG builds for each
   major.
6. **One installation workflow.** `synveda db setup-sql` renders a
   deterministic administrator script from the same role contract. The script
   detects the server major itself, uses the matching membership syntax,
   refuses unsupported servers before any change, converges only the declared
   roles, database and peer databases on rerun, and creates no extension that
   already exists. `synveda db preflight` remains the read-only verification,
   followed by the normal migration and startup.
7. **Unchanged boundaries.** Bundled images, CNPG and logical/physical backup
   and PITR stay on their qualified PostgreSQL 17 artifacts; external backup
   remains operator-owned. The shared-Keycloak bootstrap stays within its
   existing PostgreSQL 17 qualification.

## Options considered

1. **Rewrite published `0001`/`0007` and update their pins** — rejected:
   installed ledgers would falsely claim the new DDL ran.
2. **Strip or rewrite SQL while recording the original checksum** — rejected
   for the same reason.
3. **Fresh PostgreSQL 17+ installations on the published lineage, portable
   only below 17** — rejected: two fresh baselines to qualify for no gain.
4. **Read option columns through `to_jsonb()` and default them on 14/15** —
   rejected: it fabricates per-membership options instead of proving
   PostgreSQL 14/15 semantics.
5. **Remove `security_invoker` or substitute `security_barrier` or
   SECURITY DEFINER wrappers** — rejected: each evaluates row security with
   another identity or relies on application filters.
6. **Engine-specific product schemas or a migration framework** — rejected;
   one extra baseline and one convergence migration are enough.

## Consequences

- One build and one workflow serve PostgreSQL 14–18. Customers never choose a
  profile, edit SQL or fingerprints, or run version-specific binaries.
- Future schema changes must stay PostgreSQL 14 compatible and be copied into
  both lineages; the gate refuses known newer syntax and missing copies.
- `pg_dump` output of the converged head is reviewed once; regenerating the
  portable baseline is an explicit architecture change, like editing a pin.
- PostgreSQL 14/15 cannot represent a provider's ADMIN-only management grant.
  Such provider shapes stay refused there; ordinary DBA-provisioned
  installations are admitted.
- Reversal trigger: a new PostgreSQL major, a changed extension catalogue, or
  a schema change that cannot be expressed on PostgreSQL 14. Measure and add
  a finite profile, or raise the minimum in a new decision.

## Compliance notes

Embedded Cedar, forced RLS, VedaFlow-governed mutations, content-free audit,
schema ownership separation and drift refusal are unchanged. Removing the
views removes the only owner-evaluated read path; all Knowledge reads run as
the ordinary tenant transaction. No runtime role gains superuser, BYPASSRLS,
CREATEROLE or ownership. Fixture administrators only provision or tamper
with their disposable clusters; acceptance cases run as the ordinary roles.
