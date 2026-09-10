# ADR-0064: per-tenant keys seal what leaves and what we have to read back — the retrieval substrate stays readable by design, and `console_sessions` cannot hold a tenant key because ADR-0056 was right to take its tenant away

- **Status**: Accepted, **amended three times** (decision 12 named the wrong
  vocabulary; decision 5 had an unrecorded consequence for decision 12; and
  CPR-45 made key-provision evidence safely convergent)
- **Date**: 2026-08-11
- **Feature(s)**: TEN-4
- **Deciders**: sujitn

## Amendment 1 (2026-08-11): decision 12 named the wrong vocabulary

Decision 12 said rotation and export "take new `Action` variants (packs to
`@17`, goldens re-recorded)". They do not, and should not.

A Cedar `Action` exists for a surface the PDP authorizes. Both of these acts
are **operator commands against the database** — `synveda tenant key rotate`
and `synveda tenant export` — in the same category as `db migrate`,
`tenant create` and `audit verify`, which reach no gateway route and pass no
policy decision. Adding Cedar actions for them would have added two variants
nothing evaluates, taken every pack to `@17`, and re-recorded every golden, to
authorize a caller who is already holding the database credentials.

What they take instead is **audit** actions: `tenant.key.provisioned`,
`tenant.key.rotated`, `tenant.exported`, `tenant.secret.stored` and
`tenant.secret.cleared`, chained as break-glass events under the operator's
own name — which is what `tenant.created` has done since TEN-1. Packs stay at
`@16` and no golden moved.

The rest of decision 12 stands unchanged, including the part that turned out
to matter most: a failed open is audited. With one exception the decision
could not have anticipated and decision 5 makes unavoidable — see amendment 2.

## Amendment 2 (2026-08-11): two things the tests found

**A failed open on `console_sessions` cannot be a chain entry.** Decision 12
says a failed open is an audit event; the audit chain is per-tenant (AUD-1);
and decision 5's whole point is that a console session row is read *before*
the tenant exists. So for exactly the table this feature was named after,
there is no tenant to chain the failure to. It is a counter and a log line
there (`synveda_key_open_failures_total{scope,purpose}`) and a chained event
everywhere else. This is decision 5's consequence one step further out than
decision 5 noticed.

**`opening_key` selected a key by version and never checked the envelope's
scope tag.** Asking a tenant's key ring to open a deployment-sealed payload
read a row and spent a KMS unwrap before `SealingKey::open` refused it on the
tag. The refusal inside `open` is the load-bearing one and stays; the ring now
also compares the tag first, so the common mistake costs one comparison and
produces an error that names the disagreement rather than the uniform "did not
open". Found by a test asserting the wrong step, which is a better outcome
than the test passing.

## Amendment 3 (2026-08-31): provisioning evidence is a repairable witness

ADR-0019 decision 7 normally commits a CLI mutation and its break-glass event
in one transaction. Key provisioning cannot do that without holding a tenant
transaction across an external KMS wrap. The existing separation avoided that
outage-amplifying lock but left two bad retry choices: audit every rerun and
duplicate evidence, or audit only the key-row creator and make a crash after
the key commit permanently unauditable.

Provisioning therefore uses one narrow repair protocol. The key row converges
first; the caller must then unwrap the current generation, proving actual KMS
custody rather than treating a wrapped database row as success. In a fresh
tenant transaction it reads the authoritative generation-1 `kek_ref`, locks
the tenant audit-chain head, and appends the exact
`tenant.key.provisioned` generation-1 witness only when absent. A crash before
the witness commits rolls that transaction back and the exact rerun repairs
it. Concurrent reruns serialize on the existing chain-head lock. Disabled,
wrong or externally denied KMS access fails before success evidence.

