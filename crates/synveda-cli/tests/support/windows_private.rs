//! Independent .NET ACL fixtures. Used only on task-owned Windows test paths.

use std::path::Path;
use std::process::Command;

pub fn private(path: &Path) {
    powershell(
        path,
        None,
        r#"
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$item = Get-Item -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -Force
if ($item.PSIsContainer) {
  $acl = [System.Security.AccessControl.DirectorySecurity]::new()
  $inherit = [System.Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit'
} else {
  $acl = [System.Security.AccessControl.FileSecurity]::new()
  $inherit = [System.Security.AccessControl.InheritanceFlags]::None
}
$acl.SetOwner($sid)
$acl.SetAccessRuleProtection($true, $false)
foreach ($identity in @($sid, [System.Security.Principal.SecurityIdentifier]::new('S-1-5-18'), [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))) {
  $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($identity, 'FullControl', $inherit, 'None', 'Allow')
  $acl.AddAccessRule($rule)
}
Set-Acl -LiteralPath $item.FullName -AclObject $acl
"#,
    );
}

/// Adds one allow entry with a DACL-only write. `Set-Acl` with a `Get-Acl`
/// object also writes the SACL, which a non-elevated token cannot do.
#[allow(dead_code)] // Shared support module: not every test crate edits DACLs.
pub fn allow(path: &Path, sid: &str, rights: &str, inheritance: &str) {
    powershell(
        path,
        None,
        &format!(
            r#"
$item = Get-Item -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -Force
$acl = $item.GetAccessControl('Access')
$identity = [System.Security.Principal.SecurityIdentifier]::new('{sid}')
$acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($identity, '{rights}', '{inheritance}', 'None', 'Allow'))
$item.SetAccessControl($acl)
"#
        ),
    );
}

/// The OS-rendered DACL, read independently of the product's parser.
#[allow(dead_code)] // Shared support module: not every test crate inspects DACLs.
pub fn dacl(path: &Path) -> String {
    powershell(
        path,
        None,
        "(Get-Item -LiteralPath $env:SYNVEDA_TEST_ACL_PATH -Force).GetAccessControl('Access').GetSecurityDescriptorSddlForm('Access')",
    )
    .trim()
    .to_owned()
}

pub fn powershell(path: &Path, target: Option<&Path>, script: &str) -> String {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!("$ErrorActionPreference = 'Stop'; {script}"))
        // A PowerShell 7 runner passes its incompatible module paths through
        // Cargo. Let Windows PowerShell rebuild its own built-in module path.
        .env_remove("PSModulePath")
        .env("SYNVEDA_TEST_ACL_PATH", path);
    if let Some(target) = target {
        command.env("SYNVEDA_TEST_ACL_TARGET", target);
    }
    let output = command.output().expect("Windows native ACL fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
