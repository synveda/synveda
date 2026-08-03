//! `synveda context-pack` — the bundle registry from a terminal (PRMT-2,
//! ADR-0050).
//!
//! HTTP-only, on `synveda prompt`'s precedent (ADR-0035 decision 1):
//! authoring is a `ContextPackWrite` decision, listing is a
//! `ContextPackRead`, and both chain their own event under the caller's
//! identity. A CLI that wrote the rows itself would leave no decision in
//! the trail, would have to invent an author, and — here — would have to
//! chunk, scan and embed on the client, which is three chances for a
//! terminal to disagree with the server about what a document's address is.
//!
//! Reviewing and publishing are deliberately *not* here: they are
//! `synveda proposal`'s, unchanged, because a pack proposal is an ordinary
//! proposal (ADR-0050 decision 1). The one thing this adds to the review
//! flow is `propose`, for the reason the prompt module gives.
//!
//! **There is no `show <name>` verb that resolves a pack for a consumer**,
//! and that absence is the feature rather than a gap: a prompt is fetched
//! by name, and a pack's content arrives in a session through
//! `synveda inject`, ranked against everything else the reader may see.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::json;
use synveda_types::{ScopeId, Sensitivity};

use crate::api::{Api, Origin};

// ── The wire shapes (`crates/synveda-gateway/src/packs.rs`) ────────────

#[derive(Deserialize)]
struct Listing {
    scope_path: String,
    packs: Vec<PackEntry>,
}

#[derive(Deserialize)]
struct PackEntry {
    name: String,
    description: String,
    documents: Vec<DocumentEntry>,
}

#[derive(Deserialize)]
struct DocumentEntry {
    name: String,
    title: String,
    sensitivity: Sensitivity,
    object_hash: String,
    chunks: u32,
    published: Option<Published>,
}

#[derive(Deserialize)]
struct Published {
    commit: String,
    current: bool,
}

/// `synveda context-pack list --scope <id>` — the registry at one scope:
/// what is drafted, what is published, and whether they are the same bytes.
pub async fn list(profile: &str, scope: ScopeId) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let listing: Listing = api
        .get_as(&format!("/v1/context-packs?scope_id={scope}"))
        .await?;
    if listing.packs.is_empty() {
        println!("no context packs at {}", listing.scope_path);
        return Ok(());
    }
    println!("context packs at {}\n", listing.scope_path);
    for pack in &listing.packs {
        let chunks: u32 = pack.documents.iter().map(|document| document.chunks).sum();
        println!(
            "  {}  {}  ({} document(s), {chunks} chunk(s))",
            pack.name,
            pack.description,
            pack.documents.len(),
        );
        for document in &pack.documents {
            // The mark is the answer to "is what a session would compose
            // the thing I last wrote": `✓` published and current, `~`
            // published and behind the draft — which is also exactly when
            // the *old* version's chunks are the ones still composing
            // (ADR-0050 decision 3) — and `·` never reviewed.
            let (mark, note) = match &document.published {
                Some(published) if published.current => {
                    ("✓", format!("published {}", short(&published.commit)))
                }
                Some(published) => (
                    "~",
                    format!(
                        "published {} — the draft has moved, so sessions still compose \
                         the reviewed version",
                        short(&published.commit)
                    ),
                ),
                None => ("·", "draft only — no review has carried it".to_owned()),
            };
            println!(
                "    {mark} {}  [{}]  {}",
                document.name,
                document.sensitivity.as_str(),
                document.title
            );
            println!(
                "        {note}  draft {}  {} chunk(s)",
                short(&document.object_hash),
                document.chunks
            );
        }
        println!();
    }
    Ok(())
}

/// What an `author` is writing.
pub struct Bundle<'a> {
    /// The pack's name — one segment, the identifier a scope's override is
    /// expressed in.
    pub name: &'a str,
    pub scope: ScopeId,
    pub description: &'a str,
    /// The files to put in it. Each becomes one document, named by its
    /// path relative to `root` when one is given, or by its file name.
    pub files: &'a [PathBuf],
    /// The directory the document names are relative to.
    pub root: Option<&'a PathBuf>,
    /// The tier every document in this request carries. Per document on
    /// the wire (ADR-0050 decision 12); one flag here, because a terminal
    /// uploading five runbooks at once is uploading five runbooks.
    pub sensitivity: Option<Sensitivity>,
}

