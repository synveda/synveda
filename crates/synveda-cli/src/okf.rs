//! `synveda okf` — local OKF v0.2 validation and public-API exchange
//! (CPR-28, ADR-0087).
//!
//! The CLI owns only the path the person selected. It applies the same pure,
//! pinned adapter as the gateway, packages inert bytes, and calls the public
//! project API for every governed operation. It never writes a database,
//! runs Git, fetches a source URL or publishes Knowledge directly.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synveda_okf::{
    BundleEncoding, BundleInput, BundleInspection, ExportBundle, InputEntryKind,
    KnowledgeFormatAdapter, MAX_ARCHIVE_BYTES, OkfAdapter, SourceDescriptor, SourceKind,
    normalise_logical_path, read_local_directory, validate_export_bundle,
};
use synveda_types::{ImportJobId, KnowledgeItemId, ProjectId};

use crate::api::{Api, Origin};

#[derive(Debug)]
struct LoadedBundle {
    inspection: BundleInspection,
    request: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct JobView {
    id: ImportJobId,
    project_id: ProjectId,
    format_version: String,
    specification_commit: String,
    source_kind: String,
    source_locator: String,
    source_revision: Option<String>,
    bundle_digest: String,
    state: String,
    artifact_count: i32,
    mapping_count: i32,
    candidate_count: i32,
    capture_batch_id: Option<String>,
    notices: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ArtifactView {
    logical_path: String,
    kind: String,
    content_hash: String,
    frontmatter: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MappingView {
    okf_type: String,
    knowledge_type: String,
    classification: String,
    content_hash: String,
    materializable: bool,
    matched_item_id: Option<KnowledgeItemId>,
    candidate_id: Option<String>,
    content: Value,
    proposed_relations: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PlanView {
    job: JobView,
    artifacts: Vec<ArtifactView>,
    mappings: Vec<MappingView>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CandidateView {
    id: String,
    state: String,
    content: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MaterializationView {
    job: JobView,
    candidates: Vec<CandidateView>,
}

/// Validate one local bundle without making a network call.
pub fn validate(
    path: &Path,
    source_revision: Option<&str>,
    json_output: bool,
) -> Result<(), String> {
    let loaded = load(path, source_revision)?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&loaded.inspection).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    let inspection = loaded.inspection;
    println!(
        "valid OKF {} — {} artifact(s), {} concept(s), digest {}",
        inspection.format_version,
        inspection.artifacts.len(),
        inspection.concepts.len(),
        short(&inspection.bundle_digest),
    );
    println!("specification commit {}", inspection.specification_commit);
    for notice in inspection.notices {
        println!("notice: {notice}");
    }
    Ok(())
}

/// Inspect one local bundle without making a network call.
pub fn inspect(
    path: &Path,
    source_revision: Option<&str>,
    json_output: bool,
) -> Result<(), String> {
    let inspection = load(path, source_revision)?.inspection;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&inspection).map_err(|err| err.to_string())?
        );
        return Ok(());
    }

    println!(
        "OKF {} at {} ({})",
        inspection.format_version,
        inspection.source.locator,
        inspection.source.kind.as_str(),
    );
    if let Some(revision) = &inspection.source.revision {
        println!("source revision {revision}");
    }
    println!(
        "{} artifact(s), {} concept(s), bundle {}",
        inspection.artifacts.len(),
        inspection.concepts.len(),
        short(&inspection.bundle_digest),
    );
    for artifact in &inspection.artifacts {
        println!(
            "\n{} · {:?} · {}",
            artifact.logical_path,
            artifact.kind,
            short(&artifact.content_hash),
        );
        println!(
            "frontmatter {}",
            serde_json::to_string(&artifact.frontmatter).map_err(|err| err.to_string())?
        );
        if let Some(concept) = inspection
            .concepts
            .iter()
            .find(|concept| concept.logical_path == artifact.logical_path)
        {
            println!(
                "type {} → {} · {}",
                concept.okf_type,
                concept.knowledge_type.as_str(),
                if concept.materializable {
                    "reviewable"
                } else {
                    "retained, not materializable"
                },
            );
            if !concept.links.is_empty() {
                println!("links {}", concept.links.len());
            }
        }
    }
    for notice in inspection.notices {
        println!("\nnotice: {notice}");
    }
    Ok(())
}

/// Persist a dry-run plan and optionally materialise its review candidates.
pub async fn import(
    profile: &str,
    path: &Path,
    project: ProjectId,
    source_revision: Option<&str>,
    dry_run: bool,
    json_output: bool,
) -> Result<(), String> {
    let loaded = load(path, source_revision)?;
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let plan_key = stable_key("okf-plan", &loaded.request)?;
    let plan: PlanView = api
        .post_idempotent_as(
            &format!("/v1/projects/{project}/okf/imports"),
            Some(loaded.request),
            &plan_key,
        )
        .await?;

    if dry_run {
        render_plan(&plan, json_output)?;
        if !json_output {
            println!(
                "dry-run only: no candidate or Knowledge item was created; materialise job {} by rerunning without --dry-run",
                plan.job.id
            );
        }
        return Ok(());
    }

    let materialized: MaterializationView = api
        .post_idempotent_as(
            &format!("/v1/okf/imports/{}/materialize", plan.job.id),
            None,
            &format!("okf-materialize-{}", plan.job.id),
        )
        .await?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&materialized).map_err(|err| err.to_string())?
        );
        return Ok(());
    }

    render_plan(&plan, false)?;
    println!(
        "\nmaterialized {} reviewable candidate(s) in batch {}",
        materialized.candidates.len(),
        materialized
            .job
            .capture_batch_id
            .as_deref()
            .unwrap_or("unknown"),
    );
    println!(
        "review them at {}/console/learnings (nothing is active Knowledge until accepted)",
        api.gateway()
    );
    Ok(())
}

