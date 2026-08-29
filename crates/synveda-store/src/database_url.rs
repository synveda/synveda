//! Content-free PostgreSQL connection URL validation.
//!
//! SQLx 0.8 accepts any URL scheme and logs unrecognised query keys together
//! with their values before ignoring them. Product boundaries therefore
//! validate the provider and the pinned SQLx query vocabulary before handing
//! an untrusted configuration value to SQLx.

use sqlx::postgres::PgConnectOptions;
use synveda_types::{Error, Result};

// `PgConnectOptions::from_str` starts from these libpq-compatible process
// settings before it applies the URL. A deployment file must be the complete
// connection authority: ambient credentials, endpoints or TLS inputs are an
// ambiguous second source and SQLx may log malformed pgpass lines verbatim.
const SQLX_POSTGRES_ENVIRONMENT: &[&str] = &[
    "PGHOSTADDR",
    "PGHOST",
    "PGPORT",
    "PGUSER",
    "PGPASSWORD",
    "PGDATABASE",
    "PGSSLROOTCERT",
    "PGSSLCERT",
    "PGSSLKEY",
    "PGSSLMODE",
    "PGAPPNAME",
    "PGOPTIONS",
    "PGPASSFILE",
];

/// Parses one PostgreSQL URL without allowing ignored values to reach logs.
///
/// `setting` is an application-owned configuration-key label, never URL input.
/// Errors deliberately omit the URL, parser detail and query values.
pub fn parse(setting: &str, value: &str) -> Result<PgConnectOptions> {
    let parsed = url::Url::parse(value).map_err(|_| invalid(setting))?;
    let mut database_is_explicit = !parsed.path().trim_start_matches('/').is_empty();
    let mut host_is_explicit = parsed.host_str().is_some();
    let mut username_is_explicit = !parsed.username().is_empty();
    let mut password_is_explicit = parsed.password().is_some();
    if !matches!(parsed.scheme(), "postgres" | "postgresql") || parsed.fragment().is_some() {
        return Err(invalid(setting));
    }
    for (key, query_value) in parsed.query_pairs() {
        if !is_sqlx_postgres_query_key(&key) || is_postgres_startup_option(&key) {
            return Err(invalid(setting));
        }
        // SQLx starts from PG* environment defaults. Track the last dbname
        // exactly as its parser does so an explicit empty override cannot
        // fall back to, or obscure, an ambient destructive target.
        if key == "dbname" {
            database_is_explicit = !query_value.is_empty();
        }
        if key == "host" || key == "hostaddr" {
            host_is_explicit = !query_value.is_empty();
        }
        if key == "user" {
            username_is_explicit = !query_value.is_empty();
        }
        if key == "password" {
            // An explicitly empty password is valid for peer, trust or
            // certificate authentication and suppresses pgpass discovery.
            password_is_explicit = true;
        }
    }
    if !database_is_explicit || !host_is_explicit || !username_is_explicit || !password_is_explicit
    {
        return Err(invalid(setting));
    }

    if SQLX_POSTGRES_ENVIRONMENT
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return Err(ambient_environment(setting));
    }

    let options: PgConnectOptions = value.parse().map_err(|_| invalid(setting))?;
    // SQLx seeds this field from PGOPTIONS before applying URL parameters.
    // Startup `-c` settings can change search_path, row_security and
    // replication semantics before a sentinel query runs, so the closed URL
    // vocabulary alone is not enough: refuse the ambient value as well.
    if options.get_options().is_some() {
        return Err(invalid(setting));
    }
    Ok(options)
}

fn ambient_environment(setting: &str) -> Error {
    Error::Invalid {
        message: format!("{setting} cannot be combined with ambient PostgreSQL client settings"),
    }
}

fn invalid(setting: &str) -> Error {
    Error::Invalid {
        message: format!("{setting} is not a valid PostgreSQL connection URL"),
    }
}

/// Query keys consumed by pinned `sqlx-postgres` 0.8.6.
///
/// This is intentionally closed: an ignored option is ambiguous configuration,
/// and SQLx logs its value at WARN. Review this list when SQLx changes.
fn is_sqlx_postgres_query_key(key: &str) -> bool {
    matches!(
        key,
        "sslmode"
            | "ssl-mode"
            | "sslrootcert"
            | "ssl-root-cert"
            | "ssl-ca"
            | "sslcert"
            | "ssl-cert"
            | "sslkey"
            | "ssl-key"
            | "statement-cache-capacity"
            | "host"
            | "hostaddr"
            | "port"
            | "dbname"
            | "user"
            | "password"
            | "application_name"
    )
}

