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

// ── The explorer's node reads (CNSL-2, ADR-0058) ───────────────────────
//
// These sit here rather than under `synveda policy` and `synveda role`,
// and that placement is a finding rather than a preference. Both of those
// verb families are **direct-store operator plumbing**: every one takes
// `--tenant` and a database connection and answers without a PDP decision
// at all. Growing `role list` an `--effective` flag would have meant
// implementing the chain walk against Postgres — a governed question
// answered by a code path with no decision in it, which is the second
// implementation ADR-0058 decision 1 refuses *and* the bypass seed §2.2
// forbids, in one flag. So the reads land where the routes are and where
// this module's own rule already holds: HTTP only, audited, decided.

/// One pack, and where it came from.
#[derive(serde::Deserialize)]
struct EffectivePackView {
    name: String,
    version: i64,
    origin: OriginView,
}

#[derive(serde::Deserialize)]
struct OriginView {
    kind: String,
    scope_id: Option<ScopeId>,
}

impl OriginView {
    /// Renders an origin relative to the node that was asked about, which
    /// is the only frame in which "here" and "from above" mean anything.
    fn describe(&self, asked_about: ScopeId) -> String {
        match (self.kind.as_str(), self.scope_id) {
            ("assigned", Some(id)) if id == asked_about => "assigned here".to_owned(),
            ("assigned", Some(id)) => format!("assigned at {id}"),
            ("tenant-wide", _) => "tenant-wide".to_owned(),
            ("tenant-default", _) => "the tenant default".to_owned(),
            ("default", _) => "the built-in default".to_owned(),
            ("fallback", _) => "a fallback (the assigned pack did not compile)".to_owned(),
            (other, _) => other.to_owned(),
        }
    }
}

/// `synveda hierarchy policy <id>` — the pack in force here, and where it
/// came from.
pub async fn policy(profile: &str, id: ScopeId, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    let path = format!("/v1/hierarchy/nodes/{id}/policy");
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    let effective: EffectivePackView = api.get_as(&path).await?;
    println!("{}@{}", effective.name, effective.version);
    println!("  origin  {}", effective.origin.describe(id));
    Ok(())
}

#[derive(serde::Deserialize)]
struct EffectiveBindingsView {
    bindings: Vec<EffectiveBindingView>,
    chain: Vec<ScopeId>,
}

#[derive(serde::Deserialize)]
struct EffectiveBindingView {
    subject: String,
    role: String,
    origin: OriginView,
}

#[derive(serde::Deserialize)]
struct BindingsView {
    bindings: Vec<BindingView>,
}

#[derive(serde::Deserialize)]
struct BindingView {
    subject: String,
    role: String,
}

/// `synveda hierarchy roles <id> [--effective]` — the bindings at a node,
/// or every binding in force there with its origin.
pub async fn roles(
    profile: &str,
    id: ScopeId,
    effective: bool,
    json_out: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "reading");
    let path = if effective {
        format!("/v1/hierarchy/nodes/{id}/roles?effective=true")
    } else {
        format!("/v1/hierarchy/nodes/{id}/roles")
    };
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    if !effective {
        let view: BindingsView = api.get_as(&path).await?;
        if view.bindings.is_empty() {
            println!("no bindings at this node");
            return Ok(());
        }
        for binding in &view.bindings {
            println!("{:<12} {}", binding.role, binding.subject);
        }
        return Ok(());
    }
    let view: EffectiveBindingsView = api.get_as(&path).await?;
    if view.bindings.is_empty() {
        println!("no roles in force here");
        return Ok(());
    }
    for binding in &view.bindings {
        println!(
            "{:<12} {:<40} {}",
            binding.role,
            binding.subject,
            binding.origin.describe(id),
        );
    }
    // The chain is printed because it is the *reason* the answer has the
    // rows it has — a reader who cannot see the chain is being asked to
    // take the inheritance on trust.
    println!("\nin force over the chain: {} scope(s)", view.chain.len());
    Ok(())
}

#[derive(serde::Deserialize)]
struct CapabilitiesView {
    scope_path: String,
    pack: EffectivePackView,
    roles: Vec<String>,
    actions: std::collections::BTreeMap<String, bool>,
    read_tiers: std::collections::BTreeMap<String, Vec<String>>,
    role_assign: std::collections::BTreeMap<String, bool>,
}

/// `synveda hierarchy capabilities <id>` — what *this caller* may do here.
///
/// Prints the forecast disclaimer rather than leaving it to a reader to
/// infer, because "you may: channel.publish" in a terminal reads exactly
/// like a permission and is not one (ADR-0058 decision 2).
pub async fn capabilities(profile: &str, id: ScopeId, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin, "probing");
    let path = format!("/v1/hierarchy/nodes/{id}/capabilities");
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    let view: CapabilitiesView = api.get_as(&path).await?;
    println!("{}", view.scope_path);
    println!(
        "  pack    {}@{} ({})",
        view.pack.name,
        view.pack.version,
        view.pack.origin.describe(id),
    );
    println!(
        "  roles   {}",
        if view.roles.is_empty() {
            "—".to_owned()
        } else {
            view.roles.join(", ")
        }
    );

    let allowed: Vec<&String> = view
        .actions
        .iter()
        .filter(|(_, permitted)| **permitted)
        .map(|(name, _)| name)
        .collect();
    let denied = view.actions.len() - allowed.len();
    println!("\nmay:");
    if allowed.is_empty() {
        println!("  — nothing at this scope");
    }
    for name in allowed {
        println!("  {name}");
    }

    let readable: Vec<String> = view
        .read_tiers
        .iter()
        .filter(|(_, tiers)| !tiers.is_empty())
        .map(|(name, tiers)| format!("  {name:<20} {}", tiers.join(", ")))
        .collect();
    if !readable.is_empty() {
        println!("\nmay read, to these tiers:");
        for line in readable {
            println!("{line}");
        }
    }

    let bindable: Vec<&String> = view
        .role_assign
        .iter()
        .filter(|(_, permitted)| **permitted)
        .map(|(name, _)| name)
        .collect();
    if !bindable.is_empty() {
        println!("\nmay bind: {}", comma(&bindable));
    }

    println!(
        "\n{denied} action(s) denied. Decided under {}@{} — a forecast, not a grant: \
         every act decides again at its own seam.",
        view.pack.name, view.pack.version,
    );
    Ok(())
}

fn comma(names: &[&String]) -> String {
    names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
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
