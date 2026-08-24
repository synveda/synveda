//! Public-API client for the versioned Agent Skills catalogue (CPR-23,
//! ADR-0085). The gateway owns governance; this module only reads bundles,
//! opens typed VedaFlow changes, and materialises an already-authorised exact
//! version into a client directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synveda_types::{
    MAX_SKILL_BUNDLE_CHARS, MAX_SKILL_FILE_CHARS, MAX_SKILL_FILES, SKILL_MANIFEST, ScopeId,
    Sensitivity, SkillBindingId, SkillBundle, SkillFile, SkillFilePath, SkillId, SkillName,
    SkillVersionId,
};
use synveda_vedaflow::SkillAsset;

use crate::api::{Api, Origin};

const CLIENT_ROOTS: [(&str, &str); 2] = [
    ("claude-code", ".claude/skills"),
    ("codex", ".codex/skills"),
];

#[derive(Clone, Deserialize, Serialize)]
struct VersionView {
    id: SkillVersionId,
    skill_id: SkillId,
    ordinal: u64,
    bundle_digest: String,
    sensitivity: Sensitivity,
    manifest: Value,
    declared_tools_are_authorization: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct SkillView {
    id: SkillId,
    governing_scope_id: ScopeId,
    name: String,
    current_version_id: SkillVersionId,
    current_version: VersionView,
}

#[derive(Deserialize)]
struct SkillList {
    skills: Vec<SkillView>,
    next_cursor: Option<SkillId>,
}

#[derive(Clone, Deserialize, Serialize)]
struct BindingView {
    id: SkillBindingId,
    scope_id: ScopeId,
    skill_id: SkillId,
    pinned_version_id: Option<SkillVersionId>,
    enabled: bool,
    revision: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct AvailableEntry {
    binding: BindingView,
    name: String,
    version: VersionView,
    manifest_object_hash: String,
}

#[derive(Deserialize, Serialize)]
struct AvailableList {
    scope_id: ScopeId,
    skills: Vec<AvailableEntry>,
}

#[derive(Clone, Deserialize, Serialize)]
struct FileView {
    path: String,
    object_hash: String,
    chars: u32,
}

#[derive(Deserialize)]
struct FileList {
    files: Vec<FileView>,
}

#[derive(Deserialize)]
struct FileContent {
    path: String,
    object_hash: String,
    content: String,
}

#[derive(Serialize)]
struct Resolved {
    skill: SkillView,
    binding: BindingView,
    version: VersionView,
    files: Vec<FileContentView>,
}

#[derive(Serialize)]
struct FileContentView {
    path: String,
    object_hash: String,
    content: String,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    version: u32,
    client: String,
    directory: String,
    skill: String,
    governing_scope_id: ScopeId,
    distribution_scope_id: ScopeId,
    binding_id: SkillBindingId,
    version_id: SkillVersionId,
    bundle_digest: String,
    sensitivity: Sensitivity,
    files: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct Synced {
    skill: String,
    version_id: SkillVersionId,
    directory: String,
    files: usize,
}

#[must_use]
pub fn clients() -> String {
    CLIENT_ROOTS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn client_root(client: &str) -> Result<PathBuf, String> {
    let (_, suffix) = CLIENT_ROOTS
        .iter()
        .find(|(name, _)| *name == client)
        .ok_or_else(|| format!("unknown client {client:?}; known clients are {}", clients()))?;
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_owned())?;
    Ok(PathBuf::from(home).join(suffix))
}

fn description(version: &VersionView) -> &str {
    version
        .manifest
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
}

async fn catalogue(api: &Api) -> Result<Vec<SkillView>, String> {
    let mut skills = Vec::new();
    let mut cursor: Option<SkillId> = None;
    loop {
        let path = cursor.map_or_else(
            || "/v1/skills?limit=200".to_owned(),
            |cursor| format!("/v1/skills?limit=200&cursor={cursor}"),
        );
        let page: SkillList = api.get_as(&path).await?;
        skills.extend(page.skills);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(skills),
        }
    }
}

async fn skill_named(api: &Api, name: &SkillName) -> Result<Option<SkillView>, String> {
    Ok(catalogue(api)
        .await?
        .into_iter()
        .find(|skill| skill.name == name.as_str()))
}

async fn available_at(api: &Api, scope: ScopeId) -> Result<AvailableList, String> {
    api.get_as(&format!("/v1/skills/available?scope_id={scope}"))
        .await
}

async fn exact_version(
    api: &Api,
    skill: SkillId,
    version: SkillVersionId,
) -> Result<VersionView, String> {
    api.get_as(&format!("/v1/skills/{skill}/versions/{version}"))
        .await
}

async fn exact_files(
    api: &Api,
    skill: SkillId,
    version: SkillVersionId,
) -> Result<Vec<FileContentView>, String> {
    let listing: FileList = api
        .get_as(&format!("/v1/skills/{skill}/versions/{version}/files"))
        .await?;
    let mut files = Vec::with_capacity(listing.files.len());
    for file in listing.files {
        let content: FileContent = api
            .get_as(&format!(
                "/v1/skills/{skill}/versions/{version}/files/{}",
                file.path
            ))
            .await?;
        if content.object_hash != file.object_hash || content.path != file.path {
            return Err(format!(
                "version {version} returned inconsistent metadata for {}",
                file.path
            ));
        }
        files.push(FileContentView {
            path: content.path,
            object_hash: content.object_hash,
            content: content.content,
        });
    }
    Ok(files)
}

async fn resolve_entry(api: &Api, entry: AvailableEntry) -> Result<Resolved, String> {
    let skill: SkillView = api
        .get_as(&format!("/v1/skills/{}", entry.version.skill_id))
        .await?;
    if skill.id != entry.binding.skill_id || skill.id != entry.version.skill_id {
        return Err("the available binding, Skill and version disagree".to_owned());
    }
    let files = exact_files(api, skill.id, entry.version.id).await?;
    Ok(Resolved {
        skill,
        binding: entry.binding,
        version: entry.version,
        files,
    })
}

async fn resolve_available(api: &Api, scope: ScopeId, name: &str) -> Result<Resolved, String> {
    let parsed = name.parse::<SkillName>().map_err(|err| err.to_string())?;
    let listing = available_at(api, scope).await?;
    let entry = listing
        .skills
        .into_iter()
        .find(|entry| entry.name == parsed.as_str())
        .ok_or_else(|| format!("skill {name:?} is not enabled and visible at scope {scope}"))?;
    resolve_entry(api, entry).await
}

/// List stable catalogue entries, optionally filtering by governing scope.
pub async fn list(profile: &str, scope: Option<ScopeId>, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let mut skills = catalogue(&api).await?;
    if let Some(scope) = scope {
        skills.retain(|skill| skill.governing_scope_id == scope);
    }
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&skills).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    if skills.is_empty() {
        println!("no visible skills");
        return Ok(());
    }
    for skill in skills {
        println!(
            "{}  {}  v{} {}  [{}]",
            skill.id,
            skill.name,
            skill.current_version.ordinal,
            short(&skill.current_version.bundle_digest),
            skill.current_version.sensitivity.as_str(),
        );
        println!(
            "    {}  governed at {}",
            description(&skill.current_version),
            skill.governing_scope_id
        );
    }
    Ok(())
}

/// Inspect one exact immutable version by tenant-unique bundle name.
pub async fn show(
    profile: &str,
    name: &str,
    version: Option<SkillVersionId>,
    json_out: bool,
    quiet: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    if !quiet {
        announce(&api, &origin);
    }
    let name = name.parse::<SkillName>().map_err(|err| err.to_string())?;
    let skill = skill_named(&api, &name)
        .await?
        .ok_or_else(|| format!("skill {name:?} is not visible"))?;
    let version_id = version.unwrap_or(skill.current_version_id);
    let version = exact_version(&api, skill.id, version_id).await?;
    let files: FileList = api
        .get_as(&format!(
            "/v1/skills/{}/versions/{}/files",
            skill.id, version.id
        ))
        .await?;
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": skill,
                "version": version,
                "files": files.files,
            }))
            .map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    println!(
        "{}  {}  version {} ({})",
        skill.name, skill.id, version.ordinal, version.id
    );
    println!(
        "    {}  [{}]  digest {}",
        description(&version),
        version.sensitivity.as_str(),
        version.bundle_digest
    );
    println!(
        "    declared tools are metadata, never authorisation: {}",
        !version.declared_tools_are_authorization
    );
    for file in files.files {
        println!(
            "    {}  {}  {} char(s)",
            file.path,
            short(&file.object_hash),
            file.chars
        );
    }
    Ok(())
}

