# CTX-9: Progressive context disclosure

## Problem and evidence

Session start renders Knowledge bodies until the budget is spent, so a large corpus crowds out the task and the agent cannot tell what it was not given. `POST /v1/sessions/{session_id}/knowledge-query` returns full bodies with no budget (`ContextKnowledgeView` in `crates/synveda-gateway/src/context_api.rs`). Exact-id fetch exists only behind `SessionDiagnostics`. Nothing records which assets an agent pulled after being offered them.

ADR-0041 built an index tier and ids-based recall; ADR-0078 removed that route, and ADR-0042 records that `ids` and `as_of` have no successor. Some pieces survive: `EntryTier::Index`, immutable selections, the private delivery proof (ADR-0144), saved-delivery re-admission (ADR-0141), and the `as_known_at` store read.

Manual Knowledge writes refuse `Secret` findings but store PII: `reject_secrets` in `crates/synveda-gateway/src/knowledge_api.rs`. Tiered text would therefore disclose PII unless admission changes first.

## Scope

[ADR-0163](../adr/adr-0163-progressive-context-disclosure.md) records the accepted design. Delivery order:

1. Apply the effective policy pack's redaction ladder to every Knowledge command over title, summary, body, tags, verification and revision metadata, and source locators, revisions and metadata.
2. Add governed `disclosure_mode` and request `disclosure` on ContextRun, with L0 entries from stored revision fields.
3. Add `POST /v1/sessions/{session_id}/knowledge-expand`, the derived `context.knowledge_expanded` event, audit, the MCP `expand` tool, and `depth` on `knowledge-query` and `recall`.
4. Re-hydrate expanded items on `restart` under fresh Cedar.
5. Report offered-versus-expanded per session, with full-depth recalls alongside it; compare `full` and `manifest` on LongMemEval.

### Current state (2026-11-12)

Delivered in source and exercised against disposable PostgreSQL:

- Every Knowledge command runs one admission scan before validation and hashing, including the reason a lifecycle command carries. PII follows the effective pack's ladder; a credential is refused on every pack, including those that set secrets to `redact`, because Knowledge always refused them. `knowledge.quarantined` chains a `quarantine` refusal with rule ids only.
- Migration `0019` (both lineages) adds the `context.knowledge_expanded` event type and the `manifest_entry` reason.
- A `manifest` ContextRun delivers entries for what the principal may read, states truncation only from authorised candidates, and shows no policy-filtering notice. Saved manifests replay under ADR-0141.
- `knowledge-expand` re-authorises each id, returns one uniform refusal, and appends one idempotent content-free event. Its audit, reader views and recall's audit mask addresses under `hashes_only` and `disabled`, taking the narrower of the policy pack's and the Configuration's retention as a context run does; the stored event keeps them for restart.
- `knowledge-query` defaults to `summary`; the MCP `recall` tool and `synveda recall` follow, and evaluation, demo and SDK workflow callers pass `full`.
- `synveda mcp` advertises `expand` in both write modes.
- A `restart` run re-reads up to 32 expanded items under fresh Cedar and delivers survivors verbatim.
- LongMemEval accepts `--disclosure manifest` (or `EVAL_LONGMEMEVAL_DISCLOSURE`), pulls the first 8 entries in manifest order, and reports the offered-versus-expanded ratio per session as pull behaviour beside the full-depth recalls it issued. A manifest run rewrites a baseline only when one is named explicitly.
- Property tests cover entry bounds, header truncation, id validation, resolve modes, pins, withheld-item views, the response cap, event identity, retention masking and restart address order.
- Seeded fuzz runs against PostgreSQL check two things. Across policy packs, roles, scopes and sensitivities, Knowledge a reader cannot read leaves no trace in a manifest or summary recall, and archiving it changes nothing that reader sees. On expansion, forged, foreign-tenant, policy-denied, archived and private ids read alike, and so do foreign, other-Session and unknown manifests and Sessions. Each run's case set is deterministic.
- Expansion and restart are tested under `hashes_only` and `disabled` as well as `full` and `redacted`. Knowledge forgotten after it was expanded reads like a forged id and never returns on restart.
- `make claude-acceptance` also replays a governed manifest SessionStart, summary recall and expansion through the plugin's MCP entry over the built CLI, and a compact restart that restores the expanded body. The live test now asks the agent to recall and expand, and asserts the audited summary recall and the expansion event.
- `make claude-acceptance-live` passed on 2026-11-10 with Claude Code 2.1.322 on macOS 27.2 (arm64). The packaged plugin was installed through `synveda plugin install`, and one `claude -p` session recalled at summary depth and expanded the id recall returned.
- The LongMemEval readiness wait no longer asks a ranking question. It reads each evidence session's accepted Knowledge back by exact id, states what does not read back beside the row rather than failing it, and waits only for a semantic index that holds no vectors. Lexical rank is stored when a revision commits (ADR-0082), so the earlier "never became rankable" lines were the probe's.
- The declared deterministic comparison ran on 2026-11-11 against `1272170b` on macOS 27.2 (arm64). Both modes used the same 10 of 500 instances of `longmemeval_s_cleaned.json` (`cd766d50fe98`, 4,927 turns) and the default deterministic embedder, so every block ranked on the lexical leg alone. The predeclared policy pulled the first 8 manifest entries. Both runs held the retrieval gate.

  | Metric | `full` | `manifest` |
  | --- | --- | --- |
  | Evidence recall, 14 sessions across 8 instances | 0.571 | 0.643 |
  | Instances with every evidence session bound | 2 of 8 | 3 of 8 |
  | Knowledge items per block where the manifest offered 12 | 7 | 8 |
  | Mean tokens | 1,413.7 | 1,805.6 (1,300.7 block, 504.9 pulled) |
  | Latency p95 | 952 ms | 1,097 ms |
  | Offered versus expanded | — | 0.701 |
  | Full-depth recalls | — | 0 |

  Among scored instances, the one binding difference is `8fb83627`, which bound both evidence sessions under `manifest` and one under `full`. Both modes report the same 26 accepted items from 8 evidence sessions as not active at readback. Twelve come from `answer_b3070ec4_1`, which the corpus dates six hours after its question, so its Knowledge is not yet valid. The other 14 have not been diagnosed item by item; the write path holds an accepted item that conflicts with existing Knowledge as transitional, and the exact-id lens returns only active items.
