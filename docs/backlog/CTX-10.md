# CTX-10: Friendlier progressive disclosure

## Problem and evidence

A review on 2026-11-16 of the delivered manifest, recall and expansion paths found no single defect. It did find that the feature costs more than it saves on a default deployment, and that it is hard for both people and agents to use.

**Token cost**
- The recorded LongMemEval comparison (CTX-9, 2026-11-11) spent about 108 tokens per manifest entry and about 63 per pulled body.
- Each entry is a JSON line with a constant `kind` and `channel`, the item and revision UUIDs, and the keys (`manifest_entry` in `crates/synveda-gateway/src/context_api.rs`).
- `SYNVEDA_EXTRACTOR` defaults to `deterministic`, which sets `summary` to the body and the title to the body's first sentence (`crates/synveda-ingest/src/extraction/deterministic.rs`). So an entry repeats the body's opening twice.

**Expansion traps**
- Passing `manifest_id` pins expansion. After any edit, the item reads "Not available at this instant" with no next step.
- An entry's revision id passed to `expand` reads like a forged id.

**No usage guidance**
- The MCP server instructions mention `recall` but not `expand`, and neither tool says when to use it.
- Claude Code defers MCP tools by default, so those instructions are the only Synveda text the model sees before it chooses to load a tool.

**Hard to switch on**
- The console does not show or propose `disclosure_mode`.
- No adapter can request a manifest.
- No user documentation explains it.

**Weak evidence of benefit**
- The judged comparison scored 0.4 against 0.2 QA on 10 instances, but the manifest run spent 28% more tokens.
- Its pulls were scripted, so no model decided what to expand.

## Scope

Recorded in the 2026-11-16 amendment to [ADR-0163](../adr/adr-0163-progressive-context-disclosure.md):

1. **Manifest entries** become one plain line: item id, type, sensitivity, title and description, and cost.
   - When the summary begins with the title, the entry shows the summary alone.
   - The header states the channel once and gives a conditional rule for when to expand.
   - Planner versions move to `v5`.
2. **Saved-delivery replay** reads entries by item id. It maps each to the revision in the run's private delivery lineage and refuses unknown, duplicate or malformed entries.
3. **`knowledge-expand`** drops `resolve` and always serves the current head after fresh checks.
   - `revised_since_offer` marks an item the named manifest offered at another revision.
   - Event identity and audit drop the resolve mode.
4. **Agent guidance**
   - The MCP server instructions and the `recall` and `expand` descriptions lead with purpose and a short conditional rule.
   - Every miss carries one fixed recovery sentence.
   - Text renderings show only the id `expand` accepts.
5. **Switching it on**
   - The console proposes `disclosure_mode`.
   - The Claude Code adapter accepts `"disclosure": "manifest"` in project configuration.
   - The product guide explains both.

### Current state (2026-11-20)

All five scope items merged on 2026-11-16 (`e424f101`) and were published in v0.4.10 on 2026-11-19. The suites, clients and entry cost below were measured before merging.

**Database-backed suites** (disposable exact-role PostgreSQL)
- `progressive_disclosure` and `context_runs` pass 25/25 each. The new and changed cases cover:
  - item-keyed replay of a saved manifest;
  - an edit served as `revised_since_offer` beside an unchanged control;
  - `resolve` refused;
  - the strict entry parser in the leak fuzz;
  - forged, foreign, archived and forgotten ids reading alike.
- `make claude-acceptance` passes 2/2. It now also asserts that the startup context holds the decision as one plain entry, and that the MCP instructions name both tools.

**Clients and generated contracts**
- Console tests pass 316/316 and Claude Code adapter tests pass 124/124.
- `make sdk-check` passes.
- The MCP corpus was re-recorded from the binary and the registry digests updated.
- OpenAPI, the console client and both SDKs were regenerated.

**Entry cost**

A unit test measures entry cost with the exact o200k counter. Random UUIDs move each figure by a few tokens between runs.

| Item | Earlier JSON entry | Plain entry | Body |
| --- | --- | --- | --- |
| Deterministic-capture item | 119–127 tokens | 64–70 tokens | 32 tokens |
| Authored item | 95–97 tokens | 48–50 tokens | — |

Entries are about 45–50% smaller, but a short captured item's entry is still about twice its body. A manifest therefore saves tokens only on long items until size-based inlining exists.

**Live client**

`make claude-acceptance-live` passed on 2026-11-20 at `116dd8e6`, with Claude Code 2.1.327 and plugin 0.2.0 installed from the packaged checkout.
- As its prompt instructed, the model recalled at `summary` depth and expanded the id that recall returned.
- The gateway served the expansion and appended one content-free `context.knowledge_expanded` event naming that item.
- The session ran under the built-in `full` delivery; a manifest at session start is covered only by the replay above.

