//! One lock for every test in this binary that touches the process
//! environment.
//!
//! # Why this exists
//!
//! Three modules mutated the environment under three *different* locks,
//! each of which therefore protected a test only from itself:
//!
//! - `credentials::tests::Scratch` — `XDG_CONFIG_HOME`, its own `Mutex`
//! - `login::tests` — `SYNVEDA_GATEWAY`, no lock, with the comment
//!   "no other thread reads this variable in this test binary"
//! - `api::tests` — `SYNVEDA_TOKEN` and `SYNVEDA_GATEWAY`, its own `Mutex`
//!
//! That comment in `login` was true when it was written and **stopped
//! being true** when FND-5's traceparent test arrived, because the new
//! test sets `SYNVEDA_GATEWAY` and then reads it back through
//! `Api::connect`. `cargo test` runs tests on a thread pool, so the two
//! interleave: when `login`'s `remove_var` lands between the other test's
//! `set_var` and the read, `login::gateway_url` falls through to its
//! default and the request goes to `http://127.0.0.1:8120` instead of the
//! ephemeral port the test is listening on.
//!
//! It reproduced roughly one run in twenty — invisible locally across
//! twenty consecutive runs, and caught by CI on the first push. The
//! failure names a *connection refused* against a port the test never
//! chose, which reads as a network flake and is not one.
//!
//! # The rule
//!
//! **Any test that reads or writes a process environment variable takes
//! this lock**, not one of its own. A per-module lock cannot see the
//! module that has not been written yet, and the environment is process-
//! global — so the lock has to be too.
//!
//! `tokio::sync::Mutex` rather than `std::sync::Mutex` because one of the
//! holders is `async` and needs the guard across an await, which clippy
//! rejects for the std guard and is right to: it parks a thread the
//! runtime wanted. Synchronous tests take it with [`blocking_lock`], which
//! is sound because no test holding this lock runs inside a runtime — the
//! two modules that call it have no `#[tokio::test]` in them.
//!
//! It does not poison, which is what we want here: one failing test should
//! fail alone rather than turn every other environment test red behind it.
//!
//! [`blocking_lock`]: tokio::sync::Mutex::blocking_lock

/// The process-wide environment lock. See the module docs for the rule.
pub(crate) static ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