Epoch-3 history needs no rewrite. Exact duplicate witnesses emitted by the
old command remain immutable and satisfy convergence, but the new path never
adds another. A malformed candidate that contains the generation-1 shape
fails closed, including a conflicting KEK reference or a payload superset. An
old post-rotation generation-2 provision event is a different fact, so it
neither masquerades as nor blocks repair of the generation-1 witness.
Candidate inspection is paged while the chain head is locked and
refuses more than 4,096 matching rows, so corrupt or adversarial history cannot
turn convergence into unbounded work. This amends only ADR-0019 decision 7's
key-provision case; ordinary mutations remain same-transaction audited. The
audit crate therefore exposes a generation-one key-provision witness type and
operation, not a generic idempotent append: callers cannot choose another
actor kind, action, outcome, resource, generation or payload shape through
this exception.

## Context

Four accepted ADRs name TEN-4 as the place their deferral gets paid, and they
do not all want the same thing:

- **ADR-0056 (CNSL-1, 2026-08-04)** — `console_sessions.access_token` and
  `.refresh_token` are stored recoverable, because the gateway *presents*
  them rather than checking them. Recorded as "an accepted exposure with a
  named successor: TEN-4 is where those columns get a key."
- **ADR-0060 decision 7 (AUTH-5, 2026-08-07)** — the outbound directory
  credential stays in deployment configuration rather than in a per-tenant
  table, because "shipping a plaintext outbound credential in tenant data
  now, to be encrypted in a later feature, is the version of this that is
  hard to walk back: the rows outlive the decision." The standing cost is a
  product limitation: **one deployment cannot pull two tenants from two
  directories until TEN-4.**
- **ADR-0030 decision 9 / ADR-0031 decision 16 (FLOW-1/2)** — commit signing
  ships as a seam with an honest default (`Signer::Unsigned` writes NULL);
  the key arrives as configuration and key *management* is "deferred (TEN-4's
  per-tenant keys are its natural home)".
- **ADR-0024 (CTX-1)** — the per-tenant Tantivy sidecars "must live on the
  same encrypted volume as the database in any deployment profile, per-tenant
  key coverage for them is a recorded TEN-4 obligation".

