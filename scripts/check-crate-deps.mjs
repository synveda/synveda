#!/usr/bin/env node
// Enforces the crate layering rule (seed §8; synveda-vedaflow added by tech plan §5,
// synveda-crypto by TEN-4/ADR-0064 decision 13):
//
//   types ← crypto ← {policy, store, identity, audit, vedaflow} ← retrieval/ingest ← gateway
//
// Nothing imports upward. Fails if any synveda crate declares a dependency on a
// synveda crate outside its allowed set, or if a workspace crate is unknown here
// (new crates must be placed in a tier deliberately).
import { execFileSync } from "node:child_process";

// Below the middle band rather than in it: store, identity and vedaflow all
// seal or open payloads, and the rule forbids middle-band crates depending on
// each other. It takes synveda-types (and nothing else) because the AAD is
// composed from typed arguments — a crypto crate dealing in `&[u8]` would sit
// one tier purer and would let a caller seal a payload without binding it to a
// tenant (ADR-0064 decisions 4 and 13).
const BASE = ["synveda-types", "synveda-crypto"];

const MIDDLE = [
  "synveda-policy",
  "synveda-store",
  "synveda-identity",
  "synveda-audit",
  "synveda-vedaflow",
];

const ALLOWED = {
  "synveda-types": [],
  "synveda-crypto": ["synveda-types"],
  "synveda-policy": [...BASE],
  "synveda-store": [...BASE],
  "synveda-identity": [...BASE],
  "synveda-audit": [...BASE],
  "synveda-vedaflow": [...BASE],
  "synveda-retrieval": [...BASE, ...MIDDLE],
  "synveda-ingest": [...BASE, ...MIDDLE],
  // External knowledge-format adapters are a leaf beside retrieval/ingest:
  // they understand shared value types but cannot see storage, policy,
  // VedaFlow, audit or the gateway (CPR-27, ADR-0087 decision 2).
  "synveda-okf": ["synveda-types"],
  "synveda-gateway": [
    ...BASE,
    ...MIDDLE,
    "synveda-retrieval",
    "synveda-ingest",
    "synveda-okf",
  ],
  // The CLI is a client of the gateway API, plus direct store/identity access
  // for the dev-bootstrap commands (db migrate, tenant create, token issue)
  // that exist precisely when no usable gateway does. Reviewed in ADR-0008.
  // Policy added with AUTHZ-1 (ADR-0012): `synveda policy apply` compile-checks
  // a pack against the same schema the gateway's reloader enforces.
  // Audit added with AUD-1 (ADR-0019): the break-glass audits itself, and
  // `synveda audit verify` is the operator's chain check.
  // VedaFlow added with SKIL-1 (ADR-0051 decision 12): `synveda skill install`
  // recomputes each written file's content address and compares it to the one
  // the published commit named. That is what makes "installs unmodified" a
  // measurement rather than a claim, and it is worth more computed by the
  // client than trusted from the server — a materialised bundle carries no
  // watermark of its own (force 2), so this hash is its whole provenance.
  // The eval harness depends on no Synveda crate at all, and this empty
  // set is the enforcement (EVAL-1, ADR-0028 decision 1). An eval that can
  // link the store can seed and read around the PDP and would then report
  // quality the product cannot deliver; one that speaks only `/v1` measures
  // what a caller gets. It is the standing the seed gives adapters and SDKs,
  // applied to the thing that grades the product.
  "synveda-eval": [],
  // Crypto added with TEN-4 (ADR-0064 decision 8): `synveda tenant export`
  // seals an archive and `synveda tenant export open` is the tool that
  // demonstrates the AC by needing the key. Both are operator commands run
  // where the gateway is not the right party to hold a tenant's export.
  "synveda-cli": [
    "synveda-types",
    "synveda-crypto",
    "synveda-store",
    "synveda-identity",
    "synveda-policy",
    "synveda-audit",
    "synveda-vedaflow",
  ],
};

const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  }),
);

let failed = false;
for (const pkg of metadata.packages) {
  const allowed = ALLOWED[pkg.name];
  if (allowed === undefined) {
    console.error(
      `FAIL: unknown workspace crate '${pkg.name}' — assign it a tier in scripts/check-crate-deps.mjs`,
    );
    failed = true;
    continue;
  }
  const allowedSet = new Set(allowed);
  for (const dep of pkg.dependencies) {
    if (dep.name.startsWith("synveda-") && !allowedSet.has(dep.name)) {
      console.error(
        `FAIL: ${pkg.name} -> ${dep.name} violates the layering rule (seed §8)`,
      );
      failed = true;
    }
  }
}

if (failed) process.exit(1);
console.log(`crate layering rule holds for ${metadata.packages.length} crates (seed §8)`);