/// Import a complete Agent Skills-compatible directory as an install or a new
/// immutable version. The returned change may still await review.
pub async fn import(
    profile: &str,
    dir: &Path,
    scope: ScopeId,
    name: Option<&str>,
    sensitivity: Option<Sensitivity>,
) -> Result<(), String> {
    let default_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no directory name", dir.display()))?;
    let name: SkillName = name
        .unwrap_or(default_name)
        .parse::<SkillName>()
        .map_err(|err| err.to_string())?;
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let bundle = SkillBundle {
        name: name.clone(),
        files: files.clone(),
    };
    bundle.validate().map_err(|err| err.to_string())?;
    let total: usize = files.iter().map(|file| file.content.chars().count()).sum();
    if total > MAX_SKILL_BUNDLE_CHARS {
        return Err(format!(
            "{} is {total} characters, over the {MAX_SKILL_BUNDLE_CHARS} bundle limit",
            dir.display()
        ));
    }

    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let existing = skill_named(&api, &name).await?;
    if existing
        .as_ref()
        .is_some_and(|skill| skill.governing_scope_id != scope)
    {
        return Err(format!(
            "skill {name} is governed at {}; installing it at {scope} would move stable identity",
            existing.expect("checked").governing_scope_id
        ));
    }
    let sensitivity = sensitivity
        .or_else(|| {
            existing
                .as_ref()
                .map(|skill| skill.current_version.sensitivity)
        })
        .unwrap_or(Sensitivity::Internal);
    let wire_files = files
        .iter()
        .map(|file| {
            json!({
                "path": file.path.as_str(),
                "content": file.content,
            })
        })
        .collect::<Vec<_>>();
    let provenance = json!({
        "kind": "authored",
        "reference": format!("local-directory:{default_name}"),
        "metadata": {"client": "synveda-cli"}
    });
    let key = SkillId::new().to_string();
    let response = match existing {
        Some(skill) => {
            let body = json!({
                "expected_current_version_id": skill.current_version_id,
                "sensitivity": sensitivity.as_str(),
                "files": wire_files,
                "provenance": provenance,
            });
            api.patch_idempotent(&format!("/v1/skills/{}", skill.id), body, &key)
                .await?
        }
        None => {
            let body = json!({
                "governing_scope_id": scope,
                "name": name.as_str(),
                "sensitivity": sensitivity.as_str(),
                "files": wire_files,
                "provenance": provenance,
            });
            api.post_with_header(
                "/v1/skills",
                Some(body),
                ("Idempotency-Key", &key),
                &api.subject,
            )
            .await?
        }
    };
    print_change(&response);
    Ok(())
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<SkillFile>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|err| format!("stat {}: {err}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "{} is a symlink; bundles carry bytes",
                path.display()
            ));
        }
        if meta.is_dir() {
            collect(root, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            return Err(format!("{} is not a regular file", path.display()));
        }
        if out.len() >= MAX_SKILL_FILES {
            return Err(format!(
                "more than {MAX_SKILL_FILES} files under {}",
                root.display()
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|err| format!("{} is not under {}: {err}", path.display(), root.display()))?
            .to_str()
            .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?
            .replace('\\', "/");
        let bundled = relative
            .parse::<SkillFilePath>()
            .map_err(|err| err.to_string())?;
        let content = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {} as UTF-8 text: {err}", path.display()))?;
        if content.chars().count() > MAX_SKILL_FILE_CHARS {
            return Err(format!(
                "{} exceeds the {MAX_SKILL_FILE_CHARS}-character file limit",
                path.display()
            ));
        }
        out.push(SkillFile {
            path: bundled,
            content,
        });
    }
    if dir == root && !out.iter().any(|file| file.path.is_manifest()) {
        return Err(format!("{} has no {SKILL_MANIFEST}", root.display()));
    }
    Ok(())
}

/// List exact versions enabled by bindings at a project or principal scope.
pub async fn available(profile: &str, scope: ScopeId, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let listing = available_at(&api, scope).await?;
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&listing).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    if listing.skills.is_empty() {
        println!("no enabled skills at {}", listing.scope_id);
        return Ok(());
    }
    for entry in listing.skills {
        let pin = entry
            .binding
            .pinned_version_id
            .map_or("follows current".to_owned(), |id| format!("pinned {id}"));
        println!(
            "{}  {}  v{} {}",
            entry.name, entry.version.id, entry.version.ordinal, pin
        );
        println!(
            "    binding {} revision {}  {}",
            entry.binding.id,
            entry.binding.revision,
            description(&entry.version)
        );
    }
    Ok(())
}

