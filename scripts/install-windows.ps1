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

function ConvertTo-ReleaseParts([string]$Tag) {
    if ($Tag -notmatch '^v(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta)\.(\d+))?$') { return $null }
    $number = 0
    if ($Matches[5]) { $number = [int]$Matches[5] }
    return @{ Major = [int]$Matches[1]; Minor = [int]$Matches[2]; Patch = [int]$Matches[3]; Lane = $Matches[4]; Number = $number }
}
function Compare-ReleaseTag([string]$Left, [string]$Right) {
    $a = ConvertTo-ReleaseParts $Left
    $b = ConvertTo-ReleaseParts $Right
    foreach ($part in @('Major', 'Minor', 'Patch')) {
        if ($a[$part] -ne $b[$part]) { if ($a[$part] -gt $b[$part]) { return 1 } else { return -1 } }
    }
    if (-not $a.Lane -and -not $b.Lane) { return 0 }
    if (-not $a.Lane) { return 1 }
    if (-not $b.Lane) { return -1 }
    if ($a.Lane -ne $b.Lane) { if ($a.Lane -eq 'beta') { return 1 } else { return -1 } }
    if ($a.Number -ne $b.Number) { if ($a.Number -gt $b.Number) { return 1 } else { return -1 } }
    return 0
}
function Select-MaxTag($Tags) {
    $best = $null
    foreach ($tag in @($Tags)) {
        if (-not $tag) { continue }
        if (-not $best -or (Compare-ReleaseTag $tag $best) -gt 0) { $best = $tag }
    }
    return $best
}
function Get-PublishedTags([string]$Api) {
    $tags = @()
    for ($page = 1; $page -le 1000; $page++) {
        $batch = @(Invoke-RestMethod -Uri "${Api}?per_page=100&page=$page")
        if ($batch.Count -eq 0 -or $null -eq $batch[0].tag_name) { break }
        foreach ($rel in $batch) {
            if ($rel.draft) { continue }
            if ($rel.tag_name -match '^v\d+\.\d+\.\d+(-(alpha|beta)\.\d+)?$') { $tags += $rel.tag_name }
        }
        if ($batch.Count -lt 100) { break }
        if ($page -eq 1000) { throw 'release pagination exceeded its bound' }
    }
    return ,$tags
}
function Select-BootstrapTag([string]$Wanted, [string]$Api) {
    $tags = @(Get-PublishedTags $Api)
    if ($Wanted -eq 'alpha') {
        foreach ($lane in @('alpha', 'beta', 'stable')) {
            $laneTags = @($tags | Where-Object {
                $parts = ConvertTo-ReleaseParts $_
                $name = if ($parts.Lane) { $parts.Lane } else { 'stable' }
                $name -eq $lane
            })
            $best = Select-MaxTag $laneTags
            if ($best) { return $best }
        }
        return $null
    }
    $accepted = @($tags | Where-Object { (ConvertTo-ReleaseParts $_).Lane -ne 'alpha' })
    return Select-MaxTag $accepted
}

if ($Channel -notmatch '^(stable|main|alpha|beta|pr-[A-Za-z0-9-]+)$') { throw "Unsupported install channel: $Channel" }
$bootstrap = $env:PLEXI_BOOTSTRAP_TAG
if (-not $bootstrap) {
    $api = if ($env:PLEXI_RELEASES_URL) { $env:PLEXI_RELEASES_URL } else { 'https://api.github.com/repos/ianjamesburke/PLEXI/releases' }
    $wanted = if ($Channel -eq 'main') { 'stable' } else { $Channel }
    if ($wanted -eq 'alpha' -or $wanted -eq 'beta') {
        $bootstrap = Select-BootstrapTag $wanted $api
    } else {
        $bootstrap = (Invoke-RestMethod -Uri "$api/latest").tag_name
    }
}
if ($bootstrap -notmatch '^v\d+\.\d+\.\d+(-(alpha|beta)\.\d+)?$') { throw 'No valid released installer tag was found.' }
Write-Host "Resolved Plexi installer $bootstrap ($Channel)."
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
