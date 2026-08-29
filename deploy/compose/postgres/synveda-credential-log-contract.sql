-- CPR-45: suppress every PostgreSQL 17 standard logger path while database
-- credentials are copied and converted to SCRAM verifiers. The cluster
-- preflight proves SET authority before persistent mutation. This file changes
-- only the short-lived bootstrap session and contains no deployment input.
--
-- log_transaction_sample_rate must be zero before BEGIN because PostgreSQL
-- chooses the sampled-transaction flag at transaction start. Reapplying the
-- complete envelope immediately before COPY also defeats changes made by an
-- extension installation inside the transaction.
set log_min_messages = 'panic';
set log_min_error_statement = 'panic';
set log_error_verbosity = 'terse';
set log_statement = 'none';
set log_min_duration_statement = -1;
set log_min_duration_sample = -1;
set log_statement_sample_rate = 0;
set log_transaction_sample_rate = 0;
set log_parameter_max_length = 0;
set log_parameter_max_length_on_error = 0;
set debug_print_parse = off;
set debug_print_rewritten = off;
set debug_print_plan = off;

select 1 / case when current_setting('log_min_messages') = 'panic'
  and current_user = session_user
  and current_setting('role') = 'none'
  and current_setting('log_min_error_statement') = 'panic'
  and current_setting('log_error_verbosity') = 'terse'
  and current_setting('log_statement') = 'none'
  and current_setting('log_min_duration_statement') = '-1'
  and current_setting('log_min_duration_sample') = '-1'
  and current_setting('log_statement_sample_rate') = '0'
  and current_setting('log_transaction_sample_rate') = '0'
  and current_setting('log_parameter_max_length') = '0'
  and current_setting('log_parameter_max_length_on_error') = '0'
  and current_setting('debug_print_parse') = 'off'
  and current_setting('debug_print_rewritten') = 'off'
  and current_setting('debug_print_plan') = 'off'
  and current_setting('shared_preload_libraries') = ''
  and current_setting('session_preload_libraries') = ''
  and current_setting('local_preload_libraries') = ''
  and current_setting('default_table_access_method') = 'heap'
  and current_setting('client_encoding') = 'UTF8'
  and current_setting('jit') = 'off'
then 1 else 0 end;
