//! `synveda prompt` — the registry from a terminal (PRMT-1, ADR-0049).
//!
//! HTTP-only, on FLOW-6's precedent (ADR-0035 decision 1): authoring is a
//! `PromptWrite` decision, resolution is a `PromptRead` decision at the tier
//! the served version carries, and both chain their own event under the
//! caller's identity. A CLI that wrote the row itself would leave no
//! decision in the trail and would have to invent an author, so this module
//! opens no database connection and its verbs take no `--database-url`.
//!
//! Reviewing and publishing are deliberately *not* here: they are
//! `synveda proposal`'s, unchanged, because a prompt proposal is an
//! ordinary proposal (ADR-0049 decision 6). The one thing this adds to the
//! review flow is `propose`, which the memory path leaves to a contributor's
//! POST — an authored asset has no pipeline to open one for it.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::json;
use synveda_types::{PromptTemplate, PromptVariable, ScopeId, Sensitivity};

use crate::api::{Api, Origin};

// ── The wire shapes (`crates/synveda-gateway/src/prompts.rs`) ──────────

#[derive(Deserialize)]
struct Resolved {
    name: String,
    scope_id: ScopeId,
    scope_path: String,
    channel: String,
    origin: String,
    commit: Option<String>,
    object_hash: String,
    sensitivity: Sensitivity,
    description: String,
    template: String,
    variables: Vec<PromptVariable>,
}

#[derive(Deserialize)]
struct Listing {
    scope_path: String,
    prompts: Vec<ListEntry>,
}

#[derive(Deserialize)]
struct ListEntry {
    name: String,
    description: String,
    sensitivity: Sensitivity,
    object_hash: String,
    variables: Vec<PromptVariable>,
    published: Option<Published>,
}

#[derive(Deserialize)]
struct Published {
    commit: String,
    current: bool,
}

/// `synveda prompt list --scope <id>` — the registry at one scope: what is
/// drafted, what is published, and whether they are the same bytes.
pub async fn list(profile: &str, scope: ScopeId) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let listing: Listing = api.get_as(&format!("/v1/prompts?scope_id={scope}")).await?;
    if listing.prompts.is_empty() {
        println!("no prompts at {}", listing.scope_path);
        return Ok(());
    }
    println!("prompts at {}\n", listing.scope_path);
    for entry in &listing.prompts {
        // The mark is the answer to "is what I would be served the thing
        // I last wrote": `✓` published and current, `~` published and
        // behind the draft, `·` never reviewed.
        let (mark, note) = match &entry.published {
            Some(published) if published.current => {
                ("✓", format!("published {}", short(&published.commit)))
            }
            Some(published) => (
                "~",
                format!(
                    "published {} — the draft has moved since",
                    short(&published.commit)
                ),
            ),
            None => ("·", "draft only — no review has carried it".to_owned()),
        };
        println!(
            "  {mark} {}  [{}]  {}",
            entry.name,
            entry.sensitivity.as_str(),
            entry.description
        );
        println!(
            "      {note}  draft {}  {} variable(s)",
            short(&entry.object_hash),
            entry.variables.len()
        );
    }
    Ok(())
}

/// What a `show` is asking for.
pub struct Ask<'a> {
    /// The prompt's name — the identifier a consumer writes in its source.
    pub name: &'a str,
    /// Which scope, when naming one. Absent walks the caller's own chain.
    pub scope: Option<ScopeId>,
    /// `draft` for the authoring copy at a named scope.
    pub draft: bool,
    /// A commit to pin to: the version this caller was built against.
    pub commit: Option<&'a str>,
    /// Values to render with, `name=value`.
    pub values: &'a [String],
}

/// `synveda prompt show <name>` — resolve, and optionally render.
pub async fn show(profile: &str, ask: Ask<'_>, json_out: bool, quiet: bool) -> Result<(), String> {
    let mut query: Vec<String> = Vec::new();
    if let Some(scope) = ask.scope {
        query.push(format!("scope_id={scope}"));
    }
    if ask.draft {
        query.push("channel=draft".to_owned());
    }
    if let Some(commit) = ask.commit {
        query.push(format!("commit={commit}"));
    }
    let path = match query.is_empty() {
        true => format!("/v1/prompts/{}", ask.name),
        false => format!("/v1/prompts/{}?{}", ask.name, query.join("&")),
    };

    let (api, origin) = Api::connect(profile).await?;
    if !quiet {
        announce(&api, &origin);
    }
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    let resolved: Resolved = api.get_as(&path).await?;

    // Where the bytes came from, said plainly: a response that cites a
    // frozen commit without saying so overstates its own freshness
    // (ADR-0036 decision 10, applied to a fetch).
    let source = match resolved.origin.as_str() {
        "pinned-commit" => " (pinned by this request)".to_owned(),
        "channel-pin" => " (the scope's channel is pinned)".to_owned(),
        "draft" => " (unreviewed)".to_owned(),
        _ => String::new(),
    };
    println!(
        "── {} [{}] {} at {}{}",
        resolved.name,
        resolved.sensitivity.as_str(),
        resolved.channel,
        resolved.scope_path,
        source
    );
    println!(
        "   {}  {}{}",
        resolved.description,
        resolved
            .commit
            .as_deref()
            .map(short)
            .unwrap_or_else(|| "no commit".to_owned()),
        format_args!("  object {}", short(&resolved.object_hash)),
    );
    println!("   scope {}", resolved.scope_id);
    println!();

    if ask.values.is_empty() {
        for variable in &resolved.variables {
            let requirement = match &variable.default {
                Some(default) => format!("optional, default {default:?}"),
                None => "required".to_owned(),
            };
            println!(
                "   {{{{ {} }}}}  {requirement}{}",
                variable.name,
                variable
                    .description
                    .as_deref()
                    .map(|text| format!(" — {text}"))
                    .unwrap_or_default()
            );
        }
        if !resolved.variables.is_empty() {
            println!();
        }
        println!("{}", resolved.template);
        return Ok(());
    }

    // Rendering is `synveda_types`' own rule, not a second implementation
    // (ADR-0049 decision 12): a missing required value and an undeclared
    // one are both refusals here, before anything reaches a model.
    let template = PromptTemplate {
        name: resolved.name.parse().map_err(|err| format!("{err}"))?,
        description: resolved.description,
        template: resolved.template,
        variables: resolved.variables,
    };
    let mut values = BTreeMap::new();
    for pair in ask.values {
        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("--var takes name=value; {pair:?} has no '='"))?;
        values.insert(name.to_owned(), value.to_owned());
    }
    println!(
        "{}",
        template.render(&values).map_err(|err| err.to_string())?
    );
    Ok(())
}

