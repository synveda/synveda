//! OPS-12: bounded local pipe protocol for Windows hooks. No product authority.

#[cfg(any(windows, test))]
use serde::Deserialize;

#[cfg(any(windows, test))]
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    ReadSpool {
        name: String,
    },
    WriteSpool {
        name: String,
        expected: Option<String>,
        bytes: String,
    },
    RemoveSpool {
        name: String,
        expected: Option<String>,
    },
    ListSpools {},
    ReadReceipt {
        key: String,
    },
    InstallationId {},
    Disclose {
        key: String,
    },
    /// One hook process's batched diagnostic lines (ADR-0117 amendment 4).
    AppendLog {
        lines: Vec<String>,
    },
}

/// Diagnostic batches: one JSON object per line, bounded per line and batch.
#[cfg(any(windows, test))]
const MAX_LOG_LINES: usize = 128;
#[cfg(any(windows, test))]
const MAX_LOG_LINE_BYTES: usize = 8 * 1024;
#[cfg(any(windows, test))]
const MAX_LOG_BATCH_BYTES: usize = 256 * 1024;

#[cfg(any(windows, test))]
fn validate_log(lines: &[String]) -> Result<(), String> {
    let total: usize = lines.iter().map(String::len).sum();
    if lines.is_empty() || lines.len() > MAX_LOG_LINES || total > MAX_LOG_BATCH_BYTES {
        return Err("invalid diagnostic batch".to_owned());
    }
    for line in lines {
        let object = (line.len() <= MAX_LOG_LINE_BYTES && !line.contains(['\r', '\n']))
            .then(|| serde_json::from_str::<serde_json::Value>(line).ok())
            .flatten();
        if !object.is_some_and(|value| value["event"].is_string()) {
            return Err("invalid diagnostic line".to_owned());
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
impl Request {
    fn validate(&self) -> Result<(), String> {
        let hex = |s: &str, len| {
            s.len() == len
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        };
        match self {
            Self::ReadSpool { name }
            | Self::WriteSpool { name, .. }
            | Self::RemoveSpool { name, .. } => {
                if name.len() > 160
                    || !name.ends_with(".json")
                    || name.starts_with('.')
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                {
                    return Err("invalid spool leaf name".to_owned());
                }
            }
            Self::ReadReceipt { key } if !hex(key, 64) => {
                return Err("invalid receipt key".to_owned());
            }
            Self::Disclose { key } if !hex(key, 16) => {
                return Err("invalid disclosure key".to_owned());
            }
            Self::AppendLog { lines } => validate_log(lines)?,
            _ => {}
        }
        if let Self::WriteSpool { expected, .. } | Self::RemoveSpool { expected, .. } = self
            && expected.as_ref().is_some_and(|s| !hex(s, 64))
        {
            return Err("invalid spool snapshot".to_owned());
        }
        if let Self::WriteSpool { bytes, .. } = self
            && bytes.len() > 24 * 1024 * 1024
        {
            return Err("private input exceeds its bound".to_owned());
        }
        Ok(())
    }
}

pub(crate) fn run() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::io::Read;
        let mut input = Vec::new();
        std::io::stdin()
            .lock()
            .take(24 * 1024 * 1024 + 1)
            .read_to_end(&mut input)
            .map_err(|_| "private input could not be read")?;
        if input.len() > 24 * 1024 * 1024 {
            return Err("private input exceeds its bound".to_owned());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            version: u8,
            request: Request,
        }
        let envelope: Envelope =
            serde_json::from_slice(&input).map_err(|_| "invalid private-state request")?;
        if envelope.version != 1 {
            return Err("unsupported private-state protocol".to_owned());
        }
        envelope.request.validate()?;
        let response = native::dispatch(envelope.request)?;
        println!(
            "{}",
            serde_json::json!({ "version": 1, "architecture": std::env::consts::ARCH, "result": response })
        );
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(
            "private-state protocol is Windows-only; Unix hooks use native filesystem access"
                .to_owned(),
        )
    }
}

#[cfg(windows)]
pub(crate) mod native {
    use super::Request;
    use crate::client_paths::{Directory as Root, resolve_directory};
    use crate::credentials::windows::Directory;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::fs::{File, TryLockError};
    use std::path::Path;
    use std::time::{Duration, Instant};

    pub(crate) const SPOOL_LIMIT: u64 = 16 * 1024 * 1024;

    pub(crate) fn lock(directory: &Directory, name: &str) -> Result<File, String> {
        let file = directory.named_lock(name).map_err(|e| e.to_string())?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if started.elapsed() < Duration::from_millis(500) => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err("private state is busy; retry without removing its lock".to_owned());
                }
                Err(TryLockError::Error(e)) => {
                    return Err(format!("private state lock failed: {e}"));
                }
            }
        }
        directory.validate(&file, name).map_err(|e| e.to_string())?;
        Ok(file)
    }

    fn directory(path: &Path, create: bool) -> Result<Option<Directory>, String> {
        Directory::bounded(path, create, SPOOL_LIMIT).map_err(|e| format!("private spool: {e}"))
    }

    pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
        let Some(dir) = directory(path.parent().ok_or("spool has no parent")?, false)? else {
            return Ok(None);
        };
        Ok(dir
            .read(leaf(path)?)
            .map_err(|e| format!("private spool: {e}"))?
            .map(|(_, bytes)| bytes))
    }

    pub(crate) fn names(path: &Path) -> Result<Vec<String>, String> {
        let Some(dir) = directory(path, false)? else {
            return Ok(Vec::new());
        };
        dir.names().map_err(|e| format!("private spool: {e}"))
    }

    pub(crate) fn write(path: &Path, bytes: &[u8], expected: Option<&str>) -> Result<(), String> {
        let dir = directory(path.parent().ok_or("spool has no parent")?, true)?
            .ok_or("spool directory is absent")?;
        let _lock = lock(&dir, "spool.lock")?;
        dir.replace_if(leaf(path)?, bytes, expected)
            .map_err(|e| format!("private spool: {e}"))
    }

    pub(crate) fn remove(path: &Path, expected: Option<&str>) -> Result<(), String> {
        let Some(dir) = directory(path.parent().ok_or("spool has no parent")?, false)? else {
            return if expected.is_none() {
                Ok(())
            } else {
                Err("spool snapshot disappeared".to_owned())
            };
        };
        let _lock = lock(&dir, "spool.lock")?;
        dir.remove_if(leaf(path)?, expected)
            .map_err(|e| format!("private spool: {e}"))
    }

    fn leaf(path: &Path) -> Result<&str, String> {
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "invalid private filename".to_owned())
    }

    pub(super) fn dispatch(request: Request) -> Result<serde_json::Value, String> {
        use serde_json::json;
        let spool = || resolve_directory(Root::State).map(|p| p.join("spool"));
        match request {
            Request::ReadSpool { name } => Ok(snapshot(read(&spool()?.join(name))?)),
            Request::WriteSpool {
                name,
                expected,
                bytes,
            } => {
                let bytes = STANDARD
                    .decode(bytes)
                    .map_err(|_| "invalid private bytes")?;
                // Reject malformed/newer spool formats before any filesystem mutation.
                let parsed: crate::spool::Spool =
                    serde_json::from_slice(&bytes).map_err(|_| "invalid spool document")?;
                if parsed.spool_version != crate::spool::SPOOL_VERSION
                    || parsed.entries.iter().any(|e| !e.intact())
                {
                    return Err("unsupported or corrupt spool document".to_owned());
                }
                write(&spool()?.join(name), &bytes, expected.as_deref())?;
                Ok(json!({"digest": crate::local_state::digest(&bytes)}))
            }
            Request::RemoveSpool { name, expected } => {
                remove(&spool()?.join(name), expected.as_deref())?;
                Ok(json!({}))
            }
            Request::ListSpools {} => Ok(
                json!({"names": names(&spool()?)?.into_iter().filter(|n| n.ends_with(".json")).collect::<Vec<_>>()}),
            ),
            Request::ReadReceipt { key } => {
                let path = resolve_directory(Root::Config)?.join("consumer");
                let Some(dir) = Directory::open(&path, false).map_err(|e| e.to_string())? else {
                    return Ok(snapshot(None));
                };
                Ok(snapshot(
                    dir.read(&format!("setup-{key}.json"))
                        .map_err(|e| e.to_string())?
                        .map(|(_, b)| b),
                ))
            }
            Request::InstallationId {} => {
                let dir = Directory::open(&resolve_directory(Root::Config)?, true)
                    .map_err(|e| e.to_string())?
                    .ok_or("config directory is absent")?;
                let _lock = lock(&dir, "installation.lock")?;
                if let Some((_, bytes)) = dir.read("installation-id").map_err(|e| e.to_string())? {
                    let id =
                        String::from_utf8(bytes).map_err(|_| "invalid installation identity")?;
                    if id.trim().is_empty() || id.len() > 200 {
                        return Err("invalid installation identity".to_owned());
                    }
                    return Ok(json!({"id": id.trim()}));
                }
                let id = synveda_types::TenantId::new().to_string();
                dir.replace_if("installation-id", id.as_bytes(), None)
                    .map_err(|e| e.to_string())?;
                Ok(json!({"id": id}))
            }
            Request::Disclose { key } => {
                let path = resolve_directory(Root::State)?.join("disclosed");
                let dir = Directory::open(&path, true)
                    .map_err(|e| e.to_string())?
                    .ok_or("disclosure directory is absent")?;
                let _lock = lock(&dir, "disclosure.lock")?;
                let created = dir.read(&key).map_err(|e| e.to_string())?.is_none();
                if created {
                    dir.replace_if(&key, b"", None).map_err(|e| e.to_string())?;
                }
                Ok(json!({"created": created}))
            }
            Request::AppendLog { lines } => {
                append_log(&resolve_directory(Root::State)?, &lines, LOG_LIMIT)?;
                Ok(json!({}))
            }
        }
    }

    /// The hook diagnostic log rotates at 4 MiB and keeps one generation.
    pub(crate) const LOG_LIMIT: u64 = 4 * 1024 * 1024;

    /// Appends one batch to the private `adapter.log` under the state root.
    pub(crate) fn append_log(state: &Path, lines: &[String], limit: u64) -> Result<(), String> {
        let dir = Directory::bounded(state, true, limit)
            .map_err(|e| format!("private log: {e}"))?
            .ok_or("state directory is absent")?;
        let _lock = lock(&dir, "log.lock")?;
        let batch: String = lines.iter().map(|line| format!("{line}\n")).collect();
        if dir
            .append("adapter.log", batch.as_bytes())
            .map_err(|e| format!("private log: {e}"))?
        {
            return Ok(());
        }
        // Full: the current log becomes the one retained generation.
        let current = dir
            .read("adapter.log")
            .map_err(|e| format!("private log: {e}"))?
            .map(|(_, bytes)| bytes)
            .unwrap_or_default();
        if batch.len() as u64 > limit {
            return Err("diagnostic batch exceeds the log bound".to_owned());
        }
        dir.replace("adapter.log.1", &current)
            .map_err(|e| format!("private log: {e}"))?;
        dir.replace_if(
            "adapter.log",
            batch.as_bytes(),
            Some(&crate::local_state::digest(&current)),
        )
        .map_err(|e| format!("private log: {e}"))
    }

    fn snapshot(bytes: Option<Vec<u8>>) -> serde_json::Value {
        match bytes {
            Some(bytes) => {
                serde_json::json!({"digest": crate::local_state::digest(&bytes), "bytes": STANDARD.encode(bytes)})
            }
            None => serde_json::json!({"digest": null, "bytes": null}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_rejects_paths_credentials_unknown_operations_and_stale_hash_shapes() {
        for request in [
            r#"{"operation":"read_spool","name":"../credentials.json"}"#,
            r#"{"operation":"read_spool","name":"C:/file.json"}"#,
            r#"{"operation":"write_spool","name":"a.json","expected":"bad","bytes":""}"#,
            r#"{"operation":"read_receipt","key":"../config"}"#,
            r#"{"operation":"disclose","key":"anywhere"}"#,
        ] {
            assert!(
                serde_json::from_str::<Request>(request)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        for request in [
            r#"{"operation":"read_credentials"}"#,
            r#"{"operation":"installation_id","path":"arbitrary"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(request).is_err());
        }
        assert!(
            serde_json::from_str::<Request>(
                r#"{"operation":"read_spool","name":"session-deadbeef.json"}"#
            )
            .unwrap()
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn append_log_requests_are_bounded_json_lines() {
        let request = |lines: serde_json::Value| {
            serde_json::from_value::<Request>(
                serde_json::json!({"operation": "append_log", "lines": lines}),
            )
            .unwrap()
            .validate()
        };
        assert!(request(serde_json::json!([r#"{"event":"hook.start"}"#])).is_ok());
        let long = format!(
            r#"{{"event":"x","pad":"{}"}}"#,
            "a".repeat(MAX_LOG_LINE_BYTES)
        );
        let many = vec![r#"{"event":"x"}"#; MAX_LOG_LINES + 1];
        for rejected in [
            serde_json::json!([]),
            serde_json::json!(["not json"]),
            serde_json::json!([r#"{"no":"event"}"#]),
            serde_json::json!(["{\"event\":\"x\"}\n{\"event\":\"y\"}"]),
            serde_json::json!([long]),
            serde_json::json!(many),
        ] {
            assert!(request(rejected.clone()).is_err(), "{rejected}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn append_log_rotates_one_generation_and_keeps_entries_private() {
        let root = std::env::temp_dir().join(format!(
            "synveda private log '文' {}",
            synveda_types::TenantId::new()
        ));
        std::fs::create_dir(&root).unwrap();
        crate::windows_private::private(&root);
        let state = root.join("state");
        let line = |n: usize| format!(r#"{{"event":"e{n}"}}"#);
        native::append_log(&state, &[line(1), line(2)], 64).unwrap();
        native::append_log(&state, &[line(3)], 64).unwrap();
        assert_eq!(
            std::fs::read_to_string(state.join("adapter.log")).unwrap(),
            format!("{}\n{}\n{}\n", line(1), line(2), line(3))
        );
        // The next batch does not fit: the full log becomes the one generation.
        native::append_log(&state, &[line(4), line(5)], 64).unwrap();
        assert_eq!(
            std::fs::read_to_string(state.join("adapter.log.1")).unwrap(),
            format!("{}\n{}\n{}\n", line(1), line(2), line(3))
        );
        assert_eq!(
            std::fs::read_to_string(state.join("adapter.log")).unwrap(),
            format!("{}\n{}\n", line(4), line(5))
        );
        let dacl = crate::windows_private::dacl(&state.join("adapter.log"));
        assert!(dacl.starts_with("D:P"), "{dacl}");
        // A hard-linked log is refused rather than appended through.
        std::fs::hard_link(state.join("adapter.log"), root.join("alias")).unwrap();
        assert!(native::append_log(&state, &[line(6)], 64).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
