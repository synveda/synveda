//! Deterministic administrator setup script for a customer-operated
//! PostgreSQL server (ADR-0152).
//!
//! The script is rendered from the same role contract the product verifies,
//! and selects the server's role-membership syntax itself, so one script
//! serves every admitted PostgreSQL major. It is safe to rerun on the state it
//! produces, touches only the declared Synveda roles, database and declared
//! peer databases, and never creates, updates or downgrades an existing
//! extension. `synveda db preflight` is the read-only verification step.

use std::fmt::Write as _;

use synveda_store::runtime_role::DatabaseRoles;

/// Extension versions created when absent. Existing admitted versions are
/// retained; migration and runtime still prove their exact catalogues.
const BTREE_GIN_VERSION: &str = "1.3";
const VECTOR_VERSION: &str = "0.8.7";

/// Renders the setup script for one target database name.
pub(crate) fn render(roles: &DatabaseRoles, database_name: &str) -> Result<String, String> {
    let database = identifier(database_name, "database")?;
    let migrator = identifier(roles.migrator(), "migrator role")?;
    let gateway = identifier(roles.gateway(), "gateway role")?;
    let worker = identifier(roles.worker(), "worker role")?;
    let administrators = identifier_list(roles.runtime().administrators(), "administrator")?;
    let peers = identifier_list(roles.forbidden_databases(), "forbidden database")?;
    let mut script = String::with_capacity(4096);
    script.push_str(HEADER);
    for (role, login) in [
        ("synveda_app", false),
        (roles.migrator(), true),
        (roles.gateway(), true),
        (roles.worker(), true),
    ] {
        push_role(&mut script, role, login)?;
    }
    write_memberships(&mut script, &gateway, &worker);
    write_database(&mut script, database_name, roles.migrator());
    let _ = write!(
        script,
        "REVOKE CONNECT, TEMPORARY ON DATABASE {peers} FROM PUBLIC;\n\
         SET ROLE {migrator};\n\
         REVOKE ALL ON DATABASE {database} FROM PUBLIC;\n\
         GRANT CREATE, CONNECT, TEMPORARY ON DATABASE {database} TO {migrator};\n\
         GRANT CONNECT ON DATABASE {database} TO {gateway}, {worker}, {administrators};\n\
         RESET ROLE;\n\
         \\connect {database}\n\
         ALTER SCHEMA public OWNER TO {migrator};\n\
         CREATE EXTENSION IF NOT EXISTS btree_gin WITH SCHEMA public VERSION '{BTREE_GIN_VERSION}';\n\
         CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public VERSION '{VECTOR_VERSION}';\n\
         SET ROLE {migrator};\n\
         REVOKE ALL ON SCHEMA public FROM PUBLIC;\n\
         GRANT USAGE ON SCHEMA public TO synveda_app;\n\
         RESET ROLE;\n"
    );
    script.push_str(FOOTER);
    Ok(script)
}

const HEADER: &str = "\
-- Synveda database setup for a customer-operated PostgreSQL 14, 15, 16, 17 or 18
-- server. Run once as the declared administrator of a dedicated server:
--   psql 'service=<provisioning service>' -X -v ON_ERROR_STOP=1 -f <this file>
-- Rerunning converges the same declared objects. Set the three login
-- passwords afterwards with \\password, then run `synveda db preflight`.
\\set ON_ERROR_STOP on
SELECT pg_catalog.current_setting('server_version_num')::integer / 10000 AS synveda_server_major \\gset
SELECT :synveda_server_major BETWEEN 14 AND 18 AS synveda_supported_server,
       :synveda_server_major >= 16 AS synveda_membership_options \\gset
\\if :synveda_supported_server
\\else
DO $synveda$ BEGIN
  RAISE EXCEPTION 'Synveda admits PostgreSQL 14, 15, 16, 17 or 18; nothing was changed';
END $synveda$;
\\endif
";

const FOOTER: &str = "\
-- PostgreSQL 14 and 15 cannot represent ADMIN-only memberships, so the role
-- contract for those servers declares no administrative_memberships.
";

fn push_role(script: &mut String, role: &str, login: bool) -> Result<(), String> {
    let quoted = identifier(role, "role")?;
    let literal = literal(role);
    let login = if login { "LOGIN" } else { "NOLOGIN" };
    let _ = write!(
        script,
        "SELECT pg_catalog.format('CREATE ROLE %I', {literal}::text)\n \
         WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = {literal}) \\gexec\n\
         ALTER ROLE {quoted} WITH {login} INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE \
         NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1;\n\
         ALTER ROLE {quoted} RESET ALL;\n"
    );
    Ok(())
}

