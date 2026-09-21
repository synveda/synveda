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

pub fn powershell(path: &Path, target: Option<&Path>, script: &str) {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!("$ErrorActionPreference = 'Stop'; {script}"))
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
}
