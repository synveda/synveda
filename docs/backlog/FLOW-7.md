---
title: "FLOW-7: Rollback & pinning"
labels:
  - epic:FLOW
  - phase:2
size: S
---

# FLOW-7: Rollback & pinning

**Epic:** FLOW — VedaFlow (git-style governance) · **Phase:** 2 · **Size:** S

## Description

Ref rollback; agents heal next session; assets pinnable to a commit per scope.

## Acceptance criteria

bad-prompt rollback demo <60s to fleet-wide effect.

A rewind can only install a state the channel has held — a proposal commit and an
orphaned publication are both refused by name (ADR-0036 decisions 1–2).

A pinned scope serves its pinned commit while publications keep landing; the block's
watermark says so; releasing the pin catches every reader up on the next session.

## Notes

The two criteria beyond the first were written before implementation, on the EVAL-1
precedent: "assets pinnable to a commit per scope" arrived with no way to fail, and the
one-line criterion measures healing without saying what a rewind is allowed to install —
which is the whole of the design (ADR-0036).

"Bad prompt" is the tech plan's example (§2.5); prompts become assets with PRMT-1, so the
demo publishes the asset kind that has a writer today — a memory record carrying an
operational instruction, which is what reaches an agent's context either way. The rollback
route is asset-kind generic and refuses kinds whose read action does not exist yet.
