//! `synveda skill` — the registry from a terminal, and the **only** thing in
//! the product that writes a skill onto a disk (SKIL-1, ADR-0051).
//!
//! HTTP-only for everything governed, on `prompt`'s precedent (ADR-0035
//! decision 1): authoring is a `SkillWrite` decision, resolution a
//! `SkillRead` at the tier the served version carries, and both chain their
//! own event under the caller's identity.
//!
//! # Why the materialisation lives here
//!
//! Seed §2.6 — the harness is a guest — and ADR-0051 decision 12. A gateway
//! that owned a per-client directory layout would need a release when one of
//! forty agentskills.io clients moved a folder; the CLI can be wrong cheaply.
//! So `install` resolves through the ordinary read route and writes what
//! comes back.
//!
//! # What "installs unmodified" means, concretely
//!
//! Three properties, and none of them is a promise:
//!
//! 1. **The bundle directory holds exactly the reviewed files.** Nothing is
//!    added — no receipt, no manifest, no header — because a file no
//!    reviewer approved inside a directory a client walks is the
//!    modification the criterion forbids (ADR-0051 option 7). The receipt
//!    goes in this CLI's own config directory.
//! 2. **Every file's content address recomputes.** [`install`] hashes what
//!    it wrote and compares it to the address the published commit named. It
//!    is the client doing the arithmetic rather than trusting the server's
//!    number — which matters because a materialised bundle carries no
//!    watermark of its own (ADR-0051 force 2), so this hash is its whole
//!    provenance.
//! 3. **The per-client difference is the root and nothing else.** The same
//!    commit installs the same bytes into `~/.claude/skills/<name>/` and
//!    `~/.codex/skills/<name>/`, which is what makes the trees comparable
//!    file for file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_types::{
    MAX_SKILL_BUNDLE_CHARS, MAX_SKILL_FILE_CHARS, MAX_SKILL_FILES, SKILL_MANIFEST, ScopeId,
    Sensitivity, SkillFile, SkillFilePath, SkillName,
};
use synveda_vedaflow::SkillAsset;

use crate::api::{Api, Origin};

// ── The wire shapes (`crates/synveda-gateway/src/skills.rs`) ───────────

#[derive(Deserialize)]
struct Resolved {
    name: String,
    scope_id: ScopeId,
    scope_path: String,
    channel: String,
    origin: String,
    commit: Option<String>,
    sensitivity: Sensitivity,
    description: String,
    files: Vec<ResolvedFile>,
}

#[derive(Deserialize)]
struct ResolvedFile {
    path: String,
    object_hash: String,
    content: String,
}

#[derive(Deserialize)]
struct Listing {
    scope_path: String,
    skills: Vec<ListEntry>,
}

#[derive(Deserialize)]
struct ListEntry {
    name: String,
    description: String,
    sensitivity: Sensitivity,
    files: Vec<ListFile>,
}

#[derive(Deserialize)]
struct ListFile {
    path: String,
    object_hash: String,
    published: Option<PublishedFile>,
}

#[derive(Deserialize)]
struct PublishedFile {
    current: bool,
}

// ── Clients ────────────────────────────────────────────────────────────

/// A client's skills root, relative to `$HOME`.
///
/// The agentskills.io ecosystem agrees on the *bundle* — `SKILL.md` plus
/// files, in a directory named for the skill — and differs only on where
/// that directory lives. This table is that difference and nothing else,
/// which is the property `install --client` is built to demonstrate.
///
/// Kept here rather than in the gateway on ADR-0051 decision 12's reasoning:
/// a client moving its folder should cost a CLI release, not a server one.
const CLIENT_ROOTS: [(&str, &str); 2] = [
    ("claude-code", ".claude/skills"),
    ("codex", ".codex/skills"),
];

/// The names `--client` takes, for a usage message.
#[must_use]
pub fn clients() -> String {
    CLIENT_ROOTS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Where `client` keeps its skills, under `$HOME`.
fn client_root(client: &str) -> Result<PathBuf, String> {
    let (_, suffix) = CLIENT_ROOTS
        .iter()
        .find(|(name, _)| *name == client)
        .ok_or_else(|| format!("unknown client {client:?}; known clients are {}", clients()))?;
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_owned())?;
    Ok(PathBuf::from(home).join(suffix))
}

// ── List and show ──────────────────────────────────────────────────────

