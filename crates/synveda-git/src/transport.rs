use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use synveda_types::{Error, Result, ScopeId, TenantId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::projection::{
    MAX_BYTES, Manifest, Projection, invalid, json_bytes, valid_oid, verify_signatures,
};
use crate::validate_target;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum retained export states admitted by the export/verification contract.
pub const MAX_EXPORT_STATES: usize = 1024;
/// Maximum cumulative manifest bytes; bounded verification also gates exports.
pub const MAX_VERIFICATION_BYTES: usize = MAX_BYTES * 8;

fn transport_error() -> Error {
    Error::Internal {
        message: "local Git transport failed; inspect destination custody and Git availability"
            .into(),
    }
}

fn divergence() -> Error {
    Error::Conflict { message: "Git export destination diverged from its recorded cursor; retain it and use a governed new target identifier".into() }
}

/// Replaceable transport seam. Caller authorization and persisted disclosure
/// intent must precede this effect; transport state grants no authority.
pub trait Transport {
    /// Advance exactly one ref with a recorded previous-head precondition.
    fn advance(
        &self,
        projection: &Projection,
        previous: Option<&str>,
    ) -> impl Future<Output = Result<bool>> + Send;
}

/// Operator-owned private root for local bare Git repositories. No network
/// protocol, credential helper or caller-selected path is admitted.
#[derive(Clone)]
pub struct LocalTransport {
    root: PathBuf,
}

impl LocalTransport {
    /// Admit an existing private absolute directory. Refuse unsupported hosts,
    /// symlinks, other owners and group/world-accessible roots.
    pub fn new(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() {
            return Err(transport_error());
        }
        let mut prefix = PathBuf::new();
        for component in root.components() {
            if !matches!(
                component,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            ) {
                return Err(transport_error());
            }
            prefix.push(component);
            let metadata = std::fs::symlink_metadata(&prefix).map_err(|_| transport_error())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(transport_error());
            }
        }
        private_directory(&root)?;
        Ok(Self { root })
    }

    /// Derive the destination from typed tenant/scope identity and a governed
    /// target ID. Used by operators and acceptance; paths stay out of audit.
    pub fn repository(&self, tenant: TenantId, scope: ScopeId, target: &str) -> Result<PathBuf> {
        validate_target(target)?;
        Ok(self
            .root
            .join(tenant.to_string())
            .join(scope.to_string())
            .join(format!("{target}.git")))
    }

    async fn prepare_repository(&self, projection: &Projection) -> Result<PathBuf> {
        private_directory(&self.root)?;
        let source = &projection.manifest.snapshot;
        let repo = self.repository(source.tenant, source.scope, &source.target)?;
        let relative = repo
            .strip_prefix(&self.root)
            .map_err(|_| transport_error())?;
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component);
            create_private_directory(&current)?;
        }
        if !repo.join("HEAD").exists() {
            // Empty templates prevent inherited executable hooks. Only a new
            // empty owned directory may be initialized, never a retained repo.
            if std::fs::read_dir(&repo)
                .map_err(|_| transport_error())?
                .next()
                .is_some()
            {
                return Err(divergence());
            }
            command(
                &repo,
                &["init", "--bare", "--object-format=sha1", "--template=", "."],
                &[],
                4096,
            )
            .await?;
            command(
                &repo,
                &["symbolic-ref", "HEAD", &source.git_ref()],
                &[],
                4096,
            )
            .await?;
        }
        let bare = command(&repo, &["rev-parse", "--is-bare-repository"], &[], 128).await?;
        let format = command(&repo, &["rev-parse", "--show-object-format"], &[], 128).await?;
        if bare != b"true\n" || format != b"sha1\n" {
            return Err(divergence());
        }
        Ok(repo)
    }

    /// Check the observed branch without writing any object. A deleted
    /// repository/ref after a completed export is a divergence as well.
    pub async fn observed(&self, projection: &Projection) -> Result<Option<String>> {
        let source = &projection.manifest.snapshot;
        let repo = self.repository(source.tenant, source.scope, &source.target)?;
        if !repo.exists() {
            return Ok(None);
        }
        private_directory(&repo)?;
        if !repo.join("HEAD").is_file() {
            return Err(divergence());
        }
        read_ref(&repo, &source.git_ref()).await
    }
}