/// Export current, visible project Knowledge and atomically materialise the
/// exact returned OKF files under a new local directory.
pub async fn export(
    profile: &str,
    project: ProjectId,
    output: &Path,
    item_ids: &[KnowledgeItemId],
    json_output: bool,
) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let bundle: ExportBundle = api
        .post_as(
            &format!("/v1/projects/{project}/okf/exports"),
            Some(json!({ "item_ids": item_ids })),
        )
        .await?;
    validate_export_bundle(&bundle).map_err(|err| err.to_string())?;
    write_export(output, &bundle)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "output": output,
                "format_version": bundle.format_version,
                "specification_commit": bundle.specification_commit,
                "bundle_digest": bundle.bundle_digest,
                "files": bundle.files,
            }))
            .map_err(|err| err.to_string())?
        );
    } else {
        println!(
            "exported OKF {} to {} — {} file(s), digest {}",
            bundle.format_version,
            output.display(),
            bundle.files.len(),
            short(&bundle.bundle_digest),
        );
    }
    Ok(())
}

fn load(path: &Path, source_revision: Option<&str>) -> Result<LoadedBundle, String> {
    let revision = source_revision
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if source_revision.is_some() && revision.is_none() {
        return Err("--source-revision cannot be blank".to_owned());
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|err| format!("inspect {}: {err}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("OKF input cannot be a symlink: {}", path.display()));
    }
    let locator = source_label(path)?;

    let (source, encoding, input) = if metadata.is_dir() {
        let entries = read_local_directory(path).map_err(|err| err.to_string())?;
        let kind = if revision.is_some() {
            SourceKind::Git
        } else {
            SourceKind::Directory
        };
        (
            SourceDescriptor {
                kind,
                locator,
                revision,
            },
            BundleEncoding::Entries,
            BundleInput::Entries(entries),
        )
    } else if metadata.is_file() {
        if revision.is_some() {
            return Err("--source-revision is valid only for a checked-out directory".to_owned());
        }
        let (kind, encoding) = archive_kind(path)?;
        if metadata.len() > MAX_ARCHIVE_BYTES as u64 {
            return Err(format!(
                "OKF archive exceeds {MAX_ARCHIVE_BYTES} bytes: {}",
                path.display()
            ));
        }
        let bytes = fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
        let input = match encoding {
            BundleEncoding::Zip => BundleInput::Zip(bytes),
            BundleEncoding::Tar => BundleInput::Tar(bytes),
            BundleEncoding::TarGzip => BundleInput::TarGzip(bytes),
            BundleEncoding::Entries => unreachable!("an archive is not entry encoded"),
        };
        (
            SourceDescriptor {
                kind,
                locator,
                revision: None,
            },
            encoding,
            input,
        )
    } else {
        return Err(format!(
            "OKF input must be a directory, zip, tar or tar-gzip archive: {}",
            path.display()
        ));
    };

    let inspection = OkfAdapter
        .inspect(source.clone(), input.clone(), Utc::now())
        .map_err(|err| err.to_string())?;
    let request = wire_request(&source, encoding, &input)?;
    Ok(LoadedBundle {
        inspection,
        request,
    })
}

fn source_label(path: &Path) -> Result<String, String> {
    let canonical =
        fs::canonicalize(path).map_err(|err| format!("resolve {}: {err}", path.display()))?;
    canonical
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "OKF source name is not valid UTF-8".to_owned())
}

