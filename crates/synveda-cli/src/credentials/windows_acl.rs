//! Closed admission policy for OS-rendered owner/DACL SDDL (OPS-12).
//!
//! This is deliberately not a Windows access evaluator. Only ordinary allow
//! entries for the process user and host administrators are admitted; unfamiliar
//! forms are refused before any private bytes are accessed.

pub(super) fn validate(sddl: &str, user: &str, directory: bool) -> Result<(), &'static str> {
    let refused = "Windows private ownership or ACL was refused";
    if sddl.len() > 65_536 {
        return Err(refused);
    }
    let (owner, dacl) = sddl
        .strip_prefix("O:")
        .and_then(|s| s.split_once("D:"))
        .ok_or(refused)?;
    if owner != user {
        return Err(refused);
    }
    let (mut control, mut entries) = dacl.split_at(dacl.find('(').ok_or(refused)?);
    while !control.is_empty() {
        control = control
            .strip_prefix('P')
            .or_else(|| control.strip_prefix("AI"))
            .or_else(|| control.strip_prefix("AR"))
            .ok_or(refused)?;
    }
    let mut access = false;
    let mut inheritance = false;
    let mut count = 0;
    while !entries.is_empty() {
        count += 1;
        if count > 128 {
            return Err(refused);
        }
        let (entry, rest) = entries
            .strip_prefix('(')
            .and_then(|s| s.split_once(')'))
            .ok_or(refused)?;
        entries = rest;
        let fields: Vec<_> = entry.split(';').collect();
        if fields.len() != 6 || fields[0] != "A" || !fields[3].is_empty() || !fields[4].is_empty() {
            return Err(refused);
        }
        let mut flags = fields[1];
        let (mut object, mut container, mut inherit_only, mut no_propagate) =
            (false, false, false, false);
        while !flags.is_empty() {
            if let Some(rest) = flags.strip_prefix("OI") {
                object = true;
                flags = rest;
            } else if let Some(rest) = flags.strip_prefix("CI") {
                container = true;
                flags = rest;
            } else if let Some(rest) = flags.strip_prefix("IO") {
                inherit_only = true;
                flags = rest;
            } else if let Some(rest) = flags.strip_prefix("NP") {
                no_propagate = true;
                flags = rest;
            } else if let Some(rest) = flags.strip_prefix("ID") {
                flags = rest;
            } else {
                return Err(refused);
            }
        }
        let full = matches!(fields[2], "FA" | "GA" | "0x1f01ff" | "0x10000000");
        // Only the OS generates this input, but bound the grammar rather than
        // treating an unknown mask or conditional ACE as an ordinary allow.
        if !full
            && !matches!(fields[2], "FR" | "FW" | "FX" | "GR" | "GW" | "GX")
            && !fields[2].strip_prefix("0x").is_some_and(|hex| {
                !hex.is_empty() && hex.len() <= 8 && hex.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(refused);
        }
        match fields[5] {
            sid if sid == user => {
                access |= full && !inherit_only;
                inheritance |= full && object && container && !no_propagate;
            }
            "SY" | "S-1-5-18" | "BA" | "S-1-5-32-544" => {}
            "CO" | "S-1-3-0" if directory && inherit_only && (object || container) => {}
            _ => return Err(refused),
        }
    }
    if !access || (directory && !inheritance) {
        return Err(refused);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const USER: &str = "S-1-5-21-1-2-3-1001";

    #[test]
    fn private_acl_accepts_only_the_user_and_privileged_host_principals() {
        for flags in ["P", "AI", "PAI"] {
            let sddl = format!(
                "O:{USER}D:{flags}(A;OICI;FA;;;{USER})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICIIO;GA;;;CO)"
            );
            assert!(validate(&sddl, USER, true).is_ok());
        }
        assert!(validate(&format!("O:{USER}D:P(A;;FA;;;{USER})"), USER, false).is_ok());
    }

    #[test]
    fn private_acl_refuses_disclosure_ambiguous_authority_and_unsafe_inheritance() {
        let private = format!("O:{USER}D:P(A;OICI;FA;;;{USER})");
        for extra in [
            "(A;;FR;;;WD)",
            "(A;IOOI;FR;;;WD)",
            "(A;;FA;;;AU)",
            "(A;;FA;;;CO)",
            "(D;;FR;;;WD)",
            "(OA;;FA;guid;;SY)",
            "(XA;;FA;;;SY;condition)",
            "(A;ZZ;FA;;;SY)",
            "(A;;unknown;;;SY)",
            "(A;;FA;;;S-1-5-21-9-8-7-1001)",
            "trailing",
        ] {
            assert!(
                validate(&format!("{private}{extra}"), USER, true).is_err(),
                "{extra}"
            );
        }
        for sddl in [
            format!("O:BAD:{private}"),
            format!("O:{USER}D:NO_ACCESS_CONTROL"),
            format!("O:{USER}D:P"),
            format!("O:{USER}D:P(A;OICIIO;FA;;;{USER})"),
            format!("O:{USER}D:P(A;OICINP;FA;;;{USER})"),
            format!("O:{USER}D:P(A;;FA;;;{USER})"),
            format!("O:{USER}D:P(A;OICI;FR;;;{USER})"),
        ] {
            assert!(validate(&sddl, USER, true).is_err(), "{sddl}");
        }
    }
}
