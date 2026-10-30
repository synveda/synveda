# Synveda client installer for Windows. No elevation or execution-policy change.
#
#   powershell -NoProfile -Command "irm https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.ps1 | iex"
#
# Installs the latest release unless -Version is given. Before running any
# downloaded code it verifies with the GitHub CLI that the checksum inventory
# was signed by this repository's release workflow for that tag. Without
# -GhPath or a current gh on PATH, it fetches a hash-pinned GitHub CLI for
# this run only. Then it adds
# the launcher directory to the user PATH and sets the user SYNVEDA_CLI that
# agent hooks use, unless -NoModifyEnvironment is given. Parameters default to
# the SYNVEDA_* environment variables install.sh reads, so the one-line form
# above is configured by setting them first.
[CmdletBinding()]
param(
    [string]$Version = $env:SYNVEDA_VERSION,
    [string]$SourceSha = $env:SYNVEDA_SOURCE_SHA,
    [string]$BaseUrl = $env:SYNVEDA_BASE_URL,
    [string]$InstallRoot = $(if ($env:SYNVEDA_HOME) { $env:SYNVEDA_HOME } else { "$env:LOCALAPPDATA\SynvedaClient" }),
    [string]$BinDirectory = $env:SYNVEDA_BIN,
    [string]$GhPath = $env:SYNVEDA_GH,
    [switch]$NoModifyEnvironment
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($PSVersionTable.PSVersion.Major -lt 5) { throw 'Windows PowerShell 5.1 or PowerShell 7 is required' }
if ($env:OS -ne 'Windows_NT') { throw 'Native Windows is required' }
if ($env:SYNVEDA_NO_MODIFY_PATH -eq '1') { $NoModifyEnvironment = $true }
# Authenticated enterprise proxies challenge for the signed-in Windows account.
if ($PSVersionTable.PSEdition -eq 'Core') {
    [System.Net.Http.HttpClient]::DefaultProxy.Credentials = [System.Net.CredentialCache]::DefaultCredentials
} elseif ($null -ne [System.Net.WebRequest]::DefaultWebProxy) {
    [System.Net.WebRequest]::DefaultWebProxy.Credentials = [System.Net.CredentialCache]::DefaultNetworkCredentials
}
$mirror = $BaseUrl -ne ''
if ($Version -eq '') {
    if ($mirror) { throw 'BaseUrl requires -Version; a directory of assets cannot be asked which release is the latest' }
    $latest = Invoke-RestMethod -Uri 'https://api.github.com/repos/synveda/synveda/releases/latest' -UseBasicParsing -TimeoutSec 60
    $Version = [string]$latest.tag_name
}
$Version = $Version -creplace '^v', ''
if ($Version.Length -gt 63 -or $Version -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$') { throw 'Invalid release version' }
if (-not $mirror) { $BaseUrl = "https://github.com/synveda/synveda/releases/download/v$Version" }
if ($SourceSha -ne '' -and $SourceSha -cnotmatch '^[0-9a-f]{40}$') { throw 'SourceSha must be a 40-character lowercase Git commit' }
$baseUri = [Uri]$BaseUrl
$localCandidate = $baseUri.IsAbsoluteUri -and $BaseUrl.StartsWith('file:///') -and -not $baseUri.IsUnc -and $baseUri.Authority -eq ''
if (-not $localCandidate -and (-not $baseUri.IsAbsoluteUri -or $baseUri.Scheme -ne 'https' -or $baseUri.UserInfo -ne '' -or $baseUri.Fragment -ne '')) {
    throw 'Use an HTTPS release URL or an absolute local file:/// candidate directory'
}
$mirrorCredentials = $null
if ($mirror -and -not $localCandidate) {
    # Windows sign-in for the mirror's host only; redirect targets and every
    # other host receive no credentials.
    $mirrorCredentials = [System.Net.CredentialCache]::new()
    foreach ($scheme in @('Negotiate', 'NTLM')) {
        $mirrorCredentials.Add([Uri]($BaseUrl.TrimEnd('/') + '/'), $scheme, [System.Net.CredentialCache]::DefaultNetworkCredentials)
    }
}
$verifyPublisher = -not $localCandidate -or $SourceSha -ne ''
# Older GitHub CLI releases lack the pinned verification flags or ask for a
# sign-in, so a PATH gh below this version gives way to the pinned copy.
$ghVersion = '2.102.0'
$ghPins = @{
    'windows-x86_64' = @('amd64', 'ae64e556ecc240b200f7eba60d550e4bb60d78e860e69dd88c449405b86067f4')
    'windows-arm64' = @('arm64', '5dcf12aa8525eabd0c46ec414f323ab6cf65229fc2cd46543cc705001bbaf223')
}
$verifier = $null
$verifierEnvironment = @{}
if ($verifyPublisher -and $GhPath -ne '') {
    if ($GhPath -cnotmatch '^[A-Za-z]:\\' -or $GhPath -notmatch '\\gh\.exe$' -or -not [System.IO.File]::Exists($GhPath)) {
        throw 'SYNVEDA_GH or -GhPath must be the full local path of an existing gh.exe'
    }
    $verifier = $GhPath
}
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

function Get-AccountLabel([string]$Sid) {
    try { $name = [System.Security.Principal.SecurityIdentifier]::new($Sid).Translate([System.Security.Principal.NTAccount]).Value }
    catch { $name = 'an unresolved account' }
    return "$name ($Sid)"
}

function Assert-Acl([string]$Path, [bool]$Private) {
    $advice = "Synveda never changes existing permissions. Set SYNVEDA_HOME or -InstallRoot to a folder outside $Path whose parent only you, SYSTEM and Administrators can change."
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or -not $item.PSIsContainer) { throw "$Path is a link or not a folder. $advice" }
    $acl = Get-Acl -LiteralPath $Path
    $owner = $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
    if ($Private -and $owner -ne $sid) { throw "$Path is owned by $(Get-AccountLabel $owner), not by you. $advice" }
    if (-not $Private -and $owner -notin ($trusted + $trustedInstaller)) { throw "$Path is owned by $(Get-AccountLabel $owner), not by you, SYSTEM, Administrators or TrustedInstaller. $advice" }
    $ownAccess = $false
    $inheritance = $false
    $rules = $acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])
    if ($rules.Count -eq 0 -or $rules.Count -gt 128) { throw "$Path has an empty or oversized access list. $advice" }
    foreach ($rule in $rules) {
        if ($rule.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow) { throw "$Path has a deny entry for $(Get-AccountLabel $rule.IdentityReference.Value), which the installer does not evaluate. $advice" }
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
        } elseif ($Private) {
            throw "$Path grants $(Get-AccountLabel $who) access, so it is not private to you, SYSTEM and Administrators. $advice"
        } else {
            throw "$Path lets $(Get-AccountLabel $who) write, delete or change entries, so the installation below it could be replaced. $advice"
        }
    }
    if ($Private -and (-not $ownAccess -or -not $inheritance)) { throw "$Path does not give you full, inheritable access. $advice" }
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
        $handler = [System.Net.Http.HttpClientHandler]::new()
        if ($null -ne $mirrorCredentials) { $handler.Credentials = $mirrorCredentials }
        $client = [System.Net.Http.HttpClient]::new($handler)
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