fn archive_kind(path: &Path) -> Result<(SourceKind, BundleEncoding), String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".zip") {
        Ok((SourceKind::Zip, BundleEncoding::Zip))
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Ok((SourceKind::Tar, BundleEncoding::TarGzip))
    } else if name.ends_with(".tar") {
        Ok((SourceKind::Tar, BundleEncoding::Tar))
    } else {
        Err(format!(
            "unsupported OKF archive {}; expected .zip, .tar, .tar.gz or .tgz",
            path.display()
        ))
    }
}

fn wire_request(
    source: &SourceDescriptor,
    encoding: BundleEncoding,
    input: &BundleInput,
) -> Result<Value, String> {
    let (entries, archive_base64) = match input {
        BundleInput::Entries(entries) => (
            entries
                .iter()
                .map(|entry| {
                    json!({
                        "logical_path": entry.logical_path,
                        "kind": entry_kind(entry.kind),
                        "content_base64": STANDARD.encode(&entry.bytes),
                    })
                })
                .collect::<Vec<_>>(),
            None,
        ),
        BundleInput::Zip(bytes) | BundleInput::Tar(bytes) | BundleInput::TarGzip(bytes) => {
            (Vec::new(), Some(STANDARD.encode(bytes)))
        }
        BundleInput::Directory(_) => {
            return Err("a gateway request cannot carry a local directory path".to_owned());
        }
    };
    Ok(json!({
        "source_kind": source.kind.as_str(),
        "source_locator": source.locator,
        "source_revision": source.revision,
        "encoding": encoding.as_str(),
        "entries": entries,
        "archive_base64": archive_base64,
    }))
}

fn entry_kind(kind: InputEntryKind) -> &'static str {
    match kind {
        InputEntryKind::File => "file",
        InputEntryKind::Directory => "directory",
        InputEntryKind::Symlink => "symlink",
        InputEntryKind::Special => "special",
    }
}

fn stable_key(prefix: &str, request: &Value) -> Result<String, String> {
    let canonical = synveda_types::json::canonicalise(request);
    let digest = Sha256::digest(
        serde_json::to_vec(&canonical).map_err(|err| format!("encode OKF request: {err}"))?,
    );
    Ok(format!("{prefix}-{}", hex(&digest)))
}

fn render_plan(plan: &PlanView, json_output: bool) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(plan).map_err(|err| err.to_string())?
        );
        return Ok(());
    }
    let mut classes = BTreeMap::<&str, usize>::new();
    for mapping in &plan.mappings {
        *classes.entry(mapping.classification.as_str()).or_default() += 1;
    }
    println!(
        "OKF import {} · {} · source {}{}",
        plan.job.id,
        plan.job.state,
        plan.job.source_locator,
        plan.job
            .source_revision
            .as_deref()
            .map_or_else(String::new, |revision| format!(" @ {revision}")),
    );
    println!(
        "validated {} artifact(s), {} mapping(s): {} addition, {} update, {} duplicate, {} conflict",
        plan.artifacts.len(),
        plan.mappings.len(),
        classes.get("addition").copied().unwrap_or_default(),
        classes.get("update").copied().unwrap_or_default(),
        classes.get("duplicate").copied().unwrap_or_default(),
        classes.get("conflict").copied().unwrap_or_default(),
    );
    for mapping in &plan.mappings {
        let title = mapping
            .content
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled");
        println!(
            "  {} · {} → {} · {}{}",
            mapping.classification,
            mapping.okf_type,
            mapping.knowledge_type,
            title,
            mapping
                .matched_item_id
                .map_or_else(String::new, |id| format!(" · matches {id}")),
        );
    }
    for notice in &plan.job.notices {
        println!("notice: {notice}");
    }
    Ok(())
}

