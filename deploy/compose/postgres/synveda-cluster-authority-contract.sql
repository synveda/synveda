-- CPR-45: shared-cluster authority allowed for the five fixed reference roles.
-- Execute from the bootstrap database before either branch's persistent
-- mutation boundary. Everything not explicitly proved below is refused.

-- A PostgreSQL backend is absent from pg_stat_activity until after it has
-- checked database admission. PostgreSQL 17 holds this exact shared-object
-- lock across that hidden startup interval and publishes activity before
-- releasing it. Preserve the one-way lock-to-activity handoff as ordered
-- statements; a combined Boolean expression could be reordered by the
-- planner and miss the transition.
select coalesce((
  select database.oid::bigint
    from pg_catalog.pg_database database
   where database.datname = 'keycloak'
), 0) as synveda_keycloak_database_oid
\gset
select (not exists (
  select 1
    from pg_catalog.pg_locks lock
   where lock.locktype = 'object'
     and lock.database = 0
     and lock.classid = 'pg_catalog.pg_database'::pg_catalog.regclass
     and lock.objid = :'synveda_keycloak_database_oid'::pg_catalog.oid
     and lock.objsubid = 0
     and lock.mode = 'RowExclusiveLock'
     and lock.pid is not null
))::text as synveda_keycloak_no_startup_locks
\gset
select pg_catalog.pg_stat_clear_snapshot()
\g /dev/null
select (not exists (
  select 1
    from pg_catalog.pg_stat_activity activity
   where activity.datid = :'synveda_keycloak_database_oid'::pg_catalog.oid
))::text as synveda_keycloak_no_activity
\gset
select (not exists (
  select 1
    from pg_catalog.pg_prepared_xacts prepared
    join pg_catalog.pg_database database on database.datname = prepared.database
   where database.oid = :'synveda_keycloak_database_oid'::pg_catalog.oid
))::text as synveda_keycloak_no_prepared_xacts
\gset

-- PostgreSQL 17 represents provider role-management authority as one row per
-- grantor. The deployment contract names that provenance explicitly: every
-- existing protected role must have every configured ADMIN-only pair, and no
-- other ADMIN row may exist. Bundled bootstrap is superuser-owned and has no
-- such rows; external bootstrap is proof-only and may not infer them.
with contract as (
  select document
    from pg_temp.synveda_database_roles
), raw_expected as (
  select membership.value
    from contract,
         lateral pg_catalog.jsonb_array_elements(
           contract.document->'administrative_memberships'
         ) membership(value)
), expected as (
  select value->>'member' as member_name,
         value->>'grantor' as grantor_name
    from raw_expected
), protected as (
  select role.oid, role.rolname
    from pg_catalog.pg_roles role
   where role.rolname in (
     'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
   )
), principal as (
  select role.rolsuper
    from pg_catalog.pg_roles role
   where role.rolname = session_user
)
select 1 / case when
  (select count(*) from contract) = 1
  and (select count(*) from raw_expected) <= 8
  and not exists (
    select 1
      from raw_expected
     where pg_catalog.jsonb_typeof(value) <> 'object'
        or (select count(*) from pg_catalog.jsonb_object_keys(value)) <> 2
        or pg_catalog.jsonb_typeof(value->'member') <> 'string'
        or pg_catalog.jsonb_typeof(value->'grantor') <> 'string'
        or pg_catalog.octet_length(value->>'member') not between 1 and 63
        or pg_catalog.octet_length(value->>'grantor') not between 1 and 63
  )
  and (
    select count(*) = count(distinct (member_name, grantor_name))
      from expected
  )
  and not exists (
    select 1
      from expected
     where member_name <> session_user
        or member_name = grantor_name
        or grantor_name in (
          'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
        )
        or not exists (
          select 1 from pg_catalog.pg_roles grantor
           where grantor.rolname = expected.grantor_name
        )
  )
  and exists (
    select 1
      from principal
     where (
       :'bundled_cluster' = 'true'
       and principal.rolsuper
       and (select count(*) from expected) = 0
     ) or (
       :'bundled_cluster' <> 'true'
       and not principal.rolsuper
       and (select count(*) from expected) between 1 and 8
     )
  )
  and not exists (
    select 1
      from protected
      cross join expected
     where not exists (
       select 1
         from pg_catalog.pg_auth_members membership
         join pg_catalog.pg_roles member on member.oid = membership.member
         join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
        where membership.roleid = protected.oid
          and member.rolname = expected.member_name
          and grantor.rolname = expected.grantor_name
          and membership.admin_option
          and not membership.inherit_option
          and not membership.set_option
     )
  )
  and not exists (
    select 1
      from pg_catalog.pg_auth_members membership
      join protected on protected.oid = membership.roleid
      join pg_catalog.pg_roles member on member.oid = membership.member
      join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
     where membership.admin_option
       and not exists (
         select 1
           from expected
          where expected.member_name = member.rolname
            and expected.grantor_name = grantor.rolname
            and not membership.inherit_option
            and not membership.set_option
       )
  )