fn is_postgres_startup_option(key: &str) -> bool {
    key == "options" || (key.starts_with("options[") && key.ends_with(']'))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[test]
    fn only_postgresql_urls_and_understood_query_keys_are_accepted() {
        for accepted in [
            "postgres://app:secret@db.example.test/synveda",
            "postgresql://app@db.example.test/synveda?sslmode=require&password=",
            "postgres:///synveda?host=%2Fvar%2Frun%2Fpostgresql&user=app&password=",
            "postgres://app@db.example.test?dbname=synveda&password=",
        ] {
            parse("DATABASE_URL", accepted).unwrap_or_else(|error| {
                panic!("accepted PostgreSQL URL {accepted:?} failed: {error}")
            });
        }

        for refused in [
            "https://app:secret@db.example.test/synveda",
            "postgres://app:secret@db.example.test/synveda#ignored-secret",
            "postgres://app:secret@db.example.test/synveda?connect_timeout=5",
            "postgres://app:secret@db.example.test/synveda?options=-c%20row_security%3Doff",
            "postgres://app:secret@db.example.test/synveda?options%5Bsearch_path%5D=synveda",
            "postgres://app:secret@db.example.test",
            "postgres://app:secret@db.example.test?dbname=",
            "postgres://app@db.example.test/synveda",
            "postgres://:secret@db.example.test/synveda",
            "postgres://app:secret@/synveda",
            "not a URL?password=secret",
        ] {
            let error = parse("DATABASE_URL", refused).expect_err("URL must be refused");
            assert_eq!(
                error.to_string(),
                "invalid: DATABASE_URL is not a valid PostgreSQL connection URL"
            );
            assert!(!error.to_string().contains("secret"), "{error}");
        }
    }

    #[derive(Clone)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("capture lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Captured {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn an_unknown_query_value_never_reaches_the_sqlx_logger() {
        const SENTINEL: &str = "SYNVEDA_DATABASE_QUERY_SECRET";
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(Captured(Arc::clone(&bytes)))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let error = parse(
                "DATABASE_URL",
                &format!("postgres://app@localhost/synveda?access_token={SENTINEL}"),
            )
            .expect_err("unknown query key must be refused before SQLx parses it");
            assert!(!error.to_string().contains(SENTINEL), "{error}");
        });

        let captured = String::from_utf8(bytes.lock().expect("capture lock").clone())
            .expect("captured tracing is UTF-8");
        assert!(
            !captured.contains(SENTINEL),
            "secret reached database URL validation logs"
        );
        assert!(captured.is_empty(), "database URL validation emitted a log");
    }

    #[test]
    fn ambient_pgpass_is_refused_without_reading_or_logging_it() {
        const CHILD: &str = "SYNVEDA_DATABASE_URL_PGPASS_CHILD";
        const SENTINEL: &str = "SYNVEDA_MALFORMED_PGPASS_SECRET";

        if std::env::var_os(CHILD).is_some() {
            let error = parse(
                "DATABASE_URL",
                "postgres://app:explicit@db.example.test/synveda",
            )
            .expect_err("ambient PGPASSFILE must be refused before SQLx parses the URL");
            assert_eq!(
                error.to_string(),
                "invalid: DATABASE_URL cannot be combined with ambient PostgreSQL client settings"
            );
            return;
        }

        let path = std::env::temp_dir().join(format!(
            "synveda-pgpass-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&path, SENTINEL).expect("write malformed pgpass fixture");

        let mut command = std::process::Command::new(
            std::env::current_exe().expect("locate database URL test binary"),
        );
        command.args([
            "--exact",
            "database_url::tests::ambient_pgpass_is_refused_without_reading_or_logging_it",
            "--nocapture",
        ]);
        for name in SQLX_POSTGRES_ENVIRONMENT {
            command.env_remove(name);
        }
        let output = command
            .env(CHILD, "1")
            .env("PGPASSFILE", &path)
            .output()
            .expect("run isolated pgpass parser test");
        let _ = std::fs::remove_file(&path);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stdout.contains(SENTINEL), "pgpass leaked to stdout");
        assert!(!stderr.contains(SENTINEL), "pgpass leaked to stderr");
        assert!(output.status.success(), "isolated pgpass test failed");
    }
}
