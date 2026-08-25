//! Public-API client for service identities (CPR-29, ADR-0088).
//!
//! Registering and revoking an agent are governed application acts. This
//! module deliberately has no store or database dependency: the gateway owns
//! scope ownership, the PDP decision, cache invalidation and the audit event.

use synveda_types::{IdentityId, ScopeId};

use crate::api::{Api, Origin};

/// `synveda service register`.
pub async fn register(
    profile: &str,
    subject: &str,
    scope: ScopeId,
    name: Option<&str>,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "registering");
    let mut body = serde_json::json!({"subject": subject, "scope_id": scope});
    if let Some(name) = name
        && let Some(object) = body.as_object_mut()
    {
        object.insert("display_name".to_owned(), name.into());
    }
    let identity = api.post("/v1/service-identities", Some(body)).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&identity).map_err(|err| err.to_string())?
    );
    Ok(())
}

/// `synveda service remove`.
pub async fn remove(profile: &str, id: IdentityId) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "revoking");
    api.delete(&format!("/v1/service-identities/{id}")).await?;
    println!("service identity {id} revoked");
    Ok(())
}

/// `synveda service list`.
pub async fn list(profile: &str) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    let identities = api.get("/v1/service-identities").await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&identities).map_err(|err| err.to_string())?
    );
    Ok(())
}

fn announce(api: &Api, origin: &Origin, verb: &str) {
    match origin {
        Origin::Profile(name) => eprintln!("{verb} as {} (profile {name})", api.subject),
        Origin::Environment => eprintln!("{verb} as {} (SYNVEDA_TOKEN)", api.subject),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn service_client_has_no_storage_authority() {
        let source = include_str!("service.rs");
        for forbidden in [
            concat!("synveda_", "store"),
            concat!("sql", "x"),
            concat!("DATABASE", "_URL"),
            concat!("record_", "break_glass"),
        ] {
            assert!(
                !source.contains(forbidden),
                "service client contains {forbidden}"
            );
        }
        for path in ["/v1/service-identities", "/v1/service-identities/{id}"] {
            assert!(source.contains(path), "service client lost {path}");
        }
    }
}
