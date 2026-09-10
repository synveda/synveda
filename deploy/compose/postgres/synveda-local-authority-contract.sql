-- CPR-45: exact target-local pg_shdepend authority for the fixed reference
-- roles. The shared cluster contract binds dependencies to the allowed target
-- database and object class; this pass resolves each local object identity.
-- It is read-only and accepts only the Synveda or Keycloak target database.

with target_database as (
  select database.oid, database.datname, database.datdba
    from pg_catalog.pg_database database
   where database.datname = pg_catalog.current_database()
     and database.datname in ('synveda', 'keycloak')
), target_owner as (
  select database.oid as database_oid,
         database.datname,
         owner.oid as owner_oid
    from target_database database
    join pg_catalog.pg_roles owner on owner.oid = database.datdba
   where database.datname = 'synveda' and owner.rolname = 'synveda_migrator'
      or database.datname = 'keycloak' and owner.rolname = 'keycloak'
), protected_role as (
  select role.oid, role.rolname
    from pg_catalog.pg_roles role
   where role.rolname in (
     'synveda_app', 'synveda_migrator', 'synveda_gateway',
     'synveda_worker', 'keycloak'
   )
), allowed_dependency(dbid, classid, objid, objsubid, refobjid, deptype) as (
  select 0::oid,
         'pg_catalog.pg_database'::regclass::oid,
         target.database_oid,
         0::integer,
         target.owner_oid,
         'o'::"char"
    from target_owner target
  union
  select target.database_oid,
         'pg_catalog.pg_namespace'::regclass::oid,
         namespace.oid,
         0::integer,
         target.owner_oid,
         'o'::"char"
    from target_owner target
    join pg_catalog.pg_namespace namespace
      on namespace.nspname = 'public'
     and namespace.nspowner = target.owner_oid
  union
  select target.database_oid,
         'pg_catalog.pg_class'::regclass::oid,
         object.oid,
         0::integer,
         target.owner_oid,
         'o'::"char"
    from target_owner target
    join pg_catalog.pg_class object on object.relowner = target.owner_oid
    join pg_catalog.pg_namespace namespace on namespace.oid = object.relnamespace
   where namespace.nspname = 'public'
  union
  select target.database_oid,
         'pg_catalog.pg_proc'::regclass::oid,
         object.oid,
         0::integer,
         target.owner_oid,
         'o'::"char"
    from target_owner target
    join pg_catalog.pg_proc object on object.proowner = target.owner_oid
    join pg_catalog.pg_namespace namespace on namespace.oid = object.pronamespace
   where namespace.nspname = 'public'
  union
  select target.database_oid,
         'pg_catalog.pg_type'::regclass::oid,
         object.oid,
         0::integer,
         target.owner_oid,
         'o'::"char"
    from target_owner target
    join pg_catalog.pg_type object on object.typowner = target.owner_oid
    join pg_catalog.pg_namespace namespace on namespace.oid = object.typnamespace
   where namespace.nspname = 'public'
  union
  select target.database_oid,
         'pg_catalog.pg_namespace'::regclass::oid,
         namespace.oid,
         0::integer,
         app.oid,
         'a'::"char"
    from target_owner target
    join pg_catalog.pg_roles app on app.rolname = 'synveda_app'
    join pg_catalog.pg_namespace namespace
      on namespace.nspname = 'public'
     and namespace.nspowner = target.owner_oid
    cross join lateral pg_catalog.aclexplode(namespace.nspacl) acl
   where target.datname = 'synveda'
     and acl.grantor = target.owner_oid
     and acl.grantee = app.oid
     and acl.privilege_type = 'USAGE'
     and not acl.is_grantable
  union
  select target.database_oid,
         'pg_catalog.pg_class'::regclass::oid,
         object.oid,
         0::integer,
         app.oid,
         'a'::"char"
    from target_owner target
    join pg_catalog.pg_roles app on app.rolname = 'synveda_app'
    join pg_catalog.pg_class object
      on object.relowner = target.owner_oid
     and object.relkind in ('r', 'p', 'v', 'm')
    join pg_catalog.pg_namespace namespace on namespace.oid = object.relnamespace
    cross join lateral pg_catalog.aclexplode(object.relacl) acl
   where target.datname = 'synveda'
     and namespace.nspname = 'public'
     and acl.grantor = target.owner_oid
     and acl.grantee = app.oid
     and acl.privilege_type in ('SELECT', 'INSERT', 'UPDATE', 'DELETE')
     and not acl.is_grantable
  union
  select target.database_oid,
         'pg_catalog.pg_class'::regclass::oid,
         object.oid,
         attribute.attnum,
         app.oid,
         'a'::"char"
    from target_owner target
    join pg_catalog.pg_roles app on app.rolname = 'synveda_app'
    join pg_catalog.pg_class object
      on object.relowner = target.owner_oid
     and object.relkind in ('r', 'p')
    join pg_catalog.pg_namespace namespace on namespace.oid = object.relnamespace
    join pg_catalog.pg_attribute attribute
      on attribute.attrelid = object.oid
     and attribute.attnum > 0
     and not attribute.attisdropped
    cross join lateral pg_catalog.aclexplode(attribute.attacl) acl
   where target.datname = 'synveda'
     and namespace.nspname = 'public'
     and acl.grantor = target.owner_oid
     and acl.grantee = app.oid
     and acl.privilege_type = 'UPDATE'
     and not acl.is_grantable
  union
  select target.database_oid,
         'pg_catalog.pg_proc'::regclass::oid,
         object.oid,
         0::integer,
         app.oid,
         'a'::"char"
    from target_owner target
    join pg_catalog.pg_roles app on app.rolname = 'synveda_app'
    join pg_catalog.pg_proc object on object.proowner = target.owner_oid
    join pg_catalog.pg_namespace namespace on namespace.oid = object.pronamespace
    cross join lateral pg_catalog.aclexplode(object.proacl) acl
   where target.datname = 'synveda'
     and namespace.nspname = 'public'
     and acl.grantor = target.owner_oid
     and acl.grantee = app.oid
     and acl.privilege_type = 'EXECUTE'
     and not acl.is_grantable
  union
  select target.database_oid,
         'pg_catalog.pg_type'::regclass::oid,
         object.oid,
         0::integer,
         app.oid,
         'a'::"char"
    from target_owner target
    join pg_catalog.pg_roles app on app.rolname = 'synveda_app'
    join pg_catalog.pg_type object on object.typowner = target.owner_oid
    join pg_catalog.pg_namespace namespace on namespace.oid = object.typnamespace
    cross join lateral pg_catalog.aclexplode(object.typacl) acl
   where target.datname = 'synveda'
     and namespace.nspname = 'public'
     and acl.grantor = target.owner_oid
     and acl.grantee = app.oid
     and acl.privilege_type = 'USAGE'
     and not acl.is_grantable
  union
  select 0::oid,
         'pg_catalog.pg_database'::regclass::oid,
         target.database_oid,
         0::integer,
         runtime.oid,
         'a'::"char"
    from target_owner target
    join protected_role runtime
      on runtime.rolname in ('synveda_gateway', 'synveda_worker')
    cross join lateral pg_catalog.aclexplode(
      (select database.datacl
         from pg_catalog.pg_database database
        where database.oid = target.database_oid)
    ) acl
   where target.datname = 'synveda'
     and acl.grantor = target.owner_oid
     and acl.grantee = runtime.oid
     and acl.privilege_type = 'CONNECT'
     and not acl.is_grantable
)
select 1 / case when (select count(*) from target_owner) = 1
  and not exists (
    select 1
      from pg_catalog.pg_shdepend dependency
      join protected_role role on role.oid = dependency.refobjid
      cross join target_database target
     where dependency.refclassid = 'pg_catalog.pg_authid'::regclass
       and (
         dependency.dbid = target.oid
         or dependency.dbid = 0
         and dependency.classid = 'pg_catalog.pg_database'::regclass
         and dependency.objid = target.oid
       )
       and not exists (
         select 1
           from allowed_dependency allowed
          where allowed.dbid = dependency.dbid
            and allowed.classid = dependency.classid
            and allowed.objid = dependency.objid
            and allowed.objsubid = dependency.objsubid
            and allowed.refobjid = dependency.refobjid
            and allowed.deptype = dependency.deptype
       )
  ) then 1 else 0 end;