/// `synveda skill list --scope <id>` — the registry at one scope.
pub async fn list(profile: &str, scope: ScopeId) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let listing: Listing = api.get_as(&format!("/v1/skills?scope_id={scope}")).await?;
    if listing.skills.is_empty() {
        println!("no skills at {}", listing.scope_path);
        return Ok(());
    }
    println!("skills at {}\n", listing.scope_path);
    for entry in &listing.skills {
        // A skill is published as a bundle, so the mark is about the bundle:
        // `✓` every file published and current, `~` published and behind the
        // draft in at least one file, `·` never reviewed.
        let published = entry.files.iter().filter(|f| f.published.is_some()).count();
        let current = entry
            .files
            .iter()
            .filter(|f| f.published.as_ref().is_some_and(|p| p.current))
            .count();
        let (mark, note) = if published == 0 {
            ("·", "draft only — no review has carried it".to_owned())
        } else if current == entry.files.len() && published == entry.files.len() {
            ("✓", "published and current".to_owned())
        } else {
            (
                "~",
                format!(
                    "published, but {} of {} file(s) have moved since",
                    entry.files.len() - current,
                    entry.files.len()
                ),
            )
        };
        println!(
            "  {mark} {}  [{}]  {}",
            entry.name,
            entry.sensitivity.as_str(),
            entry.description
        );
        println!("      {note}  {} file(s)", entry.files.len());
        for file in &entry.files {
            println!("        {}  {}", file.path, short(&file.object_hash));
        }
    }
    Ok(())
}

/// What a `show` or an `install` is asking for.
pub struct Ask<'a> {
    /// The skill's name — also the directory an install creates.
    pub name: &'a str,
    /// Which scope, when naming one. Absent walks the caller's own chain.
    pub scope: Option<ScopeId>,
    /// `draft` for the authoring copy at a named scope.
    pub draft: bool,
    /// A commit to pin to: the version this caller was built against.
    pub commit: Option<&'a str>,
}

impl Ask<'_> {
    /// The resolve path this ask produces.
    fn path(&self) -> String {
        let mut query: Vec<String> = Vec::new();
        if let Some(scope) = self.scope {
            query.push(format!("scope_id={scope}"));
        }
        if self.draft {
            query.push("channel=draft".to_owned());
        }
        if let Some(commit) = self.commit {
            query.push(format!("commit={commit}"));
        }
        match query.is_empty() {
            true => format!("/v1/skills/{}", self.name),
            false => format!("/v1/skills/{}?{}", self.name, query.join("&")),
        }
    }
}

/// `synveda skill show <name>` — resolve and render, without writing
/// anything.
pub async fn show(profile: &str, ask: Ask<'_>, json_out: bool, quiet: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    if !quiet {
        announce(&api, &origin);
    }
    let path = ask.path();
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }
    let resolved: Resolved = api.get_as(&path).await?;
    print_header(&resolved);
    for file in &resolved.files {
        println!(
            "   {}  {}  {} char(s)",
            file.path,
            short(&file.object_hash),
            file.content.chars().count()
        );
    }
    Ok(())
}

fn print_header(resolved: &Resolved) {
    // Where the bytes came from, said plainly: a response that cites a
    // frozen commit without saying so overstates its own freshness.
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
        "   {}  {}",
        resolved.description,
        resolved
            .commit
            .as_deref()
            .map(short)
            .unwrap_or_else(|| "no commit".to_owned()),
    );
    println!("   scope {}", resolved.scope_id);
    println!();
}

// ── Import ─────────────────────────────────────────────────────────────

