//! Thin, receipt-bound native routes to the packaged consumer Compose graph.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::local_state;

#[derive(Args)]
pub(crate) struct Options {
    /// Extracted plain-Compose candidate; defaults to $SYNVEDA_HOME/reference/current.
    #[arg(long)]
    pub bundle: Option<PathBuf>,
    /// The consumer project, fixed for the lifetime of its retained volumes.
    #[arg(long, default_value = "synveda-local")]
    pub project_name: String,
    /// Show fixed Compose arguments without contacting Docker or changing files.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Up,
    Down,
    Status,
    Logs(u16),
    Doctor,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u8,
    bundle: PathBuf,
    project: String,
    engine: String,
    files: BTreeMap<String, String>,
}

pub(crate) fn run(options: &Options, action: Action) -> Result<(), String> {
    validate_project(&options.project_name)?;
    let bundle = locate(options.bundle.as_deref())?;
    let files = inventory(&bundle)?;
    let args = compose_args(&bundle, &options.project_name, action);
    if options.dry_run {
        println!(
            "docker {}",
            args.iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!(
            "Volumes and keys are retained. This command uses the unpublished consumer candidate."
        );
        return Ok(());
    }
    let engine = preflight()?;
    let path = local_state::receipt("deployment", &options.project_name)?;
    let expected = Receipt {
        version: 1,
        bundle,
        project: options.project_name.clone(),
        engine,
        files,
    };
    let existing: Option<Receipt> = local_state::read_json(&path)?;
    if existing
        .as_ref()
        .is_some_and(|receipt| receipt != &expected)
    {
        return Err("deployment receipt differs from this bundle, engine or project; preserve the original bundle and retained volumes; no implicit upgrade or adoption is supported".to_owned());
    }
    if existing.is_none() {
        require_empty(&options.project_name)?;
        if matches!(action, Action::Up) {
            // An unsuccessful start may allocate volumes. Persist ownership first
            // so the exact same command can resume without adopting foreign data.
            local_state::save(&path, &expected)?;
        } else if !matches!(action, Action::Doctor) {
            return Err("no native deployment receipt; use `synveda up` with an empty consumer project first".to_owned());
        }
    }
    let mut command = docker();
    command.current_dir(&expected.bundle).args(&args);
    run_visible(
        command,
        if matches!(action, Action::Up) {
            660
        } else {
            120
        },
    )?;
    match action {
        Action::Up => println!(
            "Consumer Compose is ready. From the bundle directory, retrieve the first password deliberately with `docker compose --env-file /dev/null --project-directory . -p {} -f deploy/compose/consumer-runtime.yaml run --rm --no-deps credentials`; see CONSUMER.md.",
            expected.project
        ),
        Action::Down => println!(
            "Stopped the consumer project; databases, identity, keys and recovery volumes are retained."
        ),
        Action::Doctor => println!(
            "Local Linux Docker, Compose >=2.35, candidate files and ownership checks passed. This does not establish login, recovery or harness loading."
        ),
        _ => {}
    }
    Ok(())
}

fn locate(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => crate::plugin::synveda_home()?.join("reference/current"),
    };
    let path = path.canonicalize().map_err(|e| {
        format!("locate consumer bundle: {e}; pass --bundle with an extracted candidate")
    })?;
    if !path.join("compose.yaml").is_file()
        || !path.join("deploy/compose/consumer-runtime.yaml").is_file()
    {
        return Err("this is not a plain-Compose consumer candidate; published v0.4.0 uses its synveda-compose launcher".to_owned());
    }
    Ok(path)
}

fn validate_project(project: &str) -> Result<(), String> {
    if project == "synveda-local"
        || (project.starts_with("synveda-local-acceptance-")
            && project.len() > "synveda-local-acceptance-".len()
            && project.len() <= 80
            && project
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    {
        return Ok(());
    }
    Err("project must be synveda-local or an isolated synveda-local-acceptance-<name>".to_owned())
}

fn inventory(bundle: &Path) -> Result<BTreeMap<String, String>, String> {
    let manifest: Value = serde_json::from_slice(
        &local_state::read(&bundle.join("environment.json"))?.ok_or("missing environment.json")?,
    )
    .map_err(|_| "invalid environment.json")?;
    if manifest["schema_version"] != 1
        || manifest["deployment_contract"] != "CPR-45/ADR-0102"
        || manifest["release_version"].as_str() != Some(env!("CARGO_PKG_VERSION"))
    {
        return Err(
            "consumer bundle contract/version differs from this CLI; use matching artifacts"
                .to_owned(),
        );
    }
    let mut files = BTreeMap::new();
    let mut size = 0usize;
    let mut entries = 0usize;
    for name in [
        "compose.yaml",
        "environment.json",
        "evaluation.json",
        "deploy/compose",
    ] {
        visit(
            bundle,
            &bundle.join(name),
            &mut files,
            &mut size,
            &mut entries,
        )?;
    }
    Ok(files)
}

fn visit(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, String>,
    total: &mut usize,
    entries: &mut usize,
) -> Result<(), String> {
    *entries += 1;
    if *entries > 1024 {
        return Err("consumer bundle exceeds entry bound".to_owned());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("inspect bundle: {e}"))?;
    if metadata.is_symlink() {
        return Err("consumer bundle must not contain symlinks".to_owned());
    }
    if metadata.is_dir() {
        let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
        if relative.components().count() > 12 {
            return Err("consumer bundle exceeds directory bound".to_owned());
        }
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            visit(
                root,
                &entry.map_err(|e| e.to_string())?.path(),
                files,
                total,
                entries,
            )?;
        }
    } else {
        let bytes = local_state::read(path)?.ok_or("bundle file disappeared")?;
        *total += bytes.len();
        if files.len() >= 512 || *total > 32 * 1024 * 1024 {
            return Err("consumer bundle exceeds inventory bound".to_owned());
        }
        let name = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("bundle paths must be UTF-8")?;
        files.insert(name.to_owned(), local_state::digest(&bytes));
    }
    Ok(())
}

