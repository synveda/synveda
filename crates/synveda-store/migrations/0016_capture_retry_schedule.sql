-- OPS-13, ADR-0158: a database-time Capture retry schedule, bounded manual
-- retry cycles and installation-wide provider capacity. Additive: existing
-- rows keep their state, and a pending row without a deadline is due now.
ALTER TABLE public.capture_batches
    ADD COLUMN next_attempt_at timestamp with time zone,
    ADD COLUMN terminal_reason text,
    ADD COLUMN manual_retries smallint DEFAULT 0 NOT NULL,
    ADD COLUMN manual_retried_at timestamp with time zone;

-- The attempt counter stays a monotonic fence. A manual retry admits one more
-- bounded cycle of attempts rather than resetting the counter.
ALTER TABLE public.capture_batches DROP CONSTRAINT capture_batches_attempts_check;
ALTER TABLE public.capture_batches
    ADD CONSTRAINT capture_batches_attempts_check
        CHECK (attempts >= 0 AND attempts <= 5 * (manual_retries + 1)),
    ADD CONSTRAINT capture_batches_retry_check CHECK (
        manual_retries BETWEEN 0 AND 2
        AND (manual_retries = 0) = (manual_retried_at IS NULL)
        AND (manual_retried_at IS NULL OR manual_retried_at >= created_at)
        AND (next_attempt_at IS NULL OR state = 'pending')
        AND (terminal_reason IS NULL OR (
            state = 'failed'
            AND terminal_reason IN ('permanent', 'attempts_exhausted', 'retry_age_exhausted')
        ))
    );

CREATE OR REPLACE FUNCTION public.synveda_capture_batch_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.source_kind <> old.source_kind
       or new.session_id is distinct from old.session_id
       or new.import_job_id is distinct from old.import_job_id
       or new.scope_id <> old.scope_id or new.workspace_id <> old.workspace_id
       or new.project_id is distinct from old.project_id
       or new.principal_id <> old.principal_id or new.input_hash <> old.input_hash
       or new.event_count <> old.event_count or new.created_at <> old.created_at then
        raise exception 'capture batch evidence is immutable';
    end if;
    if new.attempts < old.attempts then
        raise exception 'capture batch attempts are a monotonic fence';
    end if;
    if old.state = 'failed' and new.state = 'pending' then
        if new.manual_retries <> old.manual_retries + 1 or new.attempts <> old.attempts
           or new.manual_retried_at is null then
            raise exception 'a failed capture batch reopens only through one bounded manual retry';
        end if;
        return new;
    end if;
    if new.manual_retries <> old.manual_retries
       or new.manual_retried_at is distinct from old.manual_retried_at then
        raise exception 'capture manual retries change only when a failed batch reopens';
    end if;
    if not (
        (old.state = 'pending' and new.state = 'running')
        or (old.state = 'running' and new.state in ('pending', 'completed', 'failed'))
        or (old.state = new.state)
    ) then
        raise exception 'invalid capture batch transition: % -> %', old.state, new.state;
    end if;
    return new;
end
$$;

-- Claims walk only due pending rows, or the few running rows whose lease may
-- have expired, instead of every batch the tenant has completed.
CREATE INDEX capture_batches_due ON public.capture_batches
    USING btree (tenant_id, (COALESCE(next_attempt_at, created_at)), id)
    WHERE state = 'pending' AND source_kind = 'session';
CREATE INDEX capture_batches_running ON public.capture_batches
    USING btree (tenant_id, created_at, id)
    WHERE state = 'running';
DROP INDEX public.capture_batches_pending;

GRANT UPDATE (next_attempt_at, terminal_reason, manual_retries, manual_retried_at)
    ON TABLE public.capture_batches TO synveda_app;

-- Provider capacity is installation-wide, so these rows precede tenant RLS
-- like the login ledger. They name no tenant, batch or endpoint: the key is a
-- digest of the configured provider identity, the holder an opaque process
-- lease owner, and a random reservation fences renewal and release.
CREATE TABLE public.capture_provider_slots (
    provider_key bytea NOT NULL,
    slot smallint NOT NULL,
    holder text,
    reservation uuid,
    reserved_at timestamp with time zone,
    expires_at timestamp with time zone,
    PRIMARY KEY (provider_key, slot),
    CONSTRAINT capture_provider_slots_key_check CHECK (octet_length(provider_key) = 32),
    CONSTRAINT capture_provider_slots_slot_check CHECK (slot BETWEEN 0 AND 63),
    CONSTRAINT capture_provider_slots_holder_check CHECK (
        (holder IS NULL) = (reservation IS NULL)
        AND (holder IS NULL) = (reserved_at IS NULL)
        AND (holder IS NULL) = (expires_at IS NULL)
        AND (holder IS NULL OR (btrim(holder) <> '' AND char_length(holder) <= 255))
        AND (expires_at IS NULL OR expires_at > reserved_at)
    )
);
GRANT SELECT, INSERT ON TABLE public.capture_provider_slots TO synveda_app;
GRANT UPDATE (holder, reservation, reserved_at, expires_at)
    ON TABLE public.capture_provider_slots TO synveda_app;