/// `synveda skill import <dir> --scope <id>` — read an anthropics/skills
/// directory and author it.
///
/// The AC's third clause ("import from anthropics/skills format"), and the
/// format is the one the open standard defines, so there is nothing to
/// convert: this reads a directory and posts it.
///
/// What it does is **refuse rather than partially import** (ADR-0051
/// decision 15). A symlink is not content and is not followed; a missing
/// `SKILL.md`, a file over the bound, or a path the grammar refuses is a
/// refusal naming the offender. Importing three files of four and calling it
/// a skill is the failure a registry exists to prevent.
pub async fn import(
    profile: &str,
    dir: &Path,
    scope: ScopeId,
    name: Option<&str>,
    sensitivity: Option<Sensitivity>,
) -> Result<(), String> {
    // The directory's own name is the default, which is the spec's rule
    // (a skill's directory is named for it) used as a convenience.
    let default_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "{} has no directory name to take a skill name from",
                dir.display()
            )
        })?;
    let name: SkillName = name
        .unwrap_or(default_name)
        .parse()
        .map_err(|err| format!("{err}"))?;

    let mut files: Vec<SkillFile> = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    if !files.iter().any(|file| file.path.is_manifest()) {
        return Err(format!(
            "{} has no {SKILL_MANIFEST}; without one it is not a skill under the open \
             spec and no client will load it",
            dir.display()
        ));
    }
    let total: usize = files.iter().map(|file| file.content.chars().count()).sum();
    if total > MAX_SKILL_BUNDLE_CHARS {
        return Err(format!(
            "{} is {total} characters across {} file(s), over the {MAX_SKILL_BUNDLE_CHARS} a \
             skill may hold",
            dir.display(),
            files.len()
        ));
    }

    let body = json!({
        "scope_id": scope,
        "name": name.as_str(),
        "sensitivity": sensitivity.map(|tier| tier.as_str()),
        "files": files.iter().map(|file| json!({
            "path": file.path.as_str(),
            "content": file.content,
        })).collect::<Vec<_>>(),
    });

    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let response = api.post("/v1/skills", Some(body)).await?;
    println!(
        "synveda: wrote {} at {}  {} file(s) from {}",
        response["name"].as_str().unwrap_or(name.as_str()),
        response["scope_path"].as_str().unwrap_or_default(),
        response["files"].as_array().map_or(0, Vec::len),
        dir.display(),
    );
    let removed = response["removed"].as_u64().unwrap_or(0);
    if removed > 0 {
        // Decision 17 made visible: a client loads a bundle whole, so the
        // request is the bundle and a dropped file is really gone.
        println!("         {removed} file(s) the bundle no longer names were removed");
    }
    match response["published_commit"].as_str() {
        None => println!("         nothing is published under this name yet — open a proposal"),
        Some(commit) => println!(
            "         consumers are still served {} — this edit reaches them through review",
            short(commit)
        ),
    }
    Ok(())
}

/// Walks `dir`, collecting every file as a bundled path relative to `root`.
///
/// Symlinks are skipped rather than followed: a symlink is a reference to
/// bytes rather than bytes, and following one would let an import pull in
/// whatever it points at (ADR-0051 decision 15).
fn collect(root: &Path, dir: &Path, out: &mut Vec<SkillFile>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|err| format!("stat {}: {err}", path.display()))?;
        if meta.is_symlink() {
            return Err(format!(
                "{} is a symlink; a bundle carries bytes, and following one would import \
                 whatever it points at (ADR-0051 decision 15)",
                path.display()
            ));
        }
        if meta.is_dir() {
            collect(root, &path, out)?;
            continue;
        }
        if out.len() >= MAX_SKILL_FILES {
            return Err(format!(
                "more than {MAX_SKILL_FILES} files under {}; split it into two skills",
                root.display()
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|err| format!("{} is not under {}: {err}", path.display(), root.display()))?;
        let relative = relative
            .to_str()
            .ok_or_else(|| format!("{} is not valid UTF-8", relative.display()))?
            // Windows separators, normalised before the grammar sees them —
            // the grammar refuses a backslash *inside a segment*, which is
            // the traversal shape, and this is the path separator.
            .replace('\\', "/");
        let bundled: SkillFilePath = relative.parse().map_err(|err| format!("{err}"))?;
        let content = std::fs::read_to_string(&path).map_err(|err| {
            format!(
                "read {}: {err} — a skill bundle carries reviewable text, never binaries",
                path.display()
            )
        })?;
        if content.chars().count() > MAX_SKILL_FILE_CHARS {
            return Err(format!(
                "{} is over the {MAX_SKILL_FILE_CHARS}-character bound for one bundled file",
                path.display()
            ));
        }
        out.push(SkillFile {
            path: bundled,
            content,
        });
    }
    Ok(())
}

// ── Install ────────────────────────────────────────────────────────────

/// What an install recorded, written beside the credentials and **never**
/// inside the bundle (ADR-0051 decision 12, option 7).
#[derive(Serialize, Deserialize)]
struct Receipt {
    version: u32,
    /// The client whose root this was written into.
    client: String,
    /// Where the bundle went.
    directory: String,
    skill: String,
    scope_id: ScopeId,
    scope_path: String,
    /// The commit the bytes came from — what a reinstall pins to, and what
    /// a rewind will refuse by name.
    commit: Option<String>,
    sensitivity: Sensitivity,
    /// Path → content address, as the CLI recomputed it from what it wrote.
    files: BTreeMap<String, String>,
}