then 1 else 0 end;

-- Every existing protected role has one bounded shape. The login roles are
-- permitted to be NOLOGIN while the global phase is converging or LOGIN after
-- the target transaction has installed credentials. PostgreSQL represents an
-- unbounded login validity as either NULL or the explicit infinity used by
-- this bootstrap. Role settings and every membership edge are default-deny.
-- Recovery and target-transaction preflight may explicitly permit the
-- bootstrap principal's exact non-admin INHERIT+SET grant into only the named
-- target owner. Complete global preflight refuses it. A protected role may not
-- appear only as a grantor either.
with contract as (
  select document
    from pg_temp.synveda_database_roles
), expected_admin as (
  select membership.value->>'member' as member_name,
         membership.value->>'grantor' as grantor_name
    from contract,
         lateral pg_catalog.jsonb_array_elements(
           contract.document->'administrative_memberships'
         ) membership(value)
), protected as (
  select role.*
    from pg_catalog.pg_roles role
   where role.rolname in (
     'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
   )
), bootstrap_principal as (
  select role.oid, role.rolsuper
    from pg_catalog.pg_roles role
   where role.rolname = session_user
), runtime_memberships as (
  select membership.*
    from pg_catalog.pg_auth_members membership
    join pg_catalog.pg_roles granted on granted.oid = membership.roleid
    join pg_catalog.pg_roles member on member.oid = membership.member
    join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
   where granted.rolname = 'synveda_app'
     and member.rolname in ('synveda_gateway', 'synveda_worker')
     and grantor.rolname = session_user
     and not membership.admin_option
     and membership.inherit_option
     and membership.set_option
)
select 1 / case when not exists (
  select 1
    from protected role
   where not role.rolinherit
      or role.rolsuper
      or role.rolcreatedb
      or role.rolcreaterole
      or role.rolreplication
      or role.rolbypassrls
      or role.rolconnlimit <> -1
      or role.rolname = 'synveda_app' and (
        role.rolcanlogin or role.rolvaliduntil is not null
      )
      or role.rolname <> 'synveda_app' and role.rolvaliduntil is not null
         and role.rolvaliduntil is distinct from 'infinity'::timestamptz
      or :'synveda_require_complete_roles' = 'true'
         and :'synveda_bootstrap_target' = 'synveda'
         and role.rolname in ('synveda_migrator', 'synveda_gateway', 'synveda_worker')
         and (
           role.rolcanlogin
           or role.rolvaliduntil is distinct from 'infinity'::timestamptz
         )
      or :'synveda_require_complete_roles' = 'true'
         and :'synveda_bootstrap_target' = 'keycloak'
         and role.rolname = 'keycloak'
         and (
           role.rolcanlogin
           or role.rolvaliduntil is distinct from 'infinity'::timestamptz
         )
) and not exists (
  select 1
    from pg_catalog.pg_db_role_setting setting
   where setting.setrole in (select role.oid from protected role)
      or setting.setrole = 0
         and setting.setdatabase in (
           select database.oid
             from pg_catalog.pg_database database
            where database.datname in ('synveda', 'keycloak', 'postgres', 'template1')
         )
      or setting.setrole = (select principal.oid from bootstrap_principal principal)
         and setting.setdatabase in (
           select database.oid
             from pg_catalog.pg_database database
            where database.datname in ('synveda', 'keycloak')
         )
) and not exists (
  select 1
    from pg_catalog.pg_auth_members membership
    join pg_catalog.pg_roles granted on granted.oid = membership.roleid
    join pg_catalog.pg_roles member on member.oid = membership.member
    join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
   where (
     granted.rolname in (
       'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
     ) or member.rolname in (
       'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
     ) or grantor.rolname in (
       'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
     )
   ) and not (
     granted.rolname in (
       'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
     )
     and membership.admin_option
     and not membership.inherit_option
     and not membership.set_option
     and exists (
       select 1
         from expected_admin expected
        where expected.member_name = member.rolname
          and expected.grantor_name = grantor.rolname
     )
     or granted.rolname = 'synveda_app'
     and member.rolname in ('synveda_gateway', 'synveda_worker')
     and grantor.rolname = session_user
     and not membership.admin_option
     and membership.inherit_option
     and membership.set_option
    or :'synveda_allow_target_owner_membership' = 'true'
     and (
       :'synveda_bootstrap_target' = 'synveda'
       and granted.rolname = 'synveda_migrator'
       or :'synveda_bootstrap_target' = 'keycloak'
       and granted.rolname = 'keycloak'
       or :'synveda_bootstrap_target' = 'synveda'
       and granted.rolname = 'keycloak'
     )
     and member.rolname = session_user
     and grantor.rolname = session_user
     and not membership.admin_option
     and membership.inherit_option
     and membership.set_option
   )
) and not exists (
  select 1
    from pg_catalog.pg_auth_members membership
    join pg_catalog.pg_roles granted on granted.oid = membership.roleid
    join pg_catalog.pg_roles member on member.oid = membership.member
    join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
   where member.rolname = session_user
     and granted.rolname not in (
       'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
     )
     and not (
       :'bundled_cluster' <> 'true'
       and not (select principal.rolsuper from bootstrap_principal principal)
       and granted.rolname = 'pg_read_all_settings'
       and not membership.admin_option
       and membership.inherit_option
       and not membership.set_option
       and exists (
         select 1
           from expected_admin expected
          where expected.grantor_name = grantor.rolname
       )
     )
) and (
  :'bundled_cluster' = 'true'
  or (
    not (select principal.rolsuper from bootstrap_principal principal)
    and (
      select count(*)
        from pg_catalog.pg_auth_members membership
        join pg_catalog.pg_roles granted on granted.oid = membership.roleid
        join pg_catalog.pg_roles member on member.oid = membership.member
        join pg_catalog.pg_roles grantor on grantor.oid = membership.grantor
       where granted.rolname = 'pg_read_all_settings'
         and member.rolname = session_user
         and not membership.admin_option
         and membership.inherit_option
         and not membership.set_option
         and exists (
           select 1
             from expected_admin expected
            where expected.grantor_name = grantor.rolname
         )
    ) = 1
  )
) and :'synveda_bootstrap_target' in ('synveda', 'keycloak')
  and :'synveda_require_complete_roles' in ('true', 'false')
  and :'synveda_allow_target_owner_membership' in ('true', 'false')
  and :'synveda_allow_target_default_acl' in ('true', 'false')
  and (
    :'synveda_allow_target_default_acl' = 'false'
    or :'synveda_require_complete_roles' = 'false'
  )
  and (
    :'synveda_require_complete_roles' = 'false'
    and (select count(*) from runtime_memberships) between 0 and 2
    or :'synveda_require_complete_roles' = 'true'
    and (
      :'synveda_bootstrap_target' = 'synveda'
      and (
        select count(*)
          from protected role
         where role.rolname in (
           'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker'
         )
      ) = 4
      or :'synveda_bootstrap_target' = 'keycloak'
      and (select count(*) from protected role) = 5
    )
    and (
      select count(*)
        from protected role
       where role.rolname in ('synveda_app', 'synveda_gateway', 'synveda_worker')
    ) = 3
    and (select count(*) from runtime_memberships) = 2
  )