impl Transport for LocalTransport {
    async fn advance(&self, projection: &Projection, previous: Option<&str>) -> Result<bool> {
        if previous != projection.manifest.previous_state.as_deref() {
            return Err(divergence());
        }
        let source = &projection.manifest.snapshot;
        let path = self.repository(source.tenant, source.scope, &source.target)?;
        if previous.is_some() && !path.join("HEAD").exists() {
            return Err(divergence());
        }
        let repo = self.prepare_repository(projection).await?;
        let observed = read_ref(&repo, &source.git_ref()).await?;
        if observed.as_deref() == Some(&projection.head) {
            verify_objects(&repo, projection).await?;
            return Ok(false);
        }
        if observed.as_deref() != previous {
            return Err(divergence());
        }
        let pack = projection.pack()?;
        command(&repo, &["index-pack", "--stdin", "--strict"], &pack, 4096).await?;
        verify_objects(&repo, projection).await?;
        // Git owns the ref lock and atomically checks the old head. No force
        // flag and no symbolic-ref following can redirect this disclosure.
        let expected = previous.unwrap_or("0000000000000000000000000000000000000000");
        if command(
            &repo,
            &[
                "update-ref",
                "--no-deref",
                &source.git_ref(),
                &projection.head,
                expected,
            ],
            &[],
            4096,
        )
        .await
        .is_err()
        {
            return Err(divergence());
        }
        Ok(true)
    }
}

/// Verify every retained source hash/signature and mapped Git object in a bare
/// repository or ordinary clone. Signed evidence requires a trusted key map.
/// The manifest is data only; its exact expected Git bytes are recomputed.
/// The retained export-state chain is bounded to 1,024 states / 64 MiB of
/// manifests; overflow refuses. The latest manifest is returned on success.
pub async fn verify_repository(
    repository: &Path,
    reference: &str,
    keys: &BTreeMap<String, [u8; 32]>,
) -> Result<Manifest> {
    if !matches!(
        reference,
        "refs/heads/synveda/prompt/published"
            | "refs/heads/synveda/prompt/staged"
            | "refs/heads/synveda/context-pack/published"
            | "refs/heads/synveda/context-pack/staged"
    ) {
        return Err(invalid());
    }
    let git_dir = if repository.join(".git").is_dir() {
        repository.join(".git")
    } else {
        repository.to_owned()
    };
    let mut head = read_ref(&git_dir, reference).await?.ok_or_else(invalid)?;
    let mut latest: Option<Manifest> = None;
    let mut states = BTreeSet::new();
    let mut verified = BTreeSet::new();
    let mut bytes = 0usize;
    loop {
        if states.len() >= MAX_EXPORT_STATES || !states.insert(head.clone()) {
            return Err(invalid());
        }
        let manifest_bytes = command(
            &git_dir,
            &["cat-file", "blob", &format!("{head}:.synveda/export.json")],
            &[],
            (MAX_VERIFICATION_BYTES - bytes).min(MAX_BYTES * 2),
        )
        .await?;
        bytes += manifest_bytes.len();
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|_| invalid())?;
        if latest.as_ref().is_some_and(|first| {
            first.snapshot.tenant != manifest.snapshot.tenant
                || first.snapshot.scope != manifest.snapshot.scope
                || first.snapshot.target != manifest.snapshot.target
                || first.snapshot.asset != manifest.snapshot.asset
                || first.snapshot.channel != manifest.snapshot.channel
        }) {
            return Err(invalid());
        }
        manifest.snapshot.validate().map_err(|_| invalid())?;
        verify_signatures(&manifest.snapshot, keys)?;
        let mut projection =
            Projection::render(manifest.snapshot.clone(), manifest.previous_state.clone())?;
        if projection.head != head
            || manifest.snapshot.git_ref() != reference
            || json_bytes(&projection.manifest)? != manifest_bytes
        {
            return Err(invalid());
        }
        projection.objects.retain(|oid, _| !verified.contains(oid));
        if !projection.objects.is_empty() {
            verify_objects(&git_dir, &projection).await?;
            verified.extend(projection.objects.into_keys());
        }
        let previous = manifest.previous_state.clone();
        if latest.is_none() {
            latest = Some(manifest);
        }
        let Some(previous) = previous else { break };
        head = previous;
    }
    latest.ok_or_else(invalid)
}

async fn verify_objects(git_dir: &Path, projection: &Projection) -> Result<()> {
    let input = projection
        .objects
        .keys()
        .map(|oid| format!("{oid}\n"))
        .collect::<String>();
    let contents = command(
        git_dir,
        &["cat-file", "--batch"],
        input.as_bytes(),
        MAX_BYTES * 5,
    )
    .await?;
    let mut rest = contents.as_slice();
    for (oid, object) in &projection.objects {
        let end = rest.iter().position(|b| *b == b'\n').ok_or_else(invalid)?;
        let expected = format!("{oid} {} {}", object.kind, object.bytes.len());
        if &rest[..end] != expected.as_bytes() {
            return Err(invalid());
        }
        rest = &rest[end + 1..];
        let length = object.bytes.len();
        if rest.len() <= length || rest[..length] != object.bytes || rest[length] != b'\n' {
            return Err(invalid());
        }
        rest = &rest[length + 1..];
    }
    if !rest.is_empty() {
        return Err(invalid());
    }
    Ok(())
}

