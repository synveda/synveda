//! The reviewed inventory of everything that holds tenant data (ADR-0157).
//!
//! Every table with a `tenant_id` column appears exactly once in
//! [`TENANT_TABLES`], in an order that restores parents before children for
//! every foreign key that is checked immediately. The database coverage test
//! compares this list with the live catalogue, so a new tenant-bearing table
//! cannot ship unclassified. Deployment tables and data held outside
//! PostgreSQL are named separately because a tenant lifecycle step may read
//! them but never treats them as tenant rows.

use sqlx::PgConnection;
use synveda_types::{Error, Result, TenantId};

/// The operator-facing grouping of a table, for inventory reports only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Scopes, grants, identities, groups, invitations, workspaces, projects.
    Access,
    /// The tenant's hash-chained audit log.
    Audit,
    /// Capture batches, candidates and their decisions.
    Capture,
    /// Governed Configuration documents, policy packs and relaxations.
    Configuration,
    /// Saved context runs, selections and delivery proof.
    Context,
    /// Tenant keys, sealed secrets and token revocations.
    Credentials,
    /// SCIM credentials, SCIM users and directory synchronization state.
    Directory,
    /// Knowledge items, revisions, provenance, relations and their headers.
    Knowledge,
    /// Durable operations, attempts, the outbox and idempotent replies.
    Operations,
    /// Procedure outcomes, candidates and evaluations.
    Procedures,
    /// Skills, Tools, Prompts and context packs.
    Registry,
    /// Sessions, events, quarantine and checkpoints.
    Sessions,
    /// Imports, imported artifacts and inbound source control.
    Sources,
    /// VedaFlow objects, trees, commits, refs and proposals.
    VedaFlow,
}

/// How a portable tenant archive treats a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Archive {
    /// Copied exactly and restored with the same identities.
    Portable,
    /// Rebuilt after restore from portable rows; never archived.
    Derived,
    /// Not portable: the rows point at deployment-sealed custody.
    Excluded,
}

/// What tenant erasure does to a table under the synthetic fixture rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Erasure {
    /// Every row of the tenant is deleted.
    Erase,
    /// Content-free evidence kept after erasure.
    RetainEvidence,
    /// Only the tenant lifecycle operation's own rows are kept.
    RetainLifecycleRows,
}

/// One tenant-bearing table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantTable {
    /// Unqualified table name in the `public` schema.
    pub name: &'static str,
    /// Reporting group.
    pub family: Family,
    /// Archive treatment.
    pub archive: Archive,
    /// Erasure treatment.
    pub erasure: Erasure,
}

impl TenantTable {
    const fn new(name: &'static str, family: Family, archive: Archive, erasure: Erasure) -> Self {
        Self {
            name,
            family,
            archive,
            erasure,
        }
    }
}

