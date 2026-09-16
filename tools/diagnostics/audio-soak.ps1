param(
    [int]$DurationSeconds = 7200,
    [int]$ActiveSeconds = -1,
    [int]$CycleSeconds = 0,
    [int]$MaxWorkerPrivateMB = 4096,
    [string]$Plugin = 'C:\Program Files\Common Files\VST3\Splice\Splice INSTRUMENT.vst3',
    [string]$WorkerBinary = '',
    [string]$ReportName = 'splice-asio-soak'
)
$ErrorActionPreference = 'Stop'
if ($DurationSeconds -lt 1 -or $DurationSeconds -gt 43200) { throw 'Duration must be 1..43200 seconds' }
if ($ActiveSeconds -lt -1 -or $ActiveSeconds -gt $DurationSeconds) { throw 'ActiveSeconds must be -1 or 0..DurationSeconds' }
if ($CycleSeconds -lt 0 -or $CycleSeconds -gt $DurationSeconds -or ($CycleSeconds -gt 0 -and ($ActiveSeconds -lt 0 -or $ActiveSeconds -gt $CycleSeconds))) { throw 'CycleSeconds requires 0..ActiveSeconds <= CycleSeconds <= DurationSeconds' }
if ($MaxWorkerPrivateMB -lt 1) { throw 'MaxWorkerPrivateMB must be positive' }
if ($ReportName -notmatch '^[a-zA-Z0-9_-]+$') { throw 'ReportName must be a filename stem' }
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (-not $WorkerBinary) { $WorkerBinary = Join-Path $repository '.cargo-target/release/vst-host-worker.exe' }
$workerPath = (Resolve-Path -LiteralPath $WorkerBinary).Path
$diagnosticPath = (Resolve-Path (Join-Path $repository '.cargo-target/debug/audio_reliability.exe')).Path
$reportDirectory = Join-Path $repository 'docs/measurements'
$reportPath = Join-Path $reportDirectory "$ReportName.json"
$memoryPath = Join-Path $reportDirectory "$ReportName-processes.jsonl"
New-Item -ItemType Directory -Path $reportDirectory -Force | Out-Null
if ((Test-Path -LiteralPath $reportPath) -or (Test-Path -LiteralPath $memoryPath)) { throw 'Choose a new report name to preserve previous evidence' }
$env:OSCMIDI_VST_WORKER_X64_PATH = $workerPath
$started = [DateTime]::UtcNow
$provenance = @{
    startedUtc = $started.ToString('o'); durationSeconds = $DurationSeconds
    activeSeconds = $ActiveSeconds; cycleSeconds = $CycleSeconds; maxWorkerPrivateMB = $MaxWorkerPrivateMB
    codeCommit = (& git -C $repository rev-parse HEAD)
    workerPath = $workerPath
    workerSha256 = (Get-FileHash -LiteralPath $workerPath -Algorithm SHA256).Hash
    diagnosticSha256 = (Get-FileHash -LiteralPath $diagnosticPath -Algorithm SHA256).Hash
    scope = 'Audio endurance with periodic process resource samples; other workload may run concurrently. No physical loopback latency measurement.'
}
$provenance | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $reportDirectory "$ReportName-provenance.json") -Encoding UTF8
$arguments = @('--vst', ('"' + $Plugin + '"'), '--duration-seconds', $DurationSeconds, '--buffer', 512, '--stop-file', ('"' + $reportPath + '.stop"'), '--report', ('"' + $reportPath + '"'))
if ($ActiveSeconds -ge 0) { $arguments += @('--active-seconds', $ActiveSeconds) }
if ($CycleSeconds -gt 0) { $arguments += @('--cycle-seconds', $CycleSeconds) }
$run = Start-Process -FilePath $diagnosticPath -ArgumentList $arguments -WorkingDirectory $repository -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $repository ".cargo-target/$ReportName.log") -RedirectStandardError (Join-Path $repository ".cargo-target/$ReportName-errors.log")
try {
    $processHandle = $run.Handle # retain the process handle for its exit code
    do {
        $ownedIds = @($run.Id) + @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($run.Id)" | Select-Object -ExpandProperty ProcessId)
        $samples = @(Get-Process -Id $ownedIds -ErrorAction SilentlyContinue | ForEach-Object {
            @{ id = $_.Id; name = $_.ProcessName; privateBytes = $_.PrivateMemorySize64; workingSetBytes = $_.WorkingSet64; handles = $_.HandleCount; threads = $_.Threads.Count; cpuSeconds = $_.TotalProcessorTime.TotalSeconds }
        })
        @{ elapsedSeconds = ([DateTime]::UtcNow - $started).TotalSeconds; processes = $samples } | ConvertTo-Json -Compress -Depth 4 | Add-Content -LiteralPath $memoryPath -Encoding UTF8
        if (([DateTime]::UtcNow - $started).TotalSeconds -gt $DurationSeconds + 180) { throw 'Endurance run exceeded shutdown allowance' }
        if ($MaxWorkerPrivateMB -gt 0 -and ($samples | Where-Object { $_.name -eq 'vst-host-worker' -and $_.privateBytes -gt ($MaxWorkerPrivateMB * 1MB) })) {
            'Worker private memory exceeded the diagnostic limit' | Set-Content -LiteralPath ($reportPath + '.stop') -Encoding UTF8
        }
        $run.WaitForExit(10000) | Out-Null
    } while (-not $run.HasExited)
    $run.WaitForExit()
    if ($run.ExitCode -ne 0) { throw "Audio diagnostic failed with exit code $($run.ExitCode); inspect the saved reports" }
    Write-Output "Completed: $reportPath"
} finally {
    if (-not $run.HasExited) { $run.Kill(); $run.WaitForExit() }
    $run.Dispose()
}
