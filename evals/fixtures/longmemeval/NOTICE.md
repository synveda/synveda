# Third-party corpus: LongMemEval

| | |
| --- | --- |
| **Corpus** | LongMemEval |
| **Upstream** | https://github.com/xiaowu0162/LongMemEval |
| **Paper** | *LongMemEval: Benchmarking Chat Assistants on Long-Term Interactive Memory* (Wu et al., ICLR 2025) |
| **Licence** | MIT |
| **Attribution** | Copyright (c) 2024 Di Wu |
| **Vendored** | No — see "Why the data is not committed" below |
| **Checked** | 2026-08-08 (EVAL-3, ADR-0061 decision 1) |

`scripts/check-corpus-licences.mjs` reads this directory's row from its own
table and fails if a directory under `evals/fixtures/` is not named there.
That gate exists because of what ADR-0061 decision 1 found: LoCoMo's corpus
is CC BY-NC 4.0, a licence that withholds exactly the commercial use this
feature's acceptance criterion names — and nothing in the build would have
caught it, because `cargo deny` governs crates and a corpus is data. The
gap is closed where the build can see it: adding a corpus directory now
fails until somebody writes down where it came from and under what licence,
which puts the licence in a diff somebody reviews.

## Why the data is not committed

`longmemeval_s` is 500 instances whose haystacks are ~115k tokens each —
hundreds of megabytes. `longmemeval_m` is larger by an order of magnitude.
Neither belongs in a git history, so this directory holds the licence
record and the instructions, and the data is fetched.

That leaves a published benchmark score with no bytes in this repo to point
at, which would make it unreproducible by anyone including us. So every run
records the **BLAKE3 digest** of the corpus file it read, and decision 11's
published row in `docs/BENCHMARKS.md` carries it beside the score. The
corpus is identified by its hash rather than by its presence.

## Fetching it

Upstream distributes the data separately from the code, on Hugging Face.
Verified 2026-08-09:

```sh
cd evals/fixtures/longmemeval
B=https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main
curl -sLO "$B/longmemeval_s_cleaned.json"          # 264.5 MiB — the default
curl -sL -o LICENSE https://raw.githubusercontent.com/xiaowu0162/LongMemEval/main/LICENSE
```

Both are ignored by git (see `.gitignore`). The licence gate asserts the
second is present whenever the first is: a vendored corpus keeps its licence
file intact, and a fetched one is no different.

## Which variant to fetch

**Mind the `_cleaned` suffix.** A 2025/09 release "further cleaned up the
history sessions to prevent interference on answer correctness", and the
files were renamed with it. The older `longmemeval_s.json` and
`longmemeval_m.json` names this directory first assumed no longer exist,
which is worth knowing before a run reports a missing file.

| File | Size | What it measures |
| --- | --- | --- |
| `longmemeval_s_cleaned.json` | 264.5 MiB | the benchmark as published — this is the default |
| `longmemeval_m_cleaned.json` | larger | the same questions under a harder retrieval load |
| `longmemeval_oracle.json` | 14.7 MiB | evidence sessions only — reading and judging with retrieval removed |

`crates/synveda-eval/src/longmemeval.rs` reads all three — they share one
format — and the report names which file and which digest, because
`longmemeval_s_cleaned` and `longmemeval_oracle` are very different claims.
`scripts/publish-benchmark.mjs` will only publish a score computed from one
of these three names.

## What the corpus turned out to be

Measured on fetch, and recorded because three of this harness's guards were
written from the published format and met the file only afterwards:

| | `longmemeval_oracle` | `longmemeval_s_cleaned` |
| --- | --- | --- |
| instances | 500 | 500 |
| sessions | 948 | 23,867 |
| turns | 10,960 | 246,750 |
| abstention instances | 30 | 30 |

Three things the transcription got wrong, all found by the loader refusing
the file rather than absorbing it:

1. **`answer` is not always a string** — 32 of the 500 are bare integers,
   because "how many" has a number for an answer.
2. **Every instance names `answer_session_ids`, abstention included.** The
   guard here asserted the opposite. An abstention question asks about
   something *half* discussed, so its named sessions are the partial
   evidence a reader needs in order to establish the absence.
3. **Thirteen session ids repeat inside a haystack and twelve turns are
   blank.** Every duplicate is byte-identical to its twin and none of
   either is in a session an instance names, so a repeat is fatal only
   when the two sessions differ, and blank turns are skipped and counted
   at seed time.

## What is read, and what is never written

The corpus is read in upstream's own field names and never converted,
rewritten or filtered. ADR-0061 decision 2: *"editing an external corpus
until it [satisfies our guards] is the one thing that would invalidate the
score."* The loader's guards are integrity checks — the three haystack
arrays line up, every evidence session named exists, the abstention marker
and the evidence list agree — and when one fails, the answer is to ask
upstream, not to edit the file. Every failure message says so.

## The corpus this one is missing

LoCoMo was to be the second corpus and is not here. Its `LICENSE.txt` is
Creative Commons Attribution-NonCommercial 4.0, and this feature's own
acceptance criterion calls the scores a marketing artefact — the paradigm
case of the use that licence withholds. **EVAL-7** carries the follow-on
with two named paths: written permission from Snap Research, or a
permissively-licensed substitute in LoCoMo's slot. Nothing under
`evals/fixtures/` may carry a non-commercial or no-derivatives licence, and
the gate scans for those by name.
