# OPS-12: client-only Windows bootstrap. No elevation or policy override.
#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$BaseUrl = "https://github.com/synveda/synveda/releases/download/v$Version",
    [string]$InstallRoot = "$env:LOCALAPPDATA\SynvedaClient",
    [string]$BinDirectory = ""
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Native Windows is required' }
if ($Version.Length -gt 63 -or $Version -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$') { throw 'Invalid release version' }
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$trusted = @($sid, 'S-1-5-18', 'S-1-5-32-544')
$trustedInstaller = 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464'

function Assert-LocalPath([string]$Path) {
    if ($Path -cnotmatch '^[A-Za-z]:[\\/]' -or $Path.Substring(2).Contains(':')) { throw 'A fully qualified local drive path is required' }
    $parts = $Path.Substring(3) -split '[\\/]'
    foreach ($part in $parts) {
        if ($part -eq '' -or $part -in @('.', '..') -or $part -match '[. ]$|[<>:"|?*\x00-\x1f]' -or
            ($part -split '\.')[0] -match '^(?i:CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|COM[1-9\u00b9\u00b2\u00b3]|LPT[1-9\u00b9\u00b2\u00b3])$') { throw 'Ambiguous Windows path component' }
    }
    $drive = [System.IO.DriveInfo]::new($Path.Substring(0, 3))
    if ($drive.DriveType -ne [System.IO.DriveType]::Fixed) { throw 'Private installation requires a local fixed drive' }
}

function Assert-Acl([string]$Path, [bool]$Private) {
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or -not $item.PSIsContainer) { throw 'Install ancestry contains a link or non-directory' }
    $acl = Get-Acl -LiteralPath $Path
    $owner = $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
    if (($Private -and $owner -ne $sid) -or (-not $Private -and $owner -notin ($trusted + $trustedInstaller))) { throw 'Install ancestry has an untrusted owner' }
    $ownAccess = $false
    $inheritance = $false
    $rules = $acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])
    if ($rules.Count -eq 0 -or $rules.Count -gt 128) { throw 'Unsupported install ACL' }
    foreach ($rule in $rules) {
        if ($rule.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow) { throw 'Unsupported install ACL entry' }
        $who = $rule.IdentityReference.Value
        $mask = [int64]$rule.FileSystemRights -band 0xffffffffL
        $inheritOnly = ($rule.PropagationFlags -band [System.Security.AccessControl.PropagationFlags]::InheritOnly) -ne 0
        if ($who -eq $sid) {
            $full = ($mask -band 0x001f01ff) -eq 0x001f01ff -or ($mask -band 0x10000000) -ne 0
            if ($full -and -not $inheritOnly) { $ownAccess = $true }
            if ($full -and $rule.InheritanceFlags -eq [System.Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit' -and
                ($rule.PropagationFlags -band [System.Security.AccessControl.PropagationFlags]::NoPropagateInherit) -eq 0) { $inheritance = $true }
        } elseif ($who -in $trusted -or (-not $Private -and $who -eq $trustedInstaller)) {
            continue
        } elseif (-not $Private -and ($inheritOnly -or ($mask -band (0xffffffffL -bxor 0xa01200adL)) -eq 0)) {
            continue
        } elseif ($who -eq 'S-1-3-0' -and $inheritOnly) {
            continue
        } else { throw 'Install ACL grants another account access' }
    }
    if ($Private -and (-not $ownAccess -or -not $inheritance)) { throw 'Install parent is not private and inheritable' }
}

function Assert-Ancestry([string]$Path, [bool]$Private) {
    Assert-LocalPath $Path
    $built = $Path.Substring(0, 3)
    Assert-Acl $built $false
    foreach ($part in ($Path.Substring(3) -split '[\\/]')) {
        $built = Join-Path $built $part
        Assert-Acl $built $false
    }
    if ($Private) { Assert-Acl $Path $true }
}