async fn read_ref(repo: &Path, reference: &str) -> Result<Option<String>> {
    // for-each-ref omits dangling symbolic refs. Probe their existence as
    // well, so a first export cannot replace a retained symbolic pointer.
    let symbolic = run_output(
        git_command(repo, &["symbolic-ref", "-q", reference]),
        &[],
        4096,
        COMMAND_TIMEOUT,
    )
    .await?;
    match symbolic.code {
        Some(0) => return Err(divergence()),
        Some(1) => {}
        _ => return Err(transport_error()),
    }
    let refs = command(
        repo,
        &[
            "for-each-ref",
            "--format=%(objectname) %(symref)",
            reference,
        ],
        &[],
        256,
    )
    .await?;
    if refs.is_empty() {
        return Ok(None);
    }
    let line = std::str::from_utf8(&refs).map_err(|_| divergence())?;
    // A symbolic ref, extra matching ref or malformed address refuses.
    let oid = line
        .strip_suffix(" \n")
        .filter(|oid| valid_oid(oid))
        .ok_or_else(divergence)?;
    Ok(Some(oid.to_owned()))
}

async fn command(repo: &Path, args: &[&str], input: &[u8], limit: usize) -> Result<Vec<u8>> {
    run_command(git_command(repo, args), input, limit, COMMAND_TIMEOUT).await
}

fn git_command(repo: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        // An empty environment allowlist overrides repository-specific
        // protocol exceptions, including promisor remotes in an input clone.
        .env("GIT_ALLOW_PROTOCOL", "")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("protocol.allow=never")
        .arg("-c")
        .arg("core.commitGraph=false")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("credential.helper=")
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .current_dir(repo);
    command
}

async fn run_command(
    command: Command,
    input: &[u8],
    limit: usize,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let output = run_output(command, input, limit, timeout).await?;
    if output.code != Some(0) {
        return Err(transport_error());
    }
    Ok(output.bytes)
}

struct CommandOutput {
    code: Option<i32>,
    bytes: Vec<u8>,
}

async fn run_output(
    mut command: Command,
    input: &[u8],
    limit: usize,
    timeout: Duration,
) -> Result<CommandOutput> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| transport_error())?;
    let mut stdin = child.stdin.take().ok_or_else(transport_error)?;
    let stdout = child.stdout.take().ok_or_else(transport_error)?;
    let stderr = child.stderr.take().ok_or_else(transport_error)?;
    let work = async {
        let write = async move {
            stdin.write_all(input).await?;
            stdin.shutdown().await?;
            // ChildStdin::shutdown flushes but does not close every platform's
            // pipe. Git batch readers need EOF while stdout is being drained.
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let read_out = async {
            let mut out = Vec::new();
            stdout.take(limit as u64 + 1).read_to_end(&mut out).await?;
            Ok::<_, std::io::Error>(out)
        };
        let read_err = async {
            let mut out = Vec::new();
            stderr.take(4097).read_to_end(&mut out).await?;
            Ok::<_, std::io::Error>(out)
        };
        let (_, out, err) =
            tokio::try_join!(write, read_out, read_err).map_err(|_| transport_error())?;
        let status = child.wait().await.map_err(|_| transport_error())?;
        if out.len() > limit || err.len() > 4096 {
            return Err(transport_error());
        }
        Ok(CommandOutput {
            code: status.code(),
            bytes: out,
        })
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| Error::Internal {
            message: "local Git transport timed out".into(),
        })?
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| transport_error())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(transport_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn private_directory(_path: &Path) -> Result<()> {
    Err(transport_error())
}

fn create_private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(transport_error()),
        }
    }
    private_directory(path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subprocess_bounds_and_errors_do_not_disclose_child_output() {
        let mut output = Command::new("/bin/sh");
        output.args(["-c", "printf 123456789"]);
        assert!(
            run_command(output, &[], 8, Duration::from_secs(1))
                .await
                .is_err()
        );
        let mut failure = Command::new("/bin/sh");
        failure.args(["-c", "printf 'source-content-secret' >&2; exit 1"]);
        let error = run_command(failure, &[], 8, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("source-content-secret"));
        let mut timeout = Command::new("/bin/sleep");
        timeout.arg("30");
        let error = run_command(timeout, &[], 8, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn cancellation_kills_the_native_child() {
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("pid");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf '%s' \"$$\" > \"$1\"; exec /bin/sleep 30",
            "test",
        ]);
        command.arg(&pid_file);
        let task =
            tokio::spawn(async { run_command(command, &[], 8, Duration::from_secs(15)).await });
        let pid = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(&pid_file)
                    && pid.parse::<u32>().is_ok()
                {
                    break pid;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let alive = Command::new("/bin/kill")
                    .args(["-0", &pid])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await
                    .unwrap();
                if !alive.success() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled child remained alive");
    }
}