function Invoke-Bounded([string]$File, [string[]]$Arguments, [hashtable]$Environment, [int]$TimeoutMs, [string]$Activity) {
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo.FileName = $File
    # Private ancestry checks exclude quotes; filenames and policy arguments
    # have no trailing backslash. No shell interprets these quoted arguments.
    $process.StartInfo.Arguments = ($Arguments | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    foreach ($name in $Environment.Keys) { $process.StartInfo.EnvironmentVariables[$name] = $Environment[$name] }
    $started = $false
    try {
        $started = $process.Start()
        if (-not $started) { throw "Could not start $File" }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutMs)) {
            $process.Kill()
            $process.WaitForExit()
            throw "$Activity exceeded $($TimeoutMs / 1000) seconds; installation stopped"
        }
        $output = $stdout.GetAwaiter().GetResult()
        $stderr.GetAwaiter().GetResult() | Out-Null
        return @{ ExitCode = $process.ExitCode; Output = $output }
    } finally {
        if ($started -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
}

function Verify-Publisher([string]$Checksums, [string]$Bundle) {
    $arguments = @('attestation', 'verify', $Checksums, '--bundle', $Bundle,
        '--hostname', 'github.com', '--repo', 'synveda/synveda',
        '--signer-workflow', 'synveda/synveda/.github/workflows/release.yml',
        '--source-ref', "refs/tags/v$Version")
    # The tag and signing workflow are always pinned; a supplied commit narrows it.
    if ($SourceSha -ne '') { $arguments += @('--source-digest', $SourceSha) }
    $arguments += @('--cert-oidc-issuer', 'https://token.actions.githubusercontent.com',
        '--predicate-type', 'https://slsa.dev/provenance/v1', '--deny-self-hosted-runners')
    $result = Invoke-Bounded $verifier $arguments $verifierEnvironment 120000 'Release publisher verification'
    if ($result.ExitCode -ne 0) { throw 'Release publisher verification failed; installation stopped' }
}

function Save-ZipEntry($Entry, [string]$Path) {
    $inputStream = $Entry.Open()
    $output = [System.IO.File]::Open($Path, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
    try {
        $buffer = New-Object byte[] 65536
        $written = 0L
        while (($count = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $written += $count
            if ($written -gt $Entry.Length) { throw 'ZIP entry exceeds its declared size' }
            $output.Write($buffer, 0, $count)
        }
        if ($written -ne $Entry.Length) { throw 'ZIP entry is truncated' }
        $output.Flush($true)
    } finally { $output.Dispose(); $inputStream.Dispose() }
    Seal-NewFile $Path
}

function Install-PinnedGh {
    $name = "gh_${ghVersion}_windows_$($ghPins[$target][0]).zip"
    if ($mirror) { $source = $BaseUrl.TrimEnd('/') + '/' + $name }
    else { $source = "https://github.com/cli/cli/releases/download/v$ghVersion/$name" }
    $archive = Join-Path $scratch $name
    try { Download $source $archive 67108864 }
    catch {
        throw "A trusted GitHub CLI (gh) is required to verify the release publisher, and $name could not be downloaded from $source ($($_.Exception.GetBaseException().Message)). Place it there, or set SYNVEDA_GH to the full path of an existing gh.exe, then rerun this installer."
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ine $ghPins[$target][1]) { throw "$name does not match the GitHub CLI $ghVersion checksum pinned in this installer" }
    $directory = Join-Path $scratch 'gh'
    Create-PrivateDirectory $directory
    $path = Join-Path $directory 'gh.exe'
    $zip = [System.IO.Compression.ZipFile]::OpenRead($archive)
    try {
        $entries = @($zip.Entries | Where-Object { $_.FullName -ceq 'bin/gh.exe' })
        if ($entries.Count -ne 1 -or $entries[0].Length -gt 134217728) { throw "$name does not contain exactly one bounded bin/gh.exe" }
        Save-ZipEntry $entries[0] $path
    } finally { $zip.Dispose() }
    Assert-Pe $path $machine
    return $path
}

function Remove-DownloadDirectory([string]$Path) {
    # Windows may briefly retain a sharing lock after the archived Node exits.
    # Retry only that condition in this invocation's private download directory.
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
            # The provider can replace the native I/O error. The direct API
            # preserves it; unwrap PowerShell's method-invocation exception.
            [System.IO.Directory]::Delete($Path, $true)
            return
        } catch [System.IO.IOException] {
            $code = $_.Exception.GetBaseException().HResult -band 0xffff
            if ($code -notin @(32, 33) -or $attempt -eq 19) { throw }
            Start-Sleep -Milliseconds 250
        }
    }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Net.Http
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
switch ($architecture) {
    'X64' { $target = 'windows-x86_64'; $machine = 0x8664 }
    'Arm64' { $target = 'windows-arm64'; $machine = 0xaa64 }
    default { throw 'Only native Windows x64 and arm64 clients are candidates' }
}
if ($verifyPublisher -and $null -eq $verifier) {
    $command = Get-Command gh -CommandType Application -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        $probe = $null
        $problem = "is older than GitHub CLI $ghVersion or did not report its version"
        try { $probe = Invoke-Bounded $command.Source @('--version') @{ GH_NO_UPDATE_NOTIFIER = '1' } 30000 'The GitHub CLI version check' }
        catch { $problem = "could not report its version ($($_.Exception.GetBaseException().Message))" }
        if ($null -ne $probe -and $probe.ExitCode -eq 0 -and $probe.Output -cmatch '^gh version ([0-9]+\.[0-9]+\.[0-9]+)' -and [version]$Matches[1] -ge [version]$ghVersion) {
            $verifier = $command.Source
        } else {
            Write-Output "$($command.Source) $problem; using a pinned copy instead."
        }
    }
}
Assert-LocalPath $InstallRoot
if ($BinDirectory -eq '') { $BinDirectory = Join-Path $InstallRoot 'bin' }
Assert-LocalPath $BinDirectory
# Downloads are staged beside the installation, so only its parent must be private.
$installParent = [System.IO.Path]::GetDirectoryName($InstallRoot)
if ($installParent.Length -le 3 -or -not [System.IO.Directory]::Exists($installParent)) {
    throw "The folder that will contain $InstallRoot must already exist and must not be a drive root"
}
Assert-Ancestry $installParent $true
$scratch = Join-Path $installParent ('.synveda-download-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch -ErrorAction Stop | Out-Null
Seal-NewDirectory $scratch
try {
    $asset = "synveda-client-$Version-$target.zip"
    $archive = Join-Path $scratch $asset
    $checksums = Join-Path $scratch 'SHA256SUMS'
    if ($verifyPublisher -and $null -eq $verifier) {
        $verifier = Install-PinnedGh
        # The pinned copy leaves no configuration, cache or telemetry identity behind.
        $ghHome = Join-Path $scratch 'gh-home'
        $verifierEnvironment = @{
            GH_CONFIG_DIR = "$ghHome\config"; XDG_CONFIG_HOME = "$ghHome\config"; XDG_CACHE_HOME = "$ghHome\cache"
            XDG_STATE_HOME = "$ghHome\state"; XDG_DATA_HOME = "$ghHome\data"
            GH_NO_UPDATE_NOTIFIER = '1'; GH_PROMPT_DISABLED = '1'; GH_TELEMETRY = 'false'
        }
        Write-Output "Using GitHub CLI $ghVersion for this installation; its archive matched the checksum pinned in this installer."
    }
    Download ($BaseUrl.TrimEnd('/') + '/SHA256SUMS') $checksums 1048576
    if ($verifyPublisher) {
        $bundle = Join-Path $scratch 'SHA256SUMS.sigstore.json'
        Download ($BaseUrl.TrimEnd('/') + '/SHA256SUMS.sigstore.json') $bundle 4194304
        Verify-Publisher $checksums $bundle
        if ($SourceSha -ne '') { Write-Output "Release publisher verified for v$Version at $SourceSha." }
        else { Write-Output "Release publisher verified for v$Version." }
    } else {
        Write-Output 'Local development candidate: publisher identity was not verified; checksums validate local bytes only.'
    }
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
            Save-ZipEntry $entry $path
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
    Remove-DownloadDirectory $scratch
}

$selected = Get-Content -Raw -LiteralPath (Join-Path $InstallRoot 'client\current.json') | ConvertFrom-Json
$cli = Join-Path (Join-Path (Join-Path $InstallRoot 'client') $selected.release) 'bin\synveda.exe'
if ($NoModifyEnvironment) {
    Write-Output "Add $BinDirectory to PATH, and set the user environment variable SYNVEDA_CLI to $cli for agent hooks."
} else {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @(if ($userPath) { $userPath -split ';' | Where-Object { $_ -ne '' } })
    if ($entries -notcontains $BinDirectory) {
        [Environment]::SetEnvironmentVariable('Path', (@($BinDirectory) + $entries) -join ';', 'User')
        Write-Output "Added $BinDirectory to your user PATH."
    }
    [Environment]::SetEnvironmentVariable('SYNVEDA_CLI', $cli, 'User')
    if ($InstallRoot -ne "$env:LOCALAPPDATA\SynvedaClient") {
        [Environment]::SetEnvironmentVariable('SYNVEDA_HOME', $InstallRoot, 'User')
    }
    Write-Output 'Set SYNVEDA_CLI for agent hooks. Open a new terminal to pick up the changes.'
}
if ((Get-ExecutionPolicy) -in @('Restricted', 'AllSigned')) {
    Write-Output 'PowerShell does not run local scripts yet, so the synveda launcher will not start. Allow it with:'
    Write-Output '  Set-ExecutionPolicy -Scope CurrentUser RemoteSigned'
}
