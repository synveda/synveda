---
title: "TEN-5: Tenant lifecycle"
labels:
  - epic:TEN
  - phase:3
size: M
---

# TEN-5: Tenant lifecycle

**Epic:** TEN — Multi-tenancy (functional requirement) · **Phase:** 3 · **Size:** M

## Description

Create/suspend/export/delete workflows (Temporal); delete produces signed destruction certificate; export = portable archive (records+assets+audit).

## What is missing today, measured (2026-08-12, found by OPS-8)

**There is no way to remove one tenant.** `tenants` is referenced by **32
foreign keys**, every one `ON DELETE NO ACTION`:

```
context_pack_chunks   context_packs        directory_sync_state  graph_vertices
group_mappings        hierarchy_nodes      identities            memory_usage
observe_events        observe_quarantine   policy_lapses         policy_pack_assignments
policy_pack_defaults  policy_packs         promotion_watermarks  prompts
role_bindings         scim_credentials     scim_group_members    scim_groups
scim_users            skill_quality_overrides  skill_reviews     skills
tenant_keys           tenant_secrets       vedaflow_commits      vedaflow_objects
vedaflow_proposal_approvals  vedaflow_proposals  vedaflow_refs   vedaflow_trees
```

So `delete from tenants where …` succeeds only for a tenant that holds
nothing — never one somebody has logged into, because the login alone writes
`hierarchy_nodes`, `identities` and `role_bindings`.

`demos/ops-1-smb-profile.sh` printed that `delete` as its teardown
instruction from OPS-1 until 2026-08-12, immediately after building a
hierarchy and observing a turn. It cannot have worked once. Corrected to say
so and to name the two things that do work: `synveda tenant export` (TEN-4)
to keep the data, and `docker compose down -v` to wipe **every** tenant.

**This is a design input, not just a bug.** `NO ACTION` everywhere is the
right default for a system whose whole claim is that memory does not
disappear quietly, so erasure is a *deliberate ordered traversal* rather than
a cascade somebody could trigger by accident. Two consequences for this
feature:

- the order is not free — `tenant_keys` and `tenant_secrets` are TEN-4's, and
  destroying a key before the rows it seals leaves ciphertext nobody can read
  but the row count still says exists;
- **crypto-shredding is not erasure** (ADR-0064 decision 7): `records`,
  `record_embeddings` and the Tantivy sidecars are not sealed, so this
  feature deletes rows rather than throwing away a key.

## Acceptance criteria

GDPR-style erasure E2E test; export re-imports into a fresh instance.

Plus, from the measurement above: removing one tenant leaves **zero** rows
referencing it across all 32 tables, and the destruction certificate names
what was removed from each. A test that deletes a tenant which was only ever
created — never logged into — would pass against today's schema and prove
nothing.
