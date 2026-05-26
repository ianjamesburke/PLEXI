<#
.SYNOPSIS
    Download the python-build-standalone runtime into assets\python\ for
    bundling into the Windows installer. Windows counterpart to
    scripts\fetch-python-runtime.sh (which is macOS-only).

.DESCRIPTION
    Fetches the `install_only` x86_64 windows-msvc build, which extracts to
    assets\python\ with python.exe at the root (matching the bundled-interpreter
    path the host resolves: {app}\Resources\assets\python\python.exe).

    Skips the download when the pinned version is already present.

.PARAMETER Version
    CPython version, e.g. 3.12.13. Defaults to $env:PYTHON_VERSION.

.PARAMETER PbsDate
    python-build-standalone release tag (YYYYMMDD). Defaults to $env:PYTHON_PBS_DATE.
#>
[CmdletBinding()]
param(
    [string]$Version = $env:PYTHON_VERSION,
    [string]$PbsDate = $env:PYTHON_PBS_DATE
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Version)) { throw 'Version not set (pass -Version or set $env:PYTHON_VERSION)' }
if ([string]::IsNullOrWhiteSpace($PbsDate)) { throw 'PbsDate not set (pass -PbsDate or set $env:PYTHON_PBS_DATE)' }

$repoRoot   = Split-Path -Parent $PSScriptRoot
$assetsDir  = Join-Path $repoRoot 'assets'
$pythonDir  = Join-Path $assetsDir 'python'
$versionFile = Join-Path $pythonDir '.pbs-version'

$pbsArch  = 'x86_64-pc-windows-msvc'
$expected = "$Version+$PbsDate-$pbsArch"

if ((Test-Path $versionFile) -and ((Get-Content $versionFile -Raw).Trim() -eq $expected)) {
    Write-Output "Python runtime $Version ($pbsArch) already present, skipping download"
    return
}

$filename = "cpython-$Version+$PbsDate-$pbsArch-install_only.tar.gz"
$url = "https://github.com/astral-sh/python-build-standalone/releases/download/$PbsDate/$filename"

Write-Output "Downloading Python $Version ($pbsArch) from python-build-standalone..."
if (Test-Path $pythonDir) { Remove-Item -Recurse -Force $pythonDir }
New-Item -ItemType Directory -Path $assetsDir -Force | Out-Null

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
try {
    $archive = Join-Path $tmp $filename
    Invoke-WebRequest -Uri $url -OutFile $archive
    # tar.exe (bsdtar) ships with Windows 10 1803+ and handles .tar.gz.
    & tar.exe -xzf $archive -C $assetsDir
    if ($LASTEXITCODE -ne 0) { throw "tar extraction failed (exit $LASTEXITCODE)" }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# Strip headers and debug symbols — not needed at runtime, saves ~25 MB.
$includeDir = Join-Path $pythonDir 'include'
if (Test-Path $includeDir) { Remove-Item -Recurse -Force $includeDir }
Get-ChildItem -Path $pythonDir -Recurse -File -Filter '*.pdb' | Remove-Item -Force

if (-not (Test-Path (Join-Path $pythonDir 'python.exe'))) {
    throw "python.exe not found in $pythonDir after extraction — unexpected archive layout"
}

Set-Content -Path $versionFile -Value $expected -NoNewline
Write-Output "Python $Version ready at $pythonDir"
