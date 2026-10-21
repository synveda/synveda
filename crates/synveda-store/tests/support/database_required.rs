//! Database prerequisite for database-backed tests.
//!
//! A plain `cargo test` is an honest unit-only run: database cases report a
//! skip and return. The database gate sets `SYNVEDA_TEST_REQUIRE_DATABASE=1`,
//! so a missing URL fails the case instead of passing it vacuously.

/// Whether this run is required database acceptance.
#[allow(dead_code)]
pub fn required() -> bool {
    match std::env::var_os("SYNVEDA_TEST_REQUIRE_DATABASE") {
        None => false,
        Some(value) if value == "1" => true,
        Some(_) => panic!("SYNVEDA_TEST_REQUIRE_DATABASE must be unset or 1"),
    }
}

/// The ordinary-role `DATABASE_URL`, or `None` for an intentional local skip.
#[allow(dead_code)]
pub fn database_url(suite: &str) -> Option<String> {
    database_url_from(suite, "DATABASE_URL")
}

/// A database setting that gates `suite`, under the same required-mode rule.
#[allow(dead_code)]
pub fn database_url_from(suite: &str, setting: &str) -> Option<String> {
    if let Ok(url) = std::env::var(setting) {
        return Some(url);
    }
    assert!(
        !required(),
        "{suite}: required database acceptance has no {setting}"
    );
    eprintln!("skipping {suite}: {setting} is not set (run `make db-test`)");
    None
}