/// What an `author` is writing.
pub struct Draft<'a> {
    pub name: &'a str,
    pub scope: ScopeId,
    pub description: &'a str,
    pub template: String,
    /// `name`, or `name=default` for an optional variable.
    pub variables: &'a [String],
    pub sensitivity: Option<Sensitivity>,
}

/// `synveda prompt author <name>` — write the draft. This moves nothing a
/// consumer reads: the published channel is somewhere else, and only the
/// approval matrix moves it.
pub async fn author(profile: &str, draft: Draft<'_>) -> Result<(), String> {
    let mut variables = Vec::with_capacity(draft.variables.len());
    for declaration in draft.variables {
        let (name, default) = match declaration.split_once('=') {
            Some((name, default)) => (name, Some(default.to_owned())),
            None => (declaration.as_str(), None),
        };
        variables.push(json!({"name": name, "default": default}));
    }
    let mut body = json!({
        "scope_id": draft.scope,
        "name": draft.name,
        "description": draft.description,
        "template": draft.template,
        "variables": variables,
    });
    if let Some(sensitivity) = draft.sensitivity {
        body["sensitivity"] = json!(sensitivity.as_str());
    }

    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let response = api.post("/v1/prompts", Some(body)).await?;
    let published = &response["published"];
    println!(
        "synveda: wrote {} at {}  draft {}",
        response["name"].as_str().unwrap_or(draft.name),
        response["scope_path"].as_str().unwrap_or_default(),
        short(response["object_hash"].as_str().unwrap_or_default()),
    );
    // The line that makes "behind review" visible from the writing side.
    if published.is_null() {
        println!("         nothing is published under this name yet — open a proposal");
    } else if published["current"] == json!(false) {
        println!(
            "         consumers are still served {} — this edit reaches them \
             through review",
            short(published["commit"].as_str().unwrap_or_default())
        );
    } else {
        println!(
            "         published and current at {}",
            short(published["commit"].as_str().unwrap_or_default())
        );
    }
    Ok(())
}

/// `synveda prompt propose <name> --scope <id>` — open the review that can
/// carry it across the trust boundary.
///
/// The verb FLOW-6 deliberately left out for memory (ADR-0035: "opening is
/// the proposer's act, and the AC is about review"). An authored asset has
/// no pipeline to open one for it, so the author does — and everything
/// after this is `synveda proposal` unchanged.
pub async fn propose(
    profile: &str,
    name: &str,
    scope: ScopeId,
    title: Option<&str>,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let response = api
        .post(
            "/v1/proposals",
            Some(json!({
                "scope_id": scope,
                "prompt_names": [name],
                "title": title.unwrap_or(name),
            })),
        )
        .await?;
    let id = response["id"].as_str().unwrap_or_default();
    println!(
        "synveda: proposal {id} open at {}",
        response["target_scope_path"].as_str().unwrap_or_default()
    );
    println!(
        "         requires {}",
        response["outstanding"].as_str().unwrap_or("nothing")
    );
    println!("         review it with `synveda proposal show {id}`");
    Ok(())
}

/// Which identity is asking — the `synveda proposal` discipline
/// (ADR-0035): never leave a caller guessing whose access answered.
fn announce(api: &Api, origin: &Origin) {
    match origin {
        Origin::Profile(profile) => eprintln!(
            "synveda: {} as {} (profile {profile})",
            api.gateway(),
            api.subject
        ),
        Origin::Environment => eprintln!(
            "synveda: {} as {} (SYNVEDA_TOKEN)",
            api.gateway(),
            api.subject
        ),
    }
}

/// The first twelve hex characters — enough to name a commit or an object
/// in a terminal, as `synveda channel history` renders them.
fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}
