//! Recovery custody bundles (OPS-5).
//!
//! A bundle carries the files needed to use a restored database — the KEK,
//! its reference, the identity convergence credential and the backup
//! repository passphrase — sealed under a custodian key that is held apart
//! from both the deployment and its backup repository. The clear header holds
//! identifiers only; its SHA-256 is the envelope's row key, so a replaced
//! header or a body transplanted between bundles fails to open.
//!
//! ```text
//!   0..8   magic        b"SVRCUST1"
//!   8..12  header length u32, big-endian
//!  12..    header bytes (one JSON object) || envelope
//! ```

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use synveda_crypto::{DataKey, KeyScope, KeyVersion, Purpose, RowKey, SealingKey};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"SVRCUST1";
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_ENTRIES: usize = 16;
const MAX_ENTRY_BYTES: usize = 4096;
const MAX_BUNDLE_BYTES: u64 = 256 * 1024;
const HEADER_FILE: &str = "header.json";
const CUSTODIAN_KEY: &str = "SYNVEDA_RECOVERY_CUSTODY_KEY";
const OPEN_REFUSED: &str = "recovery custody bundle did not open under this custodian key";

#[derive(Serialize, Deserialize)]
struct Entry {
    name: String,
    value_b64: String,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    entries: Vec<Entry>,
}

type Entries = Vec<(String, Zeroizing<Vec<u8>>)>;

/// One opened bundle: the authenticated header and its entries, by name.
struct Opened {
    header: Vec<u8>,
    entries: Entries,
}

fn custodian_key() -> Result<SealingKey, String> {
    let hex = Zeroizing::new(
        crate::settings::sensitive_setting(CUSTODIAN_KEY)?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{CUSTODIAN_KEY} or {CUSTODIAN_KEY}_FILE is required"))?,
    );
    let key = DataKey::from_hex(hex.trim())
        .map_err(|_| format!("{CUSTODIAN_KEY} must be 64 hexadecimal characters"))?;
    Ok(sealing_key(key))
}

fn sealing_key(key: DataKey) -> SealingKey {
    SealingKey::new(KeyScope::Deployment, KeyVersion::FIRST, key)
}

fn header_digest(header: &[u8]) -> [u8; 32] {
    Sha256::digest(header).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_entry_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name != HEADER_FILE
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn validate_header(header: &[u8]) -> Result<(), String> {
    if header.is_empty() || header.len() > MAX_HEADER_BYTES {
        return Err(format!(
            "recovery custody header must contain 1 through {MAX_HEADER_BYTES} bytes"
        ));
    }
    match serde_json::from_slice::<serde_json::Value>(header) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        _ => Err("recovery custody header must be one JSON object".to_owned()),
    }
}

fn read_bounded(path: &Path, limit: u64, label: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| format!("{label} cannot be read"))?;
    if !metadata.file_type().is_file() {
        return Err(format!("{label} must be a regular file"));
    }
    if metadata.len() > limit {
        return Err(format!("{label} exceeds {limit} bytes"));
    }
    let file = std::fs::File::open(path).map_err(|_| format!("{label} cannot be read"))?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{label} cannot be read"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label} exceeds {limit} bytes"));
    }
    Ok(bytes)
}

fn input_entries(directory: &Path) -> Result<Entries, String> {
    let listing = std::fs::read_dir(directory)
        .map_err(|_| "recovery custody input directory cannot be read".to_owned())?;
    let mut entries = Vec::new();
    for item in listing.take(MAX_ENTRIES + 1) {
        let item = item.map_err(|_| "recovery custody input directory cannot be read")?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| "recovery custody entry names must be UTF-8".to_owned())?;
        if !valid_entry_name(&name) {
            return Err("recovery custody entry names must match [a-z0-9_]{1,64}".to_owned());
        }
        let value = read_bounded(
            &item.path(),
            MAX_ENTRY_BYTES as u64,
            "recovery custody entry",
        )?;
        if value.is_empty() {
            return Err("recovery custody entries must not be empty".to_owned());
        }
        entries.push((name, value));
    }
    if entries.is_empty() || entries.len() > MAX_ENTRIES {
        return Err(format!(
            "recovery custody input must contain 1 through {MAX_ENTRIES} entries"
        ));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn seal_bundle(
    key: &SealingKey,
    header: &[u8],
    entries: &[(String, Zeroizing<Vec<u8>>)],
) -> Result<Vec<u8>, String> {
    validate_header(header)?;
    let payload = Payload {
        entries: entries
            .iter()
            .map(|(name, value)| Entry {
                name: name.clone(),
                value_b64: STANDARD.encode(value.as_slice()),
            })
            .collect(),
    };
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&payload).map_err(|_| "recovery custody payload failed".to_owned())?,
    );
    for entry in payload.entries {
        drop(Zeroizing::new(entry.value_b64));
    }
    let digest = header_digest(header);
    let envelope = key
        .seal(Purpose::RecoveryCustody, RowKey::Hash(&digest), &plaintext)
        .map_err(|_| "recovery custody sealing failed".to_owned())?;
    let header_len = u32::try_from(header.len())
        .map_err(|_| "recovery custody header is too large".to_owned())?;
    let mut bundle = Vec::with_capacity(MAGIC.len() + 4 + header.len() + envelope.len());
    bundle.extend_from_slice(MAGIC);
    bundle.extend_from_slice(&header_len.to_be_bytes());
    bundle.extend_from_slice(header);
    bundle.extend_from_slice(&envelope);
    Ok(bundle)
}

