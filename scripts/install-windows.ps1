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
$asset = 'plexi-installer-windows-x64.exe'
$tagPattern = '^v(\d+\.\d+\.\d+)(?:-(alpha|beta)\.(\d+))?$'

function Get-RateLimitMessage($Headers) {
    $resetAt = 'the time shown by GitHub'
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $max = [DateTimeOffset]::MaxValue.ToUnixTimeSeconds()
    $reset = 0L
    if ([long]::TryParse($Headers['x-ratelimit-reset'], [ref]$reset) -and $reset -ge 0 -and $reset -le $max) {
        $resetAt = [DateTimeOffset]::FromUnixTimeSeconds($reset).ToLocalTime().ToString('yyyy-MM-dd HH:mm zzz')
    } elseif ([long]::TryParse($Headers['retry-after'], [ref]$reset) -and $reset -ge 0 -and $reset -le ($max - $now)) {
        $resetAt = [DateTimeOffset]::FromUnixTimeSeconds($now + $reset).ToLocalTime().ToString('yyyy-MM-dd HH:mm zzz')
    }
    return "GitHub API rate limit reached (unauthenticated limit is 60 requests/hour per IP). Resets at ${resetAt}. Set GITHUB_TOKEN (or GH_TOKEN) and retry, or retry after the reset."
}

function Get-GitHubResponse([string]$Url) {
    # Windows' curl.exe exposes both the HTTP status and the transport exit code,
    # including when a response body was cut short. Match install.sh's policy.
    $curl = Get-Command curl.exe -CommandType Application -ErrorAction SilentlyContinue
    if (-not $curl) { throw "GitHub request to $Url failed (curl.exe is unavailable)." }
    $PSNativeCommandUseErrorActionPreference = $false
    $bodyFile = Join-Path $work 'response.json'
    $headerFile = Join-Path $work 'response.headers'
    $curlArgs = @('-q', '-s', '-L', '--connect-timeout', '10', '--max-time', '20',
        '-H', '@-', '-D', $headerFile, '-o', $bodyFile, '-w', '%{http_code}', $Url)
    $requestHeaders = @('Accept: application/vnd.github+json')
    $token = if ($env:GITHUB_TOKEN) { $env:GITHUB_TOKEN } else { $env:GH_TOKEN }
    if ($token) { $requestHeaders += "Authorization: Bearer $token" }
    $fatalExitCodes = @(1, 2, 3, 4, 5, 6, 23, 26, 27, 43, 47, 48, 51, 53, 54, 58, 59, 60, 66, 77, 83, 90, 91)
    $attempts = 3
    $delay = 1
    for ($attempt = 1; $attempt -le $attempts; $attempt++) {
        [IO.File]::WriteAllText($headerFile, '')
        $status = $requestHeaders | & $curl.Source @curlArgs
        $exitCode = $LASTEXITCODE
        if ($status -notmatch '^[1-9][0-9][0-9]$') { $status = $null }
        $detail = if ($status) { "HTTP $status" } else { 'HTTP status missing' }
        if ($exitCode -ne 0) {
            $detail = if ($status) { "curl exit $exitCode after HTTP $status" } else { "curl exit $exitCode" }
        }

        # The server's 4xx or 5xx verdict takes precedence over body errors.
        if ($status -match '^4[0-9]{2}$') {
            if ($status -in @('403', '429')) {
                $headers = @{}
                foreach ($line in (Get-Content -LiteralPath $headerFile)) {
                    if ($line -match '^HTTP/') { $headers.Clear() }
                    elseif ($line -match '^([^:]+):\s*(.*)$') { $headers[$Matches[1]] = $Matches[2].Trim() }
                }
                if ($status -eq '429' -or $headers['x-ratelimit-remaining'] -eq '0' -or $headers['retry-after']) {
                    throw (Get-RateLimitMessage $headers)
                }
            }
            return @{ Status = $status }
        }
        if ($status -notmatch '^5[0-9]{2}$') {
            if ($exitCode -eq 0 -and $status -match '^[23][0-9]{2}$') {
                $body = if ($status -eq '200') { Get-Content -Raw -Encoding UTF8 -LiteralPath $bodyFile | ConvertFrom-Json }
                return @{ Status = $status; Body = $body }
            }
            if ($exitCode -ge 126 -or $fatalExitCodes -contains $exitCode) {
                throw "GitHub request to $Url failed ($detail)."
            }
        }
        if ($attempt -eq $attempts) {
            throw "GitHub is slow, try again. ($detail from $Url after $attempts attempts)"
        }
        [Console]::Error.WriteLine("GitHub request failed ($detail); retrying in ${delay}s (attempt $attempt/$attempts).")
        Start-Sleep -Seconds $delay
        $delay *= 2
    }
}

