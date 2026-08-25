# ADR-0087: OKF v0.2 is a bounded exchange adapter whose imports stop at CaptureCandidate

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-27
- **Deciders**: sujitn

## Context

ADR-0068 fixes OKF as an external format, not a Synveda domain model. CPR-15
through CPR-22 have since supplied the domain it must surround: immutable
Knowledge revisions with independently authorised sources, reviewable capture
candidates, VedaFlow publication, current-only query and deterministic
provenance-bearing export inputs. There is no OKF implementation to preserve.

The canonical specification moved from the frozen
`GoogleCloudPlatform/knowledge-catalog/okf` copy to
`GoogleCloudPlatform/open-knowledge-format`. On 2026-08-25 its `main` commit is
`ad30107c31c06aec8a7d5636e0d1058118604e6f`; `SPEC.md` identifies itself as
version **0.2**, the canonical repository has no release tags, and no stable
successor is published. v0.2 is deliberately permissive: only a non-empty
`type` is always required, unknown types and metadata survive, and its own
consumer guidance includes v0.1 fallbacks. The Synveda programme explicitly
requires v0.2 only, so those fallbacks would be compatibility behaviour here.

One existing boundary needs extending rather than bypassing. ADR-0083 made a
CaptureBatch one frozen session snapshot and a CaptureCandidate cite session
events. An OKF import has no session event and manufacturing a synthetic agent
run would make provenance and timelines false. Publishing imported documents
through Knowledge commands would be equally wrong: a personal auto-apply
profile could make the import current without the required candidate review.
A separate `import_candidates` table would create the duplicate review surface
the programme forbids.

Archive and filesystem inputs also create a hostile-content boundary. A
gateway must not traverse caller paths, follow symlinks, execute an attested
computation, fetch a frontmatter URL, expand an unbounded archive or retain a
credential-bearing opaque payload merely because the format is Markdown.

## Decision

1. **Pin one exact external version.** `synveda-okf` implements OKF v0.2 at
   canonical commit `ad30107c31c06aec8a7d5636e0d1058118604e6f`. A root
   `okf_version` other than `0.2`, including `0.1`, is refused. Unknown concept
   `type` values and unknown v0.2 frontmatter keys are preserved and surfaced;
   they are not validation failures or silently coerced into a registered OKF
   taxonomy that the specification does not have.

2. **Keep format mechanics in one leaf adapter crate.** A versioned
   `KnowledgeFormatAdapter` boundary owns bundle ingestion, inspection, import
   planning and deterministic export. It depends only on shared domain types;
   it cannot see Postgres, Cedar, VedaFlow, audit or the gateway. The gateway
   orchestrates policy and persistence, and the store persists already
   validated jobs/artifacts/mappings. A later format is another adapter
   implementation, not conditionals inside Knowledge.

3. **Persist an immutable plan before materialisation.** `ImportJob` is the
   stable, tenant/project-bound operation. Immutable `ImportArtifact` rows
   retain a safe logical path, canonical bytes hash, parsed frontmatter/body,
   source kind and reported source revision; immutable `ImportMapping` rows
   retain the proposed Knowledge content, relation plan and deterministic
   `addition`, `update`, `duplicate` or `conflict` classification against only
   current Knowledge that passed an exact PDP decision. The bundle digest and
   source identity make an unchanged reimport return the same job.

4. **Generalise capture provenance as a closed union.** A CaptureBatch and
   CaptureCandidate are sourced either from one session snapshot or one OKF
   ImportJob, never both. Existing session event links remain unchanged.
   Import candidates instead link to immutable ImportArtifacts. Reading or
   deciding either shape uses the same destination Knowledge decision and the
   source-specific decision: SessionRead/Write for a session, KnowledgeRead/
   Write at the import project's governed scope for an import. Existing
   candidate decisions still invoke the one Knowledge/VedaFlow command layer.
   No synthetic session and no import-specific candidate workflow exists.

5. **Materialisation is one-way and candidate-only.** A planned job may
   materialise exactly once into one completed import-sourced CaptureBatch.
   Duplicate mappings remain visible in the dry-run but do not manufacture a
   candidate; additions, updates and conflicts become reviewable candidates
   with OKF provenance and proposed relations. Materialisation never opens a
   Knowledge command. Accept, merge or replace later does so through the
   ordinary candidate decision endpoints and may return applied,
   pending-review or rejected.

6. **Export is a freshly authorised deterministic projection.** The gateway
   resolves selected current Knowledge per row and independently filters every
   source and relation before passing a value-only projection to the adapter.
   Stable item ids produce stable collision-free logical paths; documents and
   links sort bytewise. v0.2 provenance, generation, verification, lifecycle,
   staleness and preserved extension metadata round-trip where present. Denied,
   non-current and unreviewed content never enters the adapter.

