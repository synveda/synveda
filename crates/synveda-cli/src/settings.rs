//! Direct-binary configuration shared by administration and recovery commands.
//!
//! Deployment mechanisms decide how values reach the process; this module
//! keeps their meaning identical and gives sensitive settings one
//! `NAME`/`NAME_FILE` contract with bounded file inputs.

use std::ffi::OsString;
use std::io::Read as _;
use std::path::Path;

use serde_json::Value;

const MAX_SETTING_FILE_BYTES: u64 = 1_048_576;

pub(crate) fn database_url() -> Result<String, String> {
    database_url_from(
        std::env::var_os("DATABASE_URL"),
        std::env::var_os("DATABASE_URL_FILE"),
    )
}

fn database_url_from(direct: Option<OsString>, file: Option<OsString>) -> Result<String, String> {
    resolve_optional_setting("DATABASE_URL", "DATABASE_URL_FILE", direct, file)?.ok_or_else(|| {
        "DATABASE_URL or DATABASE_URL_FILE is required; no implicit database credential exists"
            .to_owned()
    })
}

/// Reads one sensitive setting from `NAME` or `NAME_FILE` without reproducing
/// its value or path in an error.
pub(crate) fn sensitive_setting(name: &str) -> Result<Option<String>, String> {
    let file_name = format!("{name}_FILE");
    resolve_optional_setting(
        name,
        &file_name,
        std::env::var_os(name),
        std::env::var_os(&file_name),
    )
}

/// The closed database-role contract used by migration, preflight and every
/// long-running product process.
pub(crate) fn database_roles() -> Result<synveda_store::runtime_role::DatabaseRoles, String> {
    let value = resolve_required_setting(
        "SYNVEDA_DATABASE_ROLES",
        "SYNVEDA_DATABASE_ROLES_FILE",
        std::env::var_os("SYNVEDA_DATABASE_ROLES"),
        std::env::var_os("SYNVEDA_DATABASE_ROLES_FILE"),
    )?;
    if value.len() > 4_096 {
        return Err("SYNVEDA_DATABASE_ROLES exceeds the 4096-byte startup bound".to_owned());
    }
    serde_json::from_str::<Value>(&value)
        .map_err(|_| "SYNVEDA_DATABASE_ROLES is not valid JSON".to_owned())?;
    synveda_store::runtime_role::DatabaseRoles::parse_json(&value)
        .map_err(|error| error.to_string())
}

fn resolve_required_setting(
    direct_name: &str,
    file_name: &str,
    direct: Option<OsString>,
    file: Option<OsString>,
) -> Result<String, String> {
    resolve_optional_setting(direct_name, file_name, direct, file)?
        .ok_or_else(|| format!("{direct_name} or {file_name} is required"))
}

fn resolve_optional_setting(
    direct_name: &str,
    file_name: &str,
    direct: Option<OsString>,
    file: Option<OsString>,
) -> Result<Option<String>, String> {
    match (direct, file) {
        (Some(_), Some(_)) => Err(format!(
            "{direct_name} and {file_name} are mutually exclusive; configure exactly one"
        )),
        (Some(value), None) => value
            .into_string()
            .map(Some)
            .map_err(|_| format!("{direct_name} must be valid UTF-8")),
        (None, Some(path)) => read_setting_file(file_name, Path::new(&path)).map(Some),
        (None, None) => Ok(None),
    }
}

