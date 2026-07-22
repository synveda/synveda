-- MEM-3 (ADR-0022 decision 5): background pipelines act as their own
-- kind of actor. 'system' events carry the component name as the
-- subject (e.g. 'extraction'); attribution is the process's identity,
-- honestly weaker than a verified token subject, stronger than
-- break-glass. Hashed history is untouched — the constraint only
-- widens the vocabulary for new rows.

alter table audit_log drop constraint audit_log_actor_kind_check;
alter table audit_log add constraint audit_log_actor_kind_check
    check (actor_kind in ('subject', 'break_glass', 'system'));
