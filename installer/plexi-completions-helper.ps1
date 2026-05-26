#Requires -Version 5.1
<#
.SYNOPSIS
    Register or unregister Plexi's PowerShell tab-completions in $PROFILE.

.DESCRIPTION
    Invoked by the Inno Setup installer (plexi.iss) from [Run] / [UninstallRun]
    to manage the sentinel-tagged completions block in
    $PROFILE.CurrentUserAllHosts. Factored out into its own script so the
    .iss avoids inline PowerShell quoting headaches.

    The sentinel-block format matches scripts\install-windows.ps1 so the two
    install paths (manual script vs. Inno installer) produce identical output.

.PARAMETER Action
    register   — insert (or replace) the completions block.
    unregister — strip the completions block.

.PARAMETER BinaryName
    The plexi binary filename (e.g. plexi.exe, plexi-alpha.exe). The sentinel
    tags include this name so different channels have independent blocks.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('register', 'unregister')]
    [string]$Action,

    [Parameter(Mandatory)]
    [string]$BinaryName,

    # Absolute path to the installed binary. Used in the generated profile
    # block so completion-generation runs against the install we just made,
    # not whatever same-named binary happens to be earlier on PATH.
    [Parameter()]
    [string]$BinaryPath
)

$ErrorActionPreference = 'Stop'

$profileFile = $PROFILE.CurrentUserAllHosts
$profileDir  = Split-Path -Parent $profileFile

if ($Action -eq 'register') {
    if (-not (Test-Path $profileDir)) {
        New-Item -ItemType Directory -Path $profileDir -Force | Out-Null
    }
    if (-not (Test-Path $profileFile)) {
        New-Item -ItemType File -Path $profileFile -Force | Out-Null
    }
}

if (-not (Test-Path $profileFile)) {
    # Nothing to unregister.
    return
}

$beginTag = "# >>> plexi completions ($BinaryName) >>>"
$endTag   = "# <<< plexi completions ($BinaryName) <<<"
$pattern  = [Regex]::Escape($beginTag) + '[\s\S]*?' + [Regex]::Escape($endTag) + '\r?\n?'

$existing = Get-Content $profileFile -Raw
if ($null -eq $existing) { $existing = '' }
$stripped = [Regex]::Replace($existing, $pattern, '')

if ($Action -eq 'unregister') {
    if ($existing -ne $stripped) {
        Set-Content -Path $profileFile -Value $stripped -NoNewline
        Write-Output "Removed completions block from $profileFile"
    }
    return
}

# Plexi-pane cwd reporting: prompt hook writes $PWD to the sidecar so the host can
# read the shell's location. Inert outside Plexi (gated on PLEXI_PANE_CWD_FILE).
# Single-quoted here-string: emitted literally, NOT interpolated here.
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

if (-not [string]::IsNullOrEmpty($BinaryPath)) {
    $escapedPath = $BinaryPath.Replace("'", "''")
    $block = @"
$beginTag
if (Test-Path '$escapedPath') {
    & '$escapedPath' completions powershell | Out-String | Invoke-Expression
}
$cwdHook
$endTag
"@
} else {
    # Fallback for callers that don't supply -BinaryPath (older installers).
    $block = @"
$beginTag
if (Get-Command $BinaryName -ErrorAction SilentlyContinue) {
    $BinaryName completions powershell | Out-String | Invoke-Expression
}
$cwdHook
$endTag
"@
}

$tail = if ($stripped.EndsWith("`n") -or [string]::IsNullOrEmpty($stripped)) { '' } else { "`n" }
Set-Content -Path $profileFile -Value "$stripped$tail$block`n" -NoNewline
Write-Output "Registered completions in $profileFile"
