-- ADPT-2: `assertion` joins the observe vocabulary (ADR-0057 decision 8).
--
-- A widened CHECK, and the reason it is a fourth *kind* rather than a new
-- column beside `kind`.
--
-- ADPT-2 gives a model-driven MCP client a write tool, so for the first time
-- content can reach the staging buffer because a model composed it and chose
-- to store it, rather than because a hook observed a session. That is a real
-- distinction and it is not recoverable after the fact: once a model's
-- assertion and a host's observation share a value, no later feature can
-- separate them, and the corpus can never answer "did a person say this or
-- did a model decide it" about anything written before somebody noticed
-- (ADR-0057 option 8, rejected for exactly that reason).
--
-- It lands on `kind` because `kind` is already per-event, already on the
-- wire, already what extraction switches on, and already stable across
-- MEM-1/2/3. A parallel `source` column on the same axis would be a second
-- thing to keep in sync and — worse — a second thing a caller can leave
-- unset, which makes the absence of the claim ambiguous rather than false.
--
-- Widening a CHECK is expand-only and needs no backfill: every existing row
-- holds one of the three original values and stays valid, no existing reader
-- can encounter `assertion` in data written before this migration, and
-- nothing that already parses the column loses a value it used to accept.
-- `observe_events` is insert-only by grant (migration 0012), so there is no
-- update path that could rewrite an old row into the new value either.

alter table observe_events
    drop constraint observe_events_kind_check;

alter table observe_events
    add constraint observe_events_kind_check
        check (kind in ('transcript_delta', 'tool_result', 'decision', 'assertion'));
