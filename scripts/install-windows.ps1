#Requires -Version 5.1
<#
.SYNOPSIS
  Install a prebuilt Plexi Windows x64 release (no Rust / no Visual Studio).

.EXAMPLE
  # One-liner from a published tag (dogfood):
  irm https://raw.githubusercontent.com/ianjamesburke/PLEXI/v0.3.1-windows.1/scripts/install-windows.ps1 | iex

  # Explicit channel + tag:
  .\scripts\install-windows.ps1 -Channel alpha -Tag v0.3.1-windows.1
#>
[CmdletBinding()]
param(
  [ValidateSet('main', 'alpha', 'beta')]
  [string]$Channel = 'alpha',

  [string]$Tag = '',

  [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$RepoSlug = 'ianjamesburke/PLEXI'
$ReleaseBase = if ($env:PLEXI_RELEASE_BASE_URL) { $env:PLEXI_RELEASE_BASE_URL.TrimEnd('/') } else { "https://github.com/$RepoSlug/releases/download" }
$InstallRoot = if ($env:PLEXI_INSTALL_DIR) { $env:PLEXI_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Plexi' }
$BinDir = if ($env:PLEXI_BIN_DIR) { $env:PLEXI_BIN_DIR } else { Join-Path $InstallRoot 'bin' }

function Get-LatestTag([string]$ChannelName) {
  $headers = @{ 'User-Agent' = 'plexi-installer'; Accept = 'application/vnd.github+json' }
  $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$RepoSlug/releases" -Headers $headers
  if ($ChannelName -eq 'main') {
    $match = $releases | Where-Object { $_.tag_name -match '^v\d+\.\d+\.\d+$' } | Select-Object -First 1
  } elseif ($ChannelName -eq 'alpha') {
    # Prefer disposable windows dogfood tags when present; else newest alpha.
    $win = $releases | Where-Object { $_.tag_name -match '^v\d+\.\d+\.\d+-windows\.\d+$' } | Select-Object -First 1
    if ($win) { return $win.tag_name }
    $match = $releases | Where-Object { $_.tag_name -match '^v\d+\.\d+\.\d+-alpha\.\d+$' } | Select-Object -First 1
  } else {
    $match = $releases | Where-Object { $_.tag_name -match '^v\d+\.\d+\.\d+-beta\.\d+$' } | Select-Object -First 1
  }
  if (-not $match) { throw "No published $ChannelName release found" }
  return $match.tag_name
}

if (-not $Tag) {
  $Tag = Get-LatestTag $Channel
}

$Asset = 'plexi-windows-x64.zip'
$Url = "$ReleaseBase/$Tag/$Asset"
$BinaryName = if ($Channel -eq 'main') { 'plexi.exe' } else { "plexi-$Channel.exe" }
$Destination = Join-Path $InstallRoot $Channel
$ProfileSuffix = if ($Channel -eq 'main') { '' } else { "-$Channel" }
$ProfileDir = Join-Path $env:USERPROFILE ".plexi$ProfileSuffix"

Write-Host "Plexi $Tag ($Channel, windows/x64)"
Write-Host "Asset: $Url"
Write-Host "Install: $Destination; command: $(Join-Path $BinDir $BinaryName)"
if ($DryRun) { exit 0 }

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("plexi-install-" + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  $archive = Join-Path $tmp $Asset
  Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $archive
  $unpack = Join-Path $tmp 'unpack'
  New-Item -ItemType Directory -Force -Path $unpack | Out-Null
  Expand-Archive -Force -Path $archive -DestinationPath $unpack
  $source = Get-ChildItem -Path $unpack -Filter 'plexi.exe' -Recurse -File | Select-Object -First 1
  if (-not $source) { throw "Release asset did not contain plexi.exe" }

  if (Test-Path $Destination) { Remove-Item -Recurse -Force $Destination }
  New-Item -ItemType Directory -Force -Path $Destination | Out-Null
  Copy-Item -Recurse -Force (Join-Path $unpack '*') $Destination

  New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
  Copy-Item -Force $source.FullName (Join-Path $BinDir $BinaryName)

  New-Item -ItemType Directory -Force -Path $ProfileDir | Out-Null
  Set-Content -Path (Join-Path $ProfileDir 'installed_tag') -Value $Tag -NoNewline

  # Persist BinDir on the user PATH when missing.
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not $userPath) { $userPath = '' }
  $parts = $userPath -split ';' | Where-Object { $_ -and $_.Trim() -ne '' }
  if ($parts -notcontains $BinDir) {
    $newPath = if ($userPath.Trim() -eq '') { $BinDir } else { "$userPath;$BinDir" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    $env:Path = "$BinDir;$env:Path"
    Write-Host "Added $BinDir to your user PATH (new shells pick this up automatically)."
  }

  $installed = Join-Path $BinDir $BinaryName
  Write-Host "Installed $installed"
  & $installed --version
  Write-Host ("installed_tag=" + (Get-Content (Join-Path $ProfileDir 'installed_tag') -Raw))
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