**Not run**
- A model-chosen, equal-budget LongMemEval comparison (see Dependencies).

## Non-goals

- **Full-mode rendering** stays byte-identical. Compacting it would move the gated token baselines and needs its own measured change.
- **Per-item size-based inlining.** A full-mode item still carries three UUIDs and sources, so it is heavier than a compact entry and a threshold would almost never fire. Revisit after full-mode compaction.
- **Per-manifest short handles.** `hashes_only` and `disabled` runs keep no addressable selection, so a handle needs a mapping that does not depend on retained addresses. That needs its own decision.
- **A session-start map or per-prompt delivery.** ADR-0027's trigger for revisiting `UserPromptSubmit` has fired, and that revisit is a separate decision.
- **Manifest entries for authored context packs.** These still need ADR-0163's separate chunk, pin and refusal decision.
- **Changing the deterministic extractor's summaries.** That would move capture and extraction baselines. Rendering removes the duplication instead.
- **Codex and Copilot adapters** requesting a manifest is not claimed until each is tested.

## Architecture seam

**Gateway**
- Rendering stays pure in `crates/synveda-gateway/src/context_api.rs`, unit- and property-tested without I/O.
- Replay keying stays in `context_api/replay.rs`, fed the item-to-revision map from `context_api/dependencies.rs`.
- Expansion decisions stay pure in `context_api/expansion.rs`. The offered revision comes from the same private lineage the replay reads, after the existing same-tenant, same-Session manifest check.

**Generated peers**
- The route catalogue, OpenAPI, console client and SDK contracts regenerate from the DTOs.
- The MCP corpus re-records from the shipped binary.

**Clients**
- The console proposes the governed field through the existing Configuration flow.
- The Claude Code adapter adds a request field only: it narrows and never widens.

## Acceptance criteria

**Manifest output**
- A manifest entry carries no `kind`, `channel` or revision UUID.
- A focused test using the repository's `TokenCounter` reports the new entry cost beside the old one for the same revision.
- `full` output stays byte-identical.

**Leak and evaluation consumers**
- The leak fuzz, the security runner and the evaluation harness each find a known readable entry in the new format before asserting absence. A format they cannot parse fails rather than passing vacuously.

**Saved-delivery replay**
- A saved new-format manifest replays while its items stay current and readable.
- An injected unknown, duplicate or malformed entry refuses.
- A saved old-format manifest refuses with the existing conflict.

**Expansion**
- An item edited after the manifest is served with `revised_since_offer: true`.
- A withdrawn, archived, forgotten, superseded, revoked, foreign or forged id still reads identically as `not_found_at_as_of`, with the fixed recovery sentence and no reason.
- Restart re-hydration is unchanged.

**MCP surface**
- `tools/list` shows the new instructions and descriptions.
- `expand` advertises no `resolve`.
- `recall` text shows no revision id.

**Switching it on**
- The console shows and proposes `disclosure_mode`.
- The Claude Code adapter sends `disclosure` only when the project asks.

## Required tests

**Rendering**
- Pure tests for entry rendering bounds and title/summary de-duplication.
- A cost comparison against the earlier entry shape.
- A `full`-mode byte-identity check.

**Replay**
- Unit tests for item keying, unknown, duplicate and malformed refusal, and old-format refusal.
- Exact-role replay of a saved manifest.

**Expansion**
- Pure and property tests for the offered-revision flag and event identity without a resolve mode.
- Exact-role tests:
  - edit, then expand;
  - withdraw, archive, Forget, supersede and revoke, then expand;
  - the uniform-refusal fuzz with a positive control.

**Clients**
- A re-recorded MCP corpus.
- Console tests for the disclosure field.
- Adapter configuration and request tests.
- `make claude-acceptance` replay.

## Rollout and rollback

Built-in Configuration stays `full`. The manifest remains opt-in through governed Configuration or an adapter request.

- **Saved manifests:** those created before this change refuse replay and are recomposed under a new idempotency key.
- **API callers:** any caller that sent `resolve` now gets a validation error. There is no pre-1.0 compatibility path.
- **Rollback has two routes to stop.**
  - Rebinding `full` stops governed manifest delivery.
  - A request may still narrow `full` to `manifest`, so Claude Code projects that set `"disclosure": "manifest"`, or launches with `SYNVEDA_DISCLOSURE=manifest`, keep receiving manifests until that setting is removed.
  - Expansion events are content-free and stay inert.

## Dependencies

- **Measuring benefit.** Model credentials and approved spend are needed for a model-chosen, equal-budget LongMemEval comparison, at least two models, and a coding-task set. Until that runs, the benefit of manifest delivery is unmeasured.