/// Every tenant-bearing table, in restore order.
pub const TENANT_TABLES: [TenantTable; 116] = [
    TenantTable::new(
        "artifact_dependencies",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "audit_chain_heads",
        Family::Audit,
        Archive::Portable,
        Erasure::RetainEvidence,
    ),
    TenantTable::new(
        "audit_log",
        Family::Audit,
        Archive::Portable,
        Erasure::RetainEvidence,
    ),
    TenantTable::new(
        "console_session_index",
        Family::Credentials,
        Archive::Excluded,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_packs",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "directory_sync_state",
        Family::Directory,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new("groups", Family::Access, Archive::Portable, Erasure::Erase),
    TenantTable::new(
        "idempotency_records",
        Family::Operations,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_change_headers",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_revision_headers",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "historical_knowledge_dependencies",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_copy_dependencies",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_revision_source_headers",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "policy_packs",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "scim_credentials",
        Family::Directory,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new("scopes", Family::Access, Archive::Portable, Erasure::Erase),
    TenantTable::new(
        "identities",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "group_members",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "pending_invites",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "scim_users",
        Family::Directory,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "scope_closure",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "scope_grants",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_artifact_addresses",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_dependencies",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_event_addresses",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "procedure_improvement_headers",
        Family::Procedures,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "procedure_candidate_headers",
        Family::Procedures,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "procedure_evaluation_attempts",
        Family::Procedures,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "procedure_evaluation_results",
        Family::Procedures,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "procedure_outcome_headers",
        Family::Procedures,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "session_snapshot_headers",
        Family::Sessions,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tenant_administrator_bootstraps",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tenant_keys",
        Family::Credentials,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tenant_secret_reencryption_jobs",
        Family::Credentials,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tenant_secrets",
        Family::Credentials,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "token_revocations",
        Family::Credentials,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "unproven_artifact_dependencies",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_objects",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_pack_chunks",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_pack_documents",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "prompts",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_trees",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_commits",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_commit_parents",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_proposals",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "configuration_changes",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_changes",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "artifact_suppressions",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "policy_relaxation_changes",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "skill_changes",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_suppressions",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tool_changes",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_proposal_approvals",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_refs",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "vedaflow_tree_entries",
        Family::VedaFlow,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "workspaces",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "projects",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "project_repositories",
        Family::Access,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "sessions",
        Family::Sessions,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "session_events",
        Family::Sessions,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "session_event_quarantine",
        Family::Sessions,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_repository_roots",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_repository_attachments",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "configuration_artifacts",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "configuration_versions",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "configuration_bindings",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "session_context_runs",
        Family::Context,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_run_delivery_dependencies",
        Family::Context,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "unproven_context_deliveries",
        Family::Context,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capture_batches",
        Family::Capture,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capture_batch_events",
        Family::Capture,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "import_jobs",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capture_candidates",
        Family::Capture,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capture_candidate_decisions",
        Family::Capture,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capture_candidate_events",
        Family::Capture,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "import_artifacts",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capture_candidate_import_artifacts",
        Family::Capture,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_conflict_sets",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_sources",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_items",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_revisions",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capture_candidate_matches",
        Family::Capture,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_candidates",
        Family::Context,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_selections",
        Family::Context,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_feedback",
        Family::Context,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "import_mappings",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_conflict_members",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_items_history",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_relations",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "context_graph_steps",
        Family::Context,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_revision_embeddings",
        Family::Knowledge,
        Archive::Derived,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_revision_sources",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "policy_relaxations",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "policy_relaxation_versions",
        Family::Configuration,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "skills",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "skill_versions",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "durable_operations",
        Family::Operations,
        Archive::Portable,
        Erasure::RetainLifecycleRows,
    ),
    TenantTable::new(
        "artifact_cleanup_targets",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_erasure_tombstones",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "knowledge_index_invalidations",
        Family::Knowledge,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "operation_attempts",
        Family::Operations,
        Archive::Portable,
        Erasure::RetainLifecycleRows,
    ),
    TenantTable::new(
        "operation_outbox",
        Family::Operations,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "skill_bindings",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "skill_test_runs",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "skill_usage_events",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "skill_version_files",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_cleanup_targets",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_observations",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_observation_paths",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_reconcile_heads",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "source_reconcile_requests",
        Family::Sources,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tool_servers",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tool_server_versions",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "capability_snapshots",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tool_bindings",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
    TenantTable::new(
        "tool_test_runs",
        Family::Registry,
        Archive::Portable,
        Erasure::Erase,
    ),
];

/// A deployment table that a tenant can reach but that is not tenant-scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DeploymentTable {
    /// Unqualified table name in the `public` schema.
    pub name: &'static str,
    /// How a tenant reaches it, if at all.
    pub tenant_reference: &'static str,
}

/// Every table without `tenant_id`.
pub const DEPLOYMENT_TABLES: [DeploymentTable; 8] = [
    DeploymentTable {
        name: "_sqlx_migrations",
        tenant_reference: "none",
    },
    DeploymentTable {
        name: "capture_provider_slots",
        tenant_reference: "none; opaque process leases over installation-wide provider capacity",
    },
    DeploymentTable {
        name: "cli_handoffs",
        tenant_reference: "short-lived sealed login handoffs that expire within minutes",
    },
    DeploymentTable {
        name: "console_sessions",
        tenant_reference: "deployment-sealed bearer custody named by the tenant's console session index",
    },
    DeploymentTable {
        name: "deployment_keys",
        tenant_reference: "wraps no tenant data; the KEK wraps tenant keys directly",
    },
    DeploymentTable {
        name: "pending_logins",
        tenant_reference: "short-lived sealed login state that expires within minutes",
    },
    DeploymentTable {
        name: "schema_metadata",
        tenant_reference: "none",
    },
    DeploymentTable {
        name: "tenants",
        tenant_reference: "the tenant's own lifecycle row",
    },
];

/// Who owns a copy of tenant data held outside the product database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    /// Written and owned by this deployment.
    Application,
    /// Owned by the deployment operator.
    Deployment,
    /// Owned by the customer or a third party.
    Customer,
}

/// One class of tenant data held outside the product database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ExternalObject {
    /// Stable identifier.
    pub name: &'static str,
    /// Who owns the copy.
    pub owner: Owner,
    /// What a tenant lifecycle step does with it.
    pub treatment: &'static str,
}

