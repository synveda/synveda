---
title: "CPR-35: Context-platform key and secret convergence"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-35: Context-platform key and secret convergence

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Re-anchor the existing per-tenant envelope-key and sealed-secret plane on the
current Knowledge, Tool, directory and deployment model. Stable secret
references must let immutable artifacts survive credential and DEK rotation
without ever exposing plaintext.

## Acceptance criteria

- Tenant secrets have stable UUIDv7 identity, governing scope, a closed kind,
  credential-free provider/label metadata, a logical value revision, current
  key generation and active/revoked state. Rotation retains identity;
  revocation removes ciphertext and retains only content-free evidence.
- Old name-keyed secret rows are refused and deleted without translation.
  Every replacement tenant table is enabled/forced RLS and included in the
  completeness gate.
- A canonical internal Tool secret reference is tenant-, scope-, kind- and
  state-checked at version staging/application and each configuration render.
  A stale, revoked, wrong-kind or cross-tenant reference fails closed without
  revealing which predicate failed. External opaque references remain adapter
  metadata and never become authorisation.
- Directory credentials use the same stable aggregate. Rotation is auditable;
  revocation prevents a stale stored reference from silently falling back to
  deployment configuration; corrupt envelopes remain fail-closed.
- Tenant-key rotation creates and completes a durable, retryable re-encryption
  job for active local secrets. Re-encryption changes ciphertext/key
  generation only, not stable secret ids, logical value revisions or immutable
  Tool history; old export archives still open under retired generations.
- Tenant export uses a new hard-cut format containing Knowledge aggregates,
  immutable head history/revisions, normalised provenance/relations and the
  audit chain. It contains no Record section or compatibility reader, and all
  Knowledge plaintext is inside the tenant-bound envelope.
- Deployment model-provider credentials remain deployment secrets, OKF stays
  credential-free, and the documented KMS/secret-provider boundary makes no
  unsupported cloud-KMS, HSM or customer-managed-key claim.
- Adversarial tests prove cross-tenant/AAD isolation, stale references,
  logical secret rotation, DEK re-encryption, removed bindings, API/log/debug
  redaction and Knowledge-export isolation. Focused tests, acceptance demo,
  `make ci` and `make db-test` pass.

## Evidence

Delivered 2026-08-25 from
`13ba0596c75f46f30d77e605c4f7548ae44af425` under accepted ADR-0094.
Migration `0060_context_secret_plane` makes local secret identity stable,
scope-bound and forced-RLS, adds the durable re-encryption ledger and refuses
the old name-keyed shape with the reset instruction. Tool and directory
consumers resolve the same reference fail-closed; operator key rotation
preserves secret identity/logical revision while advancing ciphertext; and
sealed export format `synveda-context-export-2` contains complete Knowledge
history/provenance/relations plus audit with no Record section or old reader.

AC evidence: types 214/214 and crypto 39/39; store keys 14/14, Knowledge
export projection and forced-RLS completeness; gateway Tools 2/2 and
directory sync 10/10; CLI 158/158, OpenAPI 6/6 and console 212/212.
`demos/cpr-35-key-secret-convergence.sh` and the re-cut
`demos/ten-4-envelope-keys.sh` pass against fresh local databases; the
83-script drift gate, `make ci` and full `make db-test`
(`synveda_test_69956`, removed on success) pass. No cloud-KMS, HSM,
customer-managed-key or live external-provider claim is made. The feature
commit hash is recorded by the next checkpoint.
