-- MEM-6: decay, TTL & staleness (ADR-0040).
--
-- One column and two grants. There is deliberately no retention *state*
-- anywhere: no `expires_at`, no per-record TTL, no sweep bookkeeping table.
-- A record's fate is a function of facts it already carries — class, kind,
-- valid time — and the pack in force at its scope now, which is what makes
-- "a retention policy change re-evaluates existing records" structural
-- rather than a backfill (ADR-0040 decision 1).
--
--   policy_packs.retention  a stored pack's optional RetentionConfig,
--                           exactly as `redaction` (0013), `composition`
--                           (0017) and `dedup` (0024): null means the
--                           product default, whose record horizons are all
--                           unset.
--   records_history DELETE  the destruction half of retention (ADR-0040
--                           decision 5), gated on a named flag the
--                           append-only trigger honours.
--   staging DELETE          observe_events + observe_quarantine, the
--                           disposal migration 0012 and 0013 both said
--                           would "bring its own grants".
--
-- What is deliberately *not* here: any change to `records`. Expiry is
-- `delete from records`, which the FND-4 archive trigger already turns
-- into a closed transaction period and an archived version — the temporal
-- delete ADR-0006 built and ADR-0040 decision 5 uses unchanged.

-- ── The pack's retention configuration ──────────────────────────────────────

alter table policy_packs add column retention jsonb;

-- ── The destruction path ────────────────────────────────────────────────────

-- `records_history` has been append-only by trigger since migration 0001,
-- whose own comment says what that trigger is: "not a security boundary (a
-- superuser can drop triggers) — defence in depth against application bugs,
-- complementary to the AUD-1 hash chain". Retention needs one deliberate,
-- named way through it, and ADR-0040 decision 6 chose the flag over a
-- SECURITY DEFINER function precisely because the function would run as the
-- owner and bypass RLS — trading a defence-in-depth trigger for a hole in
-- the boundary that is one.
--
-- So: DELETE is permitted only while `synveda.retention_purge` is 'on', a
-- GUC the sweep sets with SET LOCAL inside its own tenant transaction. RLS
-- is untouched and still forces tenant_id, so a purge cannot reach another
-- tenant's history however the flag is set (the adversarial suite asserts
-- exactly that). UPDATE and TRUNCATE keep raising unconditionally: history
-- is never rewritten and never wholesale-dropped, only destroyed row by row
-- past a horizon somebody configured.
create or replace function records_history_append_only() returns trigger
language plpgsql as $$
begin
    if tg_op = 'DELETE'
       and coalesce(current_setting('synveda.retention_purge', true), 'off') = 'on'
    then
        return old;
    end if;
    raise exception 'records_history is append-only (% attempted)', tg_op;
end;
$$;

-- The trigger created in 0001 is BEFORE UPDATE OR DELETE; the function
-- above now lets one of those through under the flag. The truncate trigger
-- shares the function and is statement-level, where tg_op is 'TRUNCATE' and
-- the raise still fires.
grant delete on records_history to synveda_app;

-- ── The staging plane's disposal ────────────────────────────────────────────

-- Migration 0012: "content disposal is MEM-6/TEN-5 territory and brings its
-- own grants." Migration 0013, on the quarantine FK: "disposal (MEM-6/TEN-5)
-- retires both at the same horizon." Both discharged here.
--
-- Order matters at disposal time and the FK enforces it: the quarantine
-- marker goes before the staging row it points at. Disposal frees
-- (tenant_id, idempotency_key), which is why RetentionConfig::validate
-- refuses a staging horizon under one day — MEM-1's admission gate is worth
-- exactly as long as this plane is kept (ADR-0040 decision 7).
grant delete on observe_events to synveda_app;
grant delete on observe_quarantine to synveda_app;

-- Migration 0013 raised on *every* delete from observe_quarantine, naming
-- this feature in the message: "retired by disposal (MEM-6/TEN-5), never
-- deleted". Disposal is now here, so the trigger learns the one exception
-- and keeps refusing everything else — including TRUNCATE, and including a
-- delete by a handler that has not declared itself a disposal. Same flag as
-- the history purge: one transaction-local statement of intent, honoured by
-- every append-only surface retention is allowed through.
create or replace function synveda_observe_quarantine_immutable() returns trigger
language plpgsql
as $$
begin
    if tg_op = 'DELETE'
       and coalesce(current_setting('synveda.retention_purge', true), 'off') = 'on'
    then
        return old;
    end if;
    raise exception
        'observe_quarantine rows are retired by retention disposal (MEM-6), never deleted';
end
$$;