/// Materialise one enabled exact version into a supported client's directory.
pub async fn install(
    profile: &str,
    name: &str,
    scope: ScopeId,
    client: &str,
    root: Option<&Path>,
    json_out: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    if !json_out {
        announce(&api, &origin);
    }
    let root = root.map_or_else(|| client_root(client), |path| Ok(path.to_path_buf()))?;
    let resolved = resolve_available(&api, scope, name).await?;
    let (receipt, receipt_path) = materialise(resolved, client, &root)?;
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": receipt.skill,
                "version_id": receipt.version_id,
                "binding_id": receipt.binding_id,
                "directory": receipt.directory,
                "files": receipt.files,
                "receipt": receipt_path.display().to_string(),
            }))
            .map_err(|err| err.to_string())?
        );
    } else {
        println!(
            "synveda: installed {} version {} into {}",
            receipt.skill, receipt.version_id, receipt.directory
        );
        println!(
            "         {} file(s), every content address recomputed; receipt {}",
            receipt.files.len(),
            receipt_path.display()
        );
    }
    Ok(())
}

fn materialise(
    resolved: Resolved,
    client: &str,
    root: &Path,
) -> Result<(Receipt, PathBuf), String> {
    let name = resolved
        .skill
        .name
        .parse::<SkillName>()
        .map_err(|err| err.to_string())?;
    let directory = root.join(name.as_str());
    if directory.exists() {
        std::fs::remove_dir_all(&directory)
            .map_err(|err| format!("remove {}: {err}", directory.display()))?;
    }
    std::fs::create_dir_all(&directory)
        .map_err(|err| format!("create {}: {err}", directory.display()))?;

    let mut receipt_files = BTreeMap::new();
    for file in resolved.files {
        let bundled = file
            .path
            .parse::<SkillFilePath>()
            .map_err(|err| err.to_string())?;
        let target = directory.join(bundled.as_str());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
        std::fs::write(&target, file.content.as_bytes())
            .map_err(|err| format!("write {}: {err}", target.display()))?;
        set_readable(&target)?;
        let written = std::fs::read_to_string(&target)
            .map_err(|err| format!("re-read {}: {err}", target.display()))?;
        let asset = SkillAsset {
            scope_id: resolved.skill.governing_scope_id,
            skill: name.clone(),
            sensitivity: resolved.version.sensitivity,
            file: SkillFile {
                path: bundled.clone(),
                content: written,
            },
        };
        let recomputed = asset.address().to_hex();
        if recomputed != file.object_hash {
            return Err(format!(
                "{} hashes to {recomputed}, not the approved address {}",
                target.display(),
                file.object_hash
            ));
        }
        receipt_files.insert(bundled.to_string(), recomputed);
    }

    let receipt = Receipt {
        version: 2,
        client: client.to_owned(),
        directory: directory.display().to_string(),
        skill: name.to_string(),
        governing_scope_id: resolved.skill.governing_scope_id,
        distribution_scope_id: resolved.binding.scope_id,
        binding_id: resolved.binding.id,
        version_id: resolved.version.id,
        bundle_digest: resolved.version.bundle_digest,
        sensitivity: resolved.version.sensitivity,
        files: receipt_files,
    };
    let receipt_path = write_receipt(client, &name, &receipt)?;
    Ok((receipt, receipt_path))
}