7. **Treat every input as hostile inert data.** Directory ingestion refuses
   symlinks and paths outside the selected root. Zip and tar ingestion refuses
   absolute/parent paths, links, special files, encrypted entries and bounded-
   expansion violations. Only bounded UTF-8 Markdown is admitted; executable
   or binary payloads are refused and no content is run. Source URLs are not an
   input kind, and URLs found in frontmatter are retained as inert strings and
   never fetched, so redirects, SSRF and private-address access have no network
   seam. Git input is a checked-out tree plus an explicit reported revision;
   the gateway runs no Git command.

8. **Public APIs carry bytes, never server filesystem authority.** A client
   submits bounded file entries or bounded archive bytes with source metadata.
   It cannot name a gateway-local path. Creation is idempotent; materialisation
   is idempotent; growing collections are cursor-paginated. Jobs, artifacts and
   mappings are forced-RLS and every served row is independently PDP-filtered.

## Options considered

1. **One adapter plus generalised capture provenance (chosen).** Preserves one
   candidate/review/publication path while keeping session and file evidence
   truthful.
2. **Synthetic import sessions.** Avoids schema changes but lies in the runtime
   and audit model and produces meaningless session timelines. Rejected.
3. **Import-specific candidates.** Keeps ADR-0083 untouched by duplicating New
   Learnings, candidate decisions and VedaFlow integration. Rejected.
4. **Open Knowledge changes directly.** In a personal profile they may
   auto-apply, so imports could publish without candidate review. Rejected.
5. **Accept v0.1 through the official v0.2 fallbacks.** Interoperable, but an
   explicit compatibility implementation forbidden by this programme.
   Rejected; `timestamp` and body `# Citations` are not translated.
6. **Let the gateway read a submitted path or run Git.** Turns an API into
   filesystem and process authority. Rejected; clients send inert bytes.
7. **Fetch external sources during validation.** Adds SSRF, redirect and
   credential boundaries unrelated to exchange. Rejected; source URIs are
   provenance strings only.

## Consequences

- Positive: OKF remains replaceable at one boundary; imported material cannot
  become current without the existing candidate and VedaFlow gates; unknown
  metadata survives; dry-runs and exports are reproducible and inspectable.
- Positive: New Learnings sees session and OKF candidates through one model
  while their source evidence remains accurately different.
- Negative / accepted: capture DTOs gain an explicit source union and existing
  session-only consumers must handle an absent session; clients must read and
  package local directories/Git trees rather than grant the gateway a path.
- Negative / accepted: referenced scripts and binary assets are rejected in
  this first adapter even though OKF can point to them. The format metadata is
  preserved, but Synveda does not import executable payloads.
- Reversal triggers: a stable OKF successor appears → add a separately pinned
  adapter and explicit version selection; a trusted artifact service is added
  → it may supply validated bytes but not bypass this boundary; executable
  computation support is designed → treat it as a separately sandboxed Skill
  or tool plane, never execute it inside import.

## Threat model and abuse cases

- Archive paths, symlinks, device entries, duplicate logical paths and
  compression ratios are attacker-controlled and rejected before persistence.
- YAML aliases, nesting, scalar size, document count and total bytes are
  bounded; parsing errors disclose a logical path and rule, never neighboring
  tenant data or source content.
- Markdown links and frontmatter resource values are data. The adapter performs
  no DNS lookup, redirect, HTTP request, Git process or script execution.
- A foreign project/job/artifact id is indistinguishable from a fictional one
  through ownership checks, forced RLS and exact PDP decisions.
- Dry-run matches can reveal existing Knowledge. Every exact match is decided
  before its id, class or count is stored or returned; denied matches
  contribute nothing.
- Audit and traces carry ids, digests, counts, source kind, format/version and
  outcomes only. Markdown, frontmatter, paths that might carry secret text and
  archive bytes do not enter them.

## Compliance notes

- **PDP:** planning/materialisation uses KnowledgeWrite at the exact project
  scope; reads/exports use exact KnowledgeRead decisions, with source and
  relation filtering before adapter input.
- **VedaFlow:** import creates no Knowledge. Candidate acceptance remains the
  only bridge and always creates a typed Knowledge/apply change.
- **RLS:** all import tables and the new import-artifact candidate links are
  tenant-bound, enabled and forced RLS and enter the completeness gate.
- **Audit:** plan/materialise/export transitions are hash-chained with
  content-free metadata; candidate decisions retain their existing audit path.
- **Secrets:** opaque archive bytes are not retained, URLs are never fetched,
  executable/binary content is refused and content never enters normal logs.
