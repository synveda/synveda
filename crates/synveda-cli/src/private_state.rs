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
        }
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
}
