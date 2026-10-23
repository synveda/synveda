//! Closed admission policy for OS-rendered owner/DACL SDDL.
//!
//! This is deliberately not a Windows access evaluator. Only ordinary allow
//! entries for the process user and host administrators are admitted; unfamiliar
//! forms are refused before any private bytes are accessed.

/// Why an owner/DACL was refused. SIDs are identities, never secrets.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Refusal {
    /// The object belongs to an account this boundary does not trust.
    Owner(String),
    /// An allow entry grants another account more than this boundary admits.
    Grant(String),
    /// The process user lacks full (and, for directories, inheritable) access.
    UserAccess,
    /// A deny, object, conditional, unknown or oversized descriptor form.
    Unsupported,
}

impl Refusal {
    /// The refusal for one named object, with the next safe step.
    pub(super) fn explain(&self, path: &std::path::Path, ancestor: bool) -> String {
        let path = path.display();
        let reason = match (self, ancestor) {
            (Self::Owner(sid), true) => format!(
                "{path} is owned by {sid}, not by you, SYSTEM, Administrators or TrustedInstaller"
            ),
            (Self::Owner(sid), false) => format!(
                "{path} is owned by {sid}, not by the current user; Synveda did not create it for this account"
            ),
            (Self::Grant(sid), true) => format!(
                "{path} lets {sid} write, delete or change entries, so private state below it could be replaced"
            ),
            (Self::Grant(sid), false) => format!(
                "{path} grants {sid} access; private client state must be limited to you, SYSTEM and Administrators"
            ),
            (Self::UserAccess, _) => {
                format!("{path} does not give the current user full, inheritable access")
            }
            (Self::Unsupported, _) => format!(
                "{path} has a deny, object, conditional or unrecognised ACL entry that is not evaluated"
            ),
        };
        format!(
            "{reason}. Synveda never changes existing ACLs; preserve this object, or set \
             XDG_CONFIG_HOME and XDG_STATE_HOME to directories on a local fixed drive whose \
             ACL grants access only to you, SYSTEM and Administrators"
        )
    }
}

pub(super) fn validate(sddl: &str, user: &str, directory: bool) -> Result<(), Refusal> {
    validate_kind(sddl, user, directory, false)
}

pub(super) fn validate_ancestor(sddl: &str, user: &str) -> Result<(), Refusal> {
    validate_kind(sddl, user, true, true)
}

// The Windows Modules Installer service owns standard Windows volume roots.
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

fn privileged(sid: &str) -> bool {
    matches!(sid, "SY" | "S-1-5-18" | "BA" | "S-1-5-32-544")
}

fn mask(value: &str) -> Option<u32> {
    if let Some(hex) = value.strip_prefix("0x") {
        return (hex.len() <= 8)
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten();
    }
    let mut mask = 0;
    let mut rest = value;
    while !rest.is_empty() {
        let (code, tail) = rest.get(..2).zip(rest.get(2..))?;
        mask |= match code {
            "FA" => 0x001f_01ff,
            "FR" => 0x0012_0089,
            "FW" => 0x0012_0116,
            "FX" => 0x0012_00a0,
            "GA" => 0x1000_0000,
            "GR" => 0x8000_0000,
            "GW" => 0x4000_0000,
            "GX" => 0x2000_0000,
            "SD" => 0x0001_0000,
            "RC" => 0x0002_0000,
            "WD" => 0x0004_0000,
            "WO" => 0x0008_0000,
            // SDDL renders the directory create-subdirectory bit as LC even
            // for file-object descriptors (the same numeric bit as DS list).
            "LC" => 0x0000_0004,
            _ => return None,
        };
        rest = tail;
    }
    (!value.is_empty()).then_some(mask)
}

