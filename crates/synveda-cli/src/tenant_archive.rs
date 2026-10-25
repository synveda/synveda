//! The portable tenant archive `synveda.tenant-archive.v1` (ADR-0157).
//!
//! Layout: an eight-byte magic, a bounded cleartext JSON header, the
//! per-archive data key sealed by the named tenant key generation, then
//! frames of `kind | length | envelope`. Every envelope is sealed by the
//! archive key and bound to the archive ID, the header digest, its index and
//! its kind, so a changed header or a reordered, duplicated, truncated or
//! appended frame fails to open. Data frames carry COPY text for one table;
//! the final manifest frame carries every table's ordered digest and count
//! and the frozen audit head.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_crypto::{
    DataKey, KeyManagement as _, KeyScope, KeyVersion, Kms, Purpose, RowKey, SealingKey,
};
use synveda_store::tenant_archive::{self as rows, TableShape};
use synveda_types::{Tenant, TenantId};
use uuid::Uuid;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"SVTARC01";
/// The archive format identifier.
pub const FORMAT: &str = "synveda.tenant-archive.v1";
const MAX_HEADER_BYTES: u32 = 64 * 1024;
const MAX_WRAPPED_KEY_BYTES: u32 = 512;
/// The largest plaintext frame, and therefore the largest archived row.
pub const FRAME_BOUND: usize = 32 * 1024 * 1024;
const ENVELOPE_OVERHEAD: usize = 1024;
const DATA: u8 = b'D';
const MANIFEST: u8 = b'M';
const DATA_PREFIX: usize = 2 + 4;
/// Default and hard ceilings for one archive file.
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const HARD_MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// Default wall-clock bound for one export or import.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 3600;

/// Operator-declared bounds for one archive operation.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Largest archive file accepted or written.
    pub max_bytes: u64,
    /// Wall-clock bound for the whole operation and each statement.
    pub timeout: Duration,
}

impl Limits {
    /// Validates operator-supplied bounds.
    pub fn new(max_bytes: u64, timeout_seconds: u64) -> Result<Self, String> {
        if max_bytes == 0 || max_bytes > HARD_MAX_BYTES {
            return Err(format!(
                "--max-bytes must be between 1 and {HARD_MAX_BYTES}"
            ));
        }
        if timeout_seconds == 0 || timeout_seconds > 86_400 {
            return Err("--timeout-seconds must be between 1 and 86400".to_owned());
        }
        Ok(Self {
            max_bytes,
            timeout: Duration::from_secs(timeout_seconds),
        })
    }
}

/// The cleartext header. It names no tenant content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    format: String,
    archive_id: Uuid,
    tenant_id: TenantId,
    schema: Schema,
    product_version: String,
    snapshot_at: chrono::DateTime<chrono::Utc>,
    key: KeyRecord,
    frame_bound: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Schema {
    epoch: i32,
    baseline_revision: i32,
    migration_head: String,
}

/// The tenant key generation that seals the archive key, wrapped by the
/// deployment KEK exactly as stored, so a target deployment holding that KEK
/// can open the archive before the tenant exists there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyRecord {
    version: u32,
    kek_ref: String,
    wrapped_dek: String,
}

/// The sealed final frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    tenant: TenantRecord,
    tables: Vec<TableRecord>,
    excluded: Vec<String>,
    audit: AuditRecord,
    rows: u64,
    data_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TenantRecord {
    id: TenantId,
    slug: String,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
    status: synveda_types::TenantStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TableRecord {
    name: String,
    columns: Vec<String>,
    rows: u64,
    digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditRecord {
    head_seq: i64,
    head_hash: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) || text.len() > 2 * MAX_WRAPPED_KEY_BYTES as usize {
        return Err("archive_corrupt: a hex field is malformed".to_owned());
    }
    (0..text.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&text[at..at + 2], 16)
                .map_err(|_| "archive_corrupt: a hex field is malformed".to_owned())
        })
        .collect()
}

/// What binds one frame to its archive, header and position.
fn binding(archive: Uuid, header_digest: &str, index: u64, kind: u8) -> String {
    format!("{archive}:{header_digest}:{index}:{}", kind as char)
}

fn tenant_key(header: &Header, key: DataKey) -> Result<SealingKey, String> {
    let version = KeyVersion::new(header.key.version)
        .map_err(|_| "archive_corrupt: the tenant key version is invalid".to_owned())?;
    Ok(SealingKey::new(
        KeyScope::Tenant(header.tenant_id),
        version,
        key,
    ))
}

fn archive_key(tenant: TenantId, key: DataKey) -> SealingKey {
    SealingKey::new(KeyScope::Tenant(tenant), KeyVersion::FIRST, key)
}

// ── Writing ────────────────────────────────────────────────────────────────

/// A frame sink that seals, counts and bounds what it writes.
struct Writer {
    file: std::io::BufWriter<std::fs::File>,
    key: SealingKey,
    archive: Uuid,
    header_digest: String,
    index: u64,
    written: u64,
    max_bytes: u64,
}

impl Writer {
    fn write_raw(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.written = self.written.saturating_add(bytes.len() as u64);
        if self.written > self.max_bytes {
            return Err(format!(
                "archive_too_large: the archive would exceed --max-bytes {}",
                self.max_bytes
            ));
        }
        self.file
            .write_all(bytes)
            .map_err(|error| format!("write archive: {error}"))
    }

