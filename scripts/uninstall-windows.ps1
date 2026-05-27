#Requires -Version 5.1
<#
.SYNOPSIS
    Uninstall a Plexi build on Windows.

.DESCRIPTION
    Removes the plexi.exe binary, prunes its bin directory from the user PATH,
    and strips the channel's sentinel-tagged completions block from the current
    user's PowerShell $PROFILE.

    The install dir (%LOCALAPPDATA%\Plexi[-channel]\) holds the binary and the
    bundled Python runtime. The profile dir (~\.plexi[-channel]\) holds config,
    logs, installed apps, and secrets — these are separate trees. By default the
    profile dir is KEPT; pass -PurgeProfile to remove the install dir AND the
    profile dir.

    Per-user only — no Administrator rights required. Mirror of
    install-windows.ps1.

.PARAMETER Channel
    Channel suffix to uninstall (alpha, beta, pr-1234). Omit for the stable
    `plexi.exe` install.

.PARAMETER All
    Uninstall every channel found under %LOCALAPPDATA%\Plexi*\. Prompts for
    confirmation before proceeding.

.PARAMETER PurgeProfile
    Also delete the profile directory ~\.plexi[-channel]\ (config, logs,
    installed apps, secrets) along with the install dir. Default keeps the
    profile.

.PARAMETER NoPath
    Skip the PATH cleanup step.

.PARAMETER NoCompletions
    Skip the $PROFILE completions-block cleanup step.

.EXAMPLE
    # Uninstall the stable channel (keeps profile dir).
    .\scripts\uninstall-windows.ps1

.EXAMPLE
    # Uninstall the alpha channel.
    .\scripts\uninstall-windows.ps1 -Channel alpha

.EXAMPLE
    # Uninstall the alpha channel AND wipe its profile dir.
    .\scripts\uninstall-windows.ps1 -Channel alpha -PurgeProfile

.EXAMPLE
    # Uninstall every channel found.
    .\scripts\uninstall-windows.ps1 -All
#>

[CmdletBinding()]
param(
    [string]$Channel = '',
    [switch]$All,
    [switch]$PurgeProfile,
    [switch]$NoPath,
    [switch]$NoCompletions
)

$ErrorActionPreference = 'Stop'

function Remove-PlexiChannel {
    param(
        [string]$ChannelSuffix,
        [switch]$Purge,
        [switch]$SkipPath,
        [switch]$SkipCompletions
    )

    $installRoot = Join-Path $env:LOCALAPPDATA 'Plexi'
    $binaryName  = 'plexi.exe'
    if ($ChannelSuffix) {
        $installRoot = "$installRoot-$ChannelSuffix"
        $binaryName  = "plexi-$ChannelSuffix.exe"
    }
    $binDir = Join-Path $installRoot 'bin'
    $dest   = Join-Path $binDir $binaryName

    # Profile dir is a separate tree from the install dir — it lives under the
    # home dir with a dot prefix (matches config_dir() in the Rust host).
    $profileName = if ($ChannelSuffix) { ".plexi-$ChannelSuffix" } else { ".plexi" }
    $profileDir  = Join-Path $env:USERPROFILE $profileName

    $removed = $false

    if (Test-Path $dest) {
        Remove-Item -Path $dest -Force
        Write-Output "Removed binary: $dest"
        $removed = $true
    }

    if (-not $SkipPath) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($userPath) {
            $segments = $userPath -split ';' | Where-Object { $_ -ne $binDir -and $_ -ne '' }
            $newPath  = $segments -join ';'
            if ($newPath -ne $userPath) {
                [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
                Write-Output "Removed from user PATH: $binDir"
                $removed = $true
            }
        }
    }

    if (-not $SkipCompletions) {
        $profileFile = $PROFILE.CurrentUserAllHosts
        if (Test-Path $profileFile) {
            $existing = Get-Content $profileFile -Raw
            if ($null -eq $existing) { $existing = '' }
            $beginTag = "# >>> plexi completions ($binaryName) >>>"
            $endTag   = "# <<< plexi completions ($binaryName) <<<"
            $pattern  = [Regex]::Escape($beginTag) + '[\s\S]*?' + [Regex]::Escape($endTag) + '\r?\n?'
            if ($existing -match $pattern) {
                $updated = [Regex]::Replace($existing, $pattern, '')
                Set-Content -Path $profileFile -Value $updated -NoNewline
                Write-Output "Removed completions block from $profileFile"
                $removed = $true
            }
        }
    }

    if ($Purge) {
        if (Test-Path $installRoot) {
            Remove-Item -Path $installRoot -Recurse -Force
            Write-Output "Removed install dir: $installRoot"
            $removed = $true
        }
        if (Test-Path $profileDir) {
            Remove-Item -Path $profileDir -Recurse -Force
            Write-Output "Removed profile dir: $profileDir"
            $removed = $true
        }
    } elseif (Test-Path $profileDir) {
        Write-Output "Profile dir kept: $profileDir"
        Write-Output "  (config, logs, apps, secrets preserved; pass -PurgeProfile to remove)"
    }

    return $removed
}

if ($All) {
    $plexiDirs = Get-ChildItem -Path $env:LOCALAPPDATA -Directory -Filter 'Plexi*' -ErrorAction SilentlyContinue
    if (-not $plexiDirs) {
        Write-Output "No Plexi installations found under $env:LOCALAPPDATA."
        return
    }

    Write-Output ''
    Write-Output 'This will uninstall every Plexi channel found:'
    foreach ($d in $plexiDirs) { Write-Output "  - $($d.FullName)" }
    if ($PurgeProfile) {
        Write-Output ''
        Write-Output 'Profile directories WILL be removed (-PurgeProfile).'
    } else {
        Write-Output ''
        Write-Output 'Profile directories will be kept (pass -PurgeProfile to remove).'
    }
    Write-Output ''
    $confirm = Read-Host 'Proceed? [y/N]'
    if ($confirm -notmatch '^[Yy]') {
        Write-Output 'Aborted.'
        return
    }
    Write-Output ''

    $anyRemoved = $false
    foreach ($d in $plexiDirs) {
        # Map directory name back to channel suffix:
        #   Plexi        -> ''
        #   Plexi-alpha  -> 'alpha'
        #   Plexi-pr-783 -> 'pr-783'
        $suffix = if ($d.Name -eq 'Plexi') { '' } else { $d.Name -replace '^Plexi-','' }
        $label  = if ($suffix) { $suffix } else { 'stable' }
        Write-Output "-- Channel: $label"
        $r = Remove-PlexiChannel -ChannelSuffix $suffix -Purge:$PurgeProfile -SkipPath:$NoPath -SkipCompletions:$NoCompletions
        if ($r) { $anyRemoved = $true }
    }

    Write-Output ''
    if ($anyRemoved) {
        Write-Output 'Done. Open a new terminal to pick up PATH / completion changes.'
    } else {
        Write-Output 'Nothing to remove.'
    }
    return
}

$removed = Remove-PlexiChannel -ChannelSuffix $Channel -Purge:$PurgeProfile -SkipPath:$NoPath -SkipCompletions:$NoCompletions
Write-Output ''
if ($removed) {
    Write-Output 'Done. Open a new terminal to pick up PATH / completion changes.'
} else {
    Write-Output 'Nothing to remove.'
}
