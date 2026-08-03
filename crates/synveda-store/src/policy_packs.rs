//! The per-tenant stored policy packs (AUTHZ-1 ADR-0012 decision 5;
//! named-per-tenant since AUTHZ-2, ADR-0014 decision 6).
//!
//! One row per (tenant, name); [`apply`] upserts and owns the per-name
//! version bump, the gateway's refresher polls [`stored`] and reconciles
//! compiled packs into the PDP, and [`clear`] removes a pack — refusing
//! while assignments or the tenant default still reference it, so a
//! dangling reference can only come from out-of-band writes. The product
//! pack names are reserved by a check constraint: `regulated-strict` means
//! the same thing in every tenant. `policy_packs` is tenant-scoped (forced
//! RLS, ADR-0009): reach it inside [`crate::rls::begin_tenant_tx`].
//!
//! The store neither parses nor validates Cedar — that is `synveda-policy`
//! (storage knows nothing of policy, seed §2.4). Callers compile-check
//! before applying; the refresher rejects a bad pack at reload time and
//! keeps the tenant's last-good pack.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use sqlx::postgres::PgConnection;
use synveda_types::{
    ApprovalMatrix, DedupConfig, Error, LapseConfig, PackConfig, PromotionConfig, Result,
    RetentionConfig, TenantId,
};