    fn frame(&mut self, kind: u8, plaintext: &[u8]) -> Result<(), String> {
        let row = binding(self.archive, &self.header_digest, self.index, kind);
        let sealed = self
            .key
            .seal(Purpose::TenantArchiveFrame, RowKey::Name(&row), plaintext)
            .map_err(|error| error.to_string())?;
        let length =
            u32::try_from(sealed.len()).map_err(|_| "archive frame overflow".to_owned())?;
        self.write_raw(&[kind])?;
        self.write_raw(&length.to_be_bytes())?;
        self.write_raw(&sealed)?;
        self.index += 1;
        Ok(())
    }
}

/// Accumulates complete COPY rows of one table into bounded data frames.
struct TableFrames<'a> {
    writer: &'a mut Writer,
    ordinal: u16,
    buffer: Vec<u8>,
    rows_in_frame: u32,
    hasher: blake3::Hasher,
    frames: u64,
}

impl TableFrames<'_> {
    fn push(&mut self, row: &[u8]) -> Result<(), String> {
        if self.buffer.len() + row.len() > FRAME_BOUND {
            self.flush()?;
        }
        if self.buffer.is_empty() {
            self.buffer.extend_from_slice(&self.ordinal.to_be_bytes());
            self.buffer.extend_from_slice(&0_u32.to_be_bytes());
        }
        self.buffer.extend_from_slice(row);
        self.rows_in_frame += 1;
        self.hasher.update(row);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        if self.rows_in_frame == 0 {
            return Ok(());
        }
        self.buffer[2..DATA_PREFIX].copy_from_slice(&self.rows_in_frame.to_be_bytes());
        self.writer.frame(DATA, &self.buffer)?;
        self.buffer.clear();
        self.rows_in_frame = 0;
        self.frames += 1;
        Ok(())
    }
}

fn private_partial(out: &Path, archive: Uuid) -> Result<(PathBuf, std::fs::File), String> {
    let name = out
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "--out must name a file".to_owned())?;
    let partial = out.with_file_name(format!(".{name}.{archive}.partial"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options
        .open(&partial)
        .map_err(|error| format!("create {}: {error}", partial.display()))?;
    Ok((partial, file))
}

/// Publishes the finished archive without ever replacing an existing file.
fn publish(partial: &Path, out: &Path) -> Result<(), String> {
    std::fs::hard_link(partial, out).map_err(|error| {
        format!(
            "archive_exists: {} could not be published ({error}); an existing archive is never replaced",
            out.display()
        )
    })?;
    std::fs::remove_file(partial).map_err(|error| format!("remove partial archive: {error}"))?;
    if let Some(directory) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::File::open(directory)
            .and_then(|handle| handle.sync_all())
            .map_err(|error| format!("sync archive directory: {error}"))?;
    }
    Ok(())
}

/// Writes one tenant's archive and chains `tenant.archived`.
pub async fn export(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    out: &Path,
    limits: Limits,
) -> Result<(), String> {
    if out.exists() {
        return Err(format!(
            "archive_exists: {} already exists; verify it with `synveda tenant archive verify`",
            out.display()
        ));
    }
    let schema = synveda_store::epoch::verify(pool)
        .await
        .map_err(|error| error.to_string())?;
    let (stored_key, tenant_key) = current_tenant_key(pool, tenant).await?;
    let archive = Uuid::now_v7();
    let (partial, file) = private_partial(out, archive)?;
    let written = tokio::time::timeout(
        limits.timeout,
        write_archive(
            pool,
            roles,
            tenant,
            archive,
            &schema,
            &stored_key,
            tenant_key,
            file,
            limits,
        ),
    )
    .await
    .map_err(|_| "archive_timeout: the export exceeded --timeout-seconds".to_owned())
    .and_then(|result| result);
    let summary = match written {
        Ok(summary) => summary,
        Err(error) => {
            std::fs::remove_file(&partial).ok();
            return Err(error);
        }
    };
    publish(&partial, out)?;
    record_archived(pool, tenant, &summary).await?;
    println!("{}", summary.receipt());
    Ok(())
}