**A correction to the record, because it changes which obligation is oldest
and it turns out to matter.** ADR-0060 states that the outbound directory
credential "is the first recoverable secret in the product" and that "every
secret this product stores is a hash". Both sentences were three days out of
date when they were written: `console_sessions` landed on 2026-08-04 with two
columns that are recoverable by necessity, and ADR-0056 said so plainly in
the same words ("the first table in this product to store a live credential
at rest"). The mistake is harmless as a priority claim and *not* harmless as
a design input — the two secrets need different key scopes, for a reason
nobody would find by reading either ADR alone. Decision 5 is that reason.

Two more forces:

**The NFR is broader than any of those four.** Seed §10 reads "All data
encrypted at rest (per-tenant keys, KMS-pluggable) and in transit (mTLS
internal)". Read literally over `records.content` and `record_embeddings`,
that sentence deletes ADR-0024: there is no BM25 over ciphertext and no HNSW
over ciphertext, so both retrieval legs stop working the day it is
implemented. §10 is a non-functional requirement rather than a §2 product
principle, which is what makes it an ADR's business to scope; decision 7
scopes it out loud rather than by quietly shipping less.

**The acceptance criterion names an artefact that does not exist.** "Tenant
export is unreadable without that tenant's key" — and nothing in this
workspace exports a tenant. `export = portable archive (records+assets+audit)`
is TEN-5's, the WORM export is AUD-3's, backups are OPS-5's, and all three sit
behind this feature in the phase. A criterion whose subject is unbuilt is
either an untestable criterion or a scope instruction; decision 8 reads it as
the second.

The threat model is the one AUD-1 documents and ADR-0009 restates: this
defends against a **stolen artefact and a dumped table**, not against a
hostile principal inside a live gateway that is by construction able to read
what it serves. ADR-0009's own words — "hostile-principal defences are
TEN-4/TEN-5/OPS territory" — promise more than envelope encryption can
deliver, and decision 7 is where that gets narrowed honestly.

Licences: MIT/Apache-2.0/PostgreSQL in the core path, which rules out `ring`
and `aws-lc` exactly as it did for AUTH-1's JWT backend. Layering: seed §8 as
amended by tech plan §5, enforced by `scripts/check-crate-deps.mjs`.

## Decision

1. **Two levels, and a KMS only ever sees the top one.** A per-tenant data
   key (DEK) seals payloads; the DEK itself is stored only in wrapped form,
   under a key-encryption key (KEK) the KMS holds and never releases.
   `tenant_keys` (migration 0038) holds `wrapped_dek`, `kek_ref`,
   `key_version`, `algorithm` and nothing readable. The alternative — one KMS
   key per tenant, unwrapped per operation — puts a network call on a read
   path and runs into per-key quota and per-key cost in every cloud KMS at a
   tenant count this product is being sold at. Because `kek_ref` is a column
   rather than a deployment constant, **BYOK is configuration, not a
   redesign**: a customer whose contract requires their own KMS key gets it
   by that tenant's rows naming their key.

2. **`Kms` is a trait plus an enum, the `Extractor`/`Embedder`/`CommitSigner`
   shape, and its whole surface is `wrap`, `unwrap` and `key_ref`.** `Local`
   ships (key from configuration, dev and single-node deployments); AWS, GCP
   and Vault are later implementations behind the same two operations, which
   is the AC's "local dev impl + AWS/GCP/Vault impls later" read as a
   constraint on the trait rather than a promise about the roadmap. The
   surface is deliberately not "encrypt this payload": a KMS that will encrypt
   arbitrary bytes becomes a per-row network call the first time somebody is
   in a hurry, and the seam should make the expensive thing hard to reach for.

3. **Sealing is XChaCha20-Poly1305 with a random 192-bit nonce, and the
   algorithm id travels in every envelope header.** AES-256-GCM is the
   obvious choice and is the option considered; its 96-bit nonce carries a
   birthday bound around 2^32 messages per key, which is a bound somebody has
   to reason about precisely when OPS-7 makes this multi-process and no single
   process owns a counter. XChaCha's nonce is large enough that random is safe
   without coordination, which is the property worth paying for here. AES-GCM
   stays reachable rather than excluded — a FIPS-140 requirement is a real
   procurement ask — because the header names its algorithm, so a second
   algorithm is an added variant and not a migration. The KEK's own wrap
   algorithm is the KMS's business and is not constrained by this.

4. **Every ciphertext is bound to its context by AAD, so a ciphertext lifted
   from one tenant's row into another's fails to open rather than opening.**
   The additional data covers the key scope (which tenant, or the deployment),
   the purpose (which column or artefact type), and the row identity.
   `synveda-crypto` composes the AAD from typed arguments and does **not**
   accept a caller-supplied byte string, so binding is not something a caller
   can forget — the same structural-rather-than-promised move ADR-0060
   decision 8 made by putting connectors below the vocabulary they must not
   have. This is TEN-2's backstop reasoning applied to bytes: a cross-tenant
   transplant becomes a decryption failure, which is a thing TEN-6 can fuzz
   for and an audit event under decision 12.

5. **`console_sessions` is sealed under a deployment-scoped key, not a tenant
   key — because ADR-0056 removed the column that would make a tenant key
   selectable, and it was right to.** This is the finding. A session row is
   read *before* the tenant exists; reading it is one of the steps that
   establishes the tenant. Selecting a per-tenant DEK requires a tenant, so a
   per-tenant key for this table would require deriving a tenant from the
   session row — which is exactly the derivation ADR-0056's schema exists to
   make impossible, and the reason it refuses a `tenant_id` column that every
   reflex says should be there. So the key plane has **two scopes**: per-tenant
   DEKs, and one deployment DEK, both wrapped by the same KEK and both living
   in the same table shape. Naming it a scope rather than an exemption is what
   stops the next person from "fixing" the asymmetry by giving
   `console_sessions` a tenant, which would trade a real isolation invariant
   for a cosmetic one.

6. **Key version travels in the envelope, so rotation is lazy and never a
   stop-the-world rewrite.** Two rotations with very different costs, and
   conflating them is how key rotation becomes a thing nobody does: re-wrapping
   a DEK under a new KEK touches one row per tenant and no ciphertext at all;
   rotating a DEK requires re-sealing everything sealed under it. The version
   in each header is what lets the second happen on write, or in a background
   pass, or never — `console_sessions` rotates by expiry, because every row is
   already guaranteed to age out under its own absolute cap.

7. **What is deliberately not sealed: `records`, `record_embeddings`, and the
   Tantivy sidecars.** Application-level encryption of record bodies removes
   the lexical leg and of embeddings removes the dense leg — ADR-0024 in its
   entirety — so the literal reading of seed §10 is not a stricter version of
   this decision, it is a different product (searchable encryption, or a
   database per tenant). Encryption at rest for the substrate is therefore the
   **volume's**: the operator's disk, the storage class OPS-2's chart is
   installed against, and the sidecar directories on the same volume as
   ADR-0024 already requires. Per-tenant keys cover the two places where a
   per-tenant key can mean something the volume key cannot — artefacts that
   *leave* the database, and secrets we must be able to read back. ADR-0024's
   "per-tenant key coverage for [the sidecars] is a recorded TEN-4 obligation"
   is discharged as *volume coverage, stated*, not as per-tenant sealing.

8. **The AC's export does not exist, so TEN-4 builds the smallest one that
   makes the claim testable and TEN-5 inherits it.** `synveda tenant export`
   writes a sealed archive of the tenant's records and audit chain. Re-import,
   assets, the Temporal workflow and the signed destruction certificate stay
   TEN-5's — this is the container and its crypto, not the lifecycle. The
   archive is sealed under a **fresh per-export DEK, itself wrapped by the
   tenant's DEK and carried in the archive header**, so handing somebody an
   export does not hand them the key that opens the tenant's live secrets, and
   "unreadable without that tenant's key" is what the header enforces rather
   than what the prose asserts.

9. **The directory credential moves into a per-tenant sealed table, which is
   ADR-0060 decision 7's trigger firing on schedule.** A `tenant_secrets` table
   (tenant-scoped, forced RLS, sealed values under the tenant DEK) takes the
   outbound connector credential, and AUTH-5's connector resolves the table
   first and the deployment configuration second, with the precedence stated
   in one place — because a credential readable from two sources with no
   stated order is worse than either source alone. This is in scope rather
   than deferred for a reason worth stating: without it, **every consumer of
   the key plane would be on the deployment key** (decision 5), no per-tenant
   DEK would be exercised by any production path, and an unexercised key plane
   is a claim rather than a mechanism. It also deletes a named product
   limitation — one deployment can pull two tenants from two directories.

10. **Signing keys get the custody shape and not a policy change, and they
    arrive when FLOW has a verifier.** `Signer::Unsigned` stays the default.
    ADR-0030 put key *management* here, and what management means for a key
    nobody currently verifies is a table row; `tenant_secrets` from decision 9
    is the shape it takes (private key sealed, `signer_key_id` and the public
    key in plaintext beside it, because a verifier outside this database —
    FLOW-8's git mirror — needs the public half). Turning signing on is FLOW's
    ruling. Wiring a key here and defaulting it on would be precisely the "key
    management arriving through the side door" ADR-0031 decision 16 refused,
    with this ADR's number on it instead.

11. **Unwrapped DEKs are cached in memory with a stated TTL and zeroized on
    eviction, and the cross-process half is already somebody's problem.** A
    KMS unwrap per console request is a network call on a read path. The cache
    is keyed by (scope, version); `zeroize` on drop and on eviction; the key
    never reaches a `Debug` impl, a span field, an error message or a log line
    — the discipline `Ed25519Signer` already models. Rotation visibility is
    bounded by the TTL, which is the same staleness shape `ScopeChainCache`
    has and lands in the same place: OPS-7 owns cross-process invalidation,
    and this cache is named in it rather than inventing a second transport.

12. **Key operations are audited acts, and a failed open is audited louder
    than a successful one.** ~~Rotation and export take new `Action` variants
    (packs to `@17`, goldens re-recorded — the cost ADR-0060 paid for
    `DirectorySealAuthorise` and the reason to add both in one diff rather
    than two).~~ **(Amended 2026-08-11 — see amendment 1: these are operator
    commands against the database, not PDP-gated surfaces, so they take
    *audit* actions and the packs stay at `@16`.)** Provisioning rides on
    `tenant create`. A **failed** open is an
    audit event with its scope, purpose and key version, because under
    decision 4 it is either corruption or a cross-tenant transplant and both
    want to be visible; the payload carries integers and strings only, since
    an audit payload may hold no non-integer number (the defect AUTH-5 found
    at the first breaker trip). No key material, wrapped or otherwise, reaches
    a chained payload — AUTH-4's demo sweep for credential material extends to
    cover this feature's tables.

13. **`synveda-crypto` is a new crate between `synveda-types` and the middle
    band**, making the layering rule `types ← crypto ← {policy, store,
    identity, audit, vedaflow} ← retrieval/ingest ← gateway` in
    `check-crate-deps.mjs` and in tech plan §5's amendment to seed §8. It has
    to sit below the middle band because `store`, `identity` and `vedaflow`
    all need it and the rule forbids them depending on each other. It depends
    on `synveda-types` rather than on nothing, because decision 4's typed AAD
    needs the vocabulary — a crypto crate that took `&[u8]` would be one tier
    purer and would let a caller seal a payload without binding a tenant.

## Options considered

1. **Application-level per-tenant encryption of all tenant data, as seed §10
   reads literally.** The only option that satisfies the NFR without
   scoping it. Rejected: it deletes both retrieval legs (no BM25 and no HNSW
   over ciphertext), which is ADR-0024's whole architecture and CTX-1's
   shipped latency claim. What it would actually require is searchable
   encryption or a database per tenant — a different product with a different
   cost model, and a feature of its own if a customer ever buys it.
2. **pgcrypto, `pgp_sym_encrypt` in SQL.** Tempting because it is a migration
   and no new crate. Rejected on the threat model: the key travels in the
   statement text, so it lands in `pg_stat_statements`, in query logs and in
   any statement sampling the operator has on — inside the very database the
   key exists to protect against. A key that is in the dump you were worried
   about is not a key.
3. **Volume/TDE encryption only, no application keys.** Rejected as the whole
   answer and **adopted as half of it** (decision 7). It cannot make an export
   unreadable, it gives a customer nothing they can revoke, and it leaves all
   four inherited obligations exactly where they were — but it is the right
   answer for the substrate, and saying so is better than pretending the
   application layer covers it.
4. **One KMS key per tenant, no envelope.** The simplest mental model and the
   one a customer will describe. Rejected: an unwrap per operation on the read
   path, and per-key quota and cost in every cloud KMS at four figures of
   tenants. Decision 1 keeps the *property* that customer describes — their
   key, revocable — through `kek_ref`.
5. **Seal only exports; leave the recoverable secrets as they are.** The
   smallest thing that passes the AC as written. Rejected: the AC is not the
   feature — three ADRs named TEN-4 for the secrets, and one of them
   (ADR-0060) is carrying a stated product limitation until this lands.
6. **Give `console_sessions` a `tenant_id` so one key scope covers
   everything.** Rejected, and recorded because it is what a reader will
   propose in review: it re-introduces the derivation ADR-0056's schema
   forbids, so that a forged session row could name an organisation. One extra
   key scope is much cheaper than one fewer isolation invariant.
7. **Do nothing this phase.** Rejected: ADR-0060's limitation is already
   written into STATUS.md as something a customer hits, and every day the
   plaintext columns exist is more rows that outlive the decision — the exact
   argument ADR-0060 used to *not* create them.

## Consequences

- **Positive.** Three inherited deferrals are discharged and a named product
  limitation is deleted (two directories, one deployment). BYOK is a column
  rather than a redesign, which is the shape this arrives in at procurement.
  TEN-5 gets a sealed export container it would otherwise have had to invent,
  and TEN-6 gets a new leak class it can fuzz — a ciphertext that opens under
  the wrong tenant is a failing test, not a review question. The KMS seam is
  the fourth instance of a pattern this workspace already knows how to
  operate and to name in metrics.

- **Negative / accepted trade-offs.** **Crypto-shredding is not erasure**:
  destroying a tenant's DEK makes its *sealed* data unreadable, and its
  records, embeddings and sidecar are not sealed — TEN-5 must delete rows,
  and this ADR is where that stops being a promise TEN-5 might inherit by
  accident. A KMS becomes a boot-adjacent dependency: with the KMS
  unreachable, the deployment key cannot be unwrapped, so console sessions
  fail closed while `/v1` bearer traffic is untouched — an availability seam
  that did not exist before and that OPS-2's chart will need to say something
  about. The key cache is stale on rotation for its TTL, inheriting
  `ScopeChainCache`'s shape and OPS-7's fix. Seed §8 gains a tier. Two new
  actions push the packs to `@17` and re-record every golden. And the export
  built here is a partial TEN-5 — a container with no re-import — which is
  scope this feature did not ask for and cannot demonstrate its AC without.

- **Reversal triggers.**
  - A customer requires FIPS-140 validated cryptography → the algorithm id in
    the header is the seam; an AES-256-GCM variant lands beside XChaCha rather
    than replacing it, and nonce discipline becomes a counter this ADR
    deliberately avoided needing.
  - A regulator or contract requires record *bodies* under a tenant key →
    decision 7 is wrong for that customer, and the answer is option 1's
    product (searchable encryption or a database per tenant) as its own
    feature, not a patch to this one.
  - KMS unwrap latency or cost appears in console p99 → the TTL, or a
    long-lived deployment-key cache with rotation as an explicit flush.
  - Two deployments needing to open the same tenant's data (OPS-3 residency)
    → `kek_ref` per row is what makes a shared or replicated KEK expressible;
    if it is not enough, key custody becomes regional and OPS-3 owns it.
  - Anything outside this workspace needing to read an export → the header is
    versioned from the first byte, and the answer is to publish the format
    rather than to ship a decryptor.
  - `tenant_secrets` accumulating more than credentials — anything a query
    wants to filter on → sealing is wrong for that column, because a sealed
    value has no predicate; it wants a hash beside it or a different design.

## Compliance notes

- **Audit.** Two new action types plus a failed-open event; provisioning
  rides on `tenant create`. Payloads carry scope, purpose, key id and version
  as strings and integers only — never key material, wrapped or otherwise,
  and never a plaintext or ciphertext body. AUTH-4's chain sweep for
  credential material extends to `tenant_keys` and `tenant_secrets`. Deleting
  or rotating a key changes nothing about the chain over what that key's data
  did, which is the same separation ADR-0056 decision 9 drew for sessions.
- **Multi-tenancy isolation.** `tenant_keys` and `tenant_secrets` carry
  `tenant_id`, so ADR-0009's structural rule applies unchanged and the
  completeness guard in `crates/synveda-store/tests/rls.rs` picks them up
  automatically: forced row security, a tenant predicate, and least-privilege
  grants in the same migration. Decision 4's AAD is a second, independent
  boundary — RLS decides which ciphertext you can *fetch*, AAD decides
  whether it *opens* — and the two fail independently, which is what makes
  the pair a backstop rather than a duplicate. `console_sessions` keeps no
  `tenant_id` and gains no RLS policy; decision 5 is why, and it is a scope
  in the key plane rather than an exemption in the guard.
- **Policy enforcement.** No new path bypasses the PDP. Rotation and export
  are admin-plane acts and take actions through it like any other; the
  unwrap itself is not a governed act, because it is what serving an already
  authorised read costs — putting a PDP call under every decryption would put
  the PDP under itself the first time a policy pack needed a sealed value.
  AUTHZ-7's standing observation applies here too: these are direct mutations
  rather than proposal-gated ones, and a key rotation is a strictly recoverable
  act (the old wrapped DEK is retained, not destroyed) which is why it does
  not join AUTHZ-7's list.
