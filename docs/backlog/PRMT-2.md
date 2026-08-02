---
title: "PRMT-2: Context packs"
labels:
  - epic:PRMT
  - phase:2
size: M
---

# PRMT-2: Context packs

**Epic:** PRMT — Prompt & context-pack registry · **Phase:** 2 · **Size:** M

## Description

Curated doc bundles (conventions, glossaries) pinned to scopes; chunked+embedded on publish; composed by CTX-2 as pinned material.

## Acceptance criteria

Written 2026-08-02 (ADR-0050). PRMT-1 found that the governance machinery for
an authored asset already existed; that is true again here, and emphatically
not true of the read half. A prompt is fetched by name and composes into
nothing. **A context pack is the first authored asset whose content has to
enter the corpus the read path ranks** — which is why ADR-0049's third reason
for refusing "prompts as memory records" inverts and stops being an argument.

- **A pack authored at a scope reaches a session only through the review the
  pack in force asks for** — and under `regulated-strict` at a department,
  division or org that is now a curator *and* a steward, two distinct people,
  where FLOW-3 had left the cell at one curator. Publishing a bundle into
  every session in the company must not be cheaper than publishing one memory
  record at the same scope (ADR-0050 decision 15). Locally it stays at one
  curator, and `standard` and `open-collaboration` are untouched.
- **"Re-embeds atomically" is measured from the reader's side.** No inject
  ever composes half a pack: the previous version composes in full until the
  new one is entirely embedded *and* published, and the new one in full
  thereafter. The chunk rows land with their embeddings or not at all
  (ADR-0023 decision 2), and the ref cannot move to a commit whose chunks do
  not yet exist.
- **"Next session" is satisfied as "next call".** The pack channel is read
  live on the composition path, so the first inject after a publication
  composes the new version. Nothing caches a pack across sessions, and the
  AC's weaker phrasing is not what the product does.
- **Pack content composes as pinned material, ranked, and what does not fit
  is named rather than dropped.** Seed §4.4's "pinned beats derived" applies
  because the chunks *are* pinned records; ADR-0025 option 5's concern —
  canonical content must not silently vanish — is kept by the index tier
  rather than by composing a 20,000-token glossary against a 1,500-token
  budget. A block that cannot hold the runbook says the runbook exists, names
  it, and hands back a recall handle.
- **`ContextPackRead` admits pack chunks and `MemoryRead` never does.** A
  reader who holds no readable memory at a scope still receives that scope's
  conventions, which is the case packs exist for; the decision is taken per
  scope inside the plan walk composition already runs, never as a second
  authorization path. Per-scope tiers and bank mode apply to pack material
  exactly as they apply to memory.
- **A published document that is edited demotes its own chunks.** ADR-0031
  decision 5 reaching chunks through the document address the channel names:
  an edit cannot be laundered through chunks the tree still appears to name.
- **A rewind restores the previous version by moving a ref**, with no
  re-embedding and no half-swapped state, and a pin freezes what the pack
  channel serves. `ContextPackRead` is what makes both decidable, discharging
  ADR-0036 decision 3 for the second of the three kinds it refused by name and
  leaving `skill`.
- **A document carrying a live credential is quarantined at authoring.** This
  is the first surface where bulk external text enters the product — prompts
  are short and hand-written, and PRMT-1 does not scan them. MEM-2's scanner
  and its per-pack modes govern it, the scan runs ahead of the embedder, and
  no secret reaches vector space.
- **`ContextPackRead` and `ContextPackWrite` join the role×action golden
  matrix under all three packs** and the service-identity confinement list,
  with the prompt plane's discipline: the pack's own memory plane, mirrored
  tier for tier and asserted rather than stated.
- **Every act is on the chain** — `context_pack.authored`,
  `context_pack.quarantined`, and the same `vedaflow.channel.published` a
  memory publication emits with `asset` reading `context-pack` — with served
  chunks watermarked inside `context.injected` like every other entry, and no
  document text in any payload, swept for.
- **Demo script.**

Deferred with recorded triggers (ADR-0050): embedding stays at **authoring**
rather than at publish, so a curator's approval cannot fail because a model
server is down — the reversal trigger is authoring latency on a large bundle,
which moves the embed stage to a PGMQ worker without changing a decision;
there is **no separate token budget lane** for pack material, because
displacement is EVAL-4's measurement to make rather than this feature's guess;
the chunker is **structural and deterministic**, since a model-driven splitter
would give the same bytes different content addresses on different days; a
pack cannot be `restricted`, for ADR-0049 decision 5's unchanged reason; and
there is no draft deletion — retraction is FLOW-7's rewind.