function Seal-NewDirectory([string]$Path) {
    # Called only for this invocation's exclusively created empty directory.
    $acl = [System.Security.AccessControl.DirectorySecurity]::new()
    $acl.SetOwner([System.Security.Principal.SecurityIdentifier]::new($sid))
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($who in $trusted) {
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new(
            [System.Security.Principal.SecurityIdentifier]::new($who), 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
    Assert-Ancestry $Path $true
}

function Seal-NewFile([string]$Path) {
    # Inheritance does not fix an administrator token's default group owner.
    # This is used only after an exclusive creation inside our private stage.
    $acl = [System.Security.AccessControl.FileSecurity]::new()
    $acl.SetOwner([System.Security.Principal.SecurityIdentifier]::new($sid))
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($who in $trusted) {
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new(
            [System.Security.Principal.SecurityIdentifier]::new($who), 'FullControl', 'Allow'))
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
}

function Create-PrivateDirectory([string]$Path) {
    if ([System.IO.Directory]::Exists($Path)) { return }
    Create-PrivateDirectory ([System.IO.Path]::GetDirectoryName($Path))
    New-Item -ItemType Directory -Path $Path -ErrorAction Stop | Out-Null
    Seal-NewDirectory $Path
}

function Download([string]$Url, [string]$Path, [int64]$Limit) {
    $uri = [Uri]$Url
    if ($uri.Scheme -eq 'file' -and -not $uri.IsUnc) {
        $inputStream = [System.IO.File]::OpenRead($uri.LocalPath)
        $response = $null
        $client = $null
    } elseif ($uri.Scheme -eq 'https' -and $uri.UserInfo -eq '' -and $uri.Fragment -eq '') {
        $client = [System.Net.Http.HttpClient]::new()
        $client.Timeout = [TimeSpan]::FromSeconds(120)
        $response = $client.GetAsync($uri, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
        $response.EnsureSuccessStatusCode() | Out-Null
        if ($response.RequestMessage.RequestUri.Scheme -ne 'https') { throw 'Download left HTTPS' }
        $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
    } else { throw 'Use an HTTPS release URL or a local candidate directory URL' }
    $output = $null
    try {
        $output = [System.IO.File]::Open($Path, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        $buffer = New-Object byte[] 65536
        $total = 0L
        $deadline = [System.Diagnostics.Stopwatch]::StartNew()
        while ($true) {
            $remaining = 120000 - $deadline.ElapsedMilliseconds
            if ($remaining -le 0) { throw 'Download exceeded its time bound' }
            $read = $inputStream.ReadAsync($buffer, 0, $buffer.Length)
            if (-not $read.Wait([int]$remaining)) { throw 'Download exceeded its time bound' }
            $count = $read.GetAwaiter().GetResult()
            if ($count -eq 0) { break }
            $total += $count
            if ($total -gt $Limit) { throw 'Download exceeds its size bound' }
            $output.Write($buffer, 0, $count)
        }
        $output.Flush($true)
    } finally {
        if ($null -ne $output) { $output.Dispose() }
        $inputStream.Dispose()
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $client) { $client.Dispose() }
    }
    Seal-NewFile $Path
}

function Assert-Pe([string]$Path, [int]$Machine) {
    $stream = [System.IO.File]::OpenRead($Path)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'Client executable is not PE' }
        $stream.Position = 0x3c
        $offset = $reader.ReadUInt32()
        if ($offset -gt ($stream.Length - 6)) { throw 'Invalid PE header' }
        $stream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne $Machine) { throw 'Client executable architecture differs from the native host' }
    } finally { $reader.Dispose(); $stream.Dispose() }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Net.Http
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
switch ($architecture) {
    'X64' { $target = 'windows-x86_64'; $machine = 0x8664 }
    'Arm64' { $target = 'windows-arm64'; $machine = 0xaa64 }
    default { throw 'Only native Windows x64 and arm64 clients are candidates' }
}
Assert-LocalPath $InstallRoot
if ($BinDirectory -eq '') { $BinDirectory = Join-Path $InstallRoot 'bin' }
Assert-LocalPath $BinDirectory
Assert-Ancestry $env:LOCALAPPDATA $true
$scratch = Join-Path $env:LOCALAPPDATA ('.synveda-download-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch -ErrorAction Stop | Out-Null
Seal-NewDirectory $scratch
try {
    $asset = "synveda-client-$Version-$target.zip"
    $archive = Join-Path $scratch $asset
    $checksums = Join-Path $scratch 'SHA256SUMS'
    Download ($BaseUrl.TrimEnd('/') + '/SHA256SUMS') $checksums 1048576
    Download ($BaseUrl.TrimEnd('/') + '/' + $asset) $archive 268435456
    $checksumLines = @([System.IO.File]::ReadAllLines($checksums) | Where-Object { $_ -cmatch ('^[0-9a-fA-F]{64} [ *]' + [Regex]::Escape($asset) + '$') })
    if ($checksumLines.Count -ne 1 -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ine $checksumLines[0].Substring(0, 64)) { throw 'Client archive checksum missing, duplicate or mismatched' }
    $zip = [System.IO.Compression.ZipFile]::OpenRead($archive)
    try {
        if ($zip.Entries.Count -gt 1024) { throw 'Archive has too many entries' }
        $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        $total = 0L
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName
            $parts = $name.TrimEnd('/') -split '/'
            $kind = ([int64]$entry.ExternalAttributes -shr 16) -band 0xf000
            if ($name.Contains('\') -or $name.Length -gt 512 -or $parts[0] -cne 'client' -or
                -not $seen.Add($name.TrimEnd('/')) -or $kind -notin @(0, 0x4000, 0x8000) -or
                ($entry.ExternalAttributes -band 0x400) -ne 0) { throw 'Archive contains a link, duplicate or foreign path' }
            foreach ($part in $parts) {
                if ($part -cnotmatch '^[A-Za-z0-9@_.-]+$' -or $part -in @('.', '..') -or $part.EndsWith('.') -or
                    ($part -split '\.')[0] -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$') { throw 'Unsafe archive component' }
            }
            $total += $entry.Length
            if ($entry.Length -gt 268435456 -or $total -gt 536870912) { throw 'Expanded archive exceeds its bound' }
        }
        foreach ($entry in $zip.Entries) {
            # A ZIP directory marker is metadata, not an empty path component.
            $path = Join-Path $scratch $entry.FullName.TrimEnd('/')
            if ($entry.FullName.EndsWith('/')) { Create-PrivateDirectory $path; continue }
            Create-PrivateDirectory ([System.IO.Path]::GetDirectoryName($path))
            $inputStream = $entry.Open()
            $output = [System.IO.File]::Open($path, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
            try {
                $buffer = New-Object byte[] 65536
                $written = 0L
                while (($count = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $written += $count
                    if ($written -gt $entry.Length) { throw 'ZIP entry exceeds its declared size' }
                    $output.Write($buffer, 0, $count)
                }
                if ($written -ne $entry.Length) { throw 'ZIP entry is truncated' }
                $output.Flush($true)
            } finally { $output.Dispose(); $inputStream.Dispose() }
            Seal-NewFile $path
        }
    } finally { $zip.Dispose() }
    $source = Join-Path $scratch 'client'
    $node = Join-Path $source 'plugin/synveda/runtime/node.exe'
    Assert-Pe $node $machine
    Assert-Pe (Join-Path $source 'bin/synveda.exe') $machine
    $env:NODE_OPTIONS = ''
    $env:NODE_PATH = ''
    & $node (Join-Path $source 'lib/client-install.mjs') $InstallRoot $BinDirectory $Version $target
    if ($LASTEXITCODE -ne 0) { throw 'Client installation refused; prior installation retained' }
} finally {
    # This random, private, invocation-owned directory contains downloads only.
    Remove-Item -LiteralPath $scratch -Recurse -Force
}