fn split_bundle(bundle: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let malformed = || "recovery custody bundle is malformed".to_owned();
    let rest = bundle
        .strip_prefix(MAGIC.as_slice())
        .ok_or_else(malformed)?;
    let (length, rest) = rest.split_at_checked(4).ok_or_else(malformed)?;
    let length = u32::from_be_bytes(length.try_into().map_err(|_| malformed())?) as usize;
    if length == 0 || length > MAX_HEADER_BYTES {
        return Err(malformed());
    }
    rest.split_at_checked(length).ok_or_else(malformed)
}

fn open_bundle(key: &SealingKey, bundle: &[u8]) -> Result<Opened, String> {
    let (header, envelope) = split_bundle(bundle)?;
    validate_header(header)?;
    let digest = header_digest(header);
    let plaintext = key
        .open(Purpose::RecoveryCustody, RowKey::Hash(&digest), envelope)
        .map_err(|_| OPEN_REFUSED.to_owned())?;
    let payload: Payload =
        serde_json::from_slice(&plaintext).map_err(|_| OPEN_REFUSED.to_owned())?;
    if payload.entries.is_empty() || payload.entries.len() > MAX_ENTRIES {
        return Err(OPEN_REFUSED.to_owned());
    }
    let mut entries = Vec::with_capacity(payload.entries.len());
    for entry in payload.entries {
        let encoded = Zeroizing::new(entry.value_b64);
        let value = Zeroizing::new(
            STANDARD
                .decode(encoded.as_bytes())
                .map_err(|_| OPEN_REFUSED.to_owned())?,
        );
        if !valid_entry_name(&entry.name) || value.is_empty() || value.len() > MAX_ENTRY_BYTES {
            return Err(OPEN_REFUSED.to_owned());
        }
        if entries.iter().any(|(name, _)| name == &entry.name) {
            return Err(OPEN_REFUSED.to_owned());
        }
        entries.push((entry.name, value));
    }
    Ok(Opened {
        header: header.to_vec(),
        entries,
    })
}

fn write_new_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "recovery custody output could not be created".to_owned())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "recovery custody output could not be written".to_owned())
}

fn require_empty_directory(directory: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(directory)
        .map_err(|_| "recovery custody output directory is unavailable".to_owned())?;
    if !metadata.file_type().is_dir() {
        return Err("recovery custody output must be a directory".to_owned());
    }
    let mut listing = std::fs::read_dir(directory)
        .map_err(|_| "recovery custody output directory is unavailable".to_owned())?;
    if listing.next().is_some() {
        return Err("recovery custody output directory must be empty".to_owned());
    }
    Ok(())
}

fn summary(opened: &Opened) -> serde_json::Value {
    serde_json::json!({
        "header_sha256": hex(&header_digest(&opened.header)),
        "entries": opened
            .entries
            .iter()
            .map(|(name, value)| serde_json::json!({"name": name, "bytes": value.len()}))
            .collect::<Vec<_>>(),
    })
}

/// Seals every file in `input` with the header at `header` into `output`.
///
/// # Errors
/// A missing custodian key, an unsafe or oversized input, or an output path
/// that already exists.
pub fn seal(header: &Path, input: &Path, output: &Path) -> Result<(), String> {
    let key = custodian_key()?;
    let header = read_bounded(header, MAX_HEADER_BYTES as u64, "recovery custody header")?;
    let entries = input_entries(input)?;
    let bundle = seal_bundle(&key, &header, &entries)?;
    write_new_private(output, &bundle)?;
    let opened = Opened {
        header: header.to_vec(),
        entries,
    };
    println!("{}", summary(&opened));
    Ok(())
}

fn read_bundle(input: &Path) -> Result<Zeroizing<Vec<u8>>, String> {
    read_bounded(input, MAX_BUNDLE_BYTES, "recovery custody bundle")
}

/// Opens `input` and writes its header and entries into the empty `output`.
///
/// # Errors
/// A wrong custodian key, a replaced header or body, or a non-empty output.
pub fn open(input: &Path, output: &Path) -> Result<(), String> {
    let key = custodian_key()?;
    let opened = open_bundle(&key, &read_bundle(input)?)?;
    require_empty_directory(output)?;
    write_new_private(&output.join(HEADER_FILE), &opened.header)?;
    for (name, value) in &opened.entries {
        let target: PathBuf = output.join(name);
        write_new_private(&target, value)?;
    }
    println!("{}", summary(&opened));
    Ok(())
}

