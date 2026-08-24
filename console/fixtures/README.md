# The parity corpus (CNSL-1, ADR-0056 decision 7)

`ProposalDetail` payloads as the gateway actually serves them, consumed by
**both** renderers of a review: `synveda proposal review` and the console's
proposals inbox.

CNSL-1's acceptance criterion is one clause — *full review parity with CLI* —
and two renderers that agree on the day they are written is not parity. It is
a coincidence with a maintenance schedule. These files are what turns the
word into something a test can fail.

## What is here

Four skill cases recorded from the gateway, and one that no gateway can produce:

| case | what it is in the corpus for |
| --- | --- |
| `skill-clean` | A scan that ran and found nothing, over the bar, with a checklist bound to its bytes. In the corpus because *found nothing* and *no scan here* are different facts, and a renderer that conflates them fails only on this case. |
| `skill-blocking-scan` | Authored under a pack whose floor permits the `high` band, reviewed under one that refuses it — the real way a proposal comes to be blocked, with nothing about the bundle changed. One blocking finding and two non-blocking ones, so a renderer that paints them alike fails. |
| `skill-below-bar` | Two shortfalls at once, so the corpus pins that a refusal names every bar it missed rather than the first. |
| `skill-checklist-stale` | A checklist answered against an earlier draft and therefore **not found**: `requires_checklist` true, `checklist` absent (ADR-0053 decision 4). |
| `skill-unknown-severity` | **Synthesised.** What a *newer* gateway would serve: two severity bands outside `ScanSeverity`'s three, one above the pack's threshold and one below it. |

The last one is the sharp case, and the reason it is worth synthesising. A
client that meets an unfamiliar severity has to guess, and the only safe
guess is to rank it above everything — which is what the CLI's fallback does
and must. That guess is wrong for the band below the threshold. ADR-0056
decision 5 moved the verdict to the gateway precisely so no client has to
make it, and this case is where a renderer that went back to guessing fails.

## `<case>.facts.json` — what a review has to name

Beside every payload is the set of **review-relevant facts** derived from it,
and that is the file both renderers are actually held to. Without it "parity"
degenerates into diffing two transcripts, which fails on whitespace and
passes on a missing finding.

Each facts file carries the proposal's state and what its requirement still
lacks; every approval with whether it still counts; every member with what
publication would do to it, whether it drifted, and the contents a reviewer
must see; every scan finding **in served order** with its path, line, rule,
severity and the gateway's `blocking` verdict; and the two quality numbers,
the checklist's state, and each shortfall's sentence verbatim.

Three things about that list are deliberate:

- **They are data, never layout.** That a blocking finding is
  distinguishable is a fact; that the CLI writes `[blocks]` and the console
  will use a chip is not. ADR-0056 rejected serving a display model so that a
  terminal and a browser could differ where they should, and a corpus that
  pinned wording would be that display model arriving through the back door.
- **They are projections, not judgements.** `blocking` is copied, not
  computed; a shortfall's sentence is copied, not composed. That is only safe
  because decisions 5 and 6 moved those judgements to the gateway. A future
  fact needing a rule to derive it is a fact whose rule belongs on the
  gateway.
- **Conditions are encoded once, here.** A member whose effect is `none` has
  no diff to show, and a member's third content only means something when it
  has drifted — so the facts null those fields out rather than leaving each
  renderer to decide. A condition written twice is a condition two surfaces
  can disagree about.

One clause of the acceptance criterion is **not** covered: the set of actions
offered. Which acts a proposal admits is a function of its state, its pack and
the reader's own roles, and only the first is on the wire — so a corpus that
claimed it would be inventing the other two. It needs a served field, and that
is a decision to take with a screen in front of you.

## Provenance

Recorded by `crates/synveda-gateway/tests/console_parity.rs`, which drives
the real `/v1` API through the real router — no mock, and no hand-built
payload. That test is also the guard:

```sh
make db-test                                # verify: the corpus is what the gateway serves
SYNVEDA_RECORD_FIXTURES=1 make db-test      # re-record it
```

A corpus nobody checks drifts out of the shape the product serves, and then
both renderers agree about a response nobody receives — the same failure the
acceptance criterion is about, one level down.

`skill-unknown-severity.json` has no gateway to be recorded from, so what
holds it honest is its **shape**: the same test asserts it carries the same
fields as a recorded sibling, top level, scan report and finding. A field
added to `ProposalDetail` cannot leave it quietly behind.

## Normalisation

Ids, commit and object addresses and instants are replaced with stable
stand-ins, or no two runs would produce the same bytes. Substitution is
**shape-preserving on purpose**:

- an aggregate id is replaced by something that still parses as a UUID and
  is still 36 characters of hex and hyphens;
- an object address by something still 64 hex characters;
- an instant by a real RFC 3339 instant, written to the precision the
  original carried — a canonical object's `valid_from` has microseconds and
  a `created_at` does not;
- nested identifiers inside string-carried canonical objects are scrubbed
  rather than left alone.

Both renderers key behaviour off those shapes — the CLI abbreviates a
uuid-shaped member name and leaves a path alone — so a corpus that replaced
a UUID with `uuid-01` would be one in which that rule is never exercised, and
both surfaces would agree on a rendering neither produces.

Equal inputs stay equal, so the payload remains internally consistent: a
member's id and another field naming the same identity still match.
What is not promised is that the stand-ins preserve the originals' relative
order.

## Editing these files

Don't, except through the recorder. If a renderer disagrees with the corpus,
the renderer is what changes. A corpus edited to match a renderer has stopped
being evidence — ADR-0056 names that as the reversal trigger for decision 7,
and the replacement would be a generated display model rather than a corpus.
