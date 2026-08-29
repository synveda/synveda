//! Deployment-time proof that every product database credential selects one
//! writable database generation with the authority assigned to that process.

use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::future::Future;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rustix::fs::{Mode, OFlags};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use synveda_store::runtime_role::DatabaseIdentity;
use zeroize::Zeroizing;

use crate::init;

const TARGET_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const EXPECTED_HOST_SETTING: &str = "SYNVEDA_DATABASE_EXPECTED_HOST";
const EXPECTED_PORT_SETTING: &str = "SYNVEDA_DATABASE_EXPECTED_PORT";
const EXPECTED_DATABASE_SETTING: &str = "SYNVEDA_DATABASE_EXPECTED_NAME";
const REQUIRED_PEER_SETTING: &str = "SYNVEDA_DATABASE_REQUIRED_PEER";
const PEER_WITNESS_SETTING: &str = "SYNVEDA_DATABASE_PEER_WITNESS_FILE";
const PEER_WITNESS_MAX_BYTES: u64 = 256;

#[derive(Clone, Copy)]
enum Authority {
    Migrator,
    Runtime,
}

struct Target {
    direct_setting: &'static str,
    file_setting: &'static str,
    authority: Authority,
}

const MIGRATOR: Target = Target {
    direct_setting: "SYNVEDA_MIGRATOR_DATABASE_URL",
    file_setting: "SYNVEDA_MIGRATOR_DATABASE_URL_FILE",
    authority: Authority::Migrator,
};
const GATEWAY: Target = Target {
    direct_setting: "SYNVEDA_GATEWAY_DATABASE_URL",
    file_setting: "SYNVEDA_GATEWAY_DATABASE_URL_FILE",
    authority: Authority::Runtime,
};
const WORKER: Target = Target {
    direct_setting: "SYNVEDA_WORKER_DATABASE_URL",
    file_setting: "SYNVEDA_WORKER_DATABASE_URL_FILE",
    authority: Authority::Runtime,
};

#[derive(Debug)]
struct ExpectedEndpoint {
    host: String,
    port: u16,
    database: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawPeerWitness {
    version: u8,
    database: String,
    cluster_system_identifier: String,
    postmaster_started_at: String,
    database_oid: i64,
}

#[derive(Debug)]
struct PeerWitness {
    database: String,
    cluster_system_identifier: String,
    postmaster_started_at: DateTime<Utc>,
    database_oid: i64,
}

#[derive(Debug)]
struct InspectedTarget {
    database: DatabaseIdentity,
    peer_database_oid: Option<i64>,
}

#[derive(Debug)]
struct TopologyRequirements {
    endpoint: Option<ExpectedEndpoint>,
    required_peer: Option<String>,
    witness: Option<PeerWitness>,
}

impl TopologyRequirements {
    fn from_environment(
        roles: &synveda_store::runtime_role::DatabaseRoles,
    ) -> Result<Self, String> {
        let witness = std::env::var_os(PEER_WITNESS_SETTING)
            .map(read_peer_witness)
            .transpose()?;
        Self::from_values(
            std::env::var_os(EXPECTED_HOST_SETTING),
            std::env::var_os(EXPECTED_PORT_SETTING),
            std::env::var_os(EXPECTED_DATABASE_SETTING),
            std::env::var_os(REQUIRED_PEER_SETTING),
            witness,
            roles,
        )
    }

