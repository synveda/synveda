//! OPS-12: native filesystem operations for the existing JavaScript installer.
//! Public artifact bytes only; no gateway, shell or elevated authority.

pub(crate) fn run() -> Result<(), String> {
    #[cfg(windows)]
    {
        native::run()
    }
    #[cfg(not(windows))]
    {
        Err("installer-state is Windows-only".to_owned())
    }
}

#[cfg(windows)]
mod native {
    use crate::credentials::windows::Directory;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::io::Read;
    use std::path::{Path, PathBuf};

    #[derive(Deserialize)]
    #[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
    enum Request {
        Directory {
            path: PathBuf,
            create: bool,
        },
        Tree {
            path: PathBuf,
        },
        Read {
            path: PathBuf,
        },
        Write {
            path: PathBuf,
            expected: Option<String>,
            bytes: String,
        },
    }

    pub(super) fn run() -> Result<(), String> {
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "installer input failed")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("installer input exceeds its bound".to_owned());
        }
        let request = serde_json::from_slice(&bytes).map_err(|_| "invalid installer request")?;
        let result = dispatch(request)?;
        println!("{}", json!({"version": 1, "result": result}));
        Ok(())
    }

    fn dispatch(request: Request) -> Result<Value, String> {
        match request {
            Request::Directory { path, create } => {
                let present = Directory::open(&path, create)
                    .map_err(|e| e.to_string())?
                    .is_some();
                Ok(json!({"present": present}))
            }
            Request::Tree { path } => {
                let mut files = 0;
                let mut size = 0;
                tree(&path, &mut files, &mut size, 0)?;
                Ok(json!({"files": files, "bytes": size}))
            }
            Request::Read { path } => {
                let parent = path.parent().ok_or("installer file has no parent")?;
                let Some(dir) = Directory::open(parent, false).map_err(|e| e.to_string())? else {
                    return Ok(json!({"bytes": null}));
                };
                let bytes = dir
                    .read(leaf(&path)?)
                    .map_err(|e| e.to_string())?
                    .map(|(_, bytes)| STANDARD.encode(bytes));
                Ok(json!({"bytes": bytes}))
            }
            Request::Write {
                path,
                expected,
                bytes,
            } => {
                let dir =
                    Directory::open(path.parent().ok_or("installer file has no parent")?, false)
                        .map_err(|e| e.to_string())?
                        .ok_or("installer directory is absent")?;
                let bytes = STANDARD
                    .decode(bytes)
                    .map_err(|_| "invalid installer bytes")?;
                dir.replace_if(leaf(&path)?, &bytes, expected.as_deref())
                    .map_err(|e| e.to_string())?;
                Ok(json!({}))
            }
        }
    }

    fn leaf(path: &Path) -> Result<&str, String> {
        path.file_name()
            .and_then(|p| p.to_str())
            .ok_or_else(|| "invalid installer filename".to_owned())
    }

    fn tree(path: &Path, count: &mut usize, total: &mut u64, depth: usize) -> Result<(), String> {
        if depth > 16 || *count > 1024 {
            return Err("installer tree exceeds its bound".to_owned());
        }
        let dir = Directory::bounded(path, false, 256 * 1024 * 1024)
            .map_err(|e| e.to_string())?
            .ok_or("installer directory is absent")?;
        for name in dir.names().map_err(|e| e.to_string())? {
            *count += 1;
            if *count > 1024 {
                return Err("installer tree exceeds its bound".to_owned());
            }
            let child = path.join(&name);
            let metadata = std::fs::symlink_metadata(&child).map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                tree(&child, count, total, depth + 1)?;
            } else {
                dir.check_file(&name).map_err(|e| e.to_string())?;
                *total += metadata.len();
                if *total > 512 * 1024 * 1024 {
                    return Err("installer tree exceeds its byte bound".to_owned());
                }
            }
        }
        Ok(())
    }
}