- The judged tier ran on 2026-11-12 against `a8053e72` on macOS 27.2 (arm64), over the same slice and pull policy. `claude-opus-5` at high effort read every block and graded every answer the reader gave. That is the model the judged baseline names, and every call was served by it. The judge agreed with all 10 starter pairs in both runs. Evidence recall, bound instances, mean tokens, the pull ratio and the readback findings matched the deterministic comparison, while latency p95 did not (1,342 ms under `full`, 926 ms under `manifest`). Both runs held the retrieval gate.

  | Metric | `full` | `manifest` |
  | --- | --- | --- |
  | QA accuracy | 0.200 | 0.400 |
  | QA accuracy, mean over question types | 0.167 | 0.367 |
  | Abstention accuracy, 2 instances | 1.000 | 1.000 |
  | Answerable questions the reader declined | 6 of 8 | 5 of 8 |
  | Model tokens in, out | 48,689, 2,229 | 54,245, 2,408 |

  Two answers differ. `8fb83627` bound both evidence sessions under `manifest` and was answered there; under `full` it bound one and the reader declined. `118b2229` bound its one evidence session in both modes; the reader gave the reference's 45 minutes under `manifest` and about an hour under `full`. That second difference is one reader call per mode, so it cannot be separated from the reader's own variance, and two of ten instances from one run per mode does not show that manifest delivery answers better. In both modes the reader declined `2b8f3739` although its block bound all three evidence sessions. Under `full`, both QA figures fall below the judged baseline's floors of 0.27 and 0.237, which gate nothing.

Remaining before this feature closes:

- The owner's decision to close it, including whether these judged figures are published. `docs/BENCHMARKS.md` rows name no disclosure mode.

### How the request maps to the current product

| Request | Delivered as |
| --- | --- |
| `inject` manifest | `manifest` ContextRun; `manifest_id` is the run id |
| `synveda:recall` | MCP `recall` over `knowledge-query`, with `depth` |
| `synveda:expand` | MCP `expand` over `knowledge-expand` |
| `allowed` | `served` |
| `denied`, `not_found_at_as_of` | One uniform `not_found_at_as_of` |
| `redacted` | Not emitted; redaction happens at admission |
| Results carry the manifest `as_of` | Recall stays current and reports its own `as_of` |
| `published` / `staged` / `derived` on Knowledge | Existing `ConfigurationContextChannel` label; packs keep real channels |
| Generated L0/L1 text | Stored title and summary of the admitted revision |
| `include_resources` | Removed; provenance is always returned |
| PreCompact `snapshot` | The Session's recorded expansion events, newest first and bounded |
| Stop `observe` | Removed product surface; expansion is itself a Session event |
| Per-tenant manifest budget | Existing `token_budget` and `budget_tokens` |

