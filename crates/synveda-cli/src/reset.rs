//! `synveda reset --database --force` — the one supported way past the schema
//! epoch guard (CPR-2, ADR-0069).
//!
//! The context-platform redesign is a pre-1.0 hard cut: nothing translates a
//! database written before it, so [`synveda_store::epoch::verify`] refuses one
//! at startup and this is what an operator runs next. It destroys the
//! application database and builds a fresh one at the current epoch. It is not
//! an upgrade and does not pretend to be.
//!
//! ## What it deliberately does not touch
//!
//! `kms.key`, the compose profile, the console bundle, the installed binaries,
//! `~/.config/synveda/credentials.json`, the Docker volumes, and the other
//! databases on the same server. That is exactly why this drops a *database*
//! rather than a volume.
//!
use std::ffi::OsString;
use std::path::Path;
use std::time::Instant;

use crate::settings;

const STOPPED_DEPLOYMENT_ATTESTATION: &str = "stop every gateway and worker using this database first; --force confirms that precondition as well as destruction";

/// What one `reset` was asked for.
pub struct Plan {
    /// `--database`. The target, named rather than assumed.
    pub database: bool,
    /// `--force`. Required; nothing is destroyed without it.
    pub force: bool,
}

pub async fn reset(plan: Plan) -> Result<(), String> {
    // Two separate refusals rather than one, because they are two different
    // mistakes. Nothing is destroyed by default and nothing is destroyed by
    // omission: `synveda reset` on its own must not be a command that did
    // something.
    if !plan.database {
        return Err(
            "`reset` needs to be told what to reset. Today there is one \
                    thing:\n\n    synveda reset --database --force\n"
                .to_owned(),
        );
    }
    if !plan.force {
        return Err(format!(
            "this destroys the configured database — every tenant, every record, every \
             audit event. {STOPPED_DEPLOYMENT_ATTESTATION}.\n\nRe-run with --force if that is what you want:\n\n    \
             {}\n",
            synveda_store::epoch::RESET_COMMAND,
        ));
    }

    let started = Instant::now();
    let url = settings::database_url()?;
    let admin_url = reset_admin_database_url()?;
    refuse_a_database_that_is_not_this_machine_s(&url)?;
    refuse_a_database_that_is_not_this_machine_s(&admin_url)?;

    println!("synveda reset");
    println!();
    println!("    database   {}", describe(&url));
    println!("    processes  externally stopped (asserted by --force; not discovered)");

    // This direct-binary recovery command never discovers or kills product
    // processes. The operator must first stop the canonical deployment so no
    // gateway or worker survives the forced database recreation.
    let database_roles = settings::database_roles()?;
    let outcome = synveda_store::reset::recreate(&admin_url, &url, &database_roles)
        .await
        .map_err(|err| err.to_string())?;

    println!(
        "    dropped    {}",
        if outcome.existed_before {
            "yes"
        } else {
            "nothing was there"
        }
    );
    for extension in &outcome.extensions {
        println!("    extension  {} ready", extension.name);
    }
    println!(
        "    schema     epoch {}, baseline revision {} at migration {} ({} — {})",
        outcome.metadata.epoch,
        outcome.metadata.baseline_revision,
        outcome.metadata.migration_head,
        outcome.metadata.created_by_version,
        outcome.metadata.created_at.to_rfc3339(),
    );

    println!();
    println!("synveda: reset in {}s", started.elapsed().as_secs());
    println!();
    println!("The database is empty and at the current schema epoch. Nothing was");
    println!("carried across — there is no migration from the previous model, which");
    println!("is what this command exists to make true rather than to work around.");
    println!();
    println!("Re-run the deployment-owned bootstrap, then log in through that deployment.");
    println!("The reserved `synveda init` verb is not a deployment lifecycle.");
    println!();
    println!("See docs/INSTALL.md for the current cutover boundary.");
    Ok(())
}

fn reset_admin_database_url() -> Result<String, String> {
    let direct = std::env::var_os("SYNVEDA_RESET_ADMIN_DATABASE_URL");
    let file = std::env::var_os("SYNVEDA_RESET_ADMIN_DATABASE_URL_FILE");
    match (direct, file) {
        (Some(_), Some(_)) => Err("SYNVEDA_RESET_ADMIN_DATABASE_URL and \
             SYNVEDA_RESET_ADMIN_DATABASE_URL_FILE are mutually exclusive"
            .to_owned()),
        (Some(value), None) => os_string_setting("SYNVEDA_RESET_ADMIN_DATABASE_URL", value),
        (None, Some(path)) => {
            settings::read_setting_file("SYNVEDA_RESET_ADMIN_DATABASE_URL_FILE", Path::new(&path))
        }
        (None, None) => Err("SYNVEDA_RESET_ADMIN_DATABASE_URL or \
             SYNVEDA_RESET_ADMIN_DATABASE_URL_FILE is required; reset never reuses the \
             application migrator credential as cluster administrator"
            .to_owned()),
    }
}

fn os_string_setting(name: &str, value: OsString) -> Result<String, String> {
    value
        .into_string()
        .map_err(|_| format!("{name} must be valid UTF-8"))
}

