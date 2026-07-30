# The labelled extraction corpus (EVAL-2, ADR-0046)

Transcript-shaped observe events with the records a careful reader would
say they contain. One corpus, two readers:

- `crates/synveda-ingest/tests/extraction_precision.rs` reads it as a fast,
  hermetic, no-stack tripwire on the extractor **function**.
- `crates/synveda-eval` reads the same files to measure the **product
  path** — observe → redact → extract → embed → dedup → commit → serve —
  over HTTP only.

Both deserialize the *full* format with `deny_unknown_fields`, so a field
added for one reader cannot be silently ignored by the other. It is a data
dependency, not a crate dependency: the eval's empty dependency set
(ADR-0028 decision 1) is untouched, and neither reader can reach the store.

## Format

One file per **group**; a group is one eval actor's worth of fixtures.

```json
{
  "group": "alpha",
  "actor": "extract-alpha",
  "note": "why this group exists, for whoever adds to it next",
  "fixtures": [
    {
      "name": "alpha-decision-blake3",
      "note": "optional — why this fixture is interesting, or what it is expected to miss and why",
      "input": {
        "kind": "transcript_delta | tool_result | decision",
        "session_id": "alpha-1",
        "occurred_at": "2026-07-20T10:00:00Z",
        "payload": { "text": "..." }
      },
      "expected": [{ "class": "fact | decision | preference | procedure | entity | episode",
                     "content_contains": "a distinctive term any faithful summary keeps" }],
      "must_not_extract": ["a phrase this transcript does not support"]
    }
  ]
}
```

## Rules the guards enforce

Four tests in `extraction_precision.rs` hold the corpus to these, so a
mistake here fails a test rather than quietly moving a number:

1. **Every `content_contains` token appears in its own source.** A token
   absent from the transcript can never be matched, so a mislabelled
   fixture would depress recall forever and silently — in *both* readers,
   because they share this corpus.
2. **Every `must_not_extract` phrase is absent from its own source.** Bait
   present in the source is not bait: a faithful extractor would reproduce
   it, and the hallucination axis would be measuring copying.
3. **Session ids are unique across the whole corpus.** The harness
   attributes a served record back to its fixture through
   `provenance.session_id`; a collision merges two fixtures' results.
4. **Fixture names are unique.** MEM-3's tests look two of them up by name
   (`fact-redaction-placeholder`, `empty-payload`); renaming either breaks
   that test rather than weakening it silently.

## Sizing: why ten fixtures per group

A recall sweep is bounded by `MAX_RECALL_IDS` (32), and a sweep that
returns exactly that many is **refused as a measurement** — the response's
`truncated` flag reports the scope cap, not the record cap, so a full page
and a truncated one are indistinguishable from the consumer's side
(ADR-0046 decision 3).

Ten events per group leaves room for a live model producing up to three
records per event (30 < 32). The deterministic ruleset produces exactly
one per event, so it sits at ten. **Grow the corpus by adding groups and
actors, never by adding fixtures past this arithmetic** — a group that
outgrows the cap fails the suite by name, which is correct and is still a
failure someone has to go fix.

## Writing a fixture

- **Label the ground truth, not the ruleset's behaviour.** If a careful
  reader would call it a preference, label it a preference even when the
  deterministic rules route it to `fact`. That miss is the measurement
  working; a corpus written to flatter the rules measures nothing.
  `beta-preference-tabs-implicit` is the worked example.
- **A `note` earns its place when a fixture is expected to miss.** Say why
  — one record per event, truncation at 300 characters, no marker phrase —
  so the next reader knows it is a known limit and not a regression.
- **Bait is plausible-but-absent content, never a plausible-looking
  secret.** The same discipline as the redaction and extraction fixtures:
  documentation-only content, `[REDACTED:*]` placeholders included, never
  a credential real or synthetic-but-live-format.
- **Multi-claim utterances are worth writing.** They are where recall
  becomes a real number rather than a restatement of precision.