/// `synveda skill install <name> --client <client>` — resolve a bundle and
/// write it into that client's own skills directory.
///
/// The bytes are written verbatim, the directory holds exactly the reviewed
/// files, and every one of them is re-hashed against the address the
/// published commit named before this returns. A mismatch is a hard error:
/// it means the bytes on disk are not the bytes that were reviewed, which is
/// the one thing an install must never leave true.
pub async fn install(
    profile: &str,
    ask: Ask<'_>,
    client: &str,
    root: Option<&Path>,
    json_out: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    if !json_out {
        announce(&api, &origin);
    }
    let resolved: Resolved = api.get_as(&ask.path()).await?;
    let name: SkillName = resolved.name.parse().map_err(|err| format!("{err}"))?;

    let root = match root {
        Some(root) => root.to_path_buf(),
        None => client_root(client)?,
    };
    let directory = root.join(name.as_str());
    // A fresh directory, so a file the bundle no longer names does not
    // survive an upgrade. A client loads what is there, not what a manifest
    // says is there.
    if directory.exists() {
        std::fs::remove_dir_all(&directory)
            .map_err(|err| format!("remove {}: {err}", directory.display()))?;
    }
    std::fs::create_dir_all(&directory)
        .map_err(|err| format!("create {}: {err}", directory.display()))?;

    let mut receipt_files = BTreeMap::new();
    for file in &resolved.files {
        let bundled: SkillFilePath = file.path.parse().map_err(|err| format!("{err}"))?;
        let target = directory.join(bundled.as_str());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
        std::fs::write(&target, file.content.as_bytes())
            .map_err(|err| format!("write {}: {err}", target.display()))?;
        // Non-executable, always (ADR-0051 decision 8): a governed bundle
        // cannot arrive carrying a mode nobody reviewed, and a skill invokes
        // its scripts through an interpreter.
        set_readable(&target)?;

        // The measurement, not the claim. Read back what was written and
        // recompute the address from it — the CLI's own arithmetic against
        // the number the commit named.
        let written = std::fs::read_to_string(&target)
            .map_err(|err| format!("re-read {}: {err}", target.display()))?;
        let asset = SkillAsset {
            scope_id: resolved.scope_id,
            skill: name.clone(),
            sensitivity: resolved.sensitivity,
            file: SkillFile {
                path: bundled.clone(),
                content: written,
            },
        };
        let recomputed = asset.address().to_hex();
        if recomputed != file.object_hash {
            return Err(format!(
                "{} does not hash to the address the published commit named \
                 (wrote {recomputed}, expected {}). The bytes on disk are not the bytes \
                 that were reviewed; the install is incomplete and this directory should \
                 be removed",
                target.display(),
                file.object_hash,
            ));
        }
        receipt_files.insert(bundled.as_str().to_owned(), recomputed);
    }

    let receipt = Receipt {
        version: 1,
        client: client.to_owned(),
        directory: directory.display().to_string(),
        skill: name.to_string(),
        scope_id: resolved.scope_id,
        scope_path: resolved.scope_path.clone(),
        commit: resolved.commit.clone(),
        sensitivity: resolved.sensitivity,
        files: receipt_files,
    };
    let receipt_path = write_receipt(client, &name, &receipt)?;

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": receipt.skill,
                "client": receipt.client,
                "directory": receipt.directory,
                "commit": receipt.commit,
                "scope_path": receipt.scope_path,
                "files": receipt.files,
                "receipt": receipt_path.display().to_string(),
            }))
            .map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    println!(
        "synveda: installed {} into {}",
        receipt.skill,
        directory.display()
    );
    println!(
        "         {} file(s) from {} at {}, every address recomputed",
        receipt.files.len(),
        resolved.scope_path,
        receipt
            .commit
            .as_deref()
            .map(short)
            .unwrap_or_else(|| "no commit".to_owned()),
    );
    // The sentence that says why the directory looks like nothing but a
    // skill: the provenance is here, outside it.
    println!("         receipt {}", receipt_path.display());
    Ok(())
}

/// Writes the receipt into the CLI's own config directory.
fn write_receipt(client: &str, name: &SkillName, receipt: &Receipt) -> Result<PathBuf, String> {
    let dir = crate::credentials::config_dir()?
        .join("skills")
        .join(client);
    std::fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let path = dir.join(format!("{name}.json"));
    let body = serde_json::to_string_pretty(receipt).map_err(|err| err.to_string())?;
    std::fs::write(&path, body).map_err(|err| format!("write {}: {err}", path.display()))?;
    Ok(path)
}