/// Every external copy a lifecycle operator must account for.
pub const EXTERNAL_OBJECTS: [ExternalObject; 8] = [
    ExternalObject {
        name: "git_export_repositories",
        owner: Owner::Application,
        treatment: "bare repositories under the gateway's Git export root, one directory per tenant; re-exportable from governed evidence and not archived",
    },
    ExternalObject {
        name: "git_source_repositories",
        owner: Owner::Customer,
        treatment: "read-only inputs under the Git source root; never changed",
    },
    ExternalObject {
        name: "remote_git_repositories",
        owner: Owner::Customer,
        treatment: "remote export destinations and their history; never changed",
    },
    ExternalObject {
        name: "identity_provider_accounts",
        owner: Owner::Customer,
        treatment: "OIDC accounts, sessions and SCIM sources; never changed",
    },
    ExternalObject {
        name: "delivered_client_copies",
        owner: Owner::Customer,
        treatment: "context already delivered to agents and adapter spools; cannot be recalled",
    },
    ExternalObject {
        name: "tenant_archives",
        owner: Owner::Deployment,
        treatment: "sealed archives and legacy exports held by the operator; never changed",
    },
    ExternalObject {
        name: "deployment_backups",
        owner: Owner::Deployment,
        treatment: "physical and logical backups retain tenant rows until their own expiry; never changed",
    },
    ExternalObject {
        name: "telemetry",
        owner: Owner::Deployment,
        treatment: "content-free traces and aggregate metrics; never changed",
    },
];

/// The classification for one table, if it is tenant-bearing.
#[must_use]
pub fn tenant_table(name: &str) -> Option<&'static TenantTable> {
    TENANT_TABLES.iter().find(|table| table.name == name)
}

/// The largest per-table count an inventory may request.
pub const MAX_COUNT_CAP: u64 = 10_000_000;

/// One table's rows for one tenant, counted up to a cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct TableCount {
    /// Table name.
    pub table: &'static str,
    /// Reporting group.
    pub family: Family,
    /// Archive treatment.
    pub archive: Archive,
    /// Erasure treatment.
    pub erasure: Erasure,
    /// Rows counted, never more than the cap.
    pub rows: u64,
    /// Whether the table holds more rows than the cap.
    pub capped: bool,
}

/// Counts every tenant-bearing table for one tenant, in restore order.
///
/// The caller supplies a connection whose transaction already carries the
/// tenant GUC, so forced RLS confines every count to that tenant.
pub async fn count_tenant_rows(
    connection: &mut PgConnection,
    tenant: TenantId,
    cap: u64,
) -> Result<Vec<TableCount>> {
    if cap == 0 || cap > MAX_COUNT_CAP {
        return Err(Error::Invalid {
            message: format!("the inventory cap must be between 1 and {MAX_COUNT_CAP}"),
        });
    }
    let mut counts = Vec::with_capacity(TENANT_TABLES.len());
    for table in &TENANT_TABLES {
        counts.push(count_table(connection, tenant, table, cap).await?);
    }
    Ok(counts)
}

async fn count_table(
    connection: &mut PgConnection,
    tenant: TenantId,
    table: &'static TenantTable,
    cap: u64,
) -> Result<TableCount> {
    debug_assert!(is_plain_identifier(table.name));
    // The identifier comes only from the reviewed constant, never from input.
    let statement = format!(
        r#"select count(*) from (select 1 from public."{}" where tenant_id = $1 limit $2) as bounded"#,
        table.name
    );
    let limit = i64::try_from(cap.saturating_add(1)).unwrap_or(i64::MAX);
    let counted: i64 = sqlx::query_scalar(&statement)
        .bind(tenant.as_uuid())
        .bind(limit)
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| Error::Storage {
            message: format!("count tenant rows in {}: {error}", table.name),
        })?;
    let counted = u64::try_from(counted).unwrap_or(0);
    Ok(TableCount {
        table: table.name,
        family: table.family,
        archive: table.archive,
        erasure: table.erasure,
        rows: counted.min(cap),
        capped: counted > cap,
    })
}

/// Whether `name` is a lowercase SQL identifier that needs no quoting rules
/// beyond surrounding double quotes.
#[must_use]
pub fn is_plain_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some('a'..='z' | '_'))
        && characters.all(|character| matches!(character, 'a'..='z' | '0'..='9' | '_'))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_table_is_classified_once_with_a_plain_name() {
        let mut seen = HashSet::new();
        for table in TENANT_TABLES
            .iter()
            .map(|table| table.name)
            .chain(DEPLOYMENT_TABLES.iter().map(|table| table.name))
        {
            assert!(is_plain_identifier(table), "{table} needs quoting");
            assert!(seen.insert(table), "{table} is classified twice");
        }
    }

    #[test]
    fn only_audit_and_lifecycle_evidence_survive_erasure() {
        let retained: Vec<_> = TENANT_TABLES
            .iter()
            .filter(|table| table.erasure != Erasure::Erase)
            .map(|table| table.name)
            .collect();
        assert_eq!(
            retained,
            [
                "audit_chain_heads",
                "audit_log",
                "durable_operations",
                "operation_attempts"
            ]
        );
    }

    #[test]
    fn identifiers_that_would_need_quoting_are_refused() {
        for name in [
            "",
            "Scopes",
            "1scopes",
            "scopes;drop",
            "scope\"s",
            "scopes ",
        ] {
            assert!(!is_plain_identifier(name), "{name:?}");
        }
    }
}
