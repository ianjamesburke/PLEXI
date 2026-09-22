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
  } else {
    $match = $releases | Where-Object { $_.tag_name -match ("^v\d+\.\d+\.\d+-" + [regex]::Escape($ChannelName) + "\.\d+$") } | Select-Object -First 1
  }
  if (-not $match) { throw "No published $ChannelName release found" }
  return $match.tag_name
}

if (-not $Tag) {
  $Tag = Get-LatestTag $Channel
}

$Asset = 'plexi-windows-x64.zip'
$Url = "$ReleaseBase/$Tag/$Asset"
$ChecksumUrl = "$Url.sha256"
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
  $checksumFile = Join-Path $tmp "$Asset.sha256"
  Invoke-WebRequest -UseBasicParsing -Uri $ChecksumUrl -OutFile $checksumFile
  $checksumLine = Get-Content -Path $checksumFile | Where-Object { $_ -match "^([A-Fa-f0-9]{64})\\s+\\*?$([regex]::Escape($Asset))$" } | Select-Object -First 1
  if (-not $checksumLine) { throw "Release checksum file did not contain a SHA-256 for $Asset" }
  $expectedChecksum = ([regex]::Match($checksumLine, '^[A-Fa-f0-9]{64}')).Value.ToLowerInvariant()
  $actualChecksum = (Get-FileHash -Algorithm SHA256 -Path $archive).Hash.ToLowerInvariant()
  if ($actualChecksum -ne $expectedChecksum) { throw "SHA-256 mismatch for $Asset; refusing to install" }
  $unpack = Join-Path $tmp 'unpack'
  New-Item -ItemType Directory -Force -Path $unpack | Out-Null
  Expand-Archive -Force -Path $archive -DestinationPath $unpack
  $source = Get-ChildItem -Path $unpack -Filter 'plexi.exe' -Recurse -File | Select-Object -First 1
  if (-not $source) { throw "Release asset did not contain plexi.exe" }

  New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
  New-Item -ItemType Directory -Force -Path $ProfileDir | Out-Null
  # Stage the payload and command before replacing either live path. If any
  # activation step fails, restore the previous install and its success marker.
  $stagedDestination = Join-Path $tmp 'staged-payload'
  Copy-Item -Recurse -Force $unpack $stagedDestination
  $stagedBinary = Join-Path $tmp $BinaryName
  Copy-Item -Force $source.FullName $stagedBinary
  $installedBinary = Join-Path $BinDir $BinaryName
  $previousDestination = Join-Path $tmp 'previous-payload'
  $previousBinary = Join-Path $tmp 'previous-binary'
  $tagPath = Join-Path $ProfileDir 'installed_tag'
  $previousTag = Join-Path $tmp 'previous-installed_tag'
  $hadDestination = Test-Path $Destination
  $hadBinary = Test-Path $installedBinary
  $hadTag = Test-Path $tagPath
  if ($hadTag) { Copy-Item -Force $tagPath $previousTag }
  $destinationReplacementStarted = $false
  $binaryReplacementStarted = $false
  try {
    if ($hadDestination) { Move-Item -Force $Destination $previousDestination }
    $destinationReplacementStarted = $true
    Move-Item -Force $stagedDestination $Destination
    if ($hadBinary) { Move-Item -Force $installedBinary $previousBinary }
    $binaryReplacementStarted = $true
    Move-Item -Force $stagedBinary $installedBinary
    Set-Content -Path (Join-Path $ProfileDir 'installed_tag.new') -Value $Tag -NoNewline
    Move-Item -Force (Join-Path $ProfileDir 'installed_tag.new') $tagPath
  } catch {
    if ($destinationReplacementStarted -and (Test-Path $Destination)) { Remove-Item -Recurse -Force $Destination }
    if ($hadDestination -and (Test-Path $previousDestination)) { Move-Item -Force $previousDestination $Destination }
    if ($binaryReplacementStarted -and (Test-Path $installedBinary)) { Remove-Item -Force $installedBinary }
    if ($hadBinary -and (Test-Path $previousBinary)) { Move-Item -Force $previousBinary $installedBinary }
    if ($hadTag) { Copy-Item -Force $previousTag $tagPath } elseif (Test-Path $tagPath) { Remove-Item -Force $tagPath }
    throw
  }

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