async fn current_tenant_key(
    pool: &sqlx::PgPool,
    tenant: TenantId,
) -> Result<(synveda_store::keys::StoredKey, SealingKey), String> {
    let mut tx = synveda_store::rls::begin_tenant_custody_tx(pool, tenant)
        .await
        .map_err(|error| format!("tenant_not_found: {error}"))?;
    let stored = synveda_store::keys::tenant_current(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("key_unavailable: provision the tenant key before archiving")?;
    tx.rollback().await.map_err(|error| error.to_string())?;
    let kms = crate::keys::kms_from_env()?;
    let key = kms
        .unwrap_key(KeyScope::Tenant(tenant), &stored.wrapped_dek)
        .await
        .map_err(|_| {
            "key_unavailable: the configured KMS does not open the tenant key".to_owned()
        })?;
    let sealing = SealingKey::new(KeyScope::Tenant(tenant), stored.version, key);
    Ok((stored, sealing))
}

struct ExportSummary {
    archive: Uuid,
    manifest_digest: String,
    tables: usize,
    rows: u64,
    data_frames: u64,
    bytes: u64,
    audit_head_seq: i64,
    key_version: u32,
}

impl ExportSummary {
    fn receipt(&self) -> serde_json::Value {
        json!({
            "format": FORMAT,
            "archive_id": self.archive,
            "manifest_digest": self.manifest_digest,
            "tables": self.tables,
            "rows": self.rows,
            "data_frames": self.data_frames,
            "bytes": self.bytes,
            "audit_head_seq": self.audit_head_seq,
            "tenant_key_version": self.key_version,
        })
    }
}

#[allow(clippy::too_many_arguments)] // One export's inputs; grouping them adds a type with one use.
async fn write_archive(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    tenant: TenantId,
    archive: Uuid,
    schema: &synveda_store::epoch::SchemaMetadata,
    stored_key: &synveda_store::keys::StoredKey,
    tenant_key: SealingKey,
    file: std::fs::File,
    limits: Limits,
) -> Result<ExportSummary, String> {
    let mut tx = rows::begin_administrator_tx(pool, roles, tenant, true)
        .await
        .map_err(|error| error.to_string())?;
    snapshot_settings(&mut tx, limits).await?;
    let tenant_row = synveda_store::tenants::by_id(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("tenant_not_found: no tenant has that id")?;
    let current = synveda_store::keys::tenant_current(&mut *tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
    if current.as_ref().map(|key| (key.version, &key.wrapped_dek))
        != Some((stored_key.version, &stored_key.wrapped_dek))
    {
        return Err(
            "key_unavailable: the tenant key changed while it was opened; retry".to_owned(),
        );
    }
    let header = Header {
        format: FORMAT.to_owned(),
        archive_id: archive,
        tenant_id: tenant,
        schema: Schema {
            epoch: schema.epoch,
            baseline_revision: schema.baseline_revision,
            migration_head: schema.migration_head.clone(),
        },
        product_version: env!("CARGO_PKG_VERSION").to_owned(),
        snapshot_at: rows::snapshot_time(&mut tx)
            .await
            .map_err(|error| error.to_string())?,
        key: KeyRecord {
            version: stored_key.version.get(),
            kek_ref: stored_key.kek_ref.clone(),
            wrapped_dek: hex(&stored_key.wrapped_dek),
        },
        frame_bound: FRAME_BOUND as u64,
    };
    let mut writer = start_archive(file, &header, &tenant_key, limits)?;
    let (tables, data_frames) = write_tables(&mut tx, tenant, &mut writer).await?;
    let audit = synveda_audit::verify_report(&mut tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let synveda_audit::ChainVerification::Valid { .. } = audit.verification else {
        return Err(
            "audit_invalid: the tenant's audit chain does not verify; nothing was archived"
                .to_owned(),
        );
    };
    tx.rollback().await.map_err(|error| error.to_string())?;
    let manifest = Manifest {
        tenant: TenantRecord {
            id: tenant_row.id,
            slug: tenant_row.slug,
            name: tenant_row.name,
            created_at: tenant_row.created_at,
            status: tenant_row.status,
        },
        rows: tables.iter().map(|table| table.rows).sum(),
        tables,
        excluded: synveda_store::tenant_inventory::TENANT_TABLES
            .iter()
            .filter(|table| table.archive != synveda_store::tenant_inventory::Archive::Portable)
            .map(|table| table.name.to_owned())
            .collect(),
        audit: AuditRecord {
            head_seq: audit.head_seq,
            head_hash: hex(&audit.head_hash),
        },
        data_frames,
    };
    finish_archive(writer, &manifest, archive, &header)
}

async fn snapshot_settings(tx: &mut sqlx::PgConnection, limits: Limits) -> Result<(), String> {
    rows::bound_statements(tx, limits.timeout.as_secs())
        .await
        .map_err(|error| error.to_string())?;
    rows::pin_copy_format(tx)
        .await
        .map_err(|error| error.to_string())
}

fn start_archive(
    file: std::fs::File,
    header: &Header,
    tenant_key: &SealingKey,
    limits: Limits,
) -> Result<Writer, String> {
    let header_bytes = serde_json::to_vec(header).map_err(|error| error.to_string())?;
    let frame_key = DataKey::generate().map_err(|error| error.to_string())?;
    let wrapped = tenant_key
        .seal_data_key(
            Purpose::TenantArchiveKey,
            RowKey::Uuid(header.archive_id),
            &frame_key,
        )
        .map_err(|error| error.to_string())?;
    let mut writer = Writer {
        file: std::io::BufWriter::new(file),
        key: archive_key(header.tenant_id, frame_key),
        archive: header.archive_id,
        header_digest: hex(blake3::hash(&header_bytes).as_bytes()),
        index: 0,
        written: 0,
        max_bytes: limits.max_bytes,
    };
    writer.write_raw(MAGIC)?;
    writer.write_raw(
        &u32::try_from(header_bytes.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    )?;
    writer.write_raw(&header_bytes)?;
    writer.write_raw(
        &u32::try_from(wrapped.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    )?;
    writer.write_raw(&wrapped)?;
    Ok(writer)
}

async fn write_tables(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    writer: &mut Writer,
) -> Result<(Vec<TableRecord>, u64), String> {
    let mut tables = Vec::new();
    let mut data_frames = 0;
    for (ordinal, table) in rows::portable_tables().enumerate() {
        let shape = rows::table_shape(tx, table)
            .await
            .map_err(|error| error.to_string())?;
        let mut frames = TableFrames {
            writer,
            ordinal: u16::try_from(ordinal).map_err(|_| "too many archive tables".to_owned())?,
            buffer: Vec::new(),
            rows_in_frame: 0,
            hasher: blake3::Hasher::new(),
            frames: 0,
        };
        let mut failure = None;
        let counted = rows::copy_out_rows(tx, &shape, tenant, FRAME_BOUND - DATA_PREFIX, |row| {
            frames.push(row).map_err(|message| {
                failure = Some(message.clone());
                synveda_types::Error::Invalid { message }
            })
        })
        .await
        .map_err(|error| failure.take().unwrap_or_else(|| error.to_string()))?;
        frames.flush()?;
        data_frames += frames.frames;
        tables.push(TableRecord {
            name: table.name.to_owned(),
            columns: shape.columns,
            rows: counted,
            digest: hex(frames.hasher.finalize().as_bytes()),
        });
    }
    Ok((tables, data_frames))
}

fn finish_archive(
    mut writer: Writer,
    manifest: &Manifest,
    archive: Uuid,
    header: &Header,
) -> Result<ExportSummary, String> {
    let manifest_bytes = serde_json::to_vec(manifest).map_err(|error| error.to_string())?;
    writer.frame(MANIFEST, &manifest_bytes)?;
    let mut file = writer
        .file
        .into_inner()
        .map_err(|error| format!("flush archive: {}", error.error()))?;
    file.flush()
        .map_err(|error| format!("flush archive: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync archive: {error}"))?;
    Ok(ExportSummary {
        archive,
        manifest_digest: hex(blake3::hash(&manifest_bytes).as_bytes()),
        tables: manifest.tables.len(),
        rows: manifest.rows,
        data_frames: manifest.data_frames,
        bytes: writer.written,
        audit_head_seq: manifest.audit.head_seq,
        key_version: header.key.version,
    })
}

async fn record_archived(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    summary: &ExportSummary,
) -> Result<(), String> {
    let mut tx = synveda_store::rls::begin_tenant_custody_tx(pool, tenant)
        .await
        .map_err(|error| error.to_string())?;
    crate::record_break_glass(
        &mut tx,
        tenant,
        synveda_audit::AuditAction::TenantArchived,
        format!("tenant {tenant} archive {}", summary.archive),
        json!({
            "archive_id": summary.archive,
            "manifest_digest": summary.manifest_digest,
            "tables": i64::try_from(summary.tables).unwrap_or(i64::MAX),
            "rows": i64::try_from(summary.rows).unwrap_or(i64::MAX),
            "bytes": i64::try_from(summary.bytes).unwrap_or(i64::MAX),
            "audit_head_seq": summary.audit_head_seq,
            "tenant_key_version": summary.key_version,
        }),
    )
    .await?;
    tx.commit().await.map_err(|error| error.to_string())
}

// ── Reading ────────────────────────────────────────────────────────────────

/// A frame source that authenticates, orders and bounds what it reads.
struct Reader {
    file: std::io::BufReader<std::fs::File>,
    key: SealingKey,
    archive: Uuid,
    header_digest: String,
    index: u64,
    read: u64,
    max_bytes: u64,
    finished: bool,
}

enum Frame {
    Data {
        ordinal: u16,
        rows: u32,
        bytes: Zeroizing<Vec<u8>>,
    },
    Manifest(Zeroizing<Vec<u8>>),
}

fn read_exact(
    file: &mut impl Read,
    buffer: &mut [u8],
    read: &mut u64,
    max: u64,
) -> Result<(), String> {
    *read = read.saturating_add(buffer.len() as u64);
    if *read > max {
        return Err(format!(
            "archive_too_large: the archive exceeds --max-bytes {max}"
        ));
    }
    file.read_exact(buffer)
        .map_err(|_| "archive_truncated: the archive ended early".to_owned())
}

fn read_length(
    file: &mut impl Read,
    read: &mut u64,
    max: u64,
    bound: u32,
) -> Result<usize, String> {
    let mut length = [0_u8; 4];
    read_exact(file, &mut length, read, max)?;
    let length = u32::from_be_bytes(length);
    if length == 0 || length > bound {
        return Err("archive_corrupt: a length field is out of bounds".to_owned());
    }
    usize::try_from(length).map_err(|_| "archive_corrupt: a length field overflows".to_owned())
}

impl Reader {
    fn next(&mut self) -> Result<Option<Frame>, String> {
        if self.finished {
            let mut trailing = [0_u8; 1];
            return match self.file.read(&mut trailing) {
                Ok(0) => Ok(None),
                Ok(_) => Err("archive_corrupt: bytes follow the manifest".to_owned()),
                Err(error) => Err(format!("read archive: {error}")),
            };
        }
        let mut kind = [0_u8; 1];
        read_exact(&mut self.file, &mut kind, &mut self.read, self.max_bytes)?;
        let bound = u32::try_from(FRAME_BOUND + ENVELOPE_OVERHEAD).unwrap_or(u32::MAX);
        let length = read_length(&mut self.file, &mut self.read, self.max_bytes, bound)?;
        let mut envelope = vec![0_u8; length];
        read_exact(
            &mut self.file,
            &mut envelope,
            &mut self.read,
            self.max_bytes,
        )?;
        let row = binding(self.archive, &self.header_digest, self.index, kind[0]);
        let opened = self
            .key
            .open(Purpose::TenantArchiveFrame, RowKey::Name(&row), &envelope)
            .map_err(|_| {
                format!(
                    "archive_corrupt: frame {} does not authenticate",
                    self.index
                )
            })?;
        self.index += 1;
        match kind[0] {
            DATA => data_frame(opened).map(Some),
            MANIFEST => {
                self.finished = true;
                Ok(Some(Frame::Manifest(opened)))
            }
            _ => Err("archive_corrupt: unknown frame kind".to_owned()),
        }
    }
}

fn data_frame(opened: Zeroizing<Vec<u8>>) -> Result<Frame, String> {
    if opened.len() <= DATA_PREFIX {
        return Err("archive_corrupt: an empty data frame".to_owned());
    }
    let ordinal = u16::from_be_bytes([opened[0], opened[1]]);
    let rows = u32::from_be_bytes([opened[2], opened[3], opened[4], opened[5]]);
    let body = Zeroizing::new(opened[DATA_PREFIX..].to_vec());
    let newlines = body.iter().filter(|byte| **byte == b'\n').count();
    if rows == 0 || usize::try_from(rows).ok() != Some(newlines) || !body.ends_with(b"\n") {
        return Err("archive_corrupt: a data frame's row count does not match its rows".to_owned());
    }
    Ok(Frame::Data {
        ordinal,
        rows,
        bytes: body,
    })
}

/// Opens the header and archive key; the KMS must hold the named KEK.
async fn open(path: &Path, limits: Limits, kms: &Kms) -> Result<(Header, Reader), String> {
    let file =
        std::fs::File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut file = std::io::BufReader::new(file);
    let mut read = 0;
    let mut magic = [0_u8; 8];
    read_exact(&mut file, &mut magic, &mut read, limits.max_bytes)
        .map_err(|_| "archive_corrupt: not a tenant archive".to_owned())?;
    if &magic != MAGIC {
        return Err("archive_corrupt: not a synveda.tenant-archive.v1 file".to_owned());
    }
    let header_length = read_length(&mut file, &mut read, limits.max_bytes, MAX_HEADER_BYTES)?;
    let mut header_bytes = vec![0_u8; header_length];
    read_exact(&mut file, &mut header_bytes, &mut read, limits.max_bytes)?;
    let header: Header = serde_json::from_slice(&header_bytes)
        .map_err(|_| "archive_corrupt: the header is not a v1 header".to_owned())?;
    if header.format != FORMAT || header.frame_bound != FRAME_BOUND as u64 {
        return Err("archive_unsupported: the archive format or frame bound differs".to_owned());
    }
    let wrapped_length = read_length(
        &mut file,
        &mut read,
        limits.max_bytes,
        MAX_WRAPPED_KEY_BYTES,
    )?;
    let mut wrapped = vec![0_u8; wrapped_length];
    read_exact(&mut file, &mut wrapped, &mut read, limits.max_bytes)?;
    let key = open_archive_key(&header, &wrapped, kms).await?;
    let reader = Reader {
        file,
        key,
        archive: header.archive_id,
        header_digest: hex(blake3::hash(&header_bytes).as_bytes()),
        index: 0,
        read,
        max_bytes: limits.max_bytes,
        finished: false,
    };
    Ok((header, reader))
}

async fn open_archive_key(
    header: &Header,
    wrapped: &[u8],
    kms: &Kms,
) -> Result<SealingKey, String> {
    let refused =
        || "key_unavailable: the configured KMS does not open this archive's tenant key".to_owned();
    if header.key.kek_ref != kms.key_ref() {
        return Err(format!(
            "key_unavailable: the archive needs KEK {} and the configured KEK is {}",
            header.key.kek_ref,
            kms.key_ref()
        ));
    }
    let dek = kms
        .unwrap_key(
            KeyScope::Tenant(header.tenant_id),
            &unhex(&header.key.wrapped_dek)?,
        )
        .await
        .map_err(|_| refused())?;
    let archive = tenant_key(header, dek)?
        .open_data_key(
            Purpose::TenantArchiveKey,
            RowKey::Uuid(header.archive_id),
            wrapped,
        )
        .map_err(|_| refused())?;
    Ok(archive_key(header.tenant_id, archive))
}

/// Everything a complete authenticated pass proves.
struct Verified {
    header: Header,
    manifest: Manifest,
    manifest_digest: String,
    bytes: u64,
}

/// Authenticates every frame and checks the manifest without a database.
async fn verify_file(path: &Path, limits: Limits, kms: &Kms) -> Result<Verified, String> {
    let (header, mut reader) = open(path, limits, kms).await?;
    let expected: Vec<&'static str> = rows::portable_tables().map(|table| table.name).collect();
    let mut hashers: Vec<(u64, blake3::Hasher)> = expected
        .iter()
        .map(|_| (0, blake3::Hasher::new()))
        .collect();
    let mut last_ordinal = 0_usize;
    let mut data_frames = 0_u64;
    let manifest_bytes = loop {
        match reader.next()? {
            Some(Frame::Data {
                ordinal,
                rows,
                bytes,
            }) => {
                let ordinal = usize::from(ordinal);
                if ordinal >= hashers.len() || ordinal < last_ordinal {
                    return Err("archive_corrupt: data frames are out of table order".to_owned());
                }
                last_ordinal = ordinal;
                hashers[ordinal].0 += u64::from(rows);
                hashers[ordinal].1.update(&bytes);
                data_frames += 1;
            }
            Some(Frame::Manifest(bytes)) => break bytes,
            None => return Err("archive_truncated: the archive has no manifest".to_owned()),
        }
    };
    if reader.next()?.is_some() {
        return Err("archive_corrupt: frames follow the manifest".to_owned());
    }
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| "archive_corrupt: the manifest is not a v1 manifest".to_owned())?;
    check_manifest(&header, &manifest, &expected, &hashers, data_frames)?;
    Ok(Verified {
        manifest_digest: hex(blake3::hash(&manifest_bytes).as_bytes()),
        header,
        manifest,
        bytes: reader.read,
    })
}

fn check_manifest(
    header: &Header,
    manifest: &Manifest,
    expected: &[&str],
    hashers: &[(u64, blake3::Hasher)],
    data_frames: u64,
) -> Result<(), String> {
    let names: Vec<&str> = manifest
        .tables
        .iter()
        .map(|table| table.name.as_str())
        .collect();
    if manifest.tenant.id != header.tenant_id || names != expected {
        return Err(
            "archive_unsupported: the archive's table set differs from this build".to_owned(),
        );
    }
    for (record, (rows, hasher)) in manifest.tables.iter().zip(hashers) {
        if record.rows != *rows || record.digest != hex(hasher.finalize().as_bytes()) {
            return Err(format!(
                "archive_corrupt: {} does not match its manifest digest",
                record.name
            ));
        }
    }
    if manifest.data_frames != data_frames
        || manifest.rows != manifest.tables.iter().map(|table| table.rows).sum::<u64>()
    {
        return Err("archive_corrupt: manifest totals do not match the frames".to_owned());
    }
    Ok(())
}

/// `synveda tenant archive verify`: authenticate and check without a database.
pub async fn verify(path: &Path, limits: Limits) -> Result<(), String> {
    let kms = crate::keys::kms_from_env()?;
    let verified = tokio::time::timeout(limits.timeout, verify_file(path, limits, &kms))
        .await
        .map_err(|_| "archive_timeout: verification exceeded --timeout-seconds".to_owned())??;
    println!("{}", verified_receipt(&verified, "verified"));
    Ok(())
}

fn verified_receipt(verified: &Verified, outcome: &str) -> serde_json::Value {
    json!({
        "format": FORMAT,
        "outcome": outcome,
        "archive_id": verified.header.archive_id,
        "tenant": verified.header.tenant_id,
        "manifest_digest": verified.manifest_digest,
        "schema": {
            "epoch": verified.header.schema.epoch,
            "baseline_revision": verified.header.schema.baseline_revision,
            "migration_head": verified.header.schema.migration_head,
        },
        "tables": verified.manifest.tables.len(),
        "rows": verified.manifest.rows,
        "audit_head_seq": verified.manifest.audit.head_seq,
        "bytes": verified.bytes,
    })
}

// ── Restoring ──────────────────────────────────────────────────────────────

/// `synveda tenant archive import`: restore one archive into this deployment.
pub async fn import(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    path: &Path,
    limits: Limits,
    writer_drain: Duration,
) -> Result<(), String> {
    let outcome = tokio::time::timeout(
        limits.timeout,
        import_inner(pool, roles, path, limits, writer_drain),
    )
    .await
    .map_err(|_| {
        "archive_timeout: the import exceeded --timeout-seconds; nothing was restored".to_owned()
    })??;
    println!("{outcome}");
    Ok(())
}

async fn import_inner(
    pool: &sqlx::PgPool,
    roles: &synveda_store::runtime_role::DatabaseRoles,
    path: &Path,
    limits: Limits,
    writer_drain: Duration,
) -> Result<serde_json::Value, String> {
    let kms = crate::keys::kms_from_env()?;
    let verified = verify_file(path, limits, &kms).await?;
    check_target_schema(pool, &verified.header).await?;
    let tenant = verified.header.tenant_id;
    if let Some(receipt) = already_restored(pool, &verified).await? {
        return Ok(receipt);
    }
    synveda_store::tenant_archive::drain_runtime_writers(pool, roles, writer_drain)
        .await
        .map_err(|error| format!("writers_connected: {error}"))?;
    let mut tx = rows::begin_administrator_tx(pool, roles, tenant, false)
        .await
        .map_err(|error| error.to_string())?;
    let shapes = prepare_restore(&mut tx, &verified, limits).await?;
    load_frames(&mut tx, path, limits, &kms, &shapes).await?;
    finish_restore(&mut tx, &verified, &shapes).await?;
    tx.commit()
        .await
        .map_err(|error| format!("commit restore: {error}"))?;
    Ok(verified_receipt(&verified, "restored"))
}

async fn check_target_schema(pool: &sqlx::PgPool, header: &Header) -> Result<(), String> {
    let target = synveda_store::epoch::verify(pool)
        .await
        .map_err(|error| format!("schema_incompatible: {error}"))?;
    let archived = &header.schema;
    if (
        target.epoch,
        target.baseline_revision,
        target.migration_head.as_str(),
    ) != (
        archived.epoch,
        archived.baseline_revision,
        archived.migration_head.as_str(),
    ) {
        return Err(format!(
            "schema_incompatible: the archive is epoch {} revision {} head {} and this deployment is epoch {} revision {} head {}",
            archived.epoch,
            archived.baseline_revision,
            archived.migration_head,
            target.epoch,
            target.baseline_revision,
            target.migration_head
        ));
    }
    Ok(())
}

/// A rerun after a committed restore reports it; any other existing tenant
/// or slug is a collision.
async fn already_restored(
    pool: &sqlx::PgPool,
    verified: &Verified,
) -> Result<Option<serde_json::Value>, String> {
    let tenant = verified.header.tenant_id;
    if synveda_store::tenants::by_id(pool, tenant)
        .await
        .map_err(|error| error.to_string())?
        .is_none()
    {
        let mut connection = pool.acquire().await.map_err(|error| error.to_string())?;
        if rows::slug_taken(&mut connection, &verified.manifest.tenant.slug)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err(
                "tenant_collision: another tenant already uses the archived slug".to_owned(),
            );
        }
        return Ok(None);
    }
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let restored = synveda_audit::search(
        &mut tx,
        tenant,
        &synveda_audit::EventFilter {
            actions: vec![synveda_audit::AuditAction::TenantRestored],
            ..synveda_audit::EventFilter::default()
        },
        0,
        16,
    )
    .await
    .map_err(|error| error.to_string())?;
    tx.rollback().await.map_err(|error| error.to_string())?;
    let same = restored.items.iter().any(|event| {
        event.payload["archive_id"] == json!(verified.header.archive_id)
            && event.payload["manifest_digest"] == json!(verified.manifest_digest)
    });
    if same {
        Ok(Some(verified_receipt(verified, "already_restored")))
    } else {
        Err("tenant_collision: this deployment already holds that tenant".to_owned())
    }
}

async fn prepare_restore(
    tx: &mut sqlx::PgConnection,
    verified: &Verified,
    limits: Limits,
) -> Result<Vec<TableShape>, String> {
    rows::bound_statements(tx, limits.timeout.as_secs())
        .await
        .map_err(|error| error.to_string())?;
    rows::pin_copy_format(tx)
        .await
        .map_err(|error| error.to_string())?;
    let record = &verified.manifest.tenant;
    let tenant = Tenant {
        id: record.id,
        slug: record.slug.clone(),
        name: record.name.clone(),
        status: synveda_types::TenantStatus::Suspended,
        created_at: record.created_at,
    };
    rows::insert_restored_tenant(tx, &tenant)
        .await
        .map_err(|error| error.to_string())?;
    synveda_store::tenant_lifecycle::lock_exclusive(tx, record.id)
        .await
        .map_err(|error| error.to_string())?;
    rows::set_constraints_deferred(tx, true)
        .await
        .map_err(|error| error.to_string())?;
    rows::set_user_triggers(tx, false)
        .await
        .map_err(|error| error.to_string())?;
    let mut shapes = Vec::new();
    for (table, record) in rows::portable_tables().zip(&verified.manifest.tables) {
        let shape = rows::table_shape(tx, table)
            .await
            .map_err(|error| error.to_string())?;
        if shape.columns != record.columns {
            return Err(format!(
                "schema_incompatible: {} has different columns in this deployment",
                table.name
            ));
        }
        shapes.push(shape);
    }
    Ok(shapes)
}

async fn load_frames(
    tx: &mut sqlx::PgConnection,
    path: &Path,
    limits: Limits,
    kms: &Kms,
    shapes: &[TableShape],
) -> Result<(), String> {
    let (_, mut reader) = open(path, limits, kms).await?;
    while let Some(frame) = reader.next()? {
        let Frame::Data { ordinal, bytes, .. } = frame else {
            break;
        };
        let shape = shapes
            .get(usize::from(ordinal))
            .ok_or("archive_corrupt: a frame names an unknown table")?;
        rows::copy_in_rows(tx, shape, [bytes.as_slice()])
            .await
            .map_err(|error| format!("restore_refused: {error}"))?;
    }
    Ok(())
}

async fn finish_restore(
    tx: &mut sqlx::PgConnection,
    verified: &Verified,
    shapes: &[TableShape],
) -> Result<(), String> {
    let tenant = verified.header.tenant_id;
    // Deferred foreign-key checks must fire before ALTER TABLE may run again.
    rows::set_constraints_deferred(tx, false)
        .await
        .map_err(|error| format!("restore_refused: {error}"))?;
    rows::set_user_triggers(tx, true)
        .await
        .map_err(|error| error.to_string())?;
    for (shape, record) in shapes.iter().zip(&verified.manifest.tables) {
        let (count, digest) = rows::table_digest(tx, shape, tenant, FRAME_BOUND)
            .await
            .map_err(|error| error.to_string())?;
        if count != record.rows || hex(&digest) != record.digest {
            return Err(format!(
                "restore_refused: restored {} differs from the archive; nothing was restored",
                record.name
            ));
        }
    }
    let audit = synveda_audit::verify_report(tx, tenant)
        .await
        .map_err(|error| error.to_string())?;
    let valid = matches!(
        audit.verification,
        synveda_audit::ChainVerification::Valid { .. }
    );
    if !valid
        || audit.head_seq != verified.manifest.audit.head_seq
        || hex(&audit.head_hash) != verified.manifest.audit.head_hash
    {
        return Err(
            "restore_refused: the restored audit chain is not the frozen prefix".to_owned(),
        );
    }
    crate::record_break_glass(
        tx,
        tenant,
        synveda_audit::AuditAction::TenantRestored,
        format!("tenant {tenant} archive {}", verified.header.archive_id),
        json!({
            "archive_id": verified.header.archive_id,
            "manifest_digest": verified.manifest_digest,
            "tables": i64::try_from(verified.manifest.tables.len()).unwrap_or(i64::MAX),
            "rows": i64::try_from(verified.manifest.rows).unwrap_or(i64::MAX),
            "audit_head_seq": verified.manifest.audit.head_seq,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_binding_names_archive_header_position_and_kind() {
        let archive = Uuid::nil();
        assert_eq!(
            binding(archive, "ab", 7, DATA),
            "00000000-0000-0000-0000-000000000000:ab:7:D"
        );
        assert_ne!(
            binding(archive, "ab", 7, DATA),
            binding(archive, "ab", 7, MANIFEST)
        );
        assert_ne!(
            binding(archive, "ab", 7, DATA),
            binding(archive, "ab", 8, DATA)
        );
    }

    #[test]
    fn a_data_frame_must_carry_exactly_its_declared_rows() {
        let frame = |rows: u32, body: &[u8]| {
            let mut bytes = vec![0, 3];
            bytes.extend_from_slice(&rows.to_be_bytes());
            bytes.extend_from_slice(body);
            data_frame(Zeroizing::new(bytes))
        };
        assert!(matches!(
            frame(2, b"a\nb\n"),
            Ok(Frame::Data {
                ordinal: 3,
                rows: 2,
                ..
            })
        ));
        assert!(frame(1, b"a\nb\n").is_err());
        assert!(frame(2, b"a\nb").is_err());
        assert!(frame(0, b"").is_err());
    }

    #[test]
    fn limits_refuse_zero_and_unbounded_values() {
        assert!(Limits::new(0, 10).is_err());
        assert!(Limits::new(HARD_MAX_BYTES + 1, 10).is_err());
        assert!(Limits::new(1, 0).is_err());
        assert!(Limits::new(DEFAULT_MAX_BYTES, DEFAULT_TIMEOUT_SECONDS).is_ok());
    }

    /// Writes a minimal valid archive with two data frames for `tenant`.
    fn synthetic(dir: &Path, kms: &synveda_crypto::LocalKms, tenant: TenantId) -> PathBuf {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let tenant_dek = DataKey::generate().unwrap();
        let wrapped_dek = runtime
            .block_on(kms.wrap_key(KeyScope::Tenant(tenant), &tenant_dek))
            .unwrap();
        let header = Header {
            format: FORMAT.to_owned(),
            archive_id: Uuid::now_v7(),
            tenant_id: tenant,
            schema: Schema {
                epoch: 3,
                baseline_revision: 3,
                migration_head: "0014".to_owned(),
            },
            product_version: "test".to_owned(),
            snapshot_at: chrono::Utc::now(),
            key: KeyRecord {
                version: 1,
                kek_ref: kms.key_ref().to_owned(),
                wrapped_dek: hex(&wrapped_dek),
            },
            frame_bound: FRAME_BOUND as u64,
        };
        let path = dir.join(format!("{}.svarc", header.archive_id));
        let file = std::fs::File::create(&path).unwrap();
        let sealing = SealingKey::new(KeyScope::Tenant(tenant), KeyVersion::FIRST, tenant_dek);
        let limits = Limits::new(DEFAULT_MAX_BYTES, 60).unwrap();
        let mut writer = start_archive(file, &header, &sealing, limits).unwrap();
        let mut tables: Vec<TableRecord> = rows::portable_tables()
            .map(|table| TableRecord {
                name: table.name.to_owned(),
                columns: vec!["tenant_id".to_owned()],
                rows: 0,
                digest: hex(blake3::Hasher::new().finalize().as_bytes()),
            })
            .collect();
        for (ordinal, body) in [(0_u16, b"a\n".as_slice()), (1, b"b\nc\n".as_slice())] {
            let rows = u32::try_from(body.iter().filter(|byte| **byte == b'\n').count()).unwrap();
            let mut frame = ordinal.to_be_bytes().to_vec();
            frame.extend_from_slice(&rows.to_be_bytes());
            frame.extend_from_slice(body);
            writer.frame(DATA, &frame).unwrap();
            tables[usize::from(ordinal)].rows = u64::from(rows);
            tables[usize::from(ordinal)].digest = hex(blake3::hash(body).as_bytes());
        }
        let manifest = Manifest {
            tenant: TenantRecord {
                id: tenant,
                slug: "synthetic".to_owned(),
                name: "Synthetic".to_owned(),
                created_at: chrono::Utc::now(),
                status: synveda_types::TenantStatus::Suspended,
            },
            rows: 3,
            tables,
            excluded: Vec::new(),
            audit: AuditRecord {
                head_seq: 0,
                head_hash: String::new(),
            },
            data_frames: 2,
        };
        finish_archive(writer, &manifest, header.archive_id, &header).unwrap();
        path
    }

    /// Byte offsets of every frame after the header and wrapped key.
    fn frame_offsets(bytes: &[u8]) -> Vec<(usize, usize)> {
        let length = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let mut at = 8;
        at += 4 + length(at);
        at += 4 + length(at);
        let mut frames = Vec::new();
        while at < bytes.len() {
            let end = at + 5 + length(at + 1);
            frames.push((at, end));
            at = end;
        }
        frames
    }

    #[test]
    fn reordered_duplicated_and_headless_frames_refuse() {
        let dir = std::env::temp_dir().join(format!("synveda-archive-unit-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let kms = synveda_crypto::LocalKms::from_hex(&"42".repeat(32), "local:unit").unwrap();
        let tenant = TenantId::new();
        let path = synthetic(&dir, &kms, tenant);
        let check = |bytes: Vec<u8>| {
            let candidate = dir.join(format!("{}.svarc", Uuid::now_v7()));
            std::fs::write(&candidate, bytes).unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            runtime
                .block_on(verify_file(
                    &candidate,
                    Limits::new(DEFAULT_MAX_BYTES, 60).unwrap(),
                    &Kms::Local(
                        synveda_crypto::LocalKms::from_hex(&"42".repeat(32), "local:unit").unwrap(),
                    ),
                ))
                .map(|verified| verified.manifest.rows)
        };
        let original = std::fs::read(&path).unwrap();
        assert_eq!(check(original.clone()), Ok(3));
        let frames = frame_offsets(&original);
        assert_eq!(frames.len(), 3, "two data frames and a manifest");
        let piece = |index: usize| original[frames[index].0..frames[index].1].to_vec();
        let prefix = original[..frames[0].0].to_vec();

        let swapped = [prefix.clone(), piece(1), piece(0), piece(2)].concat();
        assert!(check(swapped).unwrap_err().starts_with("archive_corrupt"));
        let duplicated = [prefix.clone(), piece(0), piece(0), piece(1), piece(2)].concat();
        assert!(
            check(duplicated)
                .unwrap_err()
                .starts_with("archive_corrupt")
        );
        let without_manifest = [prefix.clone(), piece(0), piece(1)].concat();
        assert!(
            check(without_manifest)
                .unwrap_err()
                .starts_with("archive_truncated")
        );
        let mut retagged = original.clone();
        retagged[frames[0].0] = MANIFEST;
        assert!(check(retagged).unwrap_err().starts_with("archive_corrupt"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hex_round_trips_and_refuses_malformed_text() {
        assert_eq!(unhex(&hex(&[0, 15, 255])).unwrap(), vec![0, 15, 255]);
        assert!(unhex("abc").is_err());
        assert!(unhex("zz").is_err());
    }
}