function Get-ReleaseTag([string]$Path, [switch]$AllowMissing) {
    $response = Get-GitHubResponse "$releasesApi/$Path"
    if ($AllowMissing -and $response.Status -eq '404') { return }
    if ($response.Status -ne '200') { throw "Unable to read Plexi release ${Path}: HTTP $($response.Status)." }
    $release = $response.Body
    if (-not $release.draft -and $release.tag_name -cmatch $tagPattern -and $release.assets.name -contains $asset) {
        return $release.tag_name
    }
}

function Select-BootstrapTag([string]$Wanted) {
    if ($Wanted -in @('stable', 'main')) { return Get-ReleaseTag 'latest' }
    $response = Get-GitHubResponse "$repoApi/git/matching-refs/tags/v"
    if ($response.Status -ne '200') { throw "Unable to list Plexi release tags: HTTP $($response.Status)." }

    # Alpha prefers its own lane, then beta, then stable. Beta excludes alpha.
    # Every lane uses SemVer ordering, including the numeric prerelease suffix.
    $candidates = foreach ($ref in $response.Body) {
        $name = [string]$ref.ref
        if (-not $name.StartsWith('refs/tags/')) { continue }
        $tagName = $name.Substring('refs/tags/'.Length)
        if ($tagName -cnotmatch $tagPattern -or ($Wanted -eq 'beta' -and $Matches[2] -eq 'alpha')) { continue }
        $laneRank = 2
        if ($Matches[2] -eq 'alpha') { $laneRank = 0 }
        elseif ($Matches[2] -eq 'beta') { $laneRank = 1 }
        [pscustomobject]@{
            Tag = $tagName
            Preference = if ($Wanted -eq 'alpha') { 2 - $laneRank } else { 0 }
            Core = [version]$Matches[1]
            Lane = $laneRank
            Number = [uint64]$Matches[3]
        }
    }
    $ranked = @($candidates | Sort-Object -Property Preference, Core, Lane, Number -Descending)
    $maxTagLookups = 5
    foreach ($candidate in ($ranked | Select-Object -First $maxTagLookups)) {
        $found = Get-ReleaseTag "tags/$($candidate.Tag)" -AllowMissing
        if ($found) { return $found }
    }
    if ($ranked.Count -gt $maxTagLookups) {
        throw "None of the newest $maxTagLookups $Wanted releases has $asset."
    }
}

if ($Channel -notmatch '^(stable|main|alpha|beta|pr-[A-Za-z0-9-]+)$') { throw "Unsupported install channel: $Channel" }
$work = Join-Path ([IO.Path]::GetTempPath()) ('plexi-install-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $bootstrap = if ($env:PLEXI_BOOTSTRAP_TAG) { $env:PLEXI_BOOTSTRAP_TAG } else { $Tag }
    if (-not $bootstrap) {
        $repoApi = 'https://api.github.com/repos/ianjamesburke/PLEXI'
        $releasesApi = "$repoApi/releases"
        if ($env:PLEXI_GITHUB_REPO_API) {
            $repoApi = $env:PLEXI_GITHUB_REPO_API.TrimEnd('/')
            $releasesApi = "$repoApi/releases"
        } elseif ($env:PLEXI_RELEASES_URL) {
            $releasesApi = $env:PLEXI_RELEASES_URL.TrimEnd('/')
            $repoApi = $releasesApi -replace '/releases$', ''
        }
        $bootstrap = Select-BootstrapTag $Channel
    }
    if ($bootstrap -cnotmatch $tagPattern) { throw 'No valid released installer tag was found.' }
    Write-Host "Resolved Plexi installer $bootstrap ($Channel)."
    $installer = Join-Path $work $asset
    $checksum = Join-Path $work 'checksum'
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$bootstrap/$asset" -OutFile $installer
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$bootstrap/$asset.sha256" -OutFile $checksum
    $parts = (Get-Content -Raw $checksum).Trim() -split '\s+'
    if ($parts.Count -ne 2 -or $parts[0] -notmatch '^[a-fA-F0-9]{64}$' -or $parts[1].TrimStart('*') -ne $asset) { throw 'Malformed installer checksum.' }
    if ((Get-FileHash -Algorithm SHA256 $installer).Hash -ne $parts[0]) { throw 'Installer checksum mismatch.' }
    $arguments = @('--channel', $Channel)
    if ($Tag) { $arguments += @('--tag', $Tag) }
    elseif ($Channel -notlike 'pr-*') { $arguments += @('--tag', $bootstrap) }
    if ($InstallOnly) { $arguments += '--install-only' }
    if ($DryRun) { $arguments += '--dry-run' }
    if ($InstallDir) { $arguments += @('--install-dir', $InstallDir) }
    if ($BinDir) { $arguments += @('--bin-dir', $BinDir) }
    & $installer @arguments
    if ($LASTEXITCODE -ne 0) { throw "Plexi installer failed (exit $LASTEXITCODE)." }
} finally {
    Remove-Item -Recurse -Force $work
}
