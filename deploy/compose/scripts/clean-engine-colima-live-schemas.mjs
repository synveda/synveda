export const COLIMA_LIVE_REQUIREMENTS_SCHEMA =
  "synveda.clean-engine.colima-live-requirements.v5";
export const COLIMA_LIVE_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-observation.v5";
export const COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-public-projection.v5";
export const COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-pre-effect-root-observation.v5";
export const COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-pre-effect-root-observation.v5";

export const COLIMA_LIVE_PROVIDER_RESERVATION_NAME =
  ".synveda-clean-engine-provider-reservation";
export const COLIMA_LIVE_MAX_BASELINE_DESCENDANTS = 64;

// The observer and the pure effect validator share this exact, content-safe
// descriptor projection. Public evidence exposes only the digest paired with
// its keyed relative identity; raw names, paths and content remain private.
export const COLIMA_LIVE_BASELINE_DESCRIPTOR_FIELDS = Object.freeze([
  "content_disposition",
  "content_sha256",
  "ctime_nanoseconds",
  "depth",
  "device",
  "directory_entry_count",
  "inode",
  "links",
  "mode",
  "mtime_nanoseconds",
  "name_bytes",
  "parent_identity_hmac_sha256",
  "relative_identity_hmac_sha256",
  "size",
  "symlink_target_sha256",
  "type",
  "uid",
]);
export const COLIMA_LIVE_BASELINE_DESCRIPTOR_BINDING_FIELDS = Object.freeze([
  "descriptor_sha256",
  "relative_identity_hmac_sha256",
]);

// Each entry is a dedicated, receipt-owned namespace. A pre-effect admission
// observes its whole bounded top-level inventory and recursively commits the
// closed baseline subtree before any provider effect can be selected.
export const COLIMA_LIVE_MUTATION_SURFACE_ROLES = Object.freeze([
  "colima-cache-namespace",
  "colima-home-namespace",
  "docker-config-namespace",
  "lima-home-namespace",
  "private-home-namespace",
  "temporary-namespace",
]);
