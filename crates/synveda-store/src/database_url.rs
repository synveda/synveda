//! Content-free PostgreSQL connection URL validation.
//!
//! SQLx 0.8 accepts any URL scheme and logs unrecognised query keys together
//! with their values before ignoring them. Product boundaries therefore
//! validate the provider and the pinned SQLx query vocabulary before handing
//! an untrusted configuration value to SQLx.

use sqlx::postgres::PgConnectOptions;
use synveda_types::{Error, Result};

/// Parses one PostgreSQL URL without allowing ignored values to reach logs.
///
/// `setting` is an application-owned configuration-key label, never URL input.
/// Errors deliberately omit the URL, parser detail and query values.
pub fn parse(setting: &str, value: &str) -> Result<PgConnectOptions> {
    let parsed = url::Url::parse(value).map_err(|_| invalid(setting))?;
    let mut database_is_explicit = !parsed.path().trim_start_matches('/').is_empty();
    if !matches!(parsed.scheme(), "postgres" | "postgresql") || parsed.fragment().is_some() {
        return Err(invalid(setting));
    }
    for (key, query_value) in parsed.query_pairs() {
        if !is_sqlx_postgres_query_key(&key) {
            return Err(invalid(setting));
        }
        // SQLx starts from PG* environment defaults. Track the last dbname
        // exactly as its parser does so an explicit empty override cannot
        // fall back to, or obscure, an ambient destructive target.
        if key == "dbname" {
            database_is_explicit = !query_value.is_empty();
        }
    }
    if !database_is_explicit {
        return Err(invalid(setting));
    }

    value.parse().map_err(|_| invalid(setting))
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
            | "options"
    ) || (key.starts_with("options[") && key.ends_with(']'))
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
            "postgresql://app@db.example.test/synveda?sslmode=require",
            "postgres://app@db.example.test/synveda?options%5Bsearch_path%5D=synveda",
            "postgres:///synveda?host=%2Fvar%2Frun%2Fpostgresql",
            "postgres://app@db.example.test?dbname=synveda",
        ] {
            parse("DATABASE_URL", accepted)
                .unwrap_or_else(|error| panic!("accepted PostgreSQL URL failed: {error}"));
        }

        for refused in [
            "https://app:secret@db.example.test/synveda",
            "postgres://app:secret@db.example.test/synveda#ignored-secret",
            "postgres://app:secret@db.example.test/synveda?connect_timeout=5",
            "postgres://app:secret@db.example.test",
            "postgres://app:secret@db.example.test?dbname=",
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
            "secret reached logs: {captured}"
        );
        assert!(captured.is_empty(), "validation emitted a log: {captured}");
    }
}