fn write_export(output: &Path, bundle: &ExportBundle) -> Result<(), String> {
    validate_export_bundle(bundle).map_err(|err| err.to_string())?;
    if fs::symlink_metadata(output).is_ok() {
        return Err(format!(
            "refusing to overwrite existing OKF export path {}",
            output.display()
        ));
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "OKF export output needs a directory name".to_owned())?;
    let stage = parent.join(format!(".{name}.synveda-okf-{}.tmp", ImportJobId::new()));
    let result = (|| {
        create_private_dir(&stage)?;
        for file in &bundle.files {
            let logical =
                normalise_logical_path(&file.logical_path).map_err(|err| err.to_string())?;
            let mut target = stage.clone();
            for component in logical.split('/') {
                target.push(component);
            }
            let directory = target
                .parent()
                .ok_or_else(|| "OKF export target has no parent".to_owned())?;
            fs::create_dir_all(directory)
                .map_err(|err| format!("create {}: {err}", directory.display()))?;
            let mut handle = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&target)
                .map_err(|err| format!("create {}: {err}", target.display()))?;
            handle
                .write_all(file.content.as_bytes())
                .map_err(|err| format!("write {}: {err}", target.display()))?;
            handle
                .sync_all()
                .map_err(|err| format!("sync {}: {err}", target.display()))?;
        }
        fs::rename(&stage, output).map_err(|err| {
            format!(
                "publish OKF export {} as {}: {err}",
                stage.display(),
                output.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() && stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(path)
            .map_err(|err| format!("create {}: {err}", path.display()))
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path).map_err(|err| format!("create {}: {err}", path.display()))
    }
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

fn short(value: &str) -> String {
    value.chars().take(12).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join("synveda-cpr28-okf")
            .join(format!("{label}-{}", ImportJobId::new()));
        fs::create_dir_all(&root).expect("create scratch");
        root
    }

    #[test]
    fn local_tree_uses_the_pinned_adapter_and_packages_no_git_admin_path() {
        let root = scratch("tree");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "credential = never-send-me").unwrap();
        fs::write(
            root.join("convention.md"),
            "---\ntype: pulseboard-custom\ntitle: Request IDs\nx-owner: platform\n---\nUse traceparent.\n",
        )
        .unwrap();

        let loaded = load(&root, Some("abc123")).expect("valid checked-out tree");
        assert_eq!(loaded.inspection.format_version, "0.2");
        assert_eq!(loaded.inspection.source.kind, SourceKind::Git);
        assert_eq!(loaded.inspection.source.revision.as_deref(), Some("abc123"));
        assert_eq!(loaded.inspection.concepts[0].okf_type, "pulseboard-custom");
        let encoded = loaded.request.to_string();
        assert!(!encoded.contains(".git"), "{encoded}");
        assert!(!encoded.contains("never-send-me"), "{encoded}");
        assert_eq!(
            loaded.inspection.artifacts[0].frontmatter["x-owner"], "platform",
            "unknown metadata must survive inspection",
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stable_plan_keys_bind_source_metadata_as_well_as_bundle_bytes() {
        let first = json!({ "source_locator": "one", "entries": [{"a": 1}] });
        let second = json!({ "source_locator": "two", "entries": [{"a": 1}] });
        assert_eq!(
            stable_key("okf-plan", &first).unwrap(),
            stable_key("okf-plan", &first).unwrap()
        );
        assert_ne!(
            stable_key("okf-plan", &first).unwrap(),
            stable_key("okf-plan", &second).unwrap()
        );
    }

    #[test]
    fn export_is_verified_written_atomically_and_never_overwritten() {
        let root = scratch("export");
        let output = root.join("bundle");
        let bundle = OkfAdapter.export(&[]).expect("empty deterministic bundle");
        write_export(&output, &bundle).expect("write");
        assert!(output.join("index.md").is_file());
        assert!(
            write_export(&output, &bundle)
                .expect_err("overwrite must fail")
                .contains("refusing to overwrite")
        );

        let mut hostile = bundle;
        hostile.files[0].logical_path = "../escape.md".to_owned();
        assert!(write_export(&root.join("hostile"), &hostile).is_err());
        assert!(!root.join("escape.md").exists());

        let mut duplicate_index = OkfAdapter.export(&[]).expect("empty deterministic bundle");
        duplicate_index.files.push(duplicate_index.files[0].clone());
        assert!(
            validate_export_bundle(&duplicate_index).is_err(),
            "a second index path must never reach the filesystem",
        );
        let _ = fs::remove_dir_all(root);
    }
}
