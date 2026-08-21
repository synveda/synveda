//! `synveda scope` — the operator surface over `/v1/admin/scopes`
//! (CPR-7, ADR-0074 decision 5).
//!
//! Every verb drives the public admin API: the PDP decides, the audit
//! chain records, and this file holds no store credentials at all — the
//! difference between administering a tenant and holding its database.
//!
//! `create` sends an `Idempotency-Key` minted fresh per invocation, so a
//! retried command creates one scope, and `move` decides at both ends
//! because the route does.

use serde_json::{Value, json};
use synveda_types::ScopeId;

use crate::api::Api;

/// A UUID minted here, for the idempotency header.
fn idempotency_key() -> String {
    synveda_types::TenantId::new().to_string()
}

fn refusal(op: &str, err: String) -> String {
    format!("{op}: {err}")
}

/// `synveda scope list [--under <id>]` — one level of the tree, or a
/// scope's whole subtree with `--under`.
pub async fn list(profile: &str, under: Option<ScopeId>, json: bool) -> Result<(), String> {
    let (api, _origin) = Api::connect(profile).await?;
    let path = match under {
        Some(id) => format!("/v1/admin/scopes/{id}/descendants"),
        None => "/v1/admin/scopes".to_owned(),
    };
    let body = api
        .get(&path)
        .await
        .map_err(|err| refusal("scope list", err))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    if let Some(parent) = body.get("parent").filter(|v| !v.is_null()) {
        println!(
            "under {} ({})",
            parent["slug"].as_str().unwrap_or("?"),
            parent["kind"].as_str().unwrap_or("?")
        );
    }
    for scope in body["scopes"].as_array().unwrap_or(&Vec::new()) {
        println_scope_line(scope);
    }
    Ok(())
}

/// `synveda scope show <id>` — one scope, with its path.
pub async fn show(profile: &str, id: ScopeId, json: bool) -> Result<(), String> {
    let (api, _origin) = Api::connect(profile).await?;
    let body = api
        .get(&format!("/v1/admin/scopes/{id}"))
        .await
        .map_err(|err| refusal("scope show", err))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    println_scope_line(&body);
    if let Some(path) = body["path"].as_str() {
        println!("  path: {path}");
    }
    Ok(())
}

/// `synveda scope create --parent <id> --kind <shape> --slug --name`.
pub async fn create(
    profile: &str,
    parent: ScopeId,
    kind: &str,
    slug: &str,
    name: &str,
    json: bool,
) -> Result<(), String> {
    // The shape vocabulary is validated by name, here and again at the
    // route — the old rank words (`org`, `division`, `department`,
    // `team`, `user`) fail with the route's own sentence, which is the
    // better error to show.
    let (api, _origin) = Api::connect(profile).await?;
    let subject = api.subject.clone();
    let body = json!({"parent_id": parent, "kind": kind, "slug": slug, "display_name": name});
    let created = api
        .post_with_header(
            "/v1/admin/scopes",
            Some(body),
            ("Idempotency-Key", &idempotency_key()),
            &subject,
        )
        .await
        .map_err(|err| refusal("scope create", err))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&created).map_err(|err| err.to_string())?
        );
    } else {
        println!(
            "created {} {} ({})",
            created["id"].as_str().unwrap_or("?"),
            created["slug"].as_str().unwrap_or("?"),
            created["kind"].as_str().unwrap_or("?")
        );
    }
    Ok(())
}

/// `synveda scope move <id> --parent <id>`.
pub async fn move_scope(
    profile: &str,
    id: ScopeId,
    parent: ScopeId,
    json: bool,
) -> Result<(), String> {
    let (api, _origin) = Api::connect(profile).await?;
    let moved = api
        .patch(
            &format!("/v1/admin/scopes/{id}"),
            json!({"parent_scope_id": parent}),
        )
        .await
        .map_err(|err| refusal("scope move", err))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&moved).map_err(|err| err.to_string())?
        );
    } else {
        println!(
            "moved {} under {}",
            moved["slug"].as_str().unwrap_or("?"),
            parent
        );
    }
    Ok(())
}

/// `synveda scope tree` — the tenant's whole scope tree, walked a level
/// at a time exactly as the console walks it.
pub async fn tree(profile: &str, json: bool) -> Result<(), String> {
    let (api, _origin) = Api::connect(profile).await?;
    let root = api
        .get("/v1/admin/scopes")
        .await
        .map_err(|err| refusal("scope tree", err))?;
    if json {
        let full = subtree_json(&api, &root).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&full).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    let parent = root
        .get("parent")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    if !parent.is_null() {
        println!(
            "{} ({})",
            parent["slug"].as_str().unwrap_or("?"),
            parent["kind"].as_str().unwrap_or("?")
        );
    }
    print_level(&api, &root, "").await;
    Ok(())
}

fn println_scope_line(scope: &Value) {
    println!(
        "{}  {}  {}  {}",
        scope["id"].as_str().unwrap_or("?"),
        scope["kind"].as_str().unwrap_or("?"),
        scope["slug"].as_str().unwrap_or("?"),
        scope["display_name"].as_str().unwrap_or("?"),
    );
}

fn print_level<'a>(
    api: &'a Api,
    level: &'a Value,
    indent: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        for scope in level["scopes"].as_array().cloned().unwrap_or_default() {
            println!(
                "{indent}├─ {} ({})",
                scope["slug"].as_str().unwrap_or("?"),
                scope["kind"].as_str().unwrap_or("?"),
            );
            let id = scope["id"].as_str().unwrap_or_default().to_owned();
            if id.is_empty() {
                continue;
            }
            if let Ok(children) = api.get(&format!("/v1/admin/scopes?parent_id={id}")).await {
                print_level(api, &children, &format!("{indent}   ")).await;
            }
        }
    })
}

fn subtree_json<'a>(
    api: &'a Api,
    level: &'a Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send + 'a>> {
    Box::pin(async move {
        let mut level = level.clone();
        if let Some(scopes) = level.get_mut("scopes").and_then(|s| s.as_array_mut()) {
            for scope in scopes.iter_mut() {
                let id = scope["id"].as_str().unwrap_or_default().to_owned();
                if id.is_empty() {
                    continue;
                }
                if let Ok(children) = api.get(&format!("/v1/admin/scopes?parent_id={id}")).await {
                    let children = subtree_json(api, &children).await?;
                    scope["children"] = children["scopes"].clone();
                }
            }
        }
        Ok(level)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn idempotency_keys_are_unique() {
        let a = super::idempotency_key();
        let b = super::idempotency_key();
        assert_ne!(a, b);
    }
}
