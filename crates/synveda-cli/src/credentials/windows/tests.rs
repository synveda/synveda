use super::*;

#[path = "../../../tests/support/windows_private.rs"]
mod fixture;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!(
            "synveda Windows '文' {:032x}",
            u128::from_be_bytes(nonce)
        ));
        std::fs::create_dir(&path).unwrap();
        fixture::private(&path);
        Self(path)
    }
    fn directory(&self) -> Directory {
        Directory::open(&self.0.join("config/synveda"), true)
            .unwrap()
            .unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn windows_private_create_read_replace_and_lock_preserve_private_identity() {
    let scratch = Scratch::new();
    let directory = scratch.directory();
    assert!(directory.read("credentials.json").unwrap().is_none());
    directory
        .replace("credentials.json", b"first private value")
        .unwrap();
    let first = directory.read("credentials.json").unwrap().unwrap();
    directory
        .replace("credentials.json", b"second private value")
        .unwrap();
    let second = directory.read("credentials.json").unwrap().unwrap();
    assert_ne!(first.0, second.0);
    assert_eq!(second.1, b"second private value");
    let lock = directory.lock_file().unwrap();
    lock.try_lock().unwrap();
    assert!(matches!(
        directory.lock_file().unwrap().try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(lock);
    directory.lock_file().unwrap().try_lock().unwrap();
    assert_eq!(std::fs::read_dir(&directory.path).unwrap().count(), 2);
    // .NET independently checks the real OS ACL, not the SDDL admission parser.
    fixture::powershell(
        &directory.path.join("credentials.json"),
        None,
        r#"
$acl = Get-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH
if (-not $acl.AreAccessRulesProtected) { throw 'unprotected credential ACL' }
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
if ($acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -ne $sid) { throw 'wrong owner' }
foreach ($rule in $acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])) {
  if ($rule.IdentityReference.Value -notin @($sid, 'S-1-5-18', 'S-1-5-32-544') -or $rule.IsInherited) { throw 'unexpected credential access' }
}
"#,
    );
}

#[test]
fn windows_private_rename_failure_preserves_original_and_cleans_only_its_temporary() {
    let scratch = Scratch::new();
    let directory = scratch.directory();
    directory
        .replace("credentials.json", b"keep original")
        .unwrap();
    let path = directory.path.join("credentials.json");
    let reader = options(false, false).open(&path).unwrap();
    assert!(
        directory
            .replace("credentials.json", b"replacement must fail")
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"keep original");
    assert_eq!(std::fs::read_dir(&directory.path).unwrap().count(), 1);
    drop(reader);
    directory
        .replace("credentials.json", b"retry succeeds")
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"retry succeeds");
    assert!(std::fs::rename(&directory.path, scratch.0.join("moved")).is_err());
}

#[test]
fn windows_private_broad_acl_refuses_read_replacement_and_creation_without_repair() {
    let scratch = Scratch::new();
    let directory = scratch.directory();
    directory
        .replace("credentials.json", b"retained private bytes")
        .unwrap();
    let path = directory.path.join("credentials.json");
    fixture::powershell(
        &path,
        None,
        r#"
$acl = Get-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new([System.Security.Principal.SecurityIdentifier]::new('S-1-1-0'), 'Read', 'Allow'))
Set-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -AclObject $acl
"#,
    );
    let before = acl(&options(false, false).open(&path).unwrap()).unwrap();
    assert!(directory.read("credentials.json").is_err());
    assert!(
        directory
            .replace("credentials.json", b"never written")
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"retained private bytes");
    assert_eq!(
        acl(&options(false, false).open(&path).unwrap()).unwrap(),
        before
    );
    drop(directory);
    fixture::powershell(
        &scratch.0,
        None,
        r#"
$acl = Get-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new([System.Security.Principal.SecurityIdentifier]::new('S-1-1-0'), 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
Set-Acl -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -AclObject $acl
"#,
    );
    let absent = scratch.0.join("must-stay-absent");
    assert!(Directory::open(&absent, true).is_err());
    assert!(!absent.exists());
}

#[test]
fn windows_private_hardlinks_junctions_and_oversized_files_are_refused() {
    let scratch = Scratch::new();
    let directory = scratch.directory();
    directory.replace("credentials.json", b"retained").unwrap();
    let path = directory.path.join("credentials.json");
    let alias = directory.path.join("alias.json");
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(directory.read("credentials.json").is_err());
    assert!(
        directory
            .replace("credentials.json", b"never written")
            .is_err()
    );
    assert_eq!(std::fs::read(&alias).unwrap(), b"retained");
    std::fs::remove_file(&alias).unwrap();
    directory.lock_file().unwrap();
    let lock_path = directory.path.join("credentials.lock");
    std::fs::hard_link(&lock_path, &alias).unwrap();
    assert!(directory.lock_file().is_err());
    std::fs::remove_file(&alias).unwrap();
    options(true, false)
        .open(&path)
        .unwrap()
        .set_len(MAX_BYTES + 1)
        .unwrap();
    assert!(directory.read("credentials.json").is_err());
    let junction = scratch.0.join("junction");
    fixture::powershell(
        &junction,
        Some(&directory.path),
        r#"
New-Item -ItemType Junction -Path $env:SYNVEDA_TEST_ACL_PATH -Target $env:SYNVEDA_TEST_ACL_TARGET | Out-Null
"#,
    );
    assert!(Directory::open(&junction, false).is_err());
    assert!(Directory::open(&junction.join("child"), true).is_err());
    assert!(!directory.path.join("child").exists());
    std::fs::remove_dir(&junction).unwrap();
}

#[test]
fn windows_private_ambiguous_paths_refuse_before_mutation() {
    for path in [
        "relative",
        "C:relative",
        r"\\server\share",
        r"\\?\C:\temp",
        "C:/one/../two",
        "C:/one/./two",
        "C:/one./two",
        "C:/one /two",
        "C:/CON",
        "C:/NUL.txt",
        "C:/COM1",
        "C:/LPT².log",
        "C:/file:stream",
        "C:/one//two",
        "C:/one/*",
        "C:/one/\0",
    ] {
        assert!(Directory::open(Path::new(path), true).is_err(), "{path:?}");
    }
}
