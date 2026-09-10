with extension_state as materialized (
  select extension.oid,
         extension.extowner as owner_oid,
         extension.extname,
         jsonb_build_array(
           extension.extname,
           extension.extversion,
           namespace.nspname,
           extension.extrelocatable,
           extension.extconfig,
           extension.extcondition
         ) as metadata
    from pg_catalog.pg_extension extension
    join pg_catalog.pg_namespace namespace on namespace.oid = extension.extnamespace
   where extension.extname in ('plpgsql', 'btree_gin', 'vector')
   limit 4
), extension_dependencies as materialized (
  select dependency.classid,
         dependency.objid,
         dependency.objsubid,
         dependency.refclassid,
         dependency.refobjid,
         dependency.deptype
    from pg_catalog.pg_depend dependency
    join extension_state extension on extension.oid = dependency.refobjid
   where dependency.refclassid = 'pg_catalog.pg_extension'::regclass
     and dependency.deptype = 'e'
   limit 387
), extension_members as materialized (
  select extension.extname,
         case
           when dependency.classid = 'pg_catalog.pg_proc'::regclass then 'pg_catalog.pg_proc'
           when dependency.classid = 'pg_catalog.pg_type'::regclass then 'pg_catalog.pg_type'
           when dependency.classid = 'pg_catalog.pg_operator'::regclass then 'pg_catalog.pg_operator'
           when dependency.classid = 'pg_catalog.pg_opclass'::regclass then 'pg_catalog.pg_opclass'
           when dependency.classid = 'pg_catalog.pg_opfamily'::regclass then 'pg_catalog.pg_opfamily'
           when dependency.classid = 'pg_catalog.pg_cast'::regclass then 'pg_catalog.pg_cast'
           when dependency.classid = 'pg_catalog.pg_am'::regclass then 'pg_catalog.pg_am'
           when dependency.classid = 'pg_catalog.pg_language'::regclass then 'pg_catalog.pg_language'
           else 'unexpected'
         end as catalog,
         identity.type,
         identity.object_names,
         identity.object_args,
         case
           when dependency.classid = 'pg_catalog.pg_proc'::regclass then (
             select jsonb_build_array(
                      language.lanname,
                      routine.proowner = extension.owner_oid,
                      routine.prokind,
                      routine.prosecdef,
                      routine.proleakproof,
                      routine.proisstrict,
                      routine.proretset,
                      routine.provolatile,
                      routine.proparallel,
                      routine.procost,
                      routine.prorows,
                      routine.pronargs,
                      routine.pronargdefaults,
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_type'::regclass,
                        routine.prorettype,
                        0
                      )),
                      (
                        select coalesce(
                          jsonb_agg(
                            pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                              'pg_catalog.pg_type'::regclass,
                              argument.type_oid,
                              0
                            )) order by argument.ordinality
                          ),
                          '[]'::jsonb
                        )
                          from unnest(routine.proargtypes::oid[]) with ordinality
                               argument(type_oid, ordinality)
                      ),
                      case when routine.proallargtypes is null then null else (
                        select jsonb_agg(
                                 pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                                   'pg_catalog.pg_type'::regclass,
                                   argument.type_oid,
                                   0
                                 )) order by argument.ordinality
                               )
                          from unnest(routine.proallargtypes) with ordinality
                               argument(type_oid, ordinality)
                      ) end,
                      routine.proargmodes,
                      routine.proargnames,
                      routine.proargdefaults is null,
                      case when routine.provariadic = 0 then null else
                        pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                          'pg_catalog.pg_type'::regclass,
                          routine.provariadic,
                          0
                        ))
                      end,
                      case when routine.protrftypes is null then null else (
                        select jsonb_agg(
                                 pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                                   'pg_catalog.pg_type'::regclass,
                                   transform.type_oid,
                                   0
                                 )) order by transform.ordinality
                               )
                          from unnest(routine.protrftypes) with ordinality
                               transform(type_oid, ordinality)
                      ) end,
                      routine.prosrc,
                      routine.probin,
                      routine.prosqlbody::text,
                      routine.proconfig,
                      routine.proacl::text,
                      case when routine.prosupport = 0 then null
                           else pg_catalog.to_jsonb(
                             pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass,
                               routine.prosupport,
                               0
                             )
                           ) end,
                      case when aggregate.aggfnoid is null then null else jsonb_build_array(
                        aggregate.aggkind,
                        aggregate.aggnumdirectargs,
                        case when aggregate.aggtransfn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggtransfn, 0
                             )) end,
                        case when aggregate.aggfinalfn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggfinalfn, 0
                             )) end,
                        case when aggregate.aggcombinefn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggcombinefn, 0
                             )) end,
                        case when aggregate.aggserialfn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggserialfn, 0
                             )) end,
                        case when aggregate.aggdeserialfn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggdeserialfn, 0
                             )) end,
                        case when aggregate.aggmtransfn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggmtransfn, 0
                             )) end,
                        case when aggregate.aggminvtransfn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggminvtransfn, 0
                             )) end,
                        case when aggregate.aggmfinalfn = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_proc'::regclass, aggregate.aggmfinalfn, 0
                             )) end,
                        aggregate.aggfinalextra,
                        aggregate.aggmfinalextra,
                        aggregate.aggfinalmodify,
                        aggregate.aggmfinalmodify,
                        case when aggregate.aggsortop = 0 then null
                             else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_operator'::regclass, aggregate.aggsortop, 0
                             )) end,
                        pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                          'pg_catalog.pg_type'::regclass, aggregate.aggtranstype, 0
                        )),
                        pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                          'pg_catalog.pg_type'::regclass, aggregate.aggmtranstype, 0
                        )),
                        aggregate.aggtransspace,
                        aggregate.aggmtransspace,
                        aggregate.agginitval,
                        aggregate.aggminitval
                      ) end
                    )
               from pg_catalog.pg_proc routine
               join pg_catalog.pg_language language on language.oid = routine.prolang
               left join pg_catalog.pg_aggregate aggregate on aggregate.aggfnoid = routine.oid
              where routine.oid = dependency.objid
           )
           when dependency.classid = 'pg_catalog.pg_type'::regclass then (
             select jsonb_build_array(
                      namespace.nspname,
                      data_type.typowner = extension.owner_oid,
                      data_type.typname,
                      data_type.typlen,
                      data_type.typbyval,
                      data_type.typtype,
                      data_type.typcategory,
                      data_type.typispreferred,
                      data_type.typisdefined,
                      data_type.typdelim,
                      case when data_type.typrelid = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_class'::regclass, data_type.typrelid, 0
                           )) end,
                      case when data_type.typelem = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_type'::regclass, data_type.typelem, 0
                           )) end,
                      case when data_type.typarray = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_type'::regclass, data_type.typarray, 0
                           )) end,
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_proc'::regclass, data_type.typinput, 0
                      )),
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_proc'::regclass, data_type.typoutput, 0
                      )),
                      case when data_type.typreceive = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, data_type.typreceive, 0
                           )) end,
                      case when data_type.typsend = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, data_type.typsend, 0
                           )) end,
                      case when data_type.typmodin = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, data_type.typmodin, 0
                           )) end,
                      case when data_type.typmodout = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, data_type.typmodout, 0
                           )) end,
                      case when data_type.typanalyze = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, data_type.typanalyze, 0
                           )) end,
                      case when data_type.typsubscript = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, data_type.typsubscript, 0
                           )) end,
                      data_type.typalign,
                      data_type.typstorage,
                      data_type.typnotnull,
                      case when data_type.typbasetype = 0 then null
                           else jsonb_build_array(
                             pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                               'pg_catalog.pg_type'::regclass, data_type.typbasetype, 0
                             )),
                             data_type.typtypmod
                           ) end,
                      data_type.typndims,
                      case when data_type.typcollation = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_collation'::regclass, data_type.typcollation, 0
                           )) end,
                      data_type.typdefaultbin::text,
                      data_type.typdefault,
                      data_type.typacl::text
                    )
               from pg_catalog.pg_type data_type
               join pg_catalog.pg_namespace namespace on namespace.oid = data_type.typnamespace
              where data_type.oid = dependency.objid
           )
           when dependency.classid = 'pg_catalog.pg_operator'::regclass then (
             select jsonb_build_array(
                      namespace.nspname,
                      operator.oprowner = extension.owner_oid,
                      operator.oprname,
                      operator.oprkind,
                      operator.oprcanmerge,
                      operator.oprcanhash,
                      case when operator.oprleft = 0 then null else
                        pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                          'pg_catalog.pg_type'::regclass, operator.oprleft, 0
                        ))
                      end,
                      case when operator.oprright = 0 then null else
                        pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                          'pg_catalog.pg_type'::regclass, operator.oprright, 0
                        ))
                      end,
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_type'::regclass, operator.oprresult, 0
                      )),
                      case when operator.oprcom = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_operator'::regclass, operator.oprcom, 0
                           )) end,
                      case when operator.oprnegate = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_operator'::regclass, operator.oprnegate, 0
                           )) end,
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_proc'::regclass, operator.oprcode, 0
                      )),
                      case when operator.oprrest = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, operator.oprrest, 0
                           )) end,
                      case when operator.oprjoin = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, operator.oprjoin, 0
                           )) end
                    )
               from pg_catalog.pg_operator operator
               join pg_catalog.pg_namespace namespace on namespace.oid = operator.oprnamespace
              where operator.oid = dependency.objid
           )
           when dependency.classid = 'pg_catalog.pg_opclass'::regclass then (
             select jsonb_build_array(
                      namespace.nspname,
                      operator_class.opcowner = extension.owner_oid,
                      access_method.amname,
                      operator_class.opcname,
                      operator_class.opcdefault,
                      operator_family_namespace.nspname,
                      operator_family.opfname,
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_type'::regclass, operator_class.opcintype, 0
                      )),
                      case when operator_class.opckeytype = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_type'::regclass, operator_class.opckeytype, 0
                           )) end
                    )
               from pg_catalog.pg_opclass operator_class
               join pg_catalog.pg_namespace namespace on namespace.oid = operator_class.opcnamespace
               join pg_catalog.pg_am access_method on access_method.oid = operator_class.opcmethod
               join pg_catalog.pg_opfamily operator_family on operator_family.oid = operator_class.opcfamily
               join pg_catalog.pg_namespace operator_family_namespace
                 on operator_family_namespace.oid = operator_family.opfnamespace
              where operator_class.oid = dependency.objid
           )
           when dependency.classid = 'pg_catalog.pg_opfamily'::regclass then (
             select jsonb_build_array(
                      namespace.nspname,
                      operator_family.opfowner = extension.owner_oid,
                      access_method.amname,
                      operator_family.opfname
                    )
               from pg_catalog.pg_opfamily operator_family
               join pg_catalog.pg_namespace namespace on namespace.oid = operator_family.opfnamespace
               join pg_catalog.pg_am access_method on access_method.oid = operator_family.opfmethod
              where operator_family.oid = dependency.objid
           )
           when dependency.classid = 'pg_catalog.pg_cast'::regclass then (
             select jsonb_build_array(
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_type'::regclass, type_cast.castsource, 0
                      )),
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_type'::regclass, type_cast.casttarget, 0
                      )),
                      case when type_cast.castfunc = 0 then null
                           else pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                             'pg_catalog.pg_proc'::regclass, type_cast.castfunc, 0
                           )) end,
                      type_cast.castcontext,
                      type_cast.castmethod
                    )
               from pg_catalog.pg_cast type_cast
              where type_cast.oid = dependency.objid
           )
           when dependency.classid = 'pg_catalog.pg_am'::regclass then (
             select jsonb_build_array(
                      access_method.amname,
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_proc'::regclass, access_method.amhandler, 0
                      )),
                      access_method.amtype
                    )
               from pg_catalog.pg_am access_method
              where access_method.oid = dependency.objid
           )
           when dependency.classid = 'pg_catalog.pg_language'::regclass then (
             select jsonb_build_array(
                      language.lanowner = extension.owner_oid,
                      language.lanispl,
                      language.lanpltrusted,
                      pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                        'pg_catalog.pg_proc'::regclass, language.lanplcallfoid, 0
                      )),
                      case when language.laninline = 0 then null else
                        pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                          'pg_catalog.pg_proc'::regclass, language.laninline, 0
                        ))
                      end,
                      case when language.lanvalidator = 0 then null else
                        pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
                          'pg_catalog.pg_proc'::regclass, language.lanvalidator, 0
                        ))
                      end,
                      language.lanacl::text
                    )
               from pg_catalog.pg_language language
              where language.oid = dependency.objid
           )
           else null
         end as details
    from extension_dependencies dependency
    join extension_state extension on extension.oid = dependency.refobjid
    cross join lateral pg_catalog.pg_identify_object_as_address(
      dependency.classid,
      dependency.objid,
      dependency.objsubid
    ) identity
   limit 387
), operator_family_oids as materialized (
  select extension.extname, operator_family.oid
    from extension_dependencies dependency
    join extension_state extension on extension.oid = dependency.refobjid
    join pg_catalog.pg_opfamily operator_family
      on dependency.classid = 'pg_catalog.pg_opfamily'::regclass
     and operator_family.oid = dependency.objid
   limit 54
), access_operators as materialized (
  select family.extname,
         jsonb_build_array(
           namespace.nspname,
           operator_family.opfname,
           access_method.amname,
           operator_member.amopstrategy,
           operator_member.amoppurpose,
           pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
             'pg_catalog.pg_type'::regclass, operator_member.amoplefttype, 0
           )),
           pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
             'pg_catalog.pg_type'::regclass, operator_member.amoprighttype, 0
           )),
           pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
             'pg_catalog.pg_operator'::regclass, operator_member.amopopr, 0
           )),
           case when operator_member.amopsortfamily = 0 then null else
             concat(sort_namespace.nspname, '.', sort_family.opfname, '/', sort_method.amname)
           end
         ) as value
    from operator_family_oids family
    join pg_catalog.pg_opfamily operator_family on operator_family.oid = family.oid
    join pg_catalog.pg_namespace namespace on namespace.oid = operator_family.opfnamespace
    join pg_catalog.pg_am access_method on access_method.oid = operator_family.opfmethod
    join pg_catalog.pg_amop operator_member on operator_member.amopfamily = family.oid
    left join pg_catalog.pg_opfamily sort_family on sort_family.oid = operator_member.amopsortfamily
    left join pg_catalog.pg_namespace sort_namespace on sort_namespace.oid = sort_family.opfnamespace
    left join pg_catalog.pg_am sort_method on sort_method.oid = sort_family.opfmethod
   limit 182
), support_functions as materialized (
  select family.extname,
         jsonb_build_array(
           namespace.nspname,
           operator_family.opfname,
           access_method.amname,
           support.amprocnum,
           pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
             'pg_catalog.pg_type'::regclass, support.amproclefttype, 0
           )),
           pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
             'pg_catalog.pg_type'::regclass, support.amprocrighttype, 0
           )),
           pg_catalog.to_jsonb(pg_catalog.pg_identify_object_as_address(
             'pg_catalog.pg_proc'::regclass, support.amproc, 0
           ))
         ) as value
    from operator_family_oids family
    join pg_catalog.pg_opfamily operator_family on operator_family.oid = family.oid
    join pg_catalog.pg_namespace namespace on namespace.oid = operator_family.opfnamespace
    join pg_catalog.pg_am access_method on access_method.oid = operator_family.opfmethod
    join pg_catalog.pg_amproc support on support.amprocfamily = family.oid
   limit 200
), fingerprints as (
select extension.extname,
       pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
         jsonb_build_object(
           'metadata', extension.metadata,
           'members', (
             select coalesce(jsonb_agg(jsonb_build_array(
                       member.catalog,
                       member.type,
                       member.object_names,
                       member.object_args,
                       member.details
                     ) order by member.catalog collate "C", member.type collate "C",
                                member.object_names::text collate "C",
                                member.object_args::text collate "C"), '[]'::jsonb)
               from extension_members member
              where member.extname = extension.extname
           ),
           'access-operators', (
             select coalesce(jsonb_agg(operator.value order by operator.value::text collate "C"), '[]'::jsonb)
               from access_operators operator
              where operator.extname = extension.extname
           ),
           'support-functions', (
             select coalesce(jsonb_agg(support.value order by support.value::text collate "C"), '[]'::jsonb)
               from support_functions support
              where support.extname = extension.extname
           )
         )::text,
         'UTF8'
       )), 'hex') as fingerprint
  from extension_state extension
)
select (
  select count(*) from pg_catalog.pg_extension
) between 1 and 3 and exists (
  select 1
    from pg_catalog.pg_extension extension
   where extension.extname = 'plpgsql'
) and not exists (
  select 1
    from pg_catalog.pg_extension extension
   where extension.extname not in ('plpgsql', 'btree_gin', 'vector')
) and not exists (
  select 1
    from fingerprints actual
    left join (values
      ('btree_gin', 145::bigint, 145::bigint, 145::bigint,
       'de5d37023e87c8306c325d8b361c08220a7d77e2cd59e2407ebe01caa881577d'),
      ('plpgsql', 4::bigint, 0::bigint, 0::bigint,
       '1a4cf221e73829cba2b8eb8b659e951670d04c5eb13578cfa21d06624b3eb178'),
      ('vector', 237::bigint, 36::bigint, 54::bigint,
       '5b1552a857b437d8a0c3274d3344feaed14a4033ce0ebcdcef238ea99f84b980')
    ) expected(extension_name, member_count, access_count, support_count, fingerprint)
      on expected.extension_name = actual.extname
   where expected.extension_name is null
      or expected.member_count <> (
        select count(*) from extension_members member
         where member.extname = actual.extname
      )
      or expected.access_count <> (
        select count(*) from access_operators operator
         where operator.extname = actual.extname
      )
      or expected.support_count <> (
        select count(*) from support_functions support
         where support.extname = actual.extname
      )
      or expected.fingerprint is distinct from actual.fingerprint
) as safe