fn write_memberships(script: &mut String, gateway: &str, worker: &str) {
    let _ = write!(
        script,
        "\\if :synveda_membership_options\n\
         GRANT synveda_app TO {gateway}, {worker} WITH ADMIN FALSE, INHERIT TRUE, SET TRUE;\n\
         \\else\n\
         GRANT synveda_app TO {gateway}, {worker};\n\
         \\endif\n"
    );
}

fn write_database(script: &mut String, database_name: &str, migrator_name: &str) {
    let database = quote(database_name);
    let migrator = quote(migrator_name);
    let database_literal = literal(database_name);
    let migrator_literal = literal(migrator_name);
    let _ = write!(
        script,
        "SELECT pg_catalog.format('CREATE DATABASE %I OWNER %I TEMPLATE template0 ENCODING ''UTF8''', \
         {database_literal}::text, {migrator_literal}::text)\n \
         WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = {database_literal}) \\gexec\n\
         ALTER DATABASE {database} OWNER TO {migrator};\n"
    );
}

/// Double-quotes one identifier. Control characters are refused because psql
/// meta-command parsing is line oriented.
fn identifier(value: &str, label: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 63 || value.chars().any(char::is_control) {
        return Err(format!(
            "the {label} name cannot be rendered into the setup script; use a name of 1-63 bytes without control characters"
        ));
    }
    Ok(quote(value))
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn identifier_list(values: &[String], label: &str) -> Result<String, String> {
    values
        .iter()
        .map(|value| identifier(value, label))
        .collect::<Result<Vec<_>, _>>()
        .map(|values| values.join(", "))
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles() -> DatabaseRoles {
        DatabaseRoles::new(
            "synveda_migrator".to_owned(),
            "synveda_gateway".to_owned(),
            "synveda_worker".to_owned(),
            vec!["postgres".to_owned()],
            Vec::new(),
            vec!["postgres".to_owned(), "template1".to_owned()],
            Vec::new(),
        )
        .expect("closed role contract")
    }

    #[test]
    fn one_deterministic_script_selects_membership_syntax_by_server_major() {
        let script = render(&roles(), "synveda").expect("render setup script");
        assert_eq!(script, render(&roles(), "synveda").expect("render again"));
        assert!(script.contains(":synveda_server_major BETWEEN 14 AND 18"));
        assert!(script.contains(
            "GRANT synveda_app TO \"synveda_gateway\", \"synveda_worker\" WITH ADMIN FALSE, INHERIT TRUE, SET TRUE;"
        ));
        assert!(
            script.contains(
                "\\else\nGRANT synveda_app TO \"synveda_gateway\", \"synveda_worker\";\n"
            )
        );
        assert!(script.contains("GRANT CONNECT ON DATABASE \"synveda\" TO \"synveda_gateway\", \"synveda_worker\", \"postgres\";"));
        assert!(script.contains(
            "REVOKE CONNECT, TEMPORARY ON DATABASE \"postgres\", \"template1\" FROM PUBLIC;"
        ));
        assert!(
            script.contains(
                "CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public VERSION '0.8.7';"
            )
        );
        assert!(
            !script.contains("PASSWORD"),
            "credentials never enter the script"
        );
        assert!(!script.contains("DROP"), "setup never destroys objects");
        assert!(
            !script.contains("ALTER EXTENSION"),
            "setup never changes an extension"
        );
    }

    #[test]
    fn identifiers_are_quoted_and_unrenderable_names_are_refused() {
        let quoted = DatabaseRoles::new(
            "mig\"rator".to_owned(),
            "gate'way".to_owned(),
            "worker".to_owned(),
            vec!["postgres".to_owned()],
            Vec::new(),
            vec!["postgres".to_owned()],
            Vec::new(),
        )
        .expect("closed role contract");
        let script = render(&quoted, "synveda").expect("render quoted names");
        assert!(script.contains("ALTER ROLE \"mig\"\"rator\" WITH LOGIN"));
        assert!(script.contains("WHERE rolname = 'gate''way'"));
        let error = render(&roles(), "syn\nveda").expect_err("control characters");
        assert!(error.contains("cannot be rendered"), "{error}");
    }
}
