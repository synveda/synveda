export const COLIMA_LIVE_REQUIREMENTS_SCHEMA =
  "synveda.clean-engine.colima-live-requirements.v2";
export const COLIMA_LIVE_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-observation.v2";
export const COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-public-projection.v2";
export const COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-pre-effect-root-observation.v2";
export const COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-pre-effect-root-observation.v2";

// Each entry is a dedicated, receipt-owned namespace. A pre-effect admission
// observes its whole bounded top-level inventory, rather than guessing the
// subset of paths that pinned upstream processes may create.
export const COLIMA_LIVE_MUTATION_SURFACE_ROLES = Object.freeze([
  "colima-cache-namespace",
  "colima-home-namespace",
  "docker-config-namespace",
  "lima-home-namespace",
  "private-home-namespace",
  "temporary-namespace",
]);