/// Reconcile a client root to the exact versions enabled at one scope.
pub async fn sync(
    profile: &str,
    scope: ScopeId,
    client: &str,
    root: Option<&Path>,
    dry_run: bool,
    json_out: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    if !json_out {
        announce(&api, &origin);
    }
    let root = root.map_or_else(|| client_root(client), |path| Ok(path.to_path_buf()))?;
    let listing = available_at(&api, scope).await?;
    let mut written = Vec::new();
    let mut unchanged = Vec::new();
    let mut removed = Vec::new();

    for entry in listing.skills.clone() {
        let name = entry
            .name
            .parse::<SkillName>()
            .map_err(|err| err.to_string())?;
        if current(client, &name, entry.version.id, &root)? {
            unchanged.push(entry.name);
            continue;
        }
        if dry_run {
            written.push(Synced {
                skill: entry.name,
                version_id: entry.version.id,
                directory: root.join(name.as_str()).display().to_string(),
                files: 0,
            });
            continue;
        }
        let (receipt, _) = materialise(resolve_entry(&api, entry).await?, client, &root)?;
        written.push(Synced {
            skill: receipt.skill,
            version_id: receipt.version_id,
            directory: receipt.directory,
            files: receipt.files.len(),
        });
    }

    let served = listing
        .skills
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    for receipt in receipts(client)? {
        if receipt.distribution_scope_id != scope
            || served.contains(&receipt.skill.as_str())
            || PathBuf::from(&receipt.directory).parent() != Some(root.as_path())
        {
            continue;
        }
        removed.push(receipt.skill.clone());
        if dry_run {
            continue;
        }
        let directory = PathBuf::from(&receipt.directory);
        if directory.exists() {
            std::fs::remove_dir_all(&directory)
                .map_err(|err| format!("remove {}: {err}", directory.display()))?;
        }
        let name = receipt
            .skill
            .parse::<SkillName>()
            .map_err(|err| err.to_string())?;
        remove_receipt(client, &name)?;
    }

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "client": client,
                "scope_id": scope,
                "root": root.display().to_string(),
                "written": written,
                "unchanged": unchanged,
                "removed": removed,
                "dry_run": dry_run,
            }))
            .map_err(|err| err.to_string())?
        );
    } else {
        println!(
            "synveda: {} {} skill(s) at {} into {}",
            if dry_run { "would sync" } else { "synced" },
            listing.skills.len(),
            scope,
            root.display()
        );
        for entry in written {
            println!("         + {} {}", entry.skill, entry.version_id);
        }
        for skill in removed {
            println!("         - {skill}");
        }
        if !unchanged.is_empty() {
            println!("         = {} unchanged", unchanged.join(", "));
        }
    }
    Ok(())
}

