-- CTX-2: composition engine (ADR-0025).
--
-- policy_packs.composition — a stored pack's optional CompositionConfig
-- (estimated-token budget + inject channel rule); null means the product
-- default (budget 1500, published-and-derived — ADR-0025 decisions 2–3;
-- the config only ever narrows what already-readable material composes,
-- so the product default is the honest fallback, not a widening).
-- Embedded product packs carry compiled-in configs and no row, exactly
-- like policy_packs.redaction (migration 0013).

alter table policy_packs add column composition jsonb;