    fn from_values(
        host: Option<OsString>,
        port: Option<OsString>,
        database: Option<OsString>,
        required_peer: Option<OsString>,
        witness: Option<PeerWitness>,
        roles: &synveda_store::runtime_role::DatabaseRoles,
    ) -> Result<Self, String> {
        let endpoint = match (host, port, database) {
            (None, None, None) => None,
            (Some(host), Some(port), Some(database)) => {
                let host = bounded_setting(EXPECTED_HOST_SETTING, host, 253)?;
                let port = bounded_setting(EXPECTED_PORT_SETTING, port, 5)?;
                let parsed_port = port
                    .parse::<u16>()
                    .ok()
                    .filter(|value| *value > 0 && value.to_string() == port)
                    .ok_or_else(|| {
                        format!("{EXPECTED_PORT_SETTING} must be a canonical TCP port")
                    })?;
                let database = bounded_setting(EXPECTED_DATABASE_SETTING, database, 63)?;
                Some(ExpectedEndpoint {
                    host,
                    port: parsed_port,
                    database,
                })
            }
            _ => {
                return Err(format!(
                    "{EXPECTED_HOST_SETTING}, {EXPECTED_PORT_SETTING} and \
                     {EXPECTED_DATABASE_SETTING} must be configured together"
                ));
            }
        };

        let required_peer = required_peer
            .map(|value| bounded_setting(REQUIRED_PEER_SETTING, value, 63))
            .transpose()?;
        match (required_peer.as_ref(), witness.as_ref()) {
            (None, None) => {}
            (Some(required_peer), Some(witness)) if required_peer == &witness.database => {}
            _ => {
                return Err(format!(
                    "{REQUIRED_PEER_SETTING} and {PEER_WITNESS_SETTING} must identify one peer together"
                ));
            }
        }
        if let Some(required_peer) = required_peer.as_ref() {
            let peer_is_forbidden = roles
                .forbidden_databases()
                .iter()
                .any(|database| database == required_peer);
            let peer_is_isolated = roles
                .isolated_peer_roles()
                .iter()
                .any(|role| role == required_peer);
            if !peer_is_forbidden || !peer_is_isolated {
                return Err(format!(
                    "{REQUIRED_PEER_SETTING} is absent from the configured bidirectional isolation contract"
                ));
            }
        }

        Ok(Self {
            endpoint,
            required_peer,
            witness,
        })
    }

    fn verify_options(
        &self,
        file_setting: &str,
        options: &sqlx::postgres::PgConnectOptions,
    ) -> Result<(), String> {
        let Some(expected) = self.endpoint.as_ref() else {
            return Ok(());
        };
        if options.get_host() != expected.host
            || options.get_port() != expected.port
            || options.get_database() != Some(expected.database.as_str())
            || options.get_socket().is_some()
        {
            return Err(format!(
                "{file_setting} does not select the deployment PostgreSQL endpoint and database"
            ));
        }
        Ok(())
    }

