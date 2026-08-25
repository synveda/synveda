# ADR-0092: audit answers are recorded evidence and exports freeze a chain head

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-33
- **Deciders**: autonomous context-platform continuation

## Context

ADR-0045 already fixes the trust boundary: a tenant audit answer is complete
or refused, disclosure and authority stay separate, no content is resolved,
and the product reports recorded decisions rather than replaying a historical
PDP. The redesign retained those routes but moved the facts they describe.
Knowledge has immutable revisions and distinct valid/transaction time;
sessions and context runs are the only delivery plane; VedaFlow proposals
carry typed artifact references; runtime Configuration and policy relaxations
are immutable governed artifacts; Skills and trusted Tools have exact versions
and bindings.

The generic event route filters only scalar columns. Terminal family events do
not all repeat the typed reference carried when their proposal opened. The
Knowledge answer exposes one overloaded `at` instant and returns a cursor it
cannot accept. Context events cite a Configuration version/hash but not the
binding, aggregate, policy source or active relaxations that made it effective.
Finally, `/v1/audit/verify` checks the database in place but there is no public,
deterministic set of canonical hash inputs an offline verifier can consume.
A new audit projection would create an unsigned second truth; replaying past
authority would violate ADR-0045.

## Decision

1. **Extend the chain query, not the storage model.** `/v1/audit/events` gains
   exact JSON-containment filters for one typed artifact reference, session and
   context run. A tenant-leading payload index changes no canonical row byte.
   Every current governed family repeats its typed reference on terminal
   applied/rejected/expiry evidence so queries do not infer identifiers from
   prose or display resources.

2. **Valid time and transaction time stay distinct.** The Knowledge audit
   answer uses `valid_at` for the revision's semantic interval and
   `as_known_at` both as the chain-delivery cutoff and the immutable revision's
   transaction-time ceiling. It accepts its backwards sequence cursor. Exact
   revision timing/hash metadata may be read under tenant-wide AuditRead, but
   content remains behind KnowledgeRead. Erased and hashes-only evidence is
   returned separately as unresolved, not silently dropped or guessed.

3. **Effective governance is recorded where it is used.** Context composition
   records the exact Configuration aggregate, binding scope/binding, version,
   digest, policy-pack source and active relaxation ids/versions/hashes gathered
   for that request. The evidence describes the completed decision; it does not
   claim that a reconstructed principal could act now or at another instant.

4. **An export is a frozen contiguous prefix.** The first export page captures
   the current sequence/hash before appending its own audited read. Later pages
   name that fixed `through` sequence. Events are ordered and include tenant,
   sequence, timestamp, actor, action, resource, outcome, payload, optional
   trace id, previous hash and hash. The envelope names the v1 canonical form,
   BLAKE3 rule, tenant-bound genesis and snapshot head. A complete offline file
   begins at genesis and ends at the frozen head; the verifier rejects gaps,
   reordering, mutation, wrong tenant/genesis or an incomplete tail.

5. **The public route is cursor-paginated; the CLI assembles the artifact.**
   `GET /v1/audit/export` returns at most 1,000 contiguous rows and a next
   cursor. `synveda audit export --output` walks one fixed snapshot through the
   public API, verifies it before an atomic write, and refuses to overwrite an
   existing path. `synveda audit verify-export` needs neither bearer nor
   database. This is deterministic evidence, not WORM retention or SIEM
   delivery; AUD-3/AUD-4 remain separate enterprise work.

6. **Every read remains visible.** Event, Knowledge and export reads all append
   one `authz.decision` after taking their answer. The frozen export excludes
   those later rows by construction and reports its boundary, so repeated
   exports differ only because the chain truthfully grew.

## Options considered

1. **Extend the append-only chain and freeze exports (chosen).** Preserves one
   evidentiary truth and makes every answer independently checkable.
2. **Create searchable audit projection tables.** Faster arbitrary queries,
   but their rows are neither canonical hash inputs nor independently trusted;
   rejected until a measured need justifies a derived, rebuildable index.
3. **Replay Cedar against historical domain rows.** Produces a hypothetical
   decision rather than evidence and cannot honestly reconstruct deleted
   groups, scope moves or policy binaries. Rejected by ADR-0045.
4. **Return one unbounded export response.** Simple for clients, but an
   ever-growing chain makes response memory and latency unbounded. Rejected in
   favour of a frozen cursor walk.

## Consequences

- Positive: context-platform lifecycle, delivery and governance questions are
  addressable with exact immutable ids; exports can be verified away from the
  service; query reads cannot race their own evidence boundary.
- Negative / accepted: complete exports require multiple audited requests and
  therefore grow the live chain after their frozen snapshot; hashes-only or
  erased revision evidence cannot answer valid time and is explicitly partial.
- Reversal trigger: measured median typed-filter latency exceeds ADR-0045's
  200 ms budget at one million events despite the payload index → add a
  rebuildable derived search projection whose result still cites canonical
  sequence/hash evidence; never make it the chain authority.

## Compliance notes

- **PDP/RLS:** every route decides `AuditRead` at the tenant and then reads in a
  forced-RLS tenant transaction. No artifact read permission is inferred or
  granted by an audit query.
- **VedaFlow:** audit changes no mutation workflow; it exposes the immutable
  typed references VedaFlow already governs.
- **Audit/privacy:** query operations append after their answer. Public rows
  contain identifiers, hashes, counts, reason codes and decision provenance,
  never artifact content, session messages, provider secrets or credentials.