fn current(
    client: &str,
    name: &SkillName,
    version: SkillVersionId,
    root: &Path,
) -> Result<bool, String> {
    let Some(receipt) = read_receipt(client, name)? else {
        return Ok(false);
    };
    holds(&receipt, name, version, root)
}

fn holds(
    receipt: &Receipt,
    name: &SkillName,
    version: SkillVersionId,
    root: &Path,
) -> Result<bool, String> {
    if receipt.version_id != version {
        return Ok(false);
    }
    let directory = PathBuf::from(&receipt.directory);
    if directory.parent() != Some(root) || !directory.is_dir() {
        return Ok(false);
    }
    for (path, address) in &receipt.files {
        let bundled = path
            .parse::<SkillFilePath>()
            .map_err(|err| err.to_string())?;
        let target = directory.join(bundled.as_str());
        let Ok(content) = std::fs::read_to_string(&target) else {
            return Ok(false);
        };
        let asset = SkillAsset {
            scope_id: receipt.governing_scope_id,
            skill: name.clone(),
            sensitivity: receipt.sensitivity,
            file: SkillFile {
                path: bundled,
                content,
            },
        };
        if asset.address().to_hex() != *address {
            return Ok(false);
        }
    }
    Ok(true)
}

fn receipt_dir(client: &str) -> Result<PathBuf, String> {
    Ok(crate::credentials::config_dir()?
        .join("skills")
        .join(client))
}

fn receipts(client: &str) -> Result<Vec<Receipt>, String> {
    let dir = receipt_dir(client)?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<Receipt> = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|err| format!("read {}: {err}", dir.display()))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        if let Ok(receipt) = serde_json::from_str(&body) {
            out.push(receipt);
        }
    }
    out.sort_by(|left, right| left.skill.cmp(&right.skill));
    Ok(out)
}