then 1 else 0 end;

-- Protected roles may own only their named database and may not own a
-- tablespace. The topology contract names a bounded set of peer/maintenance
-- databases for the four Synveda roles; effective CONNECT is checked so PUBLIC
-- cannot silently reopen those boundaries. The paired Keycloak boundary is
-- symmetric. The exact maintenance database used for bootstrap is mandatory
-- in this set. `keycloak` alone may be absent while the ordered reference
-- bootstrap is still creating that peer. Synveda convergence may also observe
-- the one exact crash state produced by CREATE DATABASE: a template0-shaped,
-- connection-closed Keycloak database with its default ACL and no sessions.
-- Both ordered bootstrap phases may observe that closed state; the Keycloak
-- phase alone owns publication of its terminal ACL. Final product
-- preflight still requires every configured peer to be open and isolated.
with contract as (
  select document
    from pg_temp.synveda_database_roles
), raw_forbidden as (
  select database.value
    from contract,
         lateral pg_catalog.jsonb_array_elements(
           contract.document->'forbidden_databases'
         ) database(value)
), forbidden as (
  select value #>> '{}' as database_name
    from raw_forbidden
), synveda_roles as (
  select role.oid
    from pg_catalog.pg_roles role
   where role.rolname in (
     'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker'
   )
), template as (
  select database.*
    from pg_catalog.pg_database database
   where database.datname = 'template0'
)
select 1 / case when (
  select count(*) from forbidden
) between 1 and 8 and (select count(*) from template) = 1 and (
  select count(*) = count(distinct database_name) from forbidden
) and not exists (
  select 1
    from raw_forbidden
   where pg_catalog.jsonb_typeof(value) <> 'string'
) and (
  :'synveda_bootstrap_target' <> 'keycloak'
  or exists (
    select 1
      from forbidden
     where database_name = 'keycloak'
  )
) and pg_catalog.octet_length(:'synveda_bootstrap_database') between 1 and 63
  and :'synveda_bootstrap_database' not in ('synveda', 'keycloak')
  and exists (
    select 1
      from forbidden
     where database_name = :'synveda_bootstrap_database'
  )
  and exists (
    select 1
      from pg_catalog.pg_database database
     where database.datname = :'synveda_bootstrap_database'
) and not exists (
  select 1
    from forbidden
   where pg_catalog.octet_length(database_name) not between 1 and 63
      or database_name = 'synveda'
) and not exists (
  select 1
    from forbidden
   where database_name <> 'keycloak'
     and not exists (
       select 1
         from pg_catalog.pg_database database
        where database.datname = forbidden.database_name
     )
) and not exists (
  select 1
    from pg_catalog.pg_database database
    join pg_catalog.pg_roles owner on owner.oid = database.datdba
   where owner.rolname in (
     'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
   ) and not (
     owner.rolname = 'synveda_migrator' and database.datname = 'synveda'
     or owner.rolname = 'keycloak' and database.datname = 'keycloak'
   )
  union all
  select 1
    from pg_catalog.pg_tablespace tablespace
    join pg_catalog.pg_roles owner on owner.oid = tablespace.spcowner
   where owner.rolname in (
     'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
   )
) and not exists (
  select 1
    from forbidden
    join pg_catalog.pg_database database
      on database.datname = forbidden.database_name
    cross join synveda_roles role
   where pg_catalog.has_database_privilege(role.oid, database.oid, 'CONNECT')
     and not (
       (
         :'synveda_allow_target_default_acl' = 'true'
       and forbidden.database_name = :'synveda_bootstrap_target'
       and database.datacl is null
       and database.datallowconn
       and not database.datistemplate
       and not database.dathasloginevt
       and database.datconnlimit = -1
       and database.encoding = pg_catalog.pg_char_to_encoding('UTF8')
       and database.datdba is not distinct from case :'synveda_bootstrap_target'
         when 'synveda' then (
           select owner.oid
             from pg_catalog.pg_roles owner
            where owner.rolname = 'synveda_migrator'
         )
         when 'keycloak' then (
           select owner.oid
             from pg_catalog.pg_roles owner
            where owner.rolname = 'keycloak'
         )
         end
       ) or (
       :'synveda_bootstrap_target' in ('synveda', 'keycloak')
       and forbidden.database_name = 'keycloak'
       and database.datacl is null
       and not database.datallowconn
       and not database.datistemplate
       and not database.dathasloginevt
       and database.datconnlimit = -1
       and database.encoding = pg_catalog.pg_char_to_encoding('UTF8')
       and database.datlocprovider = (select template.datlocprovider from template)
       and database.datcollate = (select template.datcollate from template)
       and database.datctype = (select template.datctype from template)
       and database.datlocale is not distinct from
           (select template.datlocale from template)
       and database.daticurules is not distinct from
           (select template.daticurules from template)
       and database.datcollversion is not distinct from
           pg_catalog.pg_database_collation_actual_version(database.oid)
       and database.dattablespace = (select template.dattablespace from template)
       and database.datdba is not distinct from (
         select owner.oid
           from pg_catalog.pg_roles owner
          where owner.rolname = 'keycloak'
       )
       and not exists (
         select 1
           from pg_catalog.pg_db_role_setting settings
          where settings.setdatabase = database.oid
       )
       and :'synveda_keycloak_no_startup_locks' = 'true'
       and :'synveda_keycloak_no_activity' = 'true'
       and :'synveda_keycloak_no_prepared_xacts' = 'true'
       )
     )
) and not exists (
  select 1
    from forbidden
    join pg_catalog.pg_database database
      on database.datname = forbidden.database_name
    cross join pg_catalog.pg_roles role
   where forbidden.database_name <> 'keycloak'
     and role.rolname = 'keycloak'
     and pg_catalog.has_database_privilege(role.oid, database.oid, 'CONNECT')
) and not exists (
  select 1
   from pg_catalog.pg_database database
    cross join pg_catalog.pg_roles role
   where database.datname = 'synveda'
     and role.rolname = 'keycloak'
     and pg_catalog.has_database_privilege(role.oid, database.oid, 'CONNECT')
     and not (
       :'synveda_allow_target_default_acl' = 'true'
       and :'synveda_bootstrap_target' = 'synveda'
       and database.datacl is null
       and database.datallowconn
       and not database.datistemplate
       and not database.dathasloginevt
       and database.datconnlimit = -1
       and database.encoding = pg_catalog.pg_char_to_encoding('UTF8')
       and database.datdba is not distinct from (
         select owner.oid
           from pg_catalog.pg_roles owner
          where owner.rolname = 'synveda_migrator'
       )
     )
) then 1 else 0 end;

