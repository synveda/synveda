-- CPR-45: exact PostgreSQL 17 extension authority accepted by epoch 3.
-- This file is executed only through the audited database-bootstrap client.
-- It contains no deployment input and mutates no catalog state.

set statement_timeout = '10s';

\if :{?synveda_require_complete_extensions}
\else
\set synveda_require_complete_extensions false
\endif
with trusted_extension_owners(role_name) as (
  select session_user
  union
  select grantor.rolname
    from pg_catalog.pg_auth_members membership
    join pg_catalog.pg_roles protected on protected.oid = membership.roleid
    join pg_catalog.pg_roles member on member.oid = membership.member
    join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
   where protected.rolname in (
     'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
   )
     and member.rolname = session_user
     and membership.admin_option
     and not membership.inherit_option
     and not membership.set_option
   group by grantor.rolname
  having count(distinct protected.rolname) = (
    select count(*)
      from pg_catalog.pg_roles role
     where role.rolname in (
       'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
     )
  ) and count(distinct protected.rolname) between 4 and 5
)
select 1 / case when (
  select count(*)
    from pg_catalog.pg_extension extension
   where extension.extname in ('plpgsql', 'btree_gin', 'vector')
) between 1 and 3 and (
  :'synveda_require_complete_extensions' <> 'true'
  or (
    select count(*)
      from pg_catalog.pg_extension extension
     where extension.extname in ('plpgsql', 'btree_gin', 'vector')
  ) = 3
) and not exists (
  select 1
    from pg_catalog.pg_extension extension
   where extension.extname not in ('plpgsql', 'btree_gin', 'vector')
) and exists (
  select 1
    from pg_catalog.pg_extension extension
    join pg_catalog.pg_namespace namespace on namespace.oid = extension.extnamespace
    join pg_catalog.pg_roles owner on owner.oid = extension.extowner
   where extension.extname = 'plpgsql'
     and extension.extversion = '1.0'
     and namespace.nspname = 'pg_catalog'
     and not extension.extrelocatable
     and extension.extconfig is null
     and extension.extcondition is null
     and owner.rolname in (select trusted.role_name from trusted_extension_owners trusted)
) and (
  not exists (
    select 1
      from pg_catalog.pg_extension extension
     where extension.extname = 'btree_gin'
  ) or exists (
    select 1
      from pg_catalog.pg_extension extension
      join pg_catalog.pg_namespace namespace on namespace.oid = extension.extnamespace
      join pg_catalog.pg_roles owner on owner.oid = extension.extowner
     where extension.extname = 'btree_gin'
       and extension.extversion = '1.3'
       and namespace.nspname = 'public'
       and owner.rolname = session_user
  )
) and (
  not exists (
    select 1
      from pg_catalog.pg_extension extension
     where extension.extname = 'vector'
  ) or exists (
    select 1
      from pg_catalog.pg_extension extension
      join pg_catalog.pg_namespace namespace on namespace.oid = extension.extnamespace
      join pg_catalog.pg_roles owner on owner.oid = extension.extowner
     where extension.extname = 'vector'
       and extension.extversion = '0.8.6'
       and namespace.nspname = 'public'
       and owner.rolname = session_user
  )
) and not exists (
  with expected_all(extension_name, member_class, member_count) as (
    values
      ('btree_gin', 'pg_opclass', 29::bigint),
      ('btree_gin', 'pg_opfamily', 29::bigint),
      ('btree_gin', 'pg_proc', 87::bigint),
      ('vector', 'pg_am', 2::bigint),
      ('vector', 'pg_cast', 23::bigint),
      ('vector', 'pg_opclass', 24::bigint),
      ('vector', 'pg_operator', 40::bigint),
      ('vector', 'pg_opfamily', 24::bigint),
      ('vector', 'pg_proc', 118::bigint),
      ('vector', 'pg_type', 6::bigint)
  ), expected as (
    select expected_all.*
      from expected_all
      join pg_catalog.pg_extension extension
        on extension.extname = expected_all.extension_name
  ), actual as (
    select extension.extname as extension_name,
           case
             when dependency.classid = 'pg_catalog.pg_am'::regclass then 'pg_am'
             when dependency.classid = 'pg_catalog.pg_cast'::regclass then 'pg_cast'
             when dependency.classid = 'pg_catalog.pg_opclass'::regclass then 'pg_opclass'
             when dependency.classid = 'pg_catalog.pg_operator'::regclass then 'pg_operator'
             when dependency.classid = 'pg_catalog.pg_opfamily'::regclass then 'pg_opfamily'
             when dependency.classid = 'pg_catalog.pg_proc'::regclass then 'pg_proc'
             when dependency.classid = 'pg_catalog.pg_type'::regclass then 'pg_type'
             else dependency.classid::regclass::text
           end as member_class,
           count(*) as member_count
      from pg_catalog.pg_depend dependency
      join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
     where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
       and dependency.deptype = 'e'
       and extension.extname in ('btree_gin', 'vector')
     group by extension.extname, dependency.classid
  )
  select 1
    from expected
    full join actual using (extension_name, member_class)
   where expected.member_count is distinct from actual.member_count
) and not exists (
  select 1
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
    join pg_catalog.pg_proc object on object.oid = dependency.objid
   where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.classid = 'pg_catalog.pg_proc'::regclass
     and dependency.deptype = 'e'
     and extension.extname in ('btree_gin', 'vector')
     and (
       object.proowner <> extension.extowner
       or object.prosecdef
       or object.proleakproof
       or object.proconfig is not null
       or object.proacl is not null
     )
  union all
  select 1
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
    join pg_catalog.pg_type object on object.oid = dependency.objid
   where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.classid = 'pg_catalog.pg_type'::regclass
     and dependency.deptype = 'e'
     and extension.extname in ('btree_gin', 'vector')
     and (object.typowner <> extension.extowner or object.typacl is not null)
  union all
  select 1
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
    join pg_catalog.pg_operator object on object.oid = dependency.objid
   where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.classid = 'pg_catalog.pg_operator'::regclass
     and dependency.deptype = 'e'
     and extension.extname in ('btree_gin', 'vector')
     and object.oprowner <> extension.extowner
  union all
  select 1
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
    join pg_catalog.pg_opclass object on object.oid = dependency.objid
   where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.classid = 'pg_catalog.pg_opclass'::regclass
     and dependency.deptype = 'e'
     and extension.extname in ('btree_gin', 'vector')
     and object.opcowner <> extension.extowner
  union all
  select 1
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
    join pg_catalog.pg_opfamily object on object.oid = dependency.objid
   where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.classid = 'pg_catalog.pg_opfamily'::regclass
     and dependency.deptype = 'e'
     and extension.extname in ('btree_gin', 'vector')
     and object.opfowner <> extension.extowner
  union all
  select 1
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
   where extension.extname = 'plpgsql'
     and dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.deptype = 'e'
     and not (
       dependency.classid = 'pg_catalog.pg_language'::regclass
       or dependency.classid = 'pg_catalog.pg_proc'::regclass
     )
) and (
  select count(*)
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
   where extension.extname = 'plpgsql'
     and dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.deptype = 'e'
     and dependency.classid = 'pg_catalog.pg_language'::regclass
) = 1 and (
  select count(*)
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
   where extension.extname = 'plpgsql'
     and dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.deptype = 'e'
     and dependency.classid = 'pg_catalog.pg_proc'::regclass
) = 3 and exists (
  select 1
    from pg_catalog.pg_language language
    join pg_catalog.pg_roles owner on owner.oid = language.lanowner
   where language.lanname = 'plpgsql'
     and language.lanispl
     and language.lanpltrusted
     and language.lanplcallfoid = 'pg_catalog.plpgsql_call_handler()'::regprocedure
     and language.laninline = 'pg_catalog.plpgsql_inline_handler(internal)'::regprocedure
     and language.lanvalidator = 'pg_catalog.plpgsql_validator(oid)'::regprocedure
     and language.lanacl is null
     and owner.rolname in (select trusted.role_name from trusted_extension_owners trusted)
) and not exists (
  select 1
    from pg_catalog.pg_depend dependency
    join pg_catalog.pg_extension extension on extension.oid = dependency.refobjid
    join pg_catalog.pg_proc routine on routine.oid = dependency.objid
    join pg_catalog.pg_roles owner on owner.oid = routine.proowner
   where extension.extname = 'plpgsql'
     and dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.classid = 'pg_catalog.pg_proc'::regclass
     and dependency.deptype = 'e'
     and (
       routine.proname not in (
         'plpgsql_call_handler', 'plpgsql_inline_handler', 'plpgsql_validator'
       )
       or routine.prosecdef
       or routine.proleakproof
       or routine.proconfig is not null
       or routine.proacl is not null
       or routine.probin <> '$libdir/plpgsql'
       or routine.prosrc <> routine.proname
       or owner.rolname not in (
         select trusted.role_name from trusted_extension_owners trusted
       )
     )
) then 1 else 0 end;
set search_path = pg_catalog, public;
\i /usr/local/share/synveda/extension-fingerprint-assert.psql
\if :synveda_extension_safe
\else
\warn 'database-bootstrap: exact extension fingerprint was refused'
select 1 / 0;
\endif
set statement_timeout = '120s';
