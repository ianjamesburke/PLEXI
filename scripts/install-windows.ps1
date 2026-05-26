#Requires -Version 5.1
<#
.SYNOPSIS
    Install a Plexi build on Windows.

.DESCRIPTION
    Copies a built plexi.exe (alpha / beta / stable / pr-build) into
    %LOCALAPPDATA%\Plexi\bin\, ensures that directory is on the user PATH,
    and registers PowerShell completions in the current user's $PROFILE.

    Multiple channels coexist — alpha goes to plexi-alpha.exe, etc. —
    so you can run them side-by-side. The default install (no -Channel arg)
    installs as `plexi.exe` (stable).

    This script does NOT require Administrator rights. All changes are
    HKCU / per-user.

.PARAMETER Source
    Path to the plexi.exe binary to install. Defaults to
    `target\release\plexi.exe` relative to the repo root.

.PARAMETER Channel
    Optional channel suffix (alpha, beta, pr-1234). Stored as
    plexi-<channel>.exe with its own %LOCALAPPDATA%\Plexi-<channel>\ profile.
    Omit for the stable `plexi.exe` install.

.PARAMETER NoCompletions
    Skip the PowerShell-completions registration step. PATH and copy still run.

.PARAMETER NoPath
    Skip the PATH update. Copy + completions still run.

.EXAMPLE
    # From the repo root, after `cargo build --release --bin plexi`:
    .\scripts\install-windows.ps1

.EXAMPLE
    # Install an alpha build side-by-side with stable.
    .\scripts\install-windows.ps1 -Source .\target\debug\plexi.exe -Channel alpha
#>

[CmdletBinding()]
param(
    [string]$Source = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\plexi.exe'),
    [string]$Channel = '',
    [switch]$NoCompletions,
    [switch]$NoPath
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Source)) {
    Write-Error "Source binary not found: $Source`nBuild first with 'cargo build --release --bin plexi'."
}

$installRoot = Join-Path $env:LOCALAPPDATA 'Plexi'
$binaryName  = 'plexi.exe'
if ($Channel) {
    $installRoot = "$installRoot-$Channel"
    $binaryName  = "plexi-$Channel.exe"
}
$binDir = Join-Path $installRoot 'bin'
$dest   = Join-Path $binDir $binaryName

New-Item -ItemType Directory -Path $binDir -Force | Out-Null
Copy-Item -Path $Source -Destination $dest -Force
Write-Output "Installed: $dest"

if (-not $NoPath) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $segments = if ($userPath) { $userPath -split ';' } else { @() }
    if ($segments -notcontains $binDir) {
        $newPath = if ($userPath) { "$userPath;$binDir" } else { $binDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Output "Added to user PATH: $binDir"
        Write-Output "  (open a new terminal to pick up the change)"
    } else {
        Write-Output "User PATH already contains $binDir"
    }
}

if (-not $NoCompletions) {
    $profileDir = Split-Path -Parent $PROFILE.CurrentUserAllHosts
    if (-not (Test-Path $profileDir)) {
        New-Item -ItemType Directory -Path $profileDir -Force | Out-Null
    }
    $profileFile = $PROFILE.CurrentUserAllHosts
    if (-not (Test-Path $profileFile)) {
        New-Item -ItemType File -Path $profileFile -Force | Out-Null
    }

    # Sentinel-tagged block so re-installs replace atomically; non-greedy so
    # other channels' blocks survive untouched. Pin to the absolute installed
    # path ($dest) so a same-named binary earlier on PATH can't intercept
    # completion generation at shell startup.
    $beginTag = "# >>> plexi completions ($binaryName) >>>"
    $endTag   = "# <<< plexi completions ($binaryName) <<<"
    $escapedDest = $dest.Replace("'", "''")
    # Plexi-pane cwd reporting: prompt hook writes $PWD to the sidecar. Inert outside
    # Plexi (gated on PLEXI_PANE_CWD_FILE). Single-quoted here-string: NOT interpolated.
    # Keep in sync with installer\plexi-completions-helper.ps1.
    $cwdHook = @'
if ($env:PLEXI_PANE_CWD_FILE -and -not $global:__PlexiCwdHooked) {
    $global:__PlexiCwdHooked = $true
    $global:__PlexiOrigPrompt = $function:prompt
    function global:prompt {
        try { [System.IO.File]::WriteAllText($env:PLEXI_PANE_CWD_FILE, $PWD.ProviderPath) } catch { }
        if ($global:__PlexiOrigPrompt) { & $global:__PlexiOrigPrompt } else { "PS $($PWD.Path)> " }
    }
}
'@
    $block = @"
$beginTag
if (Test-Path '$escapedDest') {
    & '$escapedDest' completions powershell | Out-String | Invoke-Expression
}
$cwdHook
$endTag
"@

    $existing = Get-Content $profileFile -Raw -ErrorAction SilentlyContinue
    if ($null -eq $existing) { $existing = '' }
    $pattern = [Regex]::Escape($beginTag) + '[\s\S]*?' + [Regex]::Escape($endTag) + '\r?\n?'
    $stripped = [Regex]::Replace($existing, $pattern, '')
    $appendNewline = if ($stripped.EndsWith("`n") -or [string]::IsNullOrEmpty($stripped)) { '' } else { "`n" }
    $updated = "$stripped$appendNewline$block`n"
    Set-Content -Path $profileFile -Value $updated -NoNewline
    if ($existing -match $pattern) {
        Write-Output "Updated completions block in $profileFile"
    } else {
        Write-Output "Registered completions in $profileFile"
    }
    Write-Output "  (open a new PowerShell session, or run `. `$PROFILE`, to activate)"
}

Write-Output ''
Write-Output 'Done. Open a new terminal and run:'
Write-Output "  $binaryName --help"
