# Run the installed binary without trusting PowerShell's native-command or GUI
# wait semantics. Run both -LaunchMode Cli and Direct on an interactive desktop.
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [ValidateSet('Cli', 'Direct')][string]$LaunchMode = 'Cli',
    [ValidateRange(1, 100)][int]$Cycles = 20
)
$ErrorActionPreference = 'Stop'
$processName = [IO.Path]::GetFileNameWithoutExtension($Binary)
Set-StrictMode -Version 2
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$evidence = Join-Path $env:USERPROFILE "Documents\plexi-test-evidence\host-start-$stamp-$LaunchMode"
New-Item -ItemType Directory -Force $evidence | Out-Null
$matrix = Join-Path $evidence 'matrix.log'
$started = Get-Date
$ownedHost = $null
$hostOut = $null
$hostErr = $null
$verdict = 'FAIL'
$failure = ''
$sequence = 0
$commandSocket = $null
function Log([string]$Message) {
    $line = '{0} {1}' -f (Get-Date).ToUniversalTime().ToString('o'), $Message
    Add-Content -LiteralPath $matrix -Value $line
    Write-Host $line
}
function New-StartInfo([string[]]$CliArguments, [bool]$Ephemeral = $false) {
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $Binary
    # All command arguments here are single tokens. Reject accidental injection
    # instead of relying on ProcessStartInfo.ArgumentList (absent in PS 5.1).
    foreach ($token in $CliArguments) {
        if ($token -match '[\s"]') { throw "Unexpected multiword argument: $token" }
    }
    $info.Arguments = $CliArguments -join ' '
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.CreateNoWindow = $true
    foreach ($key in @('PLEXI_SOCKET','PLEXI_CHANNEL','PLEXI_RUNNING','PLEXI_PANE_ID',
                       'PLEXI_CONTEXT_ID','PLEXI_CONTEXT_NAME','PLEXI_CONTEXT_ROOT',
                       'PLEXI_EPHEMERAL_SESSION','PLEXI_HOST_BACKGROUND')) {
        $info.EnvironmentVariables.Remove($key)
    }
    if ($Ephemeral) { $info.EnvironmentVariables['PLEXI_EPHEMERAL_SESSION'] = '1' }
    if ($CliArguments.Count -gt 0 -and $CliArguments[0] -eq 'pane' -and $null -ne $commandSocket) {
        $info.EnvironmentVariables['PLEXI_SOCKET'] = $commandSocket
    }
    return $info
}
function Invoke-Cli([string]$Label, [string[]]$CliArguments, [bool]$AllowFailure = $false) {
    $script:sequence++
    $prefix = '{0:D4}-{1}' -f $script:sequence, $Label
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = New-StartInfo $CliArguments
    Log "START $prefix command=$($CliArguments -join ' ')"
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    $budgetMs = if ($Label -eq 'host-start') { 17000 } else { 30000 }
    try {
        if (-not $process.Start()) { throw 'Process.Start returned false' }
        $outTask = $process.StandardOutput.ReadToEndAsync()
        $errTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit([Math]::Max(1, $budgetMs - [int]$elapsed.ElapsedMilliseconds))) {
            $process.Kill()
            $null = $process.WaitForExit(2000)
            throw "CLI timed out: $Label pid=$($process.Id)"
        }
        Log "PROCESS EXIT $prefix pid=$($process.Id) code=$($process.ExitCode) elapsedMs=$($elapsed.ElapsedMilliseconds)"
        # Process exit and capture EOF are different events. An inherited writer
        # in the detached host must not turn the following reads into a hang.
        foreach ($task in @($outTask, $errTask)) {
            if (-not $task.Wait([Math]::Max(1, $budgetMs - [int]$elapsed.ElapsedMilliseconds))) {
                throw "Capture EOF timed out after process exit: $Label (possible inherited writer)"
            }
        }
        $stdout = $outTask.GetAwaiter().GetResult()
        $stderr = $errTask.GetAwaiter().GetResult()
        $exitCode = $process.ExitCode
        Set-Content -LiteralPath (Join-Path $evidence "$prefix.stdout.txt") -Value $stdout
        Set-Content -LiteralPath (Join-Path $evidence "$prefix.stderr.txt") -Value $stderr
        Log "EXIT $prefix pid=$($process.Id) code=$exitCode"
        # stderr alone is not failure (version warnings are permitted).
        if ($exitCode -ne 0 -and -not $AllowFailure) {
            throw "Native exit=$exitCode at $Label; stderr=$stderr"
        }
        return [pscustomobject]@{ ExitCode=$exitCode; Stdout=$stdout; Stderr=$stderr }
    } finally { $process.Dispose() }
}
function Snapshot([string]$Label) {
    if ($null -ne $ownedHost) {
        $ownedHost.Refresh()
        if ($ownedHost.HasExited) { Log "HOST EXIT at=$Label pid=$($ownedHost.Id) code=$($ownedHost.ExitCode)" }
        else { Log "HOST ALIVE at=$Label pid=$($ownedHost.Id)" }
    }
    $outPath = Join-Path $evidence "$Label-processes.json"
    # Prefer CIM (CommandLine/ParentProcessId). Over SSH/WMI Access denied
    # (0x80041003), fall back to Get-Process so the diag can continue.
    try {
        $cim = @(Get-CimInstance Win32_Process -Filter "Name='$processName.exe'" -ErrorAction Stop)
        $cim |
            Select-Object ProcessId,ParentProcessId,ExecutablePath,CommandLine |
            ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $outPath
        return
    } catch {
        Log "WARN Snapshot CIM failed at=$Label ($($_.Exception.Message)); falling back to Get-Process"
    }
    $procs = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
    $rows = @(foreach ($p in $procs) {
        $pathProp = $null
        $startProp = $null
        try { $pathProp = $p.Path } catch { }
        try { $startProp = $p.StartTime } catch { }
        [pscustomobject]@{
            Id = $p.Id
            ProcessName = $p.ProcessName
            Path = $pathProp
            StartTime = if ($null -ne $startProp) { $startProp.ToUniversalTime().ToString('o') } else { $null }
            ParentProcessId = $null
        }
    })
    if ($rows.Count -eq 0) {
        '[]' | Set-Content -LiteralPath $outPath
    } else {
        $rows | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $outPath
    }
}
function ConvertFrom-PaneListJson([string]$Stdout) {
    # Windows PowerShell 5.1: @($stdout | ConvertFrom-Json) nests a multi-element
    # JSON array as a single Object[] entry. Where-Object then sees "$_.id" as the
    # space-joined ids (e.g. "2 1") and never matches — Wait-PaneState false-FAIL
    # even when every poll file shows focused:true. Use -InputObject + @($parsed).
    if ([string]::IsNullOrWhiteSpace($Stdout)) { return @() }
    $text = $Stdout.Trim().TrimStart([char]0xFEFF)
    $parsed = ConvertFrom-Json -InputObject $text
    if ($null -eq $parsed) { return @() }
    return @($parsed)
}
function Get-PaneProp($Pane, [string]$Name) {
    if ($null -eq $Pane) { return $null }
    $prop = $Pane.PSObject.Properties[$Name]
    if ($null -eq $prop) {
        $prop = $Pane.PSObject.Properties | Where-Object { $_.Name -eq $Name } | Select-Object -First 1
    }
    if ($null -eq $prop) { return $null }
    return $prop.Value
}
function Pane-List([string]$Label) {
    $reply = Invoke-Cli $Label @('pane','list')
    return @(ConvertFrom-PaneListJson $reply.Stdout)
}
function Wait-PaneState([string]$Label, [string]$PaneId, [bool]$Present) {
    $limit = (Get-Date).AddSeconds(5)
    $poll = 0
    do {
        $poll++
        $panes = @(Pane-List $Label)
        $matching = @($panes | Where-Object { "$(Get-PaneProp $_ 'id')" -eq "$PaneId" })
        if ($Present -and $matching.Count -eq 1) {
            $focusedVal = Get-PaneProp $matching[0] 'focused'
            $isFocused = $false
            if ($focusedVal -is [bool]) { $isFocused = $focusedVal }
            elseif ($null -ne $focusedVal) {
                $isFocused = [System.Convert]::ToBoolean($focusedVal)
            }
            if ($isFocused) {
                Log "PASS Wait-PaneState $Label pane=$PaneId focused=true on poll $poll (panes=$($panes.Count))"
                return
            }
        }
        if (-not $Present -and $matching.Count -eq 0) {
            Log "PASS Wait-PaneState $Label pane=$PaneId absent on poll $poll (panes=$($panes.Count))"
            return
        }
        if ((Get-Date) -ge $limit) { throw "State was not observed: $Label pane=$PaneId" }
        Start-Sleep -Milliseconds 50
    } while ($true)
}
try {
    if (-not (Test-Path -LiteralPath $Binary)) { throw "Missing binary: $Binary" }
    Log "binary=$Binary sha256=$((Get-FileHash -LiteralPath $Binary -Algorithm SHA256).Hash) mode=$LaunchMode"
    $oldScript = Join-Path $PSScriptRoot 'host01-hand.ps1'
    if (Test-Path -LiteralPath $oldScript) {
        Copy-Item -LiteralPath $oldScript -Destination (Join-Path $evidence 'previous-host01-hand.ps1')
    }
    Snapshot 'before'
    if (@(Get-Process -Name $processName -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'A process for this binary is already running; refusing to alter its session.'
    }
    $version = Invoke-Cli 'version' @('--version')
    if (($version.Stdout + $version.Stderr) -notmatch 'plexi\s+\d+\.\d+\.\d+') {
        throw 'Missing binary version; inspect captured version streams.'
    }
    if ($LaunchMode -eq 'Direct') {
        $ownedHost = New-Object System.Diagnostics.Process
        $ownedHost.StartInfo = New-StartInfo @() $true
        if (-not $ownedHost.Start()) { throw 'Host Process.Start returned false' }
        $hostOut = $ownedHost.StandardOutput.ReadToEndAsync()
        $hostErr = $ownedHost.StandardError.ReadToEndAsync()
        Log "DIRECT HOST pid=$($ownedHost.Id)"
    } else {
        # Readiness must be an actual zero exit from the native process.
        $startReply = Invoke-Cli 'host-start' @('host','start','--ephemeral','--timeout-secs','15') $true
        $newHosts = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
        if ($newHosts.Count -eq 1) { $ownedHost = $newHosts[0] }
        Snapshot 'after-start'
        if ($startReply.ExitCode -ne 0) { throw 'Host start failed before any pane-new; this is not a post-spawn pipe drop.' }
    }
    $deadline = (Get-Date).AddSeconds(20)
    do {
        Snapshot 'readiness'
        if ($null -ne $ownedHost -and $ownedHost.HasExited) { throw 'Host exited before readiness' }
        $reply = Invoke-Cli 'readiness' @('host','status','--json') $true
        $state = $null
        if ($reply.ExitCode -eq 0) { $state = $reply.Stdout | ConvertFrom-Json }
        if ($null -ne $state -and $state.ready -eq $true) { break }
        if ((Get-Date) -ge $deadline) { throw 'Host never reached ready=true' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($null -eq $state.pid -or $state.pid -le 0) { throw 'Ready status has no real host PID' }
    if ($null -ne $ownedHost -and $state.pid -ne $ownedHost.Id) { throw 'Ready PID differs from launched host' }
    $commandSocket = $state.socket
    $initialPanes = @(Pane-List 'initial-pane-list')
    if ($initialPanes.Count -eq 0) { throw 'Ready host has no anchor pane' }
    for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
        $spawn = Invoke-Cli "new-$cycle" @('pane','new','--name',"astra-$cycle")
        if ($spawn.Stdout.Trim() -notmatch '^\d+$') { throw "pane-new did not return a pane id: $($spawn.Stdout)" }
        $paneId = $spawn.Stdout.Trim()
        $null = Invoke-Cli "focus-$cycle" @('pane','focus',$paneId)
        # Focus/close are fire-and-forget. Verify the resulting state too.
        Wait-PaneState "after-focus-$cycle" $paneId $true
        $null = Invoke-Cli "close-$cycle" @('pane','close',$paneId)
        Wait-PaneState "after-close-$cycle" $paneId $false
        Snapshot "cycle-$cycle"
    }
    Start-Sleep -Seconds 2
    $null = Pane-List 'reconnect-after-two-seconds'
    Snapshot 'after-burst'
    $verdict = 'PASS'
} catch {
    $failure = $_.Exception.Message
    Log "FAIL $failure"
    Log $_.ScriptStackTrace
} finally {
    try { Snapshot 'finally' } catch { Log "Snapshot failed: $_" }
    if ($null -ne $ownedHost) {
        try {
            if (-not $ownedHost.HasExited) {
                $null = Invoke-Cli 'host-stop' @('host','stop') $true
                if (-not $ownedHost.WaitForExit(10000)) {
                    $verdict = 'FAIL'
                    $failure += '; owned host did not stop'
                    Log 'Force-stopping only the diagnostic-owned host PID'
                    $ownedHost.Kill()
                    $ownedHost.WaitForExit()
                }
            }
            if ($null -ne $hostOut) {
                if (-not $hostOut.Wait(5000) -or -not $hostErr.Wait(5000)) { throw 'Host capture EOF timeout at teardown' }
                Set-Content -LiteralPath (Join-Path $evidence 'host.stdout.txt') -Value $hostOut.GetAwaiter().GetResult()
                Set-Content -LiteralPath (Join-Path $evidence 'host.stderr.txt') -Value $hostErr.GetAwaiter().GetResult()
            }
            Log "HOST FINAL pid=$($ownedHost.Id) code=$($ownedHost.ExitCode)"
        } catch { Log "Teardown failed: $_"; $verdict='FAIL'; $failure += "; teardown: $_" }
    }
    try {
        Get-WinEvent -FilterHashtable @{ LogName='Application'; StartTime=$started } -ErrorAction Stop |
            Where-Object { $_.Message -match 'plexi|d3d|dxgi' } |
            Select-Object TimeCreated,Id,ProviderName,Message | ConvertTo-Json -Depth 4 |
            Set-Content -LiteralPath (Join-Path $evidence 'application-events.json')
    } catch { Log "Event log collection: $_" }
    @("verdict=$verdict", "failure=$failure", "binary=$Binary", "launch=$LaunchMode", "cycles=$Cycles", "evidence=$evidence") |
        Set-Content -LiteralPath (Join-Path $evidence 'SUMMARY.txt')
    Log "VERDICT=$verdict evidence=$evidence"
}
if ($verdict -eq 'PASS') { exit 0 } else { exit 1 }