    fn verify_target(&self, file_setting: &str, target: &InspectedTarget) -> Result<(), String> {
        let Some(witness) = self.witness.as_ref() else {
            return Ok(());
        };
        if target.database.cluster_system_identifier != witness.cluster_system_identifier
            || target.database.postmaster_started_at != witness.postmaster_started_at
            || target.peer_database_oid != Some(witness.database_oid)
        {
            return Err(format!(
                "{file_setting} does not match the peer database cluster witness"
            ));
        }
        Ok(())
    }
}

fn read_peer_witness(path: OsString) -> Result<PeerWitness, String> {
    let path = Path::new(&path);
    if !path.is_absolute() {
        return Err(format!("{PEER_WITNESS_SETTING} must be an absolute path"));
    }
    let parent = path
        .parent()
        .filter(|parent| parent != &Path::new("/"))
        .ok_or_else(|| format!("{PEER_WITNESS_SETTING} parent is unavailable"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("{PEER_WITNESS_SETTING} is unavailable"))?;
    let directory = rustix::fs::open(
        parent,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::DIRECTORY,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| format!("{PEER_WITNESS_SETTING} parent is unavailable"))?;
    let directory_before = directory
        .metadata()
        .map_err(|_| format!("{PEER_WITNESS_SETTING} parent metadata is unavailable"))?;
    let effective_uid = rustix::process::geteuid().as_raw();
    let effective_gid = rustix::process::getegid().as_raw();
    if !directory_before.file_type().is_dir()
        || directory_before.mode() & 0o7777 != 0o700
        || directory_before.uid() != effective_uid
        || directory_before.gid() != effective_gid
    {
        return Err(format!(
            "{PEER_WITNESS_SETTING} parent must be private and process-owned"
        ));
    }
    let mut file = rustix::fs::openat(
        &directory,
        file_name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| format!("{PEER_WITNESS_SETTING} is unavailable"))?;
    let metadata_before = file
        .metadata()
        .map_err(|_| format!("{PEER_WITNESS_SETTING} metadata is unavailable"))?;
    if !metadata_before.file_type().is_file()
        || metadata_before.mode() & 0o7777 != 0o600
        || metadata_before.uid() != effective_uid
        || metadata_before.gid() != effective_gid
        || metadata_before.nlink() != 1
        || !(1..=PEER_WITNESS_MAX_BYTES).contains(&metadata_before.len())
    {
        return Err(format!(
            "{PEER_WITNESS_SETTING} must be a private process-owned bounded regular file"
        ));
    }
    let file_before = metadata_snapshot(&metadata_before);
    let parent_before = metadata_snapshot(&directory_before);
    let mut bytes = Vec::with_capacity(metadata_before.len() as usize);
    file.by_ref()
        .take(PEER_WITNESS_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{PEER_WITNESS_SETTING} could not be read"))?;
    let metadata_after = file
        .metadata()
        .map_err(|_| format!("{PEER_WITNESS_SETTING} metadata is unavailable"))?;
    let directory_after = directory
        .metadata()
        .map_err(|_| format!("{PEER_WITNESS_SETTING} parent metadata is unavailable"))?;
    if file_before != metadata_snapshot(&metadata_after)
        || parent_before != metadata_snapshot(&directory_after)
        || bytes.len() as u64 != metadata_before.len()
    {
        return Err(format!(
            "{PEER_WITNESS_SETTING} changed while it was inspected"
        ));
    }
    if bytes.last() != Some(&b'\n') || bytes[..bytes.len().saturating_sub(1)].contains(&b'\n') {
        return Err(format!(
            "{PEER_WITNESS_SETTING} must contain one canonical JSON line"
        ));
    }
    let line = &bytes[..bytes.len() - 1];
    let raw: RawPeerWitness = serde_json::from_slice(line)
        .map_err(|_| format!("{PEER_WITNESS_SETTING} must contain one canonical JSON line"))?;
    if serde_json::to_vec(&raw).ok().as_deref() != Some(line) {
        return Err(format!(
            "{PEER_WITNESS_SETTING} must contain one canonical JSON line"
        ));
    }
    let system_identifier = raw
        .cluster_system_identifier
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0 && value.to_string() == raw.cluster_system_identifier);
    let timestamp_shape = raw.postmaster_started_at.len() == 27
        && raw.postmaster_started_at.as_bytes().get(10) == Some(&b'T')
        && raw.postmaster_started_at.as_bytes().get(19) == Some(&b'.')
        && raw.postmaster_started_at.ends_with('Z');
    let postmaster_started_at = timestamp_shape
        .then(|| DateTime::parse_from_rfc3339(&raw.postmaster_started_at).ok())
        .flatten()
        .map(|value| value.with_timezone(&Utc));
    let Some(postmaster_started_at) = postmaster_started_at else {
        return Err(format!(
            "{PEER_WITNESS_SETTING} contains an unsupported witness"
        ));
    };
    if raw.version != 1
        || raw.database != "keycloak"
        || system_identifier.is_none()
        || !(1..=i64::from(u32::MAX)).contains(&raw.database_oid)
    {
        return Err(format!(
            "{PEER_WITNESS_SETTING} contains an unsupported witness"
        ));
    }
    Ok(PeerWitness {
        database: raw.database,
        cluster_system_identifier: raw.cluster_system_identifier,
        postmaster_started_at,
        database_oid: raw.database_oid,
    })
}

#[derive(Debug, PartialEq, Eq)]
struct MetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

fn metadata_snapshot(metadata: &Metadata) -> MetadataSnapshot {
    MetadataSnapshot {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        links: metadata.nlink(),
        length: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

fn bounded_setting(setting: &str, value: OsString, maximum_bytes: usize) -> Result<String, String> {
    let value = value
        .into_string()
        .map_err(|_| format!("{setting} must be bounded UTF-8"))?;
    if value.is_empty()
        || value.len() > maximum_bytes
        || value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(format!("{setting} must be bounded UTF-8"));
    }
    Ok(value)
}

pub(crate) async fn run() -> Result<(), String> {
    let roles = init::database_roles()?;
    let requirements = TopologyRequirements::from_environment(&roles)?;
    let migrator = inspect(&MIGRATOR, roles.migrator(), &roles, &requirements).await?;
    let gateway = inspect(&GATEWAY, roles.gateway(), &roles, &requirements).await?;
    let worker = inspect(&WORKER, roles.worker(), &roles, &requirements).await?;
    require_one_target(&migrator, &gateway, &worker)?;
    eprintln!("database target preflight complete");
    Ok(())
}

async fn inspect(
    target: &Target,
    expected_role: &str,
    database_roles: &synveda_store::runtime_role::DatabaseRoles,
    requirements: &TopologyRequirements,
) -> Result<InspectedTarget, String> {
    let url = Zeroizing::new(resolve_file_only_url(
        target.direct_setting,
        target.file_setting,
        std::env::var_os(target.direct_setting),
        std::env::var_os(target.file_setting),
    )?);
    let options = synveda_store::database_url::parse(target.file_setting, &url)
        .map_err(|_| format!("{} is not a valid PostgreSQL URL", target.file_setting))?;
    requirements.verify_options(target.file_setting, &options)?;
    if options.get_username() != expected_role {
        return Err(format!(
            "{} login does not match SYNVEDA_DATABASE_ROLES",
            target.file_setting
        ));
    }
    within_target_deadline(
        target.file_setting,
        TARGET_TIMEOUT,
        inspect_connected_target(target, expected_role, database_roles, requirements, options),
    )
    .await
    .and_then(|inspected| {
        requirements.verify_target(target.file_setting, &inspected)?;
        Ok(inspected)
    })
}

async fn inspect_connected_target(
    target: &Target,
    expected_role: &str,
    database_roles: &synveda_store::runtime_role::DatabaseRoles,
    requirements: &TopologyRequirements,
    options: sqlx::postgres::PgConnectOptions,
) -> Result<InspectedTarget, String> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(CONNECT_TIMEOUT)
        .connect_with(options)
        .await
        .map_err(|_| format!("{} connection failed", target.file_setting))?;

    let result = async {
        let mut connection = pool
            .acquire()
            .await
            .map_err(|_| synveda_types::Error::Storage {
                message: "acquire database preflight connection".to_owned(),
            })?;
        synveda_store::runtime_role::initialize_product_session_connection(&mut connection).await?;
        let mut transaction = sqlx::Connection::begin(&mut *connection)
            .await
            .map_err(|error| synveda_types::Error::Storage {
                message: format!("begin database preflight authority snapshot: {error}"),
            })?;
        synveda_store::runtime_role::configure_authority_snapshot_connection(&mut transaction)
            .await?;
        match target.authority {
            Authority::Migrator => {
                synveda_store::runtime_role::verify_migrator_prerequisites_connection(
                    &mut transaction,
                    database_roles,
                )
                .await?;
            }
            Authority::Runtime => {
                synveda_store::runtime_role::verify_runtime_prerequisites_connection(
                    &mut transaction,
                    expected_role,
                    database_roles,
                )
                .await?;
            }
        }
        let identity =
            synveda_store::runtime_role::database_identity_connection(&mut transaction).await?;
        let peer_database_oid = match requirements.required_peer.as_deref() {
            Some(peer) => {
                synveda_store::runtime_role::peer_database_oid_connection(&mut transaction, peer)
                    .await?
            }
            None => None,
        };
        transaction
            .commit()
            .await
            .map_err(|error| synveda_types::Error::Storage {
                message: format!("finish database preflight authority snapshot: {error}"),
            })?;
        Ok::<_, synveda_types::Error>(InspectedTarget {
            database: identity,
            peer_database_oid,
        })
    }
    .await
    .map_err(|_| {
        format!(
            "{} authority or writable-target verification failed",
            target.file_setting
        )
    });
    pool.close().await;
    result
}

async fn within_target_deadline<T>(
    file_setting: &str,
    deadline: Duration,
    operation: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| format!("{file_setting} preflight timed out"))?
}

fn resolve_file_only_url(
    direct_setting: &str,
    file_setting: &str,
    direct: Option<OsString>,
    file: Option<OsString>,
) -> Result<String, String> {
    if direct.is_some() {
        return Err(format!(
            "{direct_setting} is forbidden for database preflight; use {file_setting}"
        ));
    }
    let path = file.ok_or_else(|| format!("{file_setting} is required"))?;
    init::read_database_url_file(file_setting, Path::new(&path))
}

fn require_one_target(
    migrator: &InspectedTarget,
    gateway: &InspectedTarget,
    worker: &InspectedTarget,
) -> Result<(), String> {
    if migrator.database != gateway.database || migrator.database != worker.database {
        return Err(
            "role-specific database URLs do not identify one writable database generation"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_database_credentials_are_refused_without_rendering_values() {
        const SENTINEL: &str = "cpr45-preflight-secret-sentinel";
        let error = resolve_file_only_url(
            "SYNVEDA_WORKER_DATABASE_URL",
            "SYNVEDA_WORKER_DATABASE_URL_FILE",
            Some(OsString::from(SENTINEL)),
            None,
        )
        .expect_err("direct secret must be refused");
        assert!(!error.contains(SENTINEL));
        assert_eq!(
            error,
            "SYNVEDA_WORKER_DATABASE_URL is forbidden for database preflight; use \
             SYNVEDA_WORKER_DATABASE_URL_FILE"
        );
    }

    #[test]
    fn a_file_is_required_for_each_role() {
        let error = resolve_file_only_url(
            "SYNVEDA_GATEWAY_DATABASE_URL",
            "SYNVEDA_GATEWAY_DATABASE_URL_FILE",
            None,
            None,
        )
        .expect_err("missing role-specific file must be refused");
        assert_eq!(error, "SYNVEDA_GATEWAY_DATABASE_URL_FILE is required");
    }

    fn roles(
        forbidden_databases: Vec<String>,
        isolated_peer_roles: Vec<String>,
    ) -> synveda_store::runtime_role::DatabaseRoles {
        synveda_store::runtime_role::DatabaseRoles::new(
            "migrator".to_owned(),
            "gateway".to_owned(),
            "worker".to_owned(),
            vec!["administrator".to_owned()],
            Vec::new(),
            forbidden_databases,
            isolated_peer_roles,
        )
        .expect("test database roles")
    }

    fn peer_witness() -> PeerWitness {
        PeerWitness {
            database: "keycloak".to_owned(),
            cluster_system_identifier: "7536657783470215051".to_owned(),
            postmaster_started_at: DateTime::parse_from_rfc3339("2026-08-28T12:34:56.123456Z")
                .expect("fixed test timestamp")
                .with_timezone(&Utc),
            database_oid: 16_385,
        }
    }

    #[test]
    fn topology_requirements_bind_parsed_options_and_the_declared_peer() {
        let roles = roles(
            vec!["keycloak".to_owned(), "postgres".to_owned()],
            vec!["keycloak".to_owned()],
        );
        let requirements = TopologyRequirements::from_values(
            Some(OsString::from("database.example.test")),
            Some(OsString::from("5432")),
            Some(OsString::from("synveda")),
            Some(OsString::from("keycloak")),
            Some(peer_witness()),
            &roles,
        )
        .expect("valid topology requirements");
        let matching = sqlx::postgres::PgConnectOptions::new()
            .host("database.example.test")
            .port(5432)
            .database("synveda");
        requirements
            .verify_options("SYNVEDA_GATEWAY_DATABASE_URL_FILE", &matching)
            .expect("matching endpoint");
        let matching_target = InspectedTarget {
            database: DatabaseIdentity {
                database: "synveda".to_owned(),
                cluster_system_identifier: "7536657783470215051".to_owned(),
                database_oid: 16_384,
                postmaster_started_at: peer_witness().postmaster_started_at,
            },
            peer_database_oid: Some(16_385),
        };
        requirements
            .verify_target("SYNVEDA_GATEWAY_DATABASE_URL_FILE", &matching_target)
            .expect("matching peer witness");

        for mismatching in [
            sqlx::postgres::PgConnectOptions::new()
                .host("other.example.test")
                .port(5432)
                .database("synveda"),
            sqlx::postgres::PgConnectOptions::new()
                .host("database.example.test")
                .port(5433)
                .database("synveda"),
            sqlx::postgres::PgConnectOptions::new()
                .host("database.example.test")
                .port(5432)
                .database("other"),
            sqlx::postgres::PgConnectOptions::new()
                .host("database.example.test")
                .port(5432)
                .database("synveda")
                .socket("/tmp"),
        ] {
            assert_eq!(
                requirements
                    .verify_options("SYNVEDA_GATEWAY_DATABASE_URL_FILE", &mismatching)
                    .expect_err("topology mismatch must fail before connecting"),
                "SYNVEDA_GATEWAY_DATABASE_URL_FILE does not select the deployment PostgreSQL endpoint and database"
            );
        }

        for mismatching_target in [
            InspectedTarget {
                database: DatabaseIdentity {
                    database: "synveda".to_owned(),
                    cluster_system_identifier: "7536657783470215052".to_owned(),
                    database_oid: 16_384,
                    postmaster_started_at: peer_witness().postmaster_started_at,
                },
                peer_database_oid: Some(16_385),
            },
            InspectedTarget {
                database: DatabaseIdentity {
                    database: "synveda".to_owned(),
                    cluster_system_identifier: "7536657783470215051".to_owned(),
                    database_oid: 16_384,
                    postmaster_started_at: DateTime::parse_from_rfc3339(
                        "2026-08-28T12:34:57.123456Z",
                    )
                    .expect("fixed alternate timestamp")
                    .with_timezone(&Utc),
                },
                peer_database_oid: Some(16_385),
            },
            InspectedTarget {
                database: DatabaseIdentity {
                    database: "synveda".to_owned(),
                    cluster_system_identifier: "7536657783470215051".to_owned(),
                    database_oid: 16_384,
                    postmaster_started_at: peer_witness().postmaster_started_at,
                },
                peer_database_oid: Some(16_386),
            },
        ] {
            assert_eq!(
                requirements
                    .verify_target("SYNVEDA_GATEWAY_DATABASE_URL_FILE", &mismatching_target)
                    .expect_err("each witness identity mismatch must fail"),
                "SYNVEDA_GATEWAY_DATABASE_URL_FILE does not match the peer database cluster witness"
            );
        }
    }

    #[test]
    fn partial_topology_or_an_undeclared_peer_is_refused_content_free() {
        let base_roles = roles(vec!["postgres".to_owned()], Vec::new());
        assert_eq!(
            TopologyRequirements::from_values(
                Some(OsString::from("database.example.test")),
                None,
                Some(OsString::from("synveda")),
                None,
                None,
                &base_roles,
            )
            .expect_err("partial endpoint must fail")
            .to_string(),
            "SYNVEDA_DATABASE_EXPECTED_HOST, SYNVEDA_DATABASE_EXPECTED_PORT and SYNVEDA_DATABASE_EXPECTED_NAME must be configured together"
        );
        assert_eq!(
            TopologyRequirements::from_values(
                None,
                None,
                None,
                Some(OsString::from("keycloak")),
                Some(peer_witness()),
                &base_roles,
            )
            .expect_err("missing peer must fail")
            .to_string(),
            "SYNVEDA_DATABASE_REQUIRED_PEER is absent from the configured bidirectional isolation contract"
        );

        for roles in [
            roles(vec!["keycloak".to_owned()], Vec::new()),
            roles(vec!["postgres".to_owned()], vec!["keycloak".to_owned()]),
        ] {
            assert_eq!(
                TopologyRequirements::from_values(
                    None,
                    None,
                    None,
                    Some(OsString::from("keycloak")),
                    Some(peer_witness()),
                    &roles,
                )
                .expect_err("one-sided peer isolation must fail")
                .to_string(),
                "SYNVEDA_DATABASE_REQUIRED_PEER is absent from the configured bidirectional isolation contract"
            );
        }
    }

    #[test]
    fn peer_witness_is_private_bounded_and_byte_canonical() {
        use std::os::unix::fs::symlink;

        let scratch =
            std::env::temp_dir().join(format!("synveda-peer-witness-{}", std::process::id()));
        std::fs::remove_dir_all(&scratch).ok();
        std::fs::create_dir_all(&scratch).expect("create witness scratch");
        let mut directory_permissions = std::fs::metadata(&scratch)
            .expect("witness directory metadata")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut directory_permissions, 0o700);
        std::fs::set_permissions(&scratch, directory_permissions)
            .expect("set private witness directory mode");
        let path = scratch.join("peer.json");
        const CANONICAL: &str = "{\"version\":1,\"database\":\"keycloak\",\"cluster_system_identifier\":\"7536657783470215051\",\"postmaster_started_at\":\"2026-08-28T12:34:56.123456Z\",\"database_oid\":16385}\n";
        std::fs::write(&path, CANONICAL).expect("write canonical witness");
        let mut permissions = std::fs::metadata(&path)
            .expect("witness metadata")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o600);
        std::fs::set_permissions(&path, permissions).expect("set private witness mode");
        let witness = read_peer_witness(path.clone().into_os_string()).expect("canonical witness");
        assert_eq!(witness.database, "keycloak");
        assert_eq!(witness.database_oid, 16_385);

        for invalid in [
            CANONICAL.replace("{\"version\"", "{ \"version\""),
            CANONICAL.replace("\"version\":1", "\"version\":2"),
            CANONICAL.replace("\"database\":\"keycloak\"", "\"database\":\"other\""),
            CANONICAL.replace(
                "\"cluster_system_identifier\":\"7536657783470215051\"",
                "\"cluster_system_identifier\":\"07536657783470215051\"",
            ),
            CANONICAL.replace(
                "\"postmaster_started_at\":\"2026-08-28T12:34:56.123456Z\"",
                "\"postmaster_started_at\":\"2026-08-28T12:34:56Z\"",
            ),
            CANONICAL.replace("\"database_oid\":16385", "\"database_oid\":0"),
            CANONICAL.replace(
                "\"database_oid\":16385",
                "\"database_oid\":16385,\"unexpected\":true",
            ),
            CANONICAL.replace('\n', "\r\n"),
        ] {
            std::fs::write(&path, invalid).expect("write invalid witness");
            assert!(read_peer_witness(path.clone().into_os_string()).is_err());
        }
        std::fs::write(&path, vec![b'a'; PEER_WITNESS_MAX_BYTES as usize + 1])
            .expect("write oversized witness");
        assert!(read_peer_witness(path.clone().into_os_string()).is_err());

        std::fs::write(&path, CANONICAL).expect("restore canonical witness");
        let mut unsafe_permissions = std::fs::metadata(&path)
            .expect("witness metadata")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut unsafe_permissions, 0o640);
        std::fs::set_permissions(&path, unsafe_permissions).expect("set unsafe witness mode");
        assert!(read_peer_witness(path.clone().into_os_string()).is_err());
        let mut private_permissions = std::fs::metadata(&path)
            .expect("witness metadata")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut private_permissions, 0o600);
        std::fs::set_permissions(&path, private_permissions).expect("restore witness mode");

