//! `synveda hierarchy` — the scopes an organisation is made of (OPS-1,
//! ADR-0055 decision 3).
//!
//! HTTP-only, on FLOW-6's precedent and for the same reason `recall` and
//! `proposal` are: creating a department is a governed act whose
//! `HierarchyCreate` decision the PDP takes at the *parent* scope and whose
//! `hierarchy.node.created` event the gateway chains under the caller's own
//! identity. A CLI that inserted the row itself would leave no decision in
//! the trail — and since the installer is the one caller that runs before
//! anybody is watching, that is the last place the product can afford one
//! (seed §2.2). So this module opens no database connection and the verbs
//! take no `--database-url`.
//!
//! The org root is deliberately not creatable here. It arrives with the
//! first admin login, from the tenant's own slug and name, inside AUTH-2's
//! provisioning transaction (ADR-0055 decision 2) — so `create` always has
//! a parent, and the one node whose creation has no operator is the one
//! node no operator creates.

use serde_json::json;
use synveda_types::{HierarchyNode, ScopeId, ScopeKind};

use crate::api::{Api, Origin};

/// What one `create` asks for.
pub struct NewNode<'a> {
    /// The parent scope. Required: see the module note about the root.
    pub parent: ScopeId,
    pub kind: ScopeKind,
    pub slug: &'a str,
    pub name: &'a str,
}

/// `synveda hierarchy create` — one scope, under a parent the caller may
/// write to.
pub async fn create(profile: &str, new: NewNode<'_>, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "creating");
    let body = json!({
        "parent_id": new.parent,
        "kind": new.kind,
        "slug": new.slug,
        "name": new.name,
    });
    if json_out {
        println!("{}", api.post("/v1/hierarchy/nodes", Some(body)).await?);
        return Ok(());
    }
    let node: HierarchyNode = api.post_as("/v1/hierarchy/nodes", Some(body)).await?;
    println!("{}  {}  {}", node.id, node.kind, node.path);
    Ok(())
}

/// `synveda hierarchy show <id>` — one node.
pub async fn show(profile: &str, id: ScopeId, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    let path = format!("/v1/hierarchy/nodes/{id}");
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    let node: HierarchyNode = api.get_as(&path).await?;
    println!("{}", node.id);
    println!("  kind    {}", node.kind);
    println!("  slug    {}", node.slug);
    println!("  name    {}", node.name);
    println!("  path    {}", node.path);
    println!("  depth   {}", node.depth);
    match node.parent_id {
        Some(parent) => println!("  parent  {parent}"),
        None => println!("  parent  — (org root)"),
    }
    Ok(())
}

/// `synveda hierarchy list [--under <id>]` — the subtree, as a tree.
///
/// With no anchor the tenant's own root answers, which is the shape an
/// operator wants immediately after `synveda login`: "what does my
/// organisation look like". Two governed reads, both audited.
pub async fn list(profile: &str, under: Option<ScopeId>, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    let anchor: HierarchyNode = match under {
        Some(id) => api.get_as(&format!("/v1/hierarchy/nodes/{id}")).await?,
        None => api.get_as("/v1/hierarchy/root").await?,
    };
    let path = format!("/v1/hierarchy/nodes/{}/descendants", anchor.id);
    let descendants: Vec<HierarchyNode> = api.get_as(&path).await?;
    if json_out {
        // The anchor is in the array: `list` means the subtree, and a
        // caller who has to fetch the root separately to learn what they
        // just listed has been handed half an answer.
        let mut subtree = vec![anchor];
        subtree.extend(descendants);
        println!(
            "{}",
            serde_json::to_string_pretty(&subtree).map_err(|err| err.to_string())?
        );
        return Ok(());
    }

    render(&anchor, anchor.depth);
    for node in &descendants {
        render(node, anchor.depth);
    }
    // A personal scope per person means a tenant of any size has more user
    // nodes than everything else combined; saying how many were drawn is
    // cheaper than making a reader count them.
    let people = descendants
        .iter()
        .filter(|node| node.kind == ScopeKind::User)
        .count();
    println!(
        "\n{} scope(s) under {} — {people} personal",
        descendants.len() + 1,
        anchor.path,
    );
    Ok(())
}

/// One line of the tree, indented by depth relative to the anchor. The
/// gateway returns descendants in path order, so indentation alone
/// reconstructs the shape without a second pass to build edges.
fn render(node: &HierarchyNode, anchor_depth: i32) {
    let indent = "  ".repeat((node.depth - anchor_depth).max(0) as usize);
    println!(
        "{indent}{} {}  {}  ({})",
        match node.kind {
            ScopeKind::Org => "▪",
            ScopeKind::Division | ScopeKind::Department => "▸",
            ScopeKind::Team => "·",
            ScopeKind::User => " ",
        },
        node.slug,
        node.name,
        node.id,
    );
}

/// Which identity is acting — the `synveda proposal` discipline
/// (ADR-0035): never leave a caller guessing whose access answered.
fn announce(api: &Api, origin: &Origin, verb: &str) {
    match origin {
        Origin::Profile(name) => eprintln!("{verb} as {} (profile {name})", api.subject),
        Origin::Environment => eprintln!("{verb} as {} (SYNVEDA_TOKEN)", api.subject),
    }
}

/// `synveda hierarchy root` — the org root's id, one line.
///
/// The scriptable half of `list`: every `create` needs a parent, and the
/// first parent anybody has is the root that their own first login
/// provisioned. Without this the documented way to find it is to read a
/// tree with your eyes, which is not a way to write a script.
pub async fn root(profile: &str, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    if json_out {
        println!("{}", api.get("/v1/hierarchy/root").await?);
        return Ok(());
    }
    let node: HierarchyNode = api.get_as("/v1/hierarchy/root").await?;
    println!("{}", node.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: ScopeKind, slug: &str, depth: i32, path: &str) -> HierarchyNode {
        HierarchyNode {
            id: ScopeId::new(),
            tenant_id: synveda_types::TenantId::new(),
            parent_id: None,
            kind,
            slug: slug.to_owned(),
            name: slug.to_owned(),
            depth,
            path: path.to_owned(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn indentation_is_relative_to_the_anchor_not_the_root() {
        // Listing `--under acme/eng` must not indent every line by two
        // levels of hierarchy the reader did not ask about.
        let anchor = node(ScopeKind::Department, "eng", 1, "acme/eng");
        let team = node(ScopeKind::Team, "platform", 2, "acme/eng/platform");
        assert_eq!((team.depth - anchor.depth).max(0), 1);
        assert_eq!((anchor.depth - anchor.depth).max(0), 0);
    }

    #[test]
    fn a_shallower_node_than_the_anchor_does_not_underflow() {
        // `descendants` cannot return one, but the cast is unsigned and a
        // negative would panic in release-mode `repeat` rather than fail a
        // test, so the clamp is asserted rather than assumed.
        let anchor = node(ScopeKind::Team, "platform", 2, "acme/eng/platform");
        let stray = node(ScopeKind::Org, "acme", 0, "acme");
        assert_eq!((stray.depth - anchor.depth).max(0), 0);
    }
}