## Non-goals

- Automatic server-side expansion, client L2 caching across sessions, or streaming partial bodies.
- Serving historical bytes after withdrawal, Forget or supersession.
- A read-time redactor, a model call on the read path, or a separate approval for L0 text.
- Expansion-derived ranking. If it is ever wanted, per-principal frequency needs an explicit tenant Configuration opt-in.
- Rescanning revisions admitted before the admission ladder ships. A governed rescan that creates superseding revisions would be a separate decision.
- Manifest entries and expansion for authored context packs. Their bodies stay in the shared allowance until a separate decision establishes an exact chunk read, pin and refusal contract.
- LoCoMo: its CC BY-NC 4.0 terms are refused by `scripts/check-corpus-licences.mjs`, and the replacement is tracked in [EVAL-7](EVAL-7.md).
- Claims of provider usage, monetary saving or model task success without that measurement.

## Architecture seam

The planner, Cedar seam, delivery record and event ledger stay authoritative. `crates/synveda-gateway/src/context_api.rs` gains manifest rendering and the expansion handler; the pure pin and status decisions are unit-tested without I/O. Static SQL stays in `synveda-store`. The route catalogue, OpenAPI, console and SDK clients stay exact peers through generation. `crates/synveda-cli/src/mcp.rs` adds the tool; digest-pinned MCP fixtures keep their captured meaning and must not be hand-edited. The new Session event type needs one additive migration and epoch head.

## Acceptance criteria

- A `manifest` run returns within the governed budget with no Knowledge bodies beyond explicitly required revisions, restart evidence and re-hydrated expansions. Unreviewed candidates, where a Configuration enables them, keep their existing rendering. `full` output stays byte-identical to its previous form.
- An agent recalls at `depth=summary` and expands selected ids in a live Claude Code session through `make claude-acceptance-live`. Exit 77 means unavailable, not passed.
- A principal without read access finds no trace of a restricted asset in entries, counts, `truncated`, summaries, statuses or errors.
- Seeded PII and credentials never appear in any tier, and PII in a manual write follows the effective policy pack's ladder.
- An asset retracted after manifest issue returns `not_found_at_as_of` on pinned expansion.
- A grant revoked between manifest and expansion returns that same `not_found_at_as_of`, indistinguishable from a forged id.
- Every expansion produces an audit record with status counts and tokens returned, and no refused id.
- Restart after compaction restores expanded content that still passes fresh policy and pin checks, and omits the rest.
- The evaluation harness reports the offered-versus-expanded ratio per session, labelled as pull behaviour rather than use.

## Required tests

- Pure unit and property tests for L0 rendering bounds, pin decisions, status mapping, the byte cap and event identity.
- Policy-leak fuzz across L0 and L1 for pack, role, scope and sensitivity combinations.
- Cross-tenant fuzz on expansion with forged or foreign item and manifest ids, asserting uniform responses.
- Redaction tests with seeded PII and credentials through manual write, capture acceptance and every tier.
- Rewind test: manifest at T1, retraction at T2, expansion at T3, using explicit ordering and no sleeps.
- Mid-session revocation, supersession, Forget and archive tests under ordinary tenant transactions and test packs.
- All trace-retention modes, including `hashes_only` and `disabled`, for expansion and restart.
- Fresh-database idempotent retry and concurrent expansion tests; route, OpenAPI and generated-client agreement.
- Claude replay through `make claude-acceptance`, plus MCP `tools/list` and call coverage.
- LongMemEval `full` versus `manifest` with a predeclared expansion policy. The deterministic tier gates; the judged tier is published and non-gating.

## Rollout and rollback

Built-in Configuration stays `full`. Admission scanning ships first because it is independently valuable. `manifest` is an explicit governed Configuration choice, with `full` kept as the comparison path. Rollback publishes or rebinds `full`; expansion events stay inert for their ordinary retention. The route and additive migration are forward-only under the upgrade contract.

## Dependencies

The owner accepted ADR-0163 on 2026-11-09 with these decisions:

- One uniform refusal replaces the requested per-item `denied` status.
- PII already stored in immutable revisions stays as is for now.
- `summary` is the `recall` default.
- Authored pack bodies stay in the shared allowance for the first delivery.
- `expand_cost_tokens` is exposed to agents, labelled as an estimate.
- Expansion-frequency ranking is deferred.

The live Claude criterion needs an installed, authenticated client. The judged LongMemEval tier needs a corpus, model credentials and approved spend.