-- No protected role or PUBLIC principal may gain a cluster-global capability.
-- PL/pgSQL's built-in PUBLIC language ACL is the sole PUBLIC exception and is
-- not represented here because language rows reject protected grantees only.
select 1 / case when not exists (
  select 1
    from pg_catalog.pg_largeobject_metadata object,
         lateral pg_catalog.aclexplode(object.lomacl) acl
   where acl.grantee = 0
      or acl.grantee in (
        select role.oid from pg_catalog.pg_roles role
         where role.rolname in (
           'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
         )
      )
  union all
  select 1
    from pg_catalog.pg_foreign_data_wrapper object,
         lateral pg_catalog.aclexplode(object.fdwacl) acl
   where acl.grantee = 0
      or acl.grantee in (
        select role.oid from pg_catalog.pg_roles role
         where role.rolname in (
           'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
         )
      )
  union all
  select 1
    from pg_catalog.pg_foreign_server object,
         lateral pg_catalog.aclexplode(object.srvacl) acl
   where acl.grantee = 0
      or acl.grantee in (
        select role.oid from pg_catalog.pg_roles role
         where role.rolname in (
           'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
         )
      )
  union all
  select 1
    from pg_catalog.pg_language object,
         lateral pg_catalog.aclexplode(object.lanacl) acl
   where acl.grantee in (
     select role.oid from pg_catalog.pg_roles role
      where role.rolname in (
        'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
      )
   )
  union all
  select 1
    from pg_catalog.pg_tablespace object,
         lateral pg_catalog.aclexplode(object.spcacl) acl
   where acl.grantee = 0
      or acl.grantee in (
        select role.oid from pg_catalog.pg_roles role
         where role.rolname in (
           'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
         )
      )
  union all
  select 1
    from pg_catalog.pg_parameter_acl object,
         lateral pg_catalog.aclexplode(object.paracl) acl
   where acl.grantee = 0
      or acl.grantee in (
        select role.oid from pg_catalog.pg_roles role
         where role.rolname in (
           'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
         )
      )
  union all
  select 1
    from pg_catalog.pg_default_acl defaults
   where defaults.defaclrole in (
     select role.oid from pg_catalog.pg_roles role
      where role.rolname in (
        'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
      )
   ) or exists (
     select 1
       from pg_catalog.aclexplode(defaults.defaclacl) acl
      where acl.grantee in (
        select role.oid from pg_catalog.pg_roles role
         where role.rolname in (
           'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
         )
      )
   )
) then 1 else 0 end;