fn validate_kind(sddl: &str, user: &str, directory: bool, ancestor: bool) -> Result<(), Refusal> {
    if sddl.len() > 65_536 {
        return Err(Refusal::Unsupported);
    }
    let (owner, dacl) = sddl
        .strip_prefix("O:")
        .and_then(|s| s.split_once("D:"))
        .ok_or(Refusal::Unsupported)?;
    if owner != user && !(ancestor && (privileged(owner) || owner == TRUSTED_INSTALLER)) {
        return Err(Refusal::Owner(owner.to_owned()));
    }
    let (mut control, mut entries) = dacl.split_at(dacl.find('(').ok_or(Refusal::Unsupported)?);
    while !control.is_empty() {
        control = control
            .strip_prefix('P')
            .or_else(|| control.strip_prefix("AI"))
            .or_else(|| control.strip_prefix("AR"))
            .ok_or(Refusal::Unsupported)?;
    }
    let mut access = false;
    let mut inheritance = false;
    let mut count = 0;
    while !entries.is_empty() {
        count += 1;
        if count > 128 {
            return Err(Refusal::Unsupported);
        }
        let (entry, rest) = entries
            .strip_prefix('(')
            .and_then(|s| s.split_once(')'))
            .ok_or(Refusal::Unsupported)?;
        entries = rest;
        let fields: Vec<_> = entry.split(';').collect();
        if fields.len() != 6 || fields[0] != "A" || !fields[3].is_empty() || !fields[4].is_empty() {
            return Err(Refusal::Unsupported);
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
                return Err(Refusal::Unsupported);
            }
        }
        let mask = mask(fields[2]).ok_or(Refusal::Unsupported)?;
        let full = mask & 0x001f_01ff == 0x001f_01ff || mask & 0x1000_0000 != 0;
        match fields[5] {
            sid if sid == user => {
                access |= full && !inherit_only;
                inheritance |= full && object && container && !no_propagate;
            }
            sid if privileged(sid) || (ancestor && sid == TRUSTED_INSTALLER) => {}
            // Public traversal/read and creating a new subdirectory on a drive
            // root do not allow replacing our held paths or rewriting ACLs.
            // Inherit-only entries do not affect this ancestor; private child
            // admission independently rejects unsafe inherited access.
            _ if ancestor && (inherit_only || mask & !0xa012_00ad == 0) => {}
            "CO" | "S-1-3-0" if directory && inherit_only && (object || container) => {}
            other => return Err(Refusal::Grant(other.to_owned())),
        }
    }
    if !ancestor && (!access || (directory && !inheritance)) {
        return Err(Refusal::UserAccess);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const USER: &str = "S-1-5-21-1-2-3-1001";

    #[test]
    fn ancestor_acl_allows_traversal_but_refuses_other_account_mutation() {
        let root = format!(
            "O:{TRUSTED_INSTALLER}D:AI(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;BU)(A;;LC;;;AU)(A;OICIIO;SDGXGWGR;;;AU)(A;OICIIO;GA;;;CO)"
        );
        assert!(validate_ancestor(&root, USER).is_ok());
        for rights in ["FA", "GW", "SD", "WD", "WO", "0x2", "0x10", "0x40", "0x100"] {
            assert!(
                validate_ancestor(&format!("{root}(A;;{rights};;;WD)"), USER).is_err(),
                "{rights}"
            );
        }
        assert!(
            validate_ancestor(&format!("O:S-1-5-21-9-8-7-1001D:P(A;;FA;;;{USER})"), USER).is_err()
        );
    }

    #[test]
    fn refusals_name_the_object_principal_and_next_step() {
        // The shape a sandbox installer leaves on a user's AppData tree.
        let sandbox = "S-1-5-21-1-2-3-1016";
        let profile = format!(
            "O:{USER}D:AI(A;OICIID;0x1301ff;;;{sandbox})(A;OICIID;FA;;;SY)(A;OICIID;FA;;;{USER})"
        );
        let refusal = validate_ancestor(&profile, USER).unwrap_err();
        assert_eq!(refusal, Refusal::Grant(sandbox.to_owned()));
        let message = refusal.explain(std::path::Path::new(r"C:\Users\a b\AppData"), true);
        for part in [
            r"C:\Users\a b\AppData",
            sandbox,
            "never changes existing ACLs",
            "XDG_CONFIG_HOME",
        ] {
            assert!(message.contains(part), "{message}");
        }
        assert_eq!(
            validate(
                &format!("O:S-1-5-21-9-8-7-1001D:P(A;;FA;;;{USER})"),
                USER,
                false
            ),
            Err(Refusal::Owner("S-1-5-21-9-8-7-1001".to_owned()))
        );
        assert_eq!(
            validate(&format!("O:{USER}D:P(A;;FR;;;{USER})"), USER, false),
            Err(Refusal::UserAccess)
        );
        assert_eq!(
            validate(&format!("O:{USER}D:P(D;;FR;;;WD)"), USER, false),
            Err(Refusal::Unsupported)
        );
    }

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