fn docker() -> Command {
    let mut command = Command::new("docker");
    command.env_clear();
    for name in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "DOCKER_CONFIG",
        "DOCKER_CONTEXT",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("COMPOSE_DISABLE_ENV_FILE", "1")
        .stdin(Stdio::null());
    command
}

fn preflight() -> Result<String, String> {
    if std::env::var_os("DOCKER_HOST").is_some() {
        return Err(
            "native consumer commands require a local Docker context, without DOCKER_HOST"
                .to_owned(),
        );
    }
    let endpoint = capture(&[
        "context",
        "inspect",
        "--format",
        "{{.Endpoints.docker.Host}}",
    ])?;
    if !endpoint.trim().starts_with("unix:///") {
        return Err("native consumer lifecycle requires a local Unix-socket Docker context; remote engines are refused".to_owned());
    }
    let info = capture(&["info", "--format", "{{.OSType}} {{.ID}}"])?;
    let mut fields = info.split_whitespace();
    if fields.next() != Some("linux") {
        return Err("consumer images require a Linux Docker engine".to_owned());
    }
    let engine = fields
        .next()
        .filter(|id| !id.is_empty())
        .ok_or("Docker did not identify its engine")?
        .to_owned();
    if fields.next().is_some() {
        return Err("ambiguous Docker engine identity".to_owned());
    }
    let version = capture(&["compose", "version", "--short"])?;
    let parts: Vec<_> = version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .take(2)
        .map(str::parse::<u32>)
        .collect();
    if !matches!(parts.as_slice(), [Ok(major), Ok(minor)] if *major > 2 || (*major == 2 && *minor >= 35))
    {
        return Err("the consumer candidate requires Docker Compose >=2.35".to_owned());
    }
    Ok(engine)
}

fn require_empty(project: &str) -> Result<(), String> {
    for args in [
        vec![
            "ps".to_owned(),
            "-aq".to_owned(),
            "--filter".to_owned(),
            format!("label=com.docker.compose.project={project}"),
        ],
        vec![
            "network".to_owned(),
            "ls".to_owned(),
            "-q".to_owned(),
            "--filter".to_owned(),
            format!("label=com.docker.compose.project={project}"),
        ],
        vec![
            "volume".to_owned(),
            "ls".to_owned(),
            "-q".to_owned(),
            "--filter".to_owned(),
            format!("name=^{project}_"),
        ],
    ] {
        if !capture(&args.iter().map(String::as_str).collect::<Vec<_>>())?
            .trim()
            .is_empty()
        {
            return Err("consumer project has retained resources without this CLI's ownership receipt; no resources were adopted or changed".to_owned());
        }
    }
    Ok(())
}

fn compose_args(bundle: &Path, project: &str, action: Action) -> Vec<String> {
    let mut args = vec![
        "compose".to_owned(),
        "--env-file".to_owned(),
        "/dev/null".to_owned(),
        "--project-directory".to_owned(),
        bundle.to_string_lossy().into_owned(),
        "-p".to_owned(),
        project.to_owned(),
        "-f".to_owned(),
        bundle
            .join("deploy/compose/consumer-runtime.yaml")
            .to_string_lossy()
            .into_owned(),
    ];
    let command: Vec<String> = match action {
        Action::Up => ["up", "-d", "--wait", "--wait-timeout", "600"]
            .map(str::to_owned)
            .to_vec(),
        Action::Down => ["--profile", "*", "down"].map(str::to_owned).to_vec(),
        Action::Status => ["--profile", "*", "ps", "--all"]
            .map(str::to_owned)
            .to_vec(),
        Action::Logs(tail) => vec![
            "logs".to_owned(),
            "--no-color".to_owned(),
            "--tail".to_owned(),
            tail.to_string(),
        ],
        Action::Doctor => ["--profile", "*", "config", "--quiet"]
            .map(str::to_owned)
            .to_vec(),
    };
    args.extend(command);
    args
}

// Capture only small prerequisite inventories. A file avoids a child blocking
// forever on a full pipe; the deadline also bounds an unavailable Docker daemon.
fn capture(args: &[&str]) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!(
        "synveda-docker-{}.out",
        synveda_types::TenantId::new()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|e| format!("create Docker inventory output: {e}"))?;
    let mut command = docker();
    command.args(args).stdout(file).stderr(Stdio::null());
    let result = run_visible(command, 30).and_then(|()| {
        String::from_utf8(local_state::read(&path)?.ok_or("Docker output disappeared")?)
            .map_err(|_| "invalid Docker inventory".to_owned())
    });
    let _ = std::fs::remove_file(path);
    result
}

fn run_visible(mut command: Command, seconds: u64) -> Result<(), String> {
    let mut child = command.spawn().map_err(|e| format!("start Docker: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("wait for Docker: {e}"))?
        {
            return if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "Docker command failed ({status}); state and receipt were retained for the same command to retry"
                ))
            };
        }
        if start.elapsed() >= Duration::from_secs(seconds) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Docker command timed out; inspect the selected project before retrying; all state is retained".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