fn read_receipt(client: &str, name: &SkillName) -> Result<Option<Receipt>, String> {
    let path = receipt_dir(client)?.join(format!("{name}.json"));
    let Ok(body) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    Ok(serde_json::from_str(&body).ok())
}

fn remove_receipt(client: &str, name: &SkillName) -> Result<(), String> {
    let path = receipt_dir(client)?.join(format!("{name}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("remove {}: {err}", path.display())),
    }
}

fn write_receipt(client: &str, name: &SkillName, receipt: &Receipt) -> Result<PathBuf, String> {
    let dir = receipt_dir(client)?;
    std::fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let path = dir.join(format!("{name}.json"));
    let body = serde_json::to_string_pretty(receipt).map_err(|err| err.to_string())?;
    std::fs::write(&path, body).map_err(|err| format!("write {}: {err}", path.display()))?;
    Ok(path)
}

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

fn print_change(response: &Value) {
    println!(
        "synveda: Skill change {} — {}",
        response["change_id"].as_str().unwrap_or_default(),
        response["outcome"].as_str().unwrap_or("unknown")
    );
    if response["outcome"] == "pending_review" {
        println!(
            "         review with synveda proposal review {}",
            response["change_id"].as_str().unwrap_or_default()
        );
    }
}

fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

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

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join("synveda-cpr23-cli")
            .join(format!("{label}-{}", ScopeId::new()));
        std::fs::create_dir_all(&root).expect("create scratch root");
        root
    }

    fn installed(root: &Path, content: &str) -> (Receipt, SkillName, SkillVersionId) {
        let name: SkillName = "code-review".parse().unwrap();
        let governing_scope_id = ScopeId::new();
        let distribution_scope_id = ScopeId::new();
        let version_id = SkillVersionId::new();
        let directory = root.join(name.as_str());
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(SKILL_MANIFEST), content).unwrap();
        let asset = SkillAsset {
            scope_id: governing_scope_id,
            skill: name.clone(),
            sensitivity: Sensitivity::Internal,
            file: SkillFile {
                path: SKILL_MANIFEST.parse().unwrap(),
                content: content.to_owned(),
            },
        };
        let receipt = Receipt {
            version: 2,
            client: "claude-code".to_owned(),
            directory: directory.display().to_string(),
            skill: name.to_string(),
            governing_scope_id,
            distribution_scope_id,
            binding_id: SkillBindingId::new(),
            version_id,
            bundle_digest: "d".repeat(64),
            sensitivity: Sensitivity::Internal,
            files: BTreeMap::from([(SKILL_MANIFEST.to_owned(), asset.address().to_hex())]),
        };
        (receipt, name, version_id)
    }

    #[test]
    fn client_roots_are_relative_and_distinct() {
        assert_ne!(CLIENT_ROOTS[0].1, CLIENT_ROOTS[1].1);
        for (name, root) in CLIENT_ROOTS {
            assert!(!root.starts_with('/'), "{name}");
            assert!(root.ends_with("skills"), "{name}");
        }
    }

    #[test]
    fn exact_version_and_bytes_define_current() {
        let root = scratch("current");
        let (receipt, name, version) =
            installed(&root, "---\nname: code-review\ndescription: Review.\n---\n");
        assert!(holds(&receipt, &name, version, &root).unwrap());
        assert!(!holds(&receipt, &name, SkillVersionId::new(), &root).unwrap());
        std::fs::write(
            root.join("code-review").join(SKILL_MANIFEST),
            "---\nname: code-review\ndescription: Changed.\n---\n",
        )
        .unwrap();
        assert!(!holds(&receipt, &name, version, &root).unwrap());
    }

    #[test]
    fn receipt_stays_outside_the_bundle() {
        let root = scratch("receipt");
        let (receipt, _, _) =
            installed(&root, "---\nname: code-review\ndescription: Review.\n---\n");
        assert_eq!(
            PathBuf::from(&receipt.directory).parent(),
            Some(root.as_path())
        );
        assert!(!receipt.directory.ends_with(".json"));
    }

    #[test]
    fn windows_separators_normalise_before_path_validation() {
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
}
