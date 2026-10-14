//! MEM-9 upgrade keeps exact frozen 0007 review identities and source aliases.
#[path = "../../synveda-store/tests/support/epoch_fixture.rs"]
mod epoch_fixture;
use epoch_fixture::*;
use synveda_store::epoch;
/// Retain both legal 0007 plans: URI identity folds case while the old locator
/// uniqueness deliberately does not. Migration must not merge or delete them.
#[test]
fn exact_reconcile_prefix_preserves_case_only_reattachment_review_plans() {
    let Some(server) = server() else { return };
    let scratch = Scratch::empty(&server);
    let gateway_url = private_database_url("SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE").unwrap();
    scratch.block_on(async {
        use synveda_types::*;
        use synveda_types::configuration::{ConfigurationCommand,ConfigurationDocument,ConfigurationTemplate};
        use synveda_types::knowledge::{KnowledgeOrigin,KnowledgeRevisionContent,KnowledgeType};
        let owner=connect_pool(&scratch.options).await;
        let mut connection=owner.acquire().await.unwrap();
        sqlx::migrate::Migrator{migrations:std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..7].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT}.run(&mut *connection).await.unwrap();
        epoch::stamp_connection(&mut connection,"0.4.5").await.unwrap();drop(connection);
        let options=synveda_store::database_url::parse("fixture gateway",&gateway_url).unwrap().database(scratch.options.get_database().unwrap());
        let pool=connect_pool(&options).await;let tenant=admit_a_tenant(&owner).await;
        let mut tx=synveda_store::rls::begin_tenant_write_tx(&pool,tenant).await.unwrap();
        let workspace=synveda_store::workspaces::create(&mut tx,&synveda_store::workspaces::NewWorkspace {
            id:WorkspaceId::new(),tenant_id:tenant,slug:"case-source".to_owned(),display_name:"Case source".to_owned(),description:None,created_by:None}).await.unwrap();
        let project=synveda_store::projects::create(&mut tx,&synveda_store::projects::NewProject {
            id:ProjectId::new(),tenant_id:tenant,workspace_id:workspace.id,slug:"case-source".to_owned(),display_name:"Case source".to_owned(),description:None,created_by:None}).await.unwrap();
        let principal=synveda_store::scopes::ensure_principal_scope(&mut tx,tenant,"case-source-user","Case source user").await.unwrap();
        let actor=IdentityId::new();synveda_store::identities::create(&mut tx,actor,tenant,Some("case-source-user"),IdentityKind::User,None,None,principal.id).await.unwrap();
        let content=KnowledgeRevisionContent {title:"Current retry procedure".to_owned(),body_markdown:"Use five attempts.".to_owned(),summary:"Current comparison target".to_owned(),tags:vec![],
            sensitivity:Sensitivity::Internal,confidence_permille:900,valid_from:chrono::Utc::now(),valid_to:None,stale_after:None,verification_metadata:serde_json::json!({}),metadata:serde_json::json!({})};
        let provenance=synveda_store::knowledge::create_source(&mut tx,&synveda_store::knowledge::NewKnowledgeSource {
            id:KnowledgeSourceId::new(),tenant_id:tenant,scope_id:project.scope_id,source_type:synveda_types::knowledge::KnowledgeSourceType::Document,
            session_event_id:None,import_artifact_id:None,locator:Some("fictional:current-retry-definition".to_owned()),source_revision:Some("fixture-v1".to_owned()),
            content_hash:Some(blake3::hash(content.body_markdown.as_bytes()).to_hex().to_string()),metadata:serde_json::json!({}),created_by:Some("case-source-user".to_owned())}).await.unwrap();
        let target=synveda_store::knowledge::create_item(&mut tx,&synveda_store::knowledge::NewKnowledgeItem {
            id:KnowledgeItemId::new(),tenant_id:tenant,scope_id:project.scope_id,project_id:Some(project.id),owner_principal_id:None,
            knowledge_type:KnowledgeType::Procedure,origin:KnowledgeOrigin::Authored,created_by:Some("case-source-user".to_owned())},
            &synveda_store::knowledge::NewKnowledgeRevision {id:KnowledgeRevisionId::new(),content:content.clone(),created_by:Some("case-source-user".to_owned())},&[provenance.id]).await.unwrap();
        tx.commit().await.unwrap();
        let review=blake3::hash(serde_json::json!(["synveda.managed-review.v1",[("procedures/retries.md","update",target.item.id,target.revision.id)]]).to_string().as_bytes()).to_hex().to_string();
        let hash=blake3::hash(b"Fictional original three-attempt source.").to_hex().to_string();
        let mut jobs=Vec::new();let mut previous_repository=None;
        for uri in ["https://github.com/Acme/repo","https://github.com/acme/repo"] {
            let mut tx=synveda_store::rls::begin_tenant_write_tx(&pool,tenant).await.unwrap();
            if let Some(repository)=previous_repository {assert!(synveda_store::repositories::detach(&mut *tx,tenant,project.id,repository).await.unwrap());}
            let repository=synveda_store::repositories::attach(&mut *tx,&synveda_store::repositories::NewRepository {
                id:RepositoryId::new(),tenant_id:tenant,project_id:project.id,identity:synveda_types::repository::identify(Some(uri),None,None,None).unwrap(),
                default_branch:Some("main".to_owned()),metadata:serde_json::json!({}),created_by:Some(actor)}).await.unwrap();
            previous_repository=Some(repository.id);
            let mut document=ConfigurationDocument::template(ConfigurationTemplate::Personal);
            document.git_sources=vec![synveda_types::source::GitSourceDescriptor {source_id:"case-source".to_owned(),repository_id:repository.id,read_target:"fixture".to_owned(),
                git_ref:"refs/heads/main".to_owned(),allowed_paths:vec!["procedures/retries.md".to_owned()],enabled:true}];
            let version=ConfigurationVersionId::new();let artifact=ConfigurationArtifactId::new();
            let command=ConfigurationCommand::Create {artifact_id:artifact,version_id:version,governing_scope_id:project.scope_id,
                name:format!("case-source-{}",jobs.len()),content_hash:document.content_hash().unwrap(),document,source_template:None};
            let bytes=serde_json::to_vec(&command).unwrap();let object=synveda_vedaflow::put_object(&mut tx,tenant,AssetKind::Configuration,&bytes).await.unwrap();
            let reference=ArtifactReference::new(ArtifactFamily::Configuration,artifact.to_string(),"create",version.to_string(),None).unwrap();
            let proposal=synveda_vedaflow::proposals::open(&mut tx,tenant,&synveda_vedaflow::NewProposal {
                target_scope:project.scope_id,source_scope:project.scope_id,asset:AssetKind::Configuration,effect:ProposalEffect::Apply,
                members:&[("command".to_owned(),object.hash)],artifact_references:&[reference],sensitivity:Sensitivity::Internal,title:"Legacy source fixture Configuration",
                proposer:actor,proposer_subject:"case-source-user",committed_at:chrono::Utc::now(),policy_snapshot:&synveda_vedaflow::PolicySnapshot::new("epoch-fixture",1)},&synveda_vedaflow::Signer::Ed25519(Box::new(synveda_vedaflow::Ed25519Signer::new([19;32],"epoch-fixture-signing").unwrap()))).await.unwrap();
            let digest=blake3::hash(&bytes).to_hex().to_string();synveda_store::configuration::insert_change(&mut tx,tenant,proposal.id,&command,&digest).await.unwrap();
            let applied=synveda_store::configuration::apply(&mut tx,tenant,proposal.id,"case-source-user",&command).await.unwrap();
            synveda_store::configuration::complete_change(&mut tx,tenant,proposal.id,applied).await.unwrap();
            synveda_vedaflow::proposals::close(&mut tx,tenant,proposal.id,ProposalState::Applied,actor,None).await.unwrap();
            let operation=synveda_store::source_reconciliation::enqueue(&mut tx,tenant,project.id,project.scope_id,"case-source",repository.id,version,
                &"1".repeat(64),&"2".repeat(64),None,1,actor,&"3".repeat(64)).await.unwrap();
            tx.commit().await.unwrap();
            let mut tx=synveda_store::rls::begin_tenant_write_tx(&pool,tenant).await.unwrap();
            let claim=synveda_store::source_reconciliation::claim(&mut tx,tenant,"legacy-source-fixture").await.unwrap().unwrap();
            synveda_store::source_reconciliation::fence(&mut tx,tenant,operation,claim.request.attempts,"legacy-source-fixture").await.unwrap();
            synveda_store::source_reconciliation::pin(&mut tx,tenant,operation,&"a".repeat(40),&"b".repeat(40)).await.unwrap();
            let job=ImportJobId::new();let imported=ImportArtifactId::new();
            sqlx::query("insert into import_jobs(id,tenant_id,project_id,scope_id,workspace_id,principal_id,format,format_version,specification_commit,source_kind,source_locator,source_revision,bundle_digest,artifact_count,mapping_count,repository_id,managed_source_id,managed_review_digest) values($1,$2,$3,$4,$5,'case-source-user','okf','0.2','ad30107c31c06aec8a7d5636e0d1058118604e6f','git',$6,$7,$8,1,1,$9,'case-source',$10)")
                .bind(job.as_uuid()).bind(tenant.as_uuid()).bind(project.id.as_uuid()).bind(project.scope_id.as_uuid()).bind(workspace.id.as_uuid())
                .bind(&repository.canonical_uri).bind("a".repeat(40)).bind(&hash).bind(repository.id.as_uuid()).bind(&review).execute(&mut *tx).await.unwrap();
            sqlx::query("insert into import_artifacts(id,tenant_id,job_id,ordinal,logical_path,artifact_kind,content_hash,frontmatter,body_markdown) values($1,$2,$3,1,'procedures/retries.md','concept',$4,'{}','Fictional original three-attempt source.')")
                .bind(imported.as_uuid()).bind(tenant.as_uuid()).bind(job.as_uuid()).bind(&hash).execute(&mut *tx).await.unwrap();
            let proposed=KnowledgeRevisionContent {body_markdown:"Fictional original three-attempt source.".to_owned(),..content.clone()};
            sqlx::query("insert into import_mappings(id,tenant_id,job_id,artifact_id,ordinal,okf_type,knowledge_type,title,body_markdown,summary,tags,sensitivity,confidence_permille,valid_from,verification_metadata,metadata,content_hash,classification,matched_item_id,matched_revision_id,proposed_relations,materializable) values($1,$2,$3,$4,1,'Procedure','procedure',$5,$6,$7,'{}','internal',900,$8,'{}','{}',$9,'update',$10,$11,'[]',true)")
                .bind(ImportMappingId::new().as_uuid()).bind(tenant.as_uuid()).bind(job.as_uuid()).bind(imported.as_uuid()).bind(&proposed.title).bind(&proposed.body_markdown).bind(&proposed.summary).bind(proposed.valid_from)
                .bind(synveda_types::knowledge::knowledge_revision_content_hash(&proposed)).bind(target.item.id.as_uuid()).bind(target.revision.id.as_uuid()).execute(&mut *tx).await.unwrap();
            let request=synveda_store::source_reconciliation::request(&mut tx,tenant,operation).await.unwrap().unwrap();
            synveda_store::source_reconciliation::complete(&mut tx,tenant,&request,Some(job),&[synveda_store::source_reconciliation::ObservedPath {
                path:"procedures/retries.md".to_owned(),state:"present",blob_oid:Some("c".repeat(40)),content_hash:Some(hash.clone()),artifact_id:Some(imported)}]).await.unwrap();
            tx.commit().await.unwrap();jobs.push(job);
        }
        let before=migration_ledger(&owner).await;assert_eq!(before.len(),7);
        synveda_store::check_migration_compatibility(&owner,&scratch.roles).await.unwrap();
        synveda_store::migrate(&owner,&scratch.roles).await.unwrap();let after=migration_ledger(&owner).await;
        assert_eq!(&after[..7],&before);assert_eq!(after.len(),9);
        let mut tx=synveda_store::rls::begin_tenant_tx(&pool,tenant).await.unwrap();
        let retained:Vec<uuid::Uuid>=sqlx::query_scalar("select id from import_jobs where tenant_id=$1 order by created_at,id").bind(tenant.as_uuid()).fetch_all(&mut *tx).await.unwrap();
        assert_eq!(retained,jobs.iter().map(|id|id.as_uuid()).collect::<Vec<_>>());
        for job in jobs {let job=synveda_store::imports::get_job(&mut *tx,tenant,job).await.unwrap().unwrap();synveda_store::artifact_control::admit_job(&mut tx,tenant,&job).await.unwrap();}
        tx.commit().await.unwrap();pool.close().await;owner.close().await;
    });
}
