# The native installer owns package validation, channel policy and activation.
[CmdletBinding()]
param(
    [string]$Channel = 'stable',
    [string]$Tag,
    [switch]$InstallOnly,
    [switch]$DryRun,
    [string]$InstallDir = $env:PLEXI_INSTALL_DIR,
    [string]$BinDir = $env:PLEXI_BIN_DIR
)
$ErrorActionPreference = 'Stop'
$base = if ($env:PLEXI_RELEASE_BASE_URL) { $env:PLEXI_RELEASE_BASE_URL } else { 'https://github.com/ianjamesburke/PLEXI/releases/download' }
$bootstrap = $env:PLEXI_BOOTSTRAP_TAG
if (-not $bootstrap) {
    $api = if ($env:PLEXI_RELEASES_URL) { $env:PLEXI_RELEASES_URL } else { 'https://api.github.com/repos/ianjamesburke/PLEXI/releases' }
    $bootstrap = (Invoke-RestMethod -Uri "$api/latest").tag_name
}
if ($bootstrap -notmatch '^v\d+\.\d+\.\d+(-(alpha|beta)\.\d+)?$') { throw 'No valid released installer tag was found.' }
$asset = 'plexi-installer-windows-x64.exe'
$work = Join-Path ([IO.Path]::GetTempPath()) ('plexi-install-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $installer = Join-Path $work $asset
    $checksum = Join-Path $work 'checksum'
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$bootstrap/$asset" -OutFile $installer
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$bootstrap/$asset.sha256" -OutFile $checksum
    $parts = (Get-Content -Raw $checksum).Trim() -split '\s+'
    if ($parts.Count -ne 2 -or $parts[0] -notmatch '^[a-fA-F0-9]{64}$' -or $parts[1].TrimStart('*') -ne $asset) { throw 'Malformed installer checksum.' }
    if ((Get-FileHash -Algorithm SHA256 $installer).Hash -ne $parts[0]) { throw 'Installer checksum mismatch.' }
    $arguments = @('--channel', $Channel)
    if ($Tag) { $arguments += @('--tag', $Tag) }
    if ($InstallOnly) { $arguments += '--install-only' }
    if ($DryRun) { $arguments += '--dry-run' }
    if ($InstallDir) { $arguments += @('--install-dir', $InstallDir) }
    if ($BinDir) { $arguments += @('--bin-dir', $BinDir) }
    & $installer @arguments
    if ($LASTEXITCODE -ne 0) { throw "Plexi installer failed (exit $LASTEXITCODE)." }
} finally {
    Remove-Item -Recurse -Force $work
}
