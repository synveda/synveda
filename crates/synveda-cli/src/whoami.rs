//! `synveda whoami` — which identity is acting, and (with
//! `--capabilities`) what it may do on the tenant plane (CNSL-2, ADR-0058
//! decision 8).
//!
//! `Api::connect` already prints the acting subject on every governed verb,
//! but only as a side effect of doing something else. The first question
//! anybody asks a deployment they have just logged into is "who am I here",
//! and until this verb the answer was a `curl`.

use serde::Deserialize;

use crate::api::{Api, Origin};

#[derive(Deserialize)]
struct WhoamiView {
    subject: String,
    tenant: TenantView,
    capabilities: Option<TenantCapabilitiesView>,
}

#[derive(Deserialize)]
struct TenantView {
    id: String,
    slug: String,
    name: String,
    status: String,
}

#[derive(Deserialize)]
struct TenantCapabilitiesView {
    roles: Vec<String>,
    actions: std::collections::BTreeMap<String, bool>,
    role_assign: std::collections::BTreeMap<String, bool>,
}

/// `synveda whoami [--capabilities]`.
pub async fn show(profile: &str, capabilities: bool, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    let path = if capabilities {
        "/v1/whoami?capabilities=true"
    } else {
        "/v1/whoami"
    };
    if json_out {
        println!("{}", api.get(path).await?);
        return Ok(());
    }
    let view: WhoamiView = api.get_as(path).await?;
    println!("{}", view.subject);
    println!(
        "  tenant  {} ({}) — {}",
        view.tenant.slug, view.tenant.name, view.tenant.status
    );
    println!("  id      {}", view.tenant.id);
    match origin {
        Origin::Profile(name) => println!("  via     profile {name}"),
        Origin::Environment => println!("  via     SYNVEDA_TOKEN"),
    }

    let Some(block) = view.capabilities else {
        return Ok(());
    };
    println!(
        "\ntenant-wide roles: {}",
        if block.roles.is_empty() {
            "—".to_owned()
        } else {
            block.roles.join(", ")
        }
    );
    // The tenant plane is a much shorter vocabulary than a scope's, and
    // that is honest rather than partial: most actions in this product are
    // only ever taken at a node. Saying so beats a reader wondering what
    // happened to the other twenty.
    let allowed: Vec<&str> = block
        .actions
        .iter()
        .filter(|(_, permitted)| **permitted)
        .map(|(name, _)| name.as_str())
        .collect();
    println!("\nmay, tenant-wide:");
    if allowed.is_empty() {
        println!("  — nothing (roles here are bound at nodes, not at the tenant)");
    }
    for name in &allowed {
        println!("  {name}");
    }
    let bindable: Vec<&str> = block
        .role_assign
        .iter()
        .filter(|(_, permitted)| **permitted)
        .map(|(name, _)| name.as_str())
        .collect();
    if !bindable.is_empty() {
        println!("\nmay bind tenant-wide: {}", bindable.join(", "));
    }
    println!(
        "\n{} tenant action(s) denied. A forecast, not a grant: every act \
         decides again at its own seam. Ask a node with \
         `synveda hierarchy capabilities <id>`.",
        block.actions.len() - allowed.len(),
    );
    Ok(())
}