pub(crate) fn read_setting_file(setting: &str, path: &Path) -> Result<String, String> {
    if path.as_os_str().is_empty() {
        return Err(format!("{setting} must name a file"));
    }
    let file = open_nonblocking_read(path).map_err(|_| format!("{setting} cannot be read"))?;
    let metadata = file
        .metadata()
        .map_err(|_| format!("{setting} cannot be read"))?;
    if !metadata.is_file() {
        return Err(format!("{setting} must name a regular file"));
    }
    if metadata.len() > MAX_SETTING_FILE_BYTES {
        return Err(format!(
            "{setting} exceeds the {MAX_SETTING_FILE_BYTES} byte startup bound"
        ));
    }
    let capacity = usize::try_from(metadata.len().min(MAX_SETTING_FILE_BYTES + 1))
        .map_err(|_| format!("{setting} exceeds the startup bound"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_SETTING_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{setting} cannot be read"))?;
    if bytes.len() as u64 > MAX_SETTING_FILE_BYTES {
        return Err(format!(
            "{setting} exceeds the {MAX_SETTING_FILE_BYTES} byte startup bound"
        ));
    }
    let mut value =
        String::from_utf8(bytes).map_err(|_| format!("{setting} must contain valid UTF-8"))?;
    if let Some(stripped) = value.strip_suffix("\r\n") {
        value.truncate(stripped.len());
    } else if value.ends_with('\n') {
        value.pop();
    }
    if value.contains('\0') {
        return Err(format!("{setting} must not contain NUL bytes"));
    }
    Ok(value)
}

fn open_nonblocking_read(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Avoid blocking on FIFOs while retaining support for Kubernetes-style
        // projected-secret symlinks to immutable regular files.
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    options.open(path)
}

/// A credential-safe database target for operator diagnostics.
pub(crate) fn redacted_database_url(value: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(value) else {
        return "configured PostgreSQL database".to_owned();
    };
    if parsed.password().is_some() {
        let _ = parsed.set_password(Some("REDACTED"));
    }
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn scratch(name: &str) -> std::path::PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "synveda-settings-{name}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create settings fixture");
        path
    }

    #[test]
    fn database_target_has_no_implicit_credential() {
        let error =
            database_url_from(None, None).expect_err("a missing database target must fail closed");
        assert_eq!(
            error,
            "DATABASE_URL or DATABASE_URL_FILE is required; no implicit database credential exists"
        );
    }

    #[test]
    fn direct_and_file_settings_are_exclusive_and_content_free() {
        const SENTINEL: &str = "SYNVEDA_SETTING_SECRET";
        let error = resolve_required_setting(
            "DATABASE_URL",
            "DATABASE_URL_FILE",
            Some(OsString::from(SENTINEL)),
            Some(OsString::from(SENTINEL)),
        )
        .expect_err("ambiguous secret sources must be refused");
        assert_eq!(
            error,
            "DATABASE_URL and DATABASE_URL_FILE are mutually exclusive; configure exactly one"
        );
        assert!(!error.contains(SENTINEL));
    }

    #[test]
    fn setting_files_are_bounded_and_trim_one_line_ending() {
        let dir = scratch("files");
        let path = dir.join("database-url");
        std::fs::write(&path, "postgres://app:secret@db.example.test/synveda\r\n")
            .expect("write fixture");
        assert_eq!(
            read_setting_file("DATABASE_URL_FILE", &path).expect("read fixture"),
            "postgres://app:secret@db.example.test/synveda"
        );

        for (name, bytes, expected) in [
            (
                "invalid-utf8",
                vec![0xff],
                "DATABASE_URL_FILE must contain valid UTF-8",
            ),
            (
                "nul",
                b"postgres://app:secret@db/synveda\0".to_vec(),
                "DATABASE_URL_FILE must not contain NUL bytes",
            ),
            (
                "oversized",
                vec![b'x'; (MAX_SETTING_FILE_BYTES + 1) as usize],
                "DATABASE_URL_FILE exceeds the 1048576 byte startup bound",
            ),
        ] {
            let invalid = dir.join(name);
            std::fs::write(&invalid, bytes).expect("write invalid fixture");
            assert_eq!(
                read_setting_file("DATABASE_URL_FILE", &invalid)
                    .expect_err("invalid file must fail closed"),
                expected
            );
        }
        assert_eq!(
            read_setting_file("DATABASE_URL_FILE", &dir)
                .expect_err("a directory is not a setting file"),
            "DATABASE_URL_FILE must name a regular file"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_direct_settings_are_content_free() {
        use std::os::unix::ffi::OsStringExt as _;
        let error = resolve_required_setting(
            "DATABASE_URL",
            "DATABASE_URL_FILE",
            Some(OsString::from_vec(vec![0xff])),
            None,
        )
        .expect_err("non-UTF-8 direct setting must be refused");
        assert_eq!(error, "DATABASE_URL must be valid UTF-8");
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_waiting_for_a_writer() {
        let dir = scratch("fifo");
        let path = dir.join("database-url");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .expect("create FIFO")
                .success()
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read_setting_file("DATABASE_URL_FILE", &path));
        });
        let error = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("setting read blocked")
            .expect_err("FIFO must be refused");
        assert!(error.contains("regular file"), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_projected_secret_symlink_is_accepted() {
        use std::os::unix::fs::symlink;
        let dir = scratch("projected-secret");
        let target = dir.join("..data-url");
        let path = dir.join("database-url");
        std::fs::write(&target, b"postgres://app:secret@db.example.test/synveda\n")
            .expect("write target");
        symlink(&target, &path).expect("link projected secret");
        assert_eq!(
            read_setting_file("DATABASE_URL_FILE", &path).expect("read projected secret"),
            "postgres://app:secret@db.example.test/synveda"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn database_errors_redact_credentials_and_parameters() {
        let rendered = redacted_database_url(
            "postgres://admin:user-secret@example.test/synveda?sslmode=require&password=query-secret#fragment-secret",
        );
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(rendered.contains("REDACTED"), "{rendered}");
        assert!(!rendered.contains('?'), "{rendered}");
        assert!(!rendered.contains('#'), "{rendered}");
    }
}
