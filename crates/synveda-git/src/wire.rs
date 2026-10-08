//! The minimal smart-HTTP receive-pack exchange. No fetch, force, deletion,
//! hook capability, sideband messages or provider text is admitted (ADR-0139).

use std::collections::BTreeSet;

use synveda_types::{Error, Result};

use crate::github::{divergence, failure};
use crate::projection::valid_oid;

pub(crate) const ZERO: &str = "0000000000000000000000000000000000000000";
pub(crate) const MAX_RESPONSE: usize = 256 * 1024;
const MAX_REFS: usize = 2048;

pub(crate) struct Advertisement {
    pub(crate) head: Option<String>,
    pub(crate) object_format: bool,
}

fn packets(mut bytes: &[u8]) -> Result<Vec<Option<&[u8]>>> {
    let mut result = Vec::new();
    while !bytes.is_empty() {
        let header = bytes.get(..4).ok_or_else(failure)?;
        if !header
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(failure());
        }
        let length = usize::from_str_radix(std::str::from_utf8(header).map_err(|_| failure())?, 16)
            .map_err(|_| failure())?;
        if length == 0 {
            result.push(None);
            bytes = &bytes[4..];
        } else {
            if !(4..=65520).contains(&length) {
                return Err(failure());
            }
            result.push(Some(bytes.get(4..length).ok_or_else(failure)?));
            bytes = &bytes[length..];
        }
        if result.len() > MAX_REFS + 3 {
            return Err(failure());
        }
    }
    Ok(result)
}

fn line(bytes: &[u8]) -> Result<&str> {
    let text = std::str::from_utf8(bytes).map_err(|_| failure())?;
    Ok(text.strip_suffix('\n').unwrap_or(text))
}

pub(crate) fn advertisement(bytes: &[u8], reference: &str) -> Result<Advertisement> {
    if bytes.len() > MAX_RESPONSE {
        return Err(failure());
    }
    let packets = packets(bytes)?;
    if packets.len() < 4
        || packets[0].map(line).transpose()? != Some("# service=git-receive-pack")
        || packets[1].is_some()
        || packets.last() != Some(&None)
    {
        return Err(failure());
    }
    let mut seen = BTreeSet::new();
    let mut head = None;
    let mut object_format = false;
    for (index, packet) in packets[2..packets.len() - 1].iter().enumerate() {
        let text = line(packet.ok_or_else(failure)?)?;
        let (entry, capabilities) = if index == 0 {
            let (entry, caps) = text.split_once('\0').ok_or_else(failure)?;
            (entry, Some(caps))
        } else {
            if text.contains('\0') {
                return Err(failure());
            }
            (text, None)
        };
        if let Some(caps) = capabilities {
            let caps: BTreeSet<_> = caps.split(' ').filter(|cap| !cap.is_empty()).collect();
            if !caps.contains("report-status")
                || caps
                    .iter()
                    .any(|cap| cap.bytes().any(|b| b.is_ascii_control()))
                || caps
                    .iter()
                    .any(|cap| cap.starts_with("object-format=") && *cap != "object-format=sha1")
            {
                return Err(failure());
            }
            if caps
                .iter()
                .any(|cap| cap.starts_with(&format!("symref:{reference}:")))
            {
                return Err(divergence());
            }
            object_format = caps.contains("object-format=sha1");
        }
        let (oid, name) = entry.split_once(' ').ok_or_else(failure)?;
        if !valid_oid(oid)
            || name.is_empty()
            || name.len() > 1024
            || name
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            || !seen.insert(name)
        {
            return Err(failure());
        }
        if name == "capabilities^{}" {
            if oid != ZERO || index != 0 || packets.len() != 4 {
                return Err(failure());
            }
        } else if oid == ZERO || !(name == "HEAD" || name.starts_with("refs/")) {
            return Err(failure());
        }
        if name == reference {
            head = Some(oid.to_owned());
        }
    }
    Ok(Advertisement {
        head,
        object_format,
    })
}

pub(crate) fn packet(bytes: &[u8]) -> Result<Vec<u8>> {
    let length = bytes
        .len()
        .checked_add(4)
        .filter(|n| *n <= 65520)
        .ok_or_else(failure)?;
    let mut result = format!("{length:04x}").into_bytes();
    result.extend_from_slice(bytes);
    Ok(result)
}

pub(crate) fn update(
    old: Option<&str>,
    new: &str,
    reference: &str,
    object_format: bool,
) -> Result<Vec<u8>> {
    if !valid_oid(new) || new == ZERO || old.is_some_and(|oid| !valid_oid(oid) || oid == ZERO) {
        return Err(failure());
    }
    let format = if object_format {
        " object-format=sha1"
    } else {
        ""
    };
    let mut request = packet(
        format!(
            "{} {new} {reference}\0report-status{format}\n",
            old.unwrap_or(ZERO)
        )
        .as_bytes(),
    )?;
    request.extend_from_slice(b"0000");
    Ok(request)
}

pub(crate) fn report(bytes: &[u8], reference: &str) -> Result<()> {
    if bytes.len() > MAX_RESPONSE {
        return Err(failure());
    }
    let packets = packets(bytes)?;
    if packets.len() != 3 || packets[2].is_some() {
        return Err(failure());
    }
    if line(packets[0].ok_or_else(failure)?)? != "unpack ok" {
        return Err(failure());
    }
    let status = line(packets[1].ok_or_else(failure)?)?;
    if status == format!("ok {reference}") {
        return Ok(());
    }
    if status.starts_with(&format!("ng {reference} ")) {
        return Err(Error::Conflict { message: "GitHub export ref update was refused; inspect branch protection or destination divergence".into() });
    }
    Err(failure())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receive_pack_parser_refuses_unframed_ambiguous_or_secret_bearing_results() {
        let reference = "refs/heads/synveda/prompt/published";
        let mut empty = packet(b"# service=git-receive-pack\n").unwrap();
        empty.extend_from_slice(b"0000");
        empty.extend(
            packet(
                format!("{ZERO} capabilities^{{}}\0report-status object-format=sha1\n").as_bytes(),
            )
            .unwrap(),
        );
        empty.extend_from_slice(b"0000");
        let discovered = advertisement(&empty, reference).unwrap();
        assert!(discovered.head.is_none());
        assert!(discovered.object_format);
        for malformed in [
            b"ffffsource-secret".as_slice(),
            b"0001",
            b"00040000",
            &empty[..empty.len() - 1],
        ] {
            let error = advertisement(malformed, reference).err().unwrap();
            assert!(!error.to_string().contains("source-secret"));
        }
        let mut rejected = packet(b"unpack ok\n").unwrap();
        rejected.extend(packet(format!("ng {reference} source-secret\n").as_bytes()).unwrap());
        rejected.extend_from_slice(b"0000");
        assert!(matches!(
            report(&rejected, reference),
            Err(Error::Conflict { .. })
        ));
        assert!(
            !report(&rejected, reference)
                .unwrap_err()
                .to_string()
                .contains("source-secret")
        );
        rejected.extend_from_slice(b"0000");
        assert!(report(&rejected, reference).is_err());
    }
}
