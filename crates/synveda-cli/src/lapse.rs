//! `synveda lapse` — what is currently relaxed, and over what (CNSL-2,
//! ADR-0058 decision 8).
//!
//! The lapse machinery is this product's whole answer to seed §2.3's
//! "strict by default, relaxable by design", and until this verb existed
//! there was **no terminal in which to ask what was relaxed**. AUTHZ-4
//! shipped propose, grant and revoke as routes; FLOW-6's `synveda proposal`
//! reviews a lapse proposal because a lapse *is* a proposal; and the
//! standing grant that came out the other end could be read by nothing.
//!
//! HTTP only, like `hierarchy` and for its reason: a listing assembled from
//! the database would answer a governed question with no decision in the
//! trail, and "who could read this scope's material in March" is exactly
//! the question an auditor asks about the answer as much as about the data.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use synveda_types::ScopeId;

use crate::api::{Api, Origin};

#[derive(Deserialize)]
pub struct Listing {
    lapses: Vec<LapseView>,
    /// Present only on the scope-free form.
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    max_lapses: Option<i64>,
    /// Present only on the scoped form.
    #[serde(default)]
    scope_path: Option<String>,
}

#[derive(Deserialize)]
pub struct LapseView {
    id: String,
    grantee_scope_id: ScopeId,
    target_scope_id: ScopeId,
    grantee_scope_path: Option<String>,
    target_scope_path: Option<String>,
    action: String,
    reason: String,
    expires_at: DateTime<Utc>,
    outcome: String,
}

impl LapseView {
    /// An end's path when this caller may read that end, its id otherwise.
    ///
    /// A grant is visible from one end without the other end's path
    /// becoming the caller's to know (ADR-0058 decision 7), so the id is
    /// what is left — enough to name the row in a revoke, not enough to
    /// learn where in the organisation it sits.
    fn grantee(&self) -> String {
        end(self.grantee_scope_path.as_deref(), self.grantee_scope_id)
    }

    fn target(&self) -> String {
        end(self.target_scope_path.as_deref(), self.target_scope_id)
    }
}

fn end(path: Option<&str>, id: ScopeId) -> String {
    path.map_or_else(|| format!("<{id}>"), ToOwned::to_owned)
}

/// `synveda lapse list [--scope <id>] [--all]`.
///
/// With no `--scope` this is the standing set anywhere the caller may
/// read — the question "what is relaxed right now". With one it is that
/// scope's history, `--all` or not, because the scoped form has always
/// answered "who could read this in March" and that is a question about
/// grants that have ended.
pub async fn list(
    profile: &str,
    scope: Option<ScopeId>,
    all: bool,
    json_out: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let path = match (scope, all) {
        (Some(id), false) => format!("/v1/lapses?scope_id={id}&active=true"),
        (Some(id), true) => format!("/v1/lapses?scope_id={id}"),
        (None, false) => "/v1/lapses".to_owned(),
        (None, true) => "/v1/lapses?active=false".to_owned(),
    };
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    let listing: Listing = api.get_as(&path).await?;
    println!("{}", render_lapses(&listing, all));
    Ok(())
}

/// The listing, as a value.
///
/// A `String` rather than a `println!` so the parity corpus can read it —
/// ADR-0058 decision 10 asserts both renderers name the same facts about
/// one payload, and a renderer that exists only as a side effect on stdout
/// cannot be asserted against anything.
pub fn render_lapses(listing: &Listing, all: bool) -> String {
    let mut out = String::new();
    if let Some(scope_path) = &listing.scope_path {
        out.push_str(&format!("grants over {scope_path}\n\n"));
    }
    if listing.lapses.is_empty() {
        out.push_str(&format!(
            "no grants{}\n",
            if all { "" } else { " standing" }
        ));
        return out;
    }
    for lapse in &listing.lapses {
        // The reason is the point of a lapse — a grant with no reason is
        // the thing ADR-0037 exists to make impossible — so it is on the
        // line rather than behind `--json`.
        out.push_str(&format!(
            "{}  {:<9}  {} → {}\n",
            &lapse.id[..8.min(lapse.id.len())],
            lapse.outcome,
            lapse.grantee(),
            lapse.target(),
        ));
        out.push_str(&format!(
            "  {} until {}  — {}\n",
            lapse.action,
            lapse.expires_at.format("%Y-%m-%d %H:%M UTC"),
            lapse.reason,
        ));
    }
    out.push_str(&format!("\n{} grant(s)\n", listing.lapses.len()));
    if listing.truncated {
        // Never a silent cap (ADR-0058 decision 5).
        out.push_str(&format!(
            "warning: truncated at {} — narrow with --scope\n",
            listing.max_lapses.unwrap_or_default(),
        ));
    }
    out
}

fn announce(api: &Api, origin: &Origin) {
    match origin {
        Origin::Profile(name) => eprintln!("reading as {} (profile {name})", api.subject),
        Origin::Environment => eprintln!("reading as {} (SYNVEDA_TOKEN)", api.subject),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_end_the_caller_may_not_read_shows_its_id_and_not_its_path() {
        let id = ScopeId::new();
        assert_eq!(end(None, id), format!("<{id}>"));
        assert_eq!(end(Some("acme/eng/platform"), id), "acme/eng/platform");
    }

    #[test]
    fn a_grant_visible_from_one_end_names_the_other_without_locating_it() {
        // The steward of a granted team may see the grant their team holds
        // without learning where the disclosing team sits in the org.
        let view = LapseView {
            id: "0199aa11-2222-7333-8444-555566667777".to_owned(),
            grantee_scope_id: ScopeId::new(),
            target_scope_id: ScopeId::new(),
            grantee_scope_path: Some("acme/eng/platform".to_owned()),
            target_scope_path: None,
            action: "memory.read".to_owned(),
            reason: "joint incident review".to_owned(),
            expires_at: Utc::now(),
            outcome: "active".to_owned(),
        };
        assert_eq!(view.grantee(), "acme/eng/platform");
        assert!(view.target().starts_with('<'), "the far end is an id");
        assert!(
            !view.target().contains('/'),
            "and never a path: {}",
            view.target()
        );
    }
}