-- pg_shdepend is default-deny, including INITACL and future dependency kinds.
-- Target-local object identities are checked by the branch's locked local
-- preflight; this shared-catalog pass binds every dependency to the one allowed
-- database and object class.
select 1 / case when not exists (
  select 1
    from pg_catalog.pg_shdepend dependency
    join pg_catalog.pg_roles role on role.oid = dependency.refobjid
    left join pg_catalog.pg_database synveda_database
      on synveda_database.datname = 'synveda'
    left join pg_catalog.pg_database keycloak_database
      on keycloak_database.datname = 'keycloak'
   where dependency.refclassid = 'pg_catalog.pg_authid'::regclass
     and role.rolname in (
       'synveda_app', 'synveda_migrator', 'synveda_gateway', 'synveda_worker', 'keycloak'
     )
     and not (
       role.rolname = 'synveda_migrator'
       and dependency.deptype = 'o'
       and dependency.objsubid = 0
       and synveda_database.oid is not null
       and (
         dependency.dbid = 0
         and dependency.classid = 'pg_catalog.pg_database'::regclass
         and dependency.objid = synveda_database.oid
         or dependency.dbid = synveda_database.oid
         and dependency.classid in (
           'pg_catalog.pg_namespace'::regclass,
           'pg_catalog.pg_class'::regclass,
           'pg_catalog.pg_proc'::regclass,
           'pg_catalog.pg_type'::regclass
         )
       )
       or role.rolname = 'synveda_app'
       and dependency.deptype = 'a'
       and synveda_database.oid is not null
       and dependency.dbid = synveda_database.oid
       and (
         dependency.classid = 'pg_catalog.pg_class'::regclass
         and dependency.objsubid >= 0
         or dependency.objsubid = 0
         and dependency.classid in (
           'pg_catalog.pg_namespace'::regclass,
           'pg_catalog.pg_proc'::regclass,
           'pg_catalog.pg_type'::regclass
         )
       )
       or role.rolname in ('synveda_gateway', 'synveda_worker')
       and dependency.deptype = 'a'
       and synveda_database.oid is not null
       and dependency.dbid = 0
       and dependency.classid = 'pg_catalog.pg_database'::regclass
       and dependency.objid = synveda_database.oid
       and dependency.objsubid = 0
       and exists (
         select 1
           from pg_catalog.aclexplode(synveda_database.datacl) acl
          where acl.grantee = role.oid
            and acl.grantor = synveda_database.datdba
            and acl.privilege_type = 'CONNECT'
            and not acl.is_grantable
       )
       or role.rolname = 'keycloak'
       and dependency.deptype = 'o'
       and dependency.objsubid = 0
       and keycloak_database.oid is not null
       and (
         dependency.dbid = 0
         and dependency.classid = 'pg_catalog.pg_database'::regclass
         and dependency.objid = keycloak_database.oid
         or dependency.dbid = keycloak_database.oid
         and dependency.classid in (
           'pg_catalog.pg_namespace'::regclass,
           'pg_catalog.pg_class'::regclass,
           'pg_catalog.pg_proc'::regclass,
           'pg_catalog.pg_type'::regclass
         )
       )
     )
) then 1 else 0 end;