/// 0644 on POSIX; a no-op elsewhere, where there is no execute bit to clear.
#[cfg(unix)]
fn set_readable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
        .map_err(|err| format!("set mode on {}: {err}", path.display()))
}

#[cfg(not(unix))]
fn set_readable(_path: &Path) -> Result<(), String> {
    Ok(())
}

// ── Propose ────────────────────────────────────────────────────────────

/// `synveda skill propose <name> --scope <id>` — open the review that can
/// carry the bundle across the trust boundary.
///
/// The proposal names the **skill**, never a file: a client loads a bundle
/// whole, so every file the source holds becomes a member.
///
/// Under every pack this is the only route: the invariant floor asks for a
/// security reviewer and, since ADR-0051 decision 18, two distinct
/// approvers, so no pack makes shipping executable code a one-signature act.
pub async fn propose(
    profile: &str,
    name: &str,
    scope: ScopeId,
    source: Option<ScopeId>,
    title: &str,
) -> Result<(), String> {
    let parsed: SkillName = name.parse().map_err(|err| format!("{err}"))?;
    let mut body = json!({
        "scope_id": scope,
        "title": title,
        "skill_names": [parsed.as_str()],
    });
    if let Some(source) = source {
        body["source_scope_id"] = json!(source);
    }
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let response = api.post("/v1/proposals", Some(body)).await?;
    println!(
        "synveda: opened proposal {} — {} member(s), {}",
        response["proposal_id"].as_str().unwrap_or_default(),
        response["members"].as_array().map_or(0, Vec::len),
        response["state"].as_str().unwrap_or_default(),
    );
    if let Some(required) = response.get("required")
        && !required.is_null()
    {
        println!("         needs {required}");
    }
    println!("         review with `synveda proposal show <id>`");
    Ok(())
}

// ── Shared ─────────────────────────────────────────────────────────────

/// The first twelve hex characters — enough to name a commit or an object
/// in a terminal, as `synveda channel history` renders them.
fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// Which gateway and which identity, so a governed act never happens
/// silently against a surprise host. On stderr, so `--json` and `--quiet`
/// stay pipeable.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The table `install --client` exists to demonstrate: every client's
    /// root is a different directory and nothing else differs, which is what
    /// makes two installed trees comparable file for file.
    #[test]
    fn every_client_root_is_a_relative_path_under_home() {
        for (name, suffix) in CLIENT_ROOTS {
            assert!(!suffix.starts_with('/'), "{name} root must be under HOME");
            assert!(suffix.ends_with("skills"), "{name} root names a skills dir");
        }
        assert!(clients().contains("claude-code"));
        assert!(clients().contains("codex"));
    }

    #[test]
    fn an_unknown_client_names_the_ones_it_knows() {
        let err = client_root("emacs").unwrap_err();
        assert!(err.contains("claude-code"), "{err}");
        assert!(err.contains("codex"), "{err}");
    }

    /// The receipt is a document about a directory, never a document *in*
    /// it — which is ADR-0051 option 7 as a test rather than a comment.
    #[test]
    fn a_receipt_records_the_directory_it_is_not_inside() {
        let receipt = Receipt {
            version: 1,
            client: "claude-code".to_owned(),
            directory: "/home/dev/.claude/skills/code-review".to_owned(),
            skill: "code-review".to_owned(),
            scope_id: ScopeId::new(),
            scope_path: "/acme/eng".to_owned(),
            commit: Some("abcd".to_owned()),
            sensitivity: Sensitivity::Internal,
            files: BTreeMap::from([("SKILL.md".to_owned(), "beef".to_owned())]),
        };
        let text = serde_json::to_string(&receipt).unwrap();
        assert!(text.contains("\"directory\""));
        assert!(text.contains("code-review"));
    }

    /// The one path shape `collect` normalises before the grammar sees it.
    /// A backslash *inside a segment* stays refused — that is the traversal
    /// shape — and this is the separator.
    #[test]
    fn windows_separators_normalise_to_bundled_paths() {
        assert_eq!(
            "scripts\\check.py"
                .replace('\\', "/")
                .parse::<SkillFilePath>()
                .unwrap()
                .as_str(),
            "scripts/check.py"
        );
        assert!("scripts/che\\ck.py".parse::<SkillFilePath>().is_err());
    }

    #[test]
    fn a_skill_path_is_what_a_channel_entry_carries() {
        let path = synveda_types::SkillPath::new(
            "code-review".parse().unwrap(),
            "scripts/check.py".parse().unwrap(),
        );
        assert_eq!(path.to_string(), "code-review/scripts/check.py");
    }
}
