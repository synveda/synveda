# ADPT-5: Source-format converters

## Problem and evidence

ADR-0087 and CPR-27 already deliver bounded OKF v0.2 import/export: an immutable `ImportJob` materialises capture candidates and never active Knowledge. No source-specific converter exists for claude-mem, Cognee, or mem0 exports. Those formats and versions are external and mutable, so claiming fidelity without pinned fixtures, licences, and loss accounting would manufacture provenance.

## Scope

- Obtain licensed representative exports and pin each supported source product, exporter version, schema, and acquisition procedure.
- Implement deterministic client-side converters from claude-mem, Cognee, and mem0 exports into canonical OKF v0.2 bundles accepted by the existing import API.
- Preserve stable source identifiers, timestamps, authorship/identity claims, revision lineage, relations, tags, and source hashes where present; classify every dropped, approximated, or unsupported field.
- Provide bounded validation and dry-run reports before upload, then use the existing `ImportJob` plan/materialise/candidate review path.
- Make re-import idempotent for the same source/version/digest and report conflicts rather than silently merging them.

## Non-goals

- Writing active Knowledge directly, manufacturing session events, or reviving Record-era storage/provenance.
- Server-side filesystem/database connectors, background source polling, reverse sync, or credential custody.
- Inferring identity links, authority, sensitivity, or facts missing from the source export.
- Promising lossless round-trip back into a source system whose format cannot represent Synveda semantics.

## Architecture seam

Source formats terminate in standalone converter packages or CLI commands and produce only pinned OKF v0.2. `synveda-okf` remains the external-format leaf, while the public OKF API owns validation, immutable artifacts, `ImportJob`, capture candidates, PDP/RLS/audit, and governed acceptance. Converter-specific vocabulary never enters the core schema.

## Acceptance criteria

- Each named importer supports at least one pinned real exporter version and rejects unknown versions without best-effort coercion.
- A deterministic fidelity report accounts for every source object and field as preserved, transformed, omitted, conflicting, or unsupported, with source and output digests.
- Re-importing identical input creates no duplicate candidates or active Knowledge; changed input is a separately attributable import.
- Materialisation creates reviewable candidates only, and accepted Knowledge retains the immutable import artifact and source identifier in provenance.
- Malformed, hostile, oversized, traversing, linked, or decompression-bomb inputs fail within documented bounds and leak no content.

## Required tests

- Licensed golden exports for every supported source/version, including empty, duplicate, deleted, timestamp, relation, identity, and unknown-field cases.
- Determinism and full field-accounting snapshot tests for each converter.
- Existing OKF archive/URL boundary tests plus converter-specific size, encoding, traversal, and secret-redaction tests.
- Public API end-to-end import, materialise, candidate review, acceptance, and idempotent re-import test.
- Licence/inventory and package-install checks for converter fixtures and dependencies.

## Rollout and rollback

Release each importer independently as experimental for one pinned exporter version, with dry-run required by default. Promote only after a real export completes review. Rollback removes that version from the advertised support matrix and keeps prior import artifacts/audit evidence; it never deletes accepted Knowledge automatically.

## Dependencies

Each source requires an owner-approved exporter version, format licence/terms, representative non-sensitive export, and field-semantics review. Public packages also depend on ADPT-4's licence, registry, signing, and compatibility decisions. Source-vendor changes remain an external maintenance dependency.
