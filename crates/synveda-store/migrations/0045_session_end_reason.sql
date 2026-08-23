-- CPR-11: how a run ended, in the client's words (ADR-0077 decision 4).
--
-- CPR-10 gave a session five states, and three of them are terminal: `ended`,
-- `abandoned`, `failed`. A state says *that* a run stopped and which of the
-- three ways it stopped in; it cannot say **why**, and "why" is the whole of
-- what somebody reading a failed run came for. `task_summary` is not that
-- field — it is what the run was *about*, set at open and refined at close,
-- and overloading it would make the two indistinguishable the first time a
-- client set both.
--
-- Nullable, because most closes have nothing to add: a run that finished is
-- explained by `ended`. Bounded and free text, because the vocabulary belongs
-- to the harness — `hook timed out`, `context window exhausted`, `user
-- cancelled` — and a closed list here would be a core change per harness
-- (seed §2 principle 6), which is the same argument `client_name` is a grammar
-- rather than an enum for.
--
-- Never set while a session is `active`: a reason is part of a close, so a row
-- carrying one while still running would be a state nothing wrote.

alter table sessions add column end_reason text;

alter table sessions
    add constraint sessions_end_reason_check
    check (end_reason is null
           or (btrim(end_reason) <> '' and length(end_reason) <= 500));

alter table sessions
    add constraint sessions_end_reason_shape_check
    check (end_reason is null or status <> 'active');