/// A tenant's stored policy pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyPack {
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Pack name (slug grammar; product names are reserved), e.g.
    /// `acme-strict`.
    pub name: String,
    /// Monotonically increasing per (tenant, name); bumped by every
    /// [`apply`].
    pub version: i64,
    /// Cedar policy source.
    pub source: String,
    /// The pack's non-Cedar configuration: redaction (MEM-2, ADR-0021
    /// decision 3), composition (CTX-2, ADR-0025 decisions 2–3), the
    /// approval matrix (FLOW-3, ADR-0032 decision 3), dedup (MEM-5,
    /// ADR-0039 decision 12), and retention (MEM-6, ADR-0040). Each field
    /// is `None` when unconfigured, and each resolves to its own fail-safe
    /// downstream — for approvals that is the empty matrix, which still
    /// carries the invariant floor, never "no review"; for retention it is
    /// the product config, whose record horizons are all unset.
    pub config: PackConfig,
    /// When the pack was last applied.
    pub updated_at: DateTime<Utc>,
}

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant.
        if db.code().as_deref() == Some("23503") {
            return Error::NotFound {
                entity: "tenant".to_owned(),
            };
        }
        // 23514 check_violation: malformed name, reserved product name,
        // or empty source.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (ADR-0009).
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Applies a pack under the tenant's `name`: inserts at version 1, or
/// replaces the existing row and bumps its version — every apply is a new
/// version, so the reloader's unchanged-skip and the decision log both see
/// the change. `redaction: None` / `composition: None` clear any stored
/// config: an apply is a full statement of the pack, never a partial patch.
#[tracing::instrument(
    name = "store.policy_packs.apply",
    skip_all,
    fields(tenant.id = %tenant_id, policy.pack = name),
    err(Display)
)]
pub async fn apply(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    name: &str,
    source: &str,
    config: &PackConfig,
) -> Result<PolicyPack> {
    let config_json = |label: &str, value: serde_json::Result<serde_json::Value>| {
        value.map_err(|err| Error::Internal {
            message: format!("serialise {label} config: {err}"),
        })
    };
    let redaction_json = config
        .redaction
        .map(|value| config_json("redaction", serde_json::to_value(value)))
        .transpose()?;
    let composition_json = config
        .composition
        .map(|value| config_json("composition", serde_json::to_value(value)))
        .transpose()?;
    let approvals_json = config
        .approvals
        .as_ref()
        .map(|value| config_json("approvals", serde_json::to_value(value)))
        .transpose()?;
    let promotion_json = config
        .promotion
        .as_ref()
        .map(|value| config_json("promotion", serde_json::to_value(value)))
        .transpose()?;
    let lapse_json = config
        .lapse
        .map(|value| config_json("lapse", serde_json::to_value(value)))
        .transpose()?;
    let dedup_json = config
        .dedup
        .map(|value| config_json("dedup", serde_json::to_value(value)))
        .transpose()?;
    let retention_json = config
        .retention
        .map(|value| config_json("retention", serde_json::to_value(value)))
        .transpose()?;
    let scan_json = config
        .scan
        .map(|value| config_json("scan", serde_json::to_value(value)))
        .transpose()?;
    let row = sqlx::query_as!(
        PolicyPackRow,
        r#"
        insert into policy_packs
            (tenant_id, name, version, source, redaction, composition, approvals,
             promotion, lapse, dedup, retention, scan)
        values ($1, $2, 1, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        on conflict (tenant_id, name) do update
            set source = excluded.source,
                redaction = excluded.redaction,
                composition = excluded.composition,
                approvals = excluded.approvals,
                promotion = excluded.promotion,
                lapse = excluded.lapse,
                dedup = excluded.dedup,
                retention = excluded.retention,
                scan = excluded.scan,
                version = policy_packs.version + 1,
                updated_at = now()
        returning tenant_id, name, version, source, redaction, composition,
                  approvals, promotion, lapse, dedup, retention, scan, updated_at
        "#,
        tenant_id.as_uuid(),
        name,
        source,
        redaction_json,
        composition_json,
        approvals_json,
        promotion_json,
        lapse_json,
        dedup_json,
        retention_json,
        scan_json,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.into())
}

/// All of the tenant's stored packs — the refresher's reconciliation
/// input.
#[tracing::instrument(
    name = "store.policy_packs.stored",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn stored(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<PolicyPack>> {
    let rows = sqlx::query_as!(
        PolicyPackRow,
        r#"
        select tenant_id, name, version, source, redaction, composition,
               approvals, promotion, lapse, dedup, retention, scan, updated_at
        from policy_packs where tenant_id = $1
        order by name
        "#,
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// One stored pack by name.
#[tracing::instrument(
    name = "store.policy_packs.get",
    skip_all,
    fields(tenant.id = %tenant_id, policy.pack = name),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    name: &str,
) -> Result<Option<PolicyPack>> {
    let row = sqlx::query_as!(
        PolicyPackRow,
        r#"
        select tenant_id, name, version, source, redaction, composition,
               approvals, promotion, lapse, dedup, retention, scan, updated_at
        from policy_packs where tenant_id = $1 and name = $2
        "#,
        tenant_id.as_uuid(),
        name,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// Removes a stored pack, refusing while any assignment or the tenant
/// default still references it (ADR-0014 decision 7: the dangling-name
/// fallback exists for out-of-band writes, not for the product path).
/// Returns whether a row was removed.
#[tracing::instrument(
    name = "store.policy_packs.clear",
    skip_all,
    fields(tenant.id = %tenant_id, policy.pack = name),
    err(Display)
)]
pub async fn clear(conn: &mut PgConnection, tenant_id: TenantId, name: &str) -> Result<bool> {
    let referenced = sqlx::query_scalar!(
        r#"
        select exists (
            select 1 from policy_pack_assignments
            where tenant_id = $1 and pack_name = $2
            union all
            select 1 from policy_pack_defaults
            where tenant_id = $1 and pack_name = $2
        ) as "referenced!"
        "#,
        tenant_id.as_uuid(),
        name,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    if referenced {
        return Err(Error::Conflict {
            message: format!(
                "pack {name:?} is still assigned (or the tenant default); \
                 reassign those scopes first"
            ),
        });
    }
    let result = sqlx::query!(
        "delete from policy_packs where tenant_id = $1 and name = $2",
        tenant_id.as_uuid(),
        name,
    )
    .execute(conn)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

struct PolicyPackRow {
    tenant_id: uuid::Uuid,
    name: String,
    version: i64,
    source: String,
    redaction: Option<serde_json::Value>,
    composition: Option<serde_json::Value>,
    approvals: Option<serde_json::Value>,
    promotion: Option<serde_json::Value>,
    lapse: Option<serde_json::Value>,
    dedup: Option<serde_json::Value>,
    retention: Option<serde_json::Value>,
    scan: Option<serde_json::Value>,
    updated_at: DateTime<Utc>,
}

impl From<PolicyPackRow> for PolicyPack {
    fn from(row: PolicyPackRow) -> Self {
        // [`apply`] validated the configs on the way in; unparseable
        // stored json can only come from out-of-band writes. Fail safe:
        // treat them as unconfigured (each config's default downstream),
        // loudly.
        fn parse_config<T: serde::de::DeserializeOwned>(
            pack: &str,
            label: &str,
            value: Option<serde_json::Value>,
        ) -> Option<T> {
            value.and_then(|value| {
                serde_json::from_value(value)
                    .inspect_err(|err| {
                        tracing::warn!(
                            policy.pack = %pack,
                            error = %err,
                            "stored {label} config does not parse; \
                             treating the pack as unconfigured (default)"
                        );
                    })
                    .ok()
            })
        }
        let redaction = parse_config(&row.name, "redaction", row.redaction);
        let composition = parse_config(&row.name, "composition", row.composition);
        let approvals: Option<ApprovalMatrix> = parse_config(&row.name, "approvals", row.approvals)
            // A matrix that parses but cannot be satisfied is worse
            // than none: it would deny every proposal at the cells it
            // governs with no error anywhere. Treat it as
            // unconfigured — the floor still applies — and say so.
            .and_then(|matrix: ApprovalMatrix| {
                matrix
                    .validate()
                    .inspect_err(|err| {
                        tracing::warn!(
                            policy.pack = %row.name,
                            error = %err,
                            "stored approval matrix is unsatisfiable; \
                             treating the pack as unconfigured (floor only)"
                        );
                    })
                    .ok()
                    .map(|()| matrix)
            });
        // A rule set that parses but asks nothing (or asks the
        // impossible) would fire on everything or never — and unlike the
        // matrix, whose fail-safe is the floor, a trigger's fail-safe is
        // silence. Treat it as no rules, and say so.
        let promotion: Option<PromotionConfig> =
            parse_config(&row.name, "promotion", row.promotion).and_then(
                |config: PromotionConfig| {
                    config
                        .validate()
                        .inspect_err(|err| {
                            tracing::warn!(
                                policy.pack = %row.name,
                                error = %err,
                                "stored promotion config is invalid; \
                                 treating the pack as carrying no rules"
                            );
                        })
                        .ok()
                        .map(|()| config)
                },
            );
        // A ceiling above the product maximum is refused at apply; one that
        // reached the row anyway is treated as unconfigured — the strict
        // 30-day window — rather than clamped in silence, so the warning is
        // what an operator sees instead of a window they did not choose.
        // `resolved_max_secs` bounds it either way (ADR-0037 decision 5).
        let lapse: Option<LapseConfig> =
            parse_config(&row.name, "lapse", row.lapse).and_then(|config: LapseConfig| {
                config
                    .validate()
                    .inspect_err(|err| {
                        tracing::warn!(
                            policy.pack = %row.name,
                            error = %err,
                            "stored lapse config is invalid; \
                             treating the pack as unconfigured (the strict window)"
                        );
                    })
                    .ok()
                    .map(|()| config)
            });
        // Thresholds outside `0..=1` make a band unreachable, which reads
        // as "the feature is off" without saying so. Unconfigured is the
        // product config, which is the same fail-safe the composition
        // config takes: this config withholds nothing a reader could
        // otherwise have seen (ADR-0039 decision 12).
        let dedup: Option<DedupConfig> =
            parse_config(&row.name, "dedup", row.dedup).and_then(|config: DedupConfig| {
                config
                    .validate()
                    .inspect_err(|err| {
                        tracing::warn!(
                            policy.pack = %row.name,
                            error = %err,
                            "stored dedup config is invalid; \
                             treating the pack as unconfigured (the product config)"
                        );
                    })
                    .ok()
                    .map(|()| config)
            });
        // A horizon that reached the row anyway — a schedule written in
        // seconds, a staging horizon that would spend MEM-1's idempotency
        // guarantee — is treated as unconfigured rather than clamped: the
        // product config destroys nothing, so the fail-safe is the one
        // that cannot delete a tenant's memory (ADR-0040 decision 13).
        let retention: Option<RetentionConfig> =
            parse_config(&row.name, "retention", row.retention).and_then(
                |config: RetentionConfig| {
                    config
                        .validate()
                        .inspect_err(|err| {
                            tracing::warn!(
                                policy.pack = %row.name,
                                error = %err,
                                "stored retention config is invalid; \
                                 treating the pack as unconfigured (the product config)"
                            );
                        })
                        .ok()
                        .map(|()| config)
                },
            );
        // A threshold that does not parse is treated as unconfigured
        // like every other config here, and unconfigured is the floor —
        // so the fail-safe direction is the safe one by construction:
        // `critical` still refuses, and only the band a pack was trying
        // to tighten is lost (ADR-0052 decision 9).
        let scan = parse_config(&row.name, "scan", row.scan);
        PolicyPack {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            name: row.name,
            version: row.version,
            source: row.source,
            config: PackConfig {
                redaction,
                composition,
                approvals,
                promotion,
                lapse,
                dedup,
                retention,
                scan,
            },
            updated_at: row.updated_at,
        }
    }
}
