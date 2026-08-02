---
title: "PRMT-1: Prompt templates as assets"
labels:
  - epic:PRMT
  - phase:2
size: M
---

# PRMT-1: Prompt templates as assets

**Epic:** PRMT — Prompt & context-pack registry · **Phase:** 2 · **Size:** M

## Description

Versioned, variable-schema'd templates; draft→review→publish; consumed via API/SDK by id + channel.

## Acceptance criteria

Written 2026-08-02 (ADR-0049). The feature arrived with two clauses, and one of
them — "consumer pins channel or commit" — had already been refused in the only
reading it then had: ADR-0036 decision 12 turned down reader-side pinning,
naming this feature's phrasing as the thing it was refusing.

- **A prompt authored at a scope reaches a consumer only through the review the
  pack in force asks for.** Under the default pack the direct publish route
  refuses it by name, short of the steward and the curator the `prompt` cell has
  priced at two distinct people since FLOW-3; the same two approvals through
  `POST /v1/proposals` carry it. The direct route is not a hole to close — it
  resolves the same matrix (ADR-0032 decision 8), and the refusal is that pack's
  arithmetic rather than a rule about prompts.
- **"Behind review" is measured from the reader's side**, never at the writing
  surface. The draft is edited under its own published version: the author's
  draft read returns the edit, and the consumer keeps being served the reviewed
  bytes *at the reviewed commit* until a second proposal lands.
- **A consumer names a channel and follows publications, or names a commit and
  holds.** The pinned read is a request parameter — stored nowhere, governing
  nobody else, expiring with the request — which is what makes it a different
  thing from the stored pin ADR-0036 decision 12 refused.
- **A rewind refuses the pin rather than outliving it.** When FLOW-7 takes the
  pinned commit off the channel's first-parent line, the pinned read is a
  `Conflict` naming both commits: serving the withdrawn bytes would make "<60s
  to fleet-wide effect" a lie, and serving the head instead would make the pin
  one. The consumer learns on its next call rather than its next session.
- **A pin freezes bytes and never authority.** The same pinned read stops
  resolving when the pack behind it is replaced — CTX-4's rule for handles,
  restated for commits.
- **Resolution walks the caller's own placement chain nearest-first and skips
  the scopes the PDP refuses.** A team's version overrides the org's; a nearer
  copy nobody may read does not shadow the readable one above it; and a name
  nothing publishes is the uniform 404 rather than an existence oracle.
- **The variable schema is enforced where it can fail.** A template whose
  placeholders and declared variables disagree is refused at authoring, naming
  the offender, and rendering refuses a missing required value and an undeclared
  one. A schema returned beside a template and checked by nobody is a document.
- **`PromptRead` and `PromptWrite` join the role×action golden matrix under all
  three packs** and the service-identity confinement list. `PromptRead` is what
  makes a rewind or a pin of `prompt/published` decidable, discharging ADR-0036
  decision 3's "refused by name until PRMT-1 brings their read action".
- **Every act is on the chain** — `prompt.authored`, `prompt.resolved`, and the
  same `vedaflow.channel.published` a memory publication emits with `asset`
  reading `prompt` — with no template text in any payload, swept for.
- **Demo script.**

Deferred with recorded triggers (ADR-0049): a `restricted` prompt is
unrepresentable, because the only mechanism that mints the tier is a
classification proposal over *records*; there is no draft deletion, since
retracting a published prompt is FLOW-7's rewind and replacing a draft is an
overwrite; a template cannot contain a literal `{{`, because every one opens a
placeholder and the lenient reading ships a typo to a fleet; there is no
server-side render route, so the substitution rule stays one implementation in
`synveda-types`; and prompts do not appear in `inject`'s index tier, which is
SKIL-4's shape rather than CTX-4's.
