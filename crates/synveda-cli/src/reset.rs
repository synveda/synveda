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
//! databases on the same server — Temporal's two live in the same `pg-data`
//! volume as ours (`deploy/compose/docker-compose.yml`), which is exactly why
//! this drops a *database* rather than a volume.
//!
use std::time::Instant;

use sqlx::postgres::PgConnectOptions;

use crate::init;

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
            "this destroys the whole of {} — every tenant, every record, every \
             audit event.\n\nRe-run with --force if that is what you want:\n\n    \
             {}\n",
            describe(&init::database_url()),
            synveda_store::epoch::RESET_COMMAND,
        ));
    }

    let started = Instant::now();
    let url = init::database_url();
    refuse_a_database_that_is_not_this_machine_s(&url)?;

    println!("synveda reset");
    println!();
    println!("    database   {}", describe(&url));

    // Before the drop, not after: a gateway holding this database open would
    // be evicted by `WITH (FORCE)` and stay alive, serving from caches that
    // describe rows nobody can read any more.
    let stopped = init::stop_host_gateway();
    match stopped {
        Some(pid) => println!("    gateway    stopped (pid {pid})"),
        None => println!("    gateway    none running as a host process"),
    }

    let outcome = synveda_store::reset::recreate(&url)
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
        "    schema     epoch {} at migration {} ({} — {})",
        outcome.metadata.epoch,
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
    println!("Bring the deployment back up, and log in to provision it again:");
    println!();
    println!("    synveda init");
    println!("    synveda login");
    // The gateway runs as a *container* when `init` was given an external
    // issuer (ADR-0055 decision 9), and nothing here records which shape this
    // deployment took — so this is a conditional note rather than an
    // inference. It matters: a containerised gateway was evicted by
    // `WITH (FORCE)` rather than stopped, so it is alive, holding a scope
    // chain and a policy pack for rows that no longer exist, and `init` will
    // not restart it because its configuration did not change.
    if stopped.is_none()
        && let Some(compose) = init::compose_file_if_any()
    {
        println!();
        println!("If your gateway runs as a container (`synveda init --issuer …`),");
        println!("restart it — it was disconnected rather than stopped, and it still");
        println!("holds caches describing rows that no longer exist:");
        println!();
        println!(
            "    docker compose -f {} restart gateway",
            compose.display()
        );
    }
    Ok(())
}

/// Names a database without its password.
///
/// `DATABASE_URL` usually carries one, and this command prints the target
/// twice — in the refusal that asks for `--force` and in the run that follows
/// — so printing it verbatim would put a credential in the shell history of
/// every operator who read the refusal before deciding. Falls back to the
/// raw string only when it does not parse, which is a string that is not a
/// connection URL and therefore holds no credential to leak.
fn describe(url: &str) -> String {
    url.parse::<PgConnectOptions>().map_or_else(
        |_| url.to_owned(),
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
    let options: PgConnectOptions = url
        .parse()
        .map_err(|err| format!("{url} is not a Postgres connection URL: {err}"))?;
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
         `reset` is the local reset and refuses to destroy a database it \
         cannot see as\n\
         yours. If you meant it, do it deliberately, from a client on that \
         server:\n\
         \n\
         \x20 drop database \"{database}\" with (force);\n\
         \x20 create database \"{database}\";\n\
         \n\
         then `synveda db migrate` against it."
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
            "postgres://synveda@127.0.0.1:5432/synveda",
            "postgres://synveda@[::1]:5432/synveda",
            "postgres://synveda@pg.localhost:5432/synveda",
        ] {
            refuse_a_database_that_is_not_this_machine_s(local)
                .unwrap_or_else(|err| panic!("{local} is local: {err}"));
        }

        for remote in [
            "postgres://synveda@db.internal:5432/synveda",
            "postgres://synveda@10.0.0.7:5432/synveda",
            "postgres://synveda@synveda-prod.eu-west-1.rds.amazonaws.com/synveda",
        ] {
            let refusal = refuse_a_database_that_is_not_this_machine_s(remote)
                .expect_err("a remote database must not be destroyed by this command");
            assert!(
                refusal.contains("drop database"),
                "a refusal has to leave the deliberate path open: {refusal}"
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
        // A string that is not a connection URL holds no credential to
        // leak, and saying it back is more use than saying nothing.
        assert_eq!(describe("not a url"), "not a url");
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
    }
}
