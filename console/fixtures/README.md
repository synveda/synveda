# Console fixture evidence

Only API-shaped, non-secret evidence that remains owned by a console acceptance
test lives here. `explorer/` contains capability and policy-profile responses
used by the scope explorer parity suite.

CPR-24 deleted the old mutable-Skill proposal corpus. Immutable version scan,
rubric, provenance, file, controlled-harness and usage evidence is now asserted
through `console/src/skills.test.tsx`, over the generated CPR-23 contract. The
shared VedaFlow review renderer has inline artifact-neutral fixtures and no
Skill-only checklist or quality branch.