/// `synveda context-pack author <name> --file …` — write the draft.
///
/// This moves nothing a session composes: the published channel is
/// somewhere else, and only the approval matrix moves it. What it *does*
/// do is the expensive half — the server chunks, scans and embeds every
/// document whose bytes moved, so the response's per-document `embedded`
/// count is the honest report of what this call cost.
pub async fn author(profile: &str, bundle: Bundle<'_>) -> Result<(), String> {
    let mut documents = Vec::with_capacity(bundle.files.len());
    for file in bundle.files {
        let content = std::fs::read_to_string(file)
            .map_err(|err| format!("reading {}: {err}", file.display()))?;
        // The document's name is its path inside the bundle, which is what
        // the pack channel names it by and what a curator glob matches.
        let name = match bundle.root {
            Some(root) => file
                .strip_prefix(root)
                .map_err(|_| format!("{} is not under --root {}", file.display(), root.display()))?
                .to_string_lossy()
                .into_owned(),
            None => file
                .file_name()
                .ok_or_else(|| format!("{} has no file name", file.display()))?
                .to_string_lossy()
                .into_owned(),
        };
        // The title defaults to the document's first heading, then to its
        // name: a title is what the index tier renders when a block cannot
        // hold the body (ADR-0050 decision 10), so an empty one would spend
        // budget saying nothing.
        let title = first_heading(&content).unwrap_or_else(|| name.clone());
        let mut document = json!({
            "name": name,
            "title": title,
            "content": content,
        });
        if let Some(sensitivity) = bundle.sensitivity {
            document["sensitivity"] = json!(sensitivity.as_str());
        }
        documents.push(document);
    }

    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let response = api
        .post(
            "/v1/context-packs",
            Some(json!({
                "scope_id": bundle.scope,
                "name": bundle.name,
                "description": bundle.description,
                "documents": documents,
            })),
        )
        .await?;

    println!(
        "synveda: wrote {} at {}",
        response["name"].as_str().unwrap_or(bundle.name),
        response["scope_path"].as_str().unwrap_or_default(),
    );
    let empty = Vec::new();
    let written = response["documents"].as_array().unwrap_or(&empty);
    let mut embedded_total = 0_u64;
    for document in written {
        let embedded = document["embedded"].as_u64().unwrap_or(0);
        embedded_total += embedded;
        let cost = if embedded == 0 {
            "unchanged — nothing re-embedded".to_owned()
        } else {
            format!("{embedded} chunk(s) embedded")
        };
        println!(
            "         {}  {}  {}",
            document["name"].as_str().unwrap_or_default(),
            short(document["object_hash"].as_str().unwrap_or_default()),
            cost,
        );
        // The line that makes "behind review" visible from the writing
        // side, and for a pack it says something stronger than it does for
        // a prompt: the *previous* version's chunks are what sessions are
        // still composing, in full, until a proposal lands.
        let published = &document["published"];
        if published.is_null() {
            println!("           nothing is published under this path yet");
        } else if published["current"] == json!(false) {
            println!(
                "           sessions still compose {} — this edit reaches them \
                 through review",
                short(published["commit"].as_str().unwrap_or_default())
            );
        }
    }
    if embedded_total == 0 && !written.is_empty() {
        println!("         no document moved, so nothing was scanned, chunked or embedded");
    }
    println!(
        "         open the review with `synveda context-pack propose {}`",
        bundle.name
    );
    Ok(())
}

/// `synveda context-pack propose <name> --scope <id>` — open the review
/// that can carry the whole bundle across the trust boundary.
///
/// It proposes every document of the pack, because a bundle is what a
/// curator is being asked about. The channel still names them one at a
/// time (ADR-0050 decision 3), so a reviewer sees a per-document diff and
/// a half-published pack stays expressible — it is just not what this verb
/// does.
pub async fn propose(
    profile: &str,
    name: &str,
    scope: ScopeId,
    title: Option<&str>,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let listing: Listing = api
        .get_as(&format!("/v1/context-packs?scope_id={scope}"))
        .await?;
    let pack = listing
        .packs
        .iter()
        .find(|pack| pack.name == name)
        .ok_or_else(|| {
            format!("no context pack {name:?} at {} — `synveda context-pack list --scope {scope}` shows what is there", listing.scope_path)
        })?;
    if pack.documents.is_empty() {
        return Err(format!(
            "context pack {name:?} holds no documents; author one before proposing it"
        ));
    }
    let paths: Vec<String> = pack
        .documents
        .iter()
        .map(|document| format!("{name}/{}", document.name))
        .collect();

    let response = api
        .post(
            "/v1/proposals",
            Some(json!({
                "scope_id": scope,
                "document_paths": paths,
                "title": title.unwrap_or(name),
            })),
        )
        .await?;
    let id = response["id"].as_str().unwrap_or_default();
    println!(
        "synveda: proposal {id} open at {}  ({} document(s))",
        response["target_scope_path"].as_str().unwrap_or_default(),
        paths.len(),
    );
    println!(
        "         requires {}",
        response["outstanding"].as_str().unwrap_or("nothing")
    );
    println!("         review it with `synveda proposal show {id}`");
    Ok(())
}

/// The document's title: its first **level-one** Markdown heading.
///
/// Level one specifically, and that is the whole point rather than a
/// detail. A title names the *document*; `##` and below name sections, and
/// the index tier already renders the section beside the title
/// (`pack/document#n § heading — title`, ADR-0050 decision 10). Taking the
/// first heading of any level would label a document of a dozen `#
/// Section k` blocks with "Section 0" and render `§ Section 7 — Section 0`,
/// which reads as a contradiction and tells a reader nothing. A document
/// with no `#` gets its file name, which at least identifies it.
///
/// The heading rule itself is the server's chunker's
/// (`synveda_types::chunk`): `#` **followed by whitespace**. Two
/// implementations of "is this a heading" that disagreed would put a
/// `#hashtag` in the title slot, which is the one place a wrong answer is
/// expensive.
fn first_heading(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let text = line.trim_start();
        let hashes = text.chars().take_while(|c| *c == '#').count();
        if hashes != 1 {
            return None;
        }
        let rest = &text[hashes..];
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let rest = rest.trim();
        (!rest.is_empty()).then(|| rest.to_owned())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_the_documents_own_heading_and_never_a_sections() {
        assert_eq!(
            first_heading("# Refunds runbook\n\nEscalate.\n").as_deref(),
            Some("Refunds runbook")
        );
        assert_eq!(
            first_heading("Preamble\n\n# Refunds runbook\n").as_deref(),
            Some("Refunds runbook")
        );
        // A section is not a title. Without this, a glossary of `# Section
        // k` blocks would render `§ Section 7 — Section 0` in the index
        // tier, which reads as a contradiction.
        assert_eq!(first_heading("## Refunds\n\n### Escalation\n"), None);
        assert_eq!(first_heading("no headings here\n"), None);
        // Not a heading: no space after the hash.
        assert_eq!(first_heading("#hashtag\n"), None);
    }
}