/// Names a database without its password.
///
/// `DATABASE_URL` usually carries one, and this command prints the target
/// twice — in the refusal that asks for `--force` and in the run that follows
/// — so printing it verbatim would put a credential in the shell history of
/// every operator who read the refusal before deciding. A malformed value is
/// named generically because arbitrary invalid text can still carry secrets.
fn describe(url: &str) -> String {
    synveda_store::database_url::parse("DATABASE_URL", url).map_or_else(
        |_| "configured PostgreSQL database".to_owned(),
        |options| {
            format!(
                "postgres://{}@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options.get_database().unwrap_or("<none>"),
            )
        },
    )
}

/// Refuses a database that is not on this machine.
///
/// `--force` says "yes, destroy it"; it does not say "and I checked which
/// server I am pointed at". This command is documented as the *local* reset,
/// the deployment modes it is written for all put Postgres on loopback or a
/// unix socket, and the cost of being wrong here is somebody else's database.
/// So a remote host is refused and told exactly what to run by hand — which
/// keeps the escape hatch open without putting it behind a flag that would
/// eventually be pasted into a runbook.
fn refuse_a_database_that_is_not_this_machine_s(url: &str) -> Result<(), String> {
    let options = synveda_store::database_url::parse("DATABASE_URL", url)
        .map_err(|_| "DATABASE_URL is not a valid PostgreSQL connection URL".to_owned())?;
    let host = options.get_host();
    let local = host.starts_with('/')       // a unix socket
        || host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host == "[::1]";
    if local {
        return Ok(());
    }
    let database = options.get_database().unwrap_or("<none>");
    Err(format!(
        "DATABASE_URL points at {host}, which is not this machine.\n\
         \n\
         `reset` is the local reset and refuses to destroy database `{database}` through \
         a remote connection. Use the deployment's authenticated recovery procedure on \
         that server instead."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole of what stands between `--force` and somebody's production
    /// Postgres, so both directions are pinned.
    #[test]
    fn only_a_database_on_this_machine_is_destroyable() {
        for local in [
            "postgres://synveda:synveda-dev@localhost:5432/synveda",
            "postgres://synveda@127.0.0.1:5432/synveda?password=",
            "postgres://synveda@[::1]:5432/synveda?password=",
            "postgres://synveda@pg.localhost:5432/synveda?password=",
        ] {
            refuse_a_database_that_is_not_this_machine_s(local)
                .unwrap_or_else(|err| panic!("{local} is local: {err}"));
        }

        for remote in [
            "postgres://synveda@db.internal:5432/synveda?password=",
            "postgres://synveda@10.0.0.7:5432/synveda?password=",
            "postgres://synveda@synveda-prod.eu-west-1.rds.amazonaws.com/synveda?password=",
        ] {
            let refusal = refuse_a_database_that_is_not_this_machine_s(remote)
                .expect_err("a remote database must not be destroyed by this command");
            assert!(
                refusal.contains("authenticated recovery procedure"),
                "a refusal has to name the deployment-owned path: {refusal}"
            );
        }
    }

    /// The target is named twice before anything is destroyed, and neither
    /// time with the password `DATABASE_URL` carries.
    #[test]
    fn naming_the_target_does_not_print_its_password() {
        let described = describe("postgres://synveda:synveda-dev@localhost:5432/synveda");
        assert_eq!(described, "postgres://synveda@localhost:5432/synveda");
        assert!(!described.contains("synveda-dev"), "{described}");

        let malformed = "not a url?password=SYNVEDA_RESET_SECRET#SYNVEDA_RESET_FRAGMENT_SECRET";
        let described = describe(malformed);
        let refusal = refuse_a_database_that_is_not_this_machine_s(malformed)
            .expect_err("a malformed URL must be refused");
        for diagnostic in [&described, &refusal] {
            assert!(!diagnostic.contains("SYNVEDA_RESET_SECRET"), "{diagnostic}");
            assert!(
                !diagnostic.contains("SYNVEDA_RESET_FRAGMENT_SECRET"),
                "{diagnostic}"
            );
            assert!(!diagnostic.contains("password="), "{diagnostic}");
        }
        assert_eq!(described, "configured PostgreSQL database");
        assert_eq!(
            refusal,
            "DATABASE_URL is not a valid PostgreSQL connection URL"
        );
    }

    /// `synveda reset` with nothing else, and with only one of the two flags,
    /// must both be commands that did nothing.
    #[tokio::test]
    async fn neither_flag_alone_destroys_anything() {
        let no_target = reset(Plan {
            database: false,
            force: true,
        })
        .await
        .expect_err("no target");
        assert!(no_target.contains("--database"), "{no_target}");

        let no_force = reset(Plan {
            database: true,
            force: false,
        })
        .await
        .expect_err("no force");
        assert!(
            no_force.contains(synveda_store::epoch::RESET_COMMAND),
            "the refusal has to print the command that would work: {no_force}"
        );
        assert!(
            no_force.contains("stop every gateway and worker"),
            "the force acknowledgement must name its process-stop precondition: {no_force}"
        );
    }
}