/// Opens `input` in memory and prints only its content-free summary.
///
/// # Errors
/// A wrong custodian key or a replaced header or body.
pub fn verify(input: &Path) -> Result<(), String> {
    let key = custodian_key()?;
    let opened = open_bundle(&key, &read_bundle(input)?)?;
    println!("{}", summary(&opened));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SealingKey {
        sealing_key(DataKey::generate().expect("custodian key"))
    }

    fn entries() -> Entries {
        vec![
            (
                "synveda_kms_key".to_owned(),
                Zeroizing::new(b"aa\n".to_vec()),
            ),
            (
                "synveda_kms_key_ref".to_owned(),
                Zeroizing::new(b"local:x\n".to_vec()),
            ),
        ]
    }

    const HEADER: &[u8] = b"{\"format\":\"synveda-recovery-custody-v1\"}\n";

    #[test]
    fn a_sealed_bundle_opens_with_its_header_and_entries() {
        let key = key();
        let bundle = seal_bundle(&key, HEADER, &entries()).expect("seal");
        let opened = open_bundle(&key, &bundle).expect("open");
        assert_eq!(opened.header, HEADER);
        let names: Vec<&str> = opened
            .entries
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["synveda_kms_key", "synveda_kms_key_ref"]);
        assert_eq!(opened.entries[1].1.as_slice(), b"local:x\n");
    }

    #[test]
    fn the_clear_bundle_does_not_contain_entry_values() {
        let bundle = seal_bundle(&key(), HEADER, &entries()).expect("seal");
        assert!(!bundle.windows(7).any(|window| window == b"local:x"));
    }

    #[test]
    fn another_custodian_key_is_refused_uniformly() {
        let bundle = seal_bundle(&key(), HEADER, &entries()).expect("seal");
        let error = open_bundle(&key(), &bundle).err().expect("wrong key");
        assert_eq!(error, OPEN_REFUSED);
    }

    #[test]
    fn a_replaced_header_of_the_same_length_is_refused() {
        let key = key();
        let mut bundle = seal_bundle(&key, HEADER, &entries()).expect("seal");
        let position = 12 + HEADER.iter().position(|byte| *byte == b'v').expect("v");
        bundle[position] = b'w';
        let error = open_bundle(&key, &bundle).err().expect("replaced header");
        assert_eq!(error, OPEN_REFUSED);
    }

    #[test]
    fn a_body_moved_under_another_header_is_refused() {
        let key = key();
        let first = seal_bundle(&key, HEADER, &entries()).expect("seal");
        let other = b"{\"format\":\"synveda-recovery-custody-v2\"}\n";
        let second = seal_bundle(&key, other, &entries()).expect("seal");
        let mut moved = second[..12 + other.len()].to_vec();
        moved.extend_from_slice(&first[12 + HEADER.len()..]);
        assert_eq!(
            open_bundle(&key, &moved).err().as_deref(),
            Some(OPEN_REFUSED)
        );
    }

    #[test]
    fn truncated_or_foreign_bundles_are_malformed() {
        let key = key();
        let bundle = seal_bundle(&key, HEADER, &entries()).expect("seal");
        for candidate in [&bundle[..4], &bundle[..12], &bundle[..bundle.len() - 1]] {
            assert!(open_bundle(&key, candidate).is_err());
        }
        let mut foreign = bundle.clone();
        foreign[0] = b'X';
        assert!(
            open_bundle(&key, &foreign)
                .err()
                .expect("magic")
                .contains("malformed")
        );
    }

    #[test]
    fn headers_must_be_one_bounded_json_object() {
        let key = key();
        for header in [&b""[..], b"[1]", b"not json"] {
            assert!(seal_bundle(&key, header, &entries()).is_err());
        }
        let oversized = vec![b' '; MAX_HEADER_BYTES + 1];
        assert!(seal_bundle(&key, &oversized, &entries()).is_err());
    }

    #[test]
    fn entry_names_are_closed() {
        assert!(valid_entry_name("pgbackrest_cipher_pass"));
        for name in ["", "header.json", "../x", "UPPER", "a-b", &"x".repeat(65)] {
            assert!(!valid_entry_name(name), "{name}");
        }
    }

    #[test]
    fn open_writes_only_into_an_empty_directory() {
        let root = std::env::temp_dir().join(format!("synveda-custody-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&root).expect("root");
        std::fs::write(root.join("existing"), b"x").expect("existing");
        assert!(require_empty_directory(&root).is_err());
        std::fs::remove_file(root.join("existing")).expect("cleanup");
        assert!(require_empty_directory(&root).is_ok());
        assert!(require_empty_directory(&root.join("absent")).is_err());
        std::fs::remove_dir(&root).expect("cleanup");
    }
}
