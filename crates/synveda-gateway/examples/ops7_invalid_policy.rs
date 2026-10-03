//! OPS-7 isolated Kind fault injection. Never used by a product image or CLI.
//! A deliberately invalid test pack exercises the failed-compile lease path
//! without blocking the independent database-authority sentinel.

use std::error::Error;
use std::io::Read;
use std::str::FromStr;

use chrono::Utc;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_store::{epoch, policy_packs, rls};
use synveda_types::{PackConfig, TenantId};

const TENANT: &str = "019b53c0-7c00-7000-8000-000000000002";
const NAME: &str = "ops7-invalid-refresh";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mode = std::env::args().nth(1).ok_or("expected apply or clear")?;
    if !matches!(mode.as_str(), "apply" | "clear") || std::env::args().len() != 2 {
        return Err("expected exactly apply or clear".into());
    }
    let path = std::env::var("OPS7_DATABASE_URL_FILE")?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(4097)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err("test database URL must contain 1..=4096 bytes".into());
    }
    let url = String::from_utf8(bytes)?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(url.trim_end())
        .await?;
    epoch::verify(&pool).await?;
    let tenant = TenantId::from_str(TENANT)?;
    let mut tx = rls::begin_tenant_tx(&pool, tenant).await?;
    let action = if mode == "apply" {
        let pack =
            policy_packs::apply(&mut *tx, tenant, NAME, "permit (", &PackConfig::default()).await?;
        synveda_audit::append(
            &mut tx,
            tenant,
            &audit_event(tenant, AuditAction::PolicyPackApplied, Some(pack.version)),
        )
        .await?;
        "invalid test pack applied"
    } else {
        if policy_packs::clear(&mut tx, tenant, NAME).await? {
            synveda_audit::append(
                &mut tx,
                tenant,
                &audit_event(tenant, AuditAction::PolicyPackCleared, None),
            )
            .await?;
        }
        "invalid test pack cleared or absent"
    };
    tx.commit().await?;
    println!("OPS-7: {action}");
    pool.close().await;
    Ok(())
}

fn audit_event(tenant: TenantId, action: AuditAction, version: Option<i64>) -> AuditEvent {
    AuditEvent {
        occurred_at: Utc::now(),
        actor: Actor::break_glass("ops7-isolated-kind-fault"),
        action,
        resource: format!("tenant {tenant}"),
        outcome: Outcome::Success,
        payload: json!({"pack": NAME, "version": version, "fault": "invalid-cedar-test"}),
        trace_id: None,
    }
}
