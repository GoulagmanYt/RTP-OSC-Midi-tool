param(
    [int]$DurationSeconds = 3600,
    [string]$ReportName = 'rtp-soak'
)
$ErrorActionPreference = 'Stop'
if ($DurationSeconds -lt 1 -or $DurationSeconds -gt 43200) { throw 'Duration must be 1..43200 seconds' }
if ($ReportName -notmatch '^[a-zA-Z0-9_-]+$') { throw 'ReportName must be a filename stem' }
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$diagnosticPath = (Resolve-Path (Join-Path $repository '.cargo-target/debug/rtp_reliability.exe')).Path
$reportDirectory = Join-Path $repository 'docs/measurements'
$reportPath = Join-Path $reportDirectory "$ReportName.json"
$memoryPath = Join-Path $reportDirectory "$ReportName-processes.jsonl"
New-Item -ItemType Directory -Path $reportDirectory -Force | Out-Null
if ((Test-Path -LiteralPath $reportPath) -or (Test-Path -LiteralPath $memoryPath)) { throw 'Choose a new report name to preserve previous evidence' }
$started = [DateTime]::UtcNow
@{
    startedUtc = $started.ToString('o'); durationSeconds = $DurationSeconds
    codeCommit = (& git -C $repository rev-parse HEAD)
    diagnosticSha256 = (Get-FileHash -LiteralPath $diagnosticPath -Algorithm SHA256).Hash
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $reportDirectory "$ReportName-provenance.json") -Encoding UTF8
$arguments = @('--duration-seconds', $DurationSeconds, '--report', ('"' + $reportPath + '"'))
$run = Start-Process -FilePath $diagnosticPath -ArgumentList $arguments -WorkingDirectory $repository -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $repository ".cargo-target/$ReportName.log") -RedirectStandardError (Join-Path $repository ".cargo-target/$ReportName-errors.log")
try {
    $processHandle = $run.Handle
    do {
        $sample = Get-Process -Id $run.Id -ErrorAction SilentlyContinue
        if ($sample) {
            @{ elapsedSeconds = ([DateTime]::UtcNow - $started).TotalSeconds; id = $sample.Id; privateBytes = $sample.PrivateMemorySize64; workingSetBytes = $sample.WorkingSet64; handles = $sample.HandleCount; threads = $sample.Threads.Count } | ConvertTo-Json -Compress | Add-Content -LiteralPath $memoryPath -Encoding UTF8
            $sample.Dispose()
        }
        if (([DateTime]::UtcNow - $started).TotalSeconds -gt $DurationSeconds + 120) { throw 'RTP run exceeded shutdown allowance' }
        $run.WaitForExit(10000) | Out-Null
    } while (-not $run.HasExited)
    $run.WaitForExit()
    if ($run.ExitCode -ne 0) { throw "RTP diagnostic failed with exit code $($run.ExitCode); inspect saved report" }
    Write-Output "Completed: $reportPath"
} finally {
    if (-not $run.HasExited) { $run.Kill(); $run.WaitForExit() }
    $run.Dispose()
}