        let hardlink = scratch.join("hardlink.json");
        std::fs::hard_link(&path, &hardlink).expect("create witness hardlink");
        assert!(read_peer_witness(path.clone().into_os_string()).is_err());
        std::fs::remove_file(hardlink).expect("remove witness hardlink");

        let fifo = scratch.join("fifo.json");
        let fifo_status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("invoke mkfifo for witness regression");
        assert!(fifo_status.success(), "create witness FIFO");
        let mut fifo_permissions = std::fs::metadata(&fifo)
            .expect("witness FIFO metadata")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut fifo_permissions, 0o600);
        std::fs::set_permissions(&fifo, fifo_permissions).expect("set witness FIFO mode");
        let started = std::time::Instant::now();
        assert!(read_peer_witness(fifo.clone().into_os_string()).is_err());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a witness FIFO without a writer must fail without blocking"
        );
        std::fs::remove_file(fifo).expect("remove witness FIFO");

        std::fs::remove_file(&path).expect("remove witness before symlink case");
        let target = scratch.join("target.json");
        std::fs::write(&target, CANONICAL).expect("write symlink target");
        symlink(&target, &path).expect("create witness symlink");
        assert!(read_peer_witness(path.into_os_string()).is_err());
        std::fs::remove_dir_all(scratch).ok();
    }

    #[tokio::test]
    async fn deadline_covers_work_after_connection_acceptance() {
        let operation = async {
            std::future::ready(()).await;
            std::future::pending::<Result<(), String>>().await
        };
        let error = within_target_deadline(
            "SYNVEDA_WORKER_DATABASE_URL_FILE",
            Duration::from_millis(20),
            operation,
        )
        .await
        .expect_err("a query stalled after connection acceptance must time out");
        assert_eq!(
            error,
            "SYNVEDA_WORKER_DATABASE_URL_FILE preflight timed out"
        );
    }

    #[test]
    fn connection_failure_precedes_the_whole_target_deadline() {
        assert!(CONNECT_TIMEOUT < TARGET_TIMEOUT);
    }
}
