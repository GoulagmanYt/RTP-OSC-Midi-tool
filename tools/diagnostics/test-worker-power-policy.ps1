param(
    [Parameter(Mandatory)][string]$Scenario,
    [Parameter(Mandatory)][string]$Report,
    [string]$DiagnosticBinary = '.cargo-target/release/audio_scenario.exe'
)
# Hardware regression: the real worker must still own its timer policy AFTER
# native plug-in/driver initialization. Run sequentially, outside other builds.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'process-tree.ps1')
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class AudioWorkerPowerPolicy {
    [StructLayout(LayoutKind.Sequential)]
    struct State { public uint Version, ControlMask, StateMask; }
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool GetProcessInformation(IntPtr process, int kind, ref State state, uint size);
    public static uint[] Read(int pid) {
        using (var p = Process.GetProcessById(pid)) {
            var s = new State { Version = 1 };
            if (!GetProcessInformation(p.Handle, 4, ref s, 12))
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            return new uint[] {s.ControlMask, s.StateMask};
        }
    }
}
'@
$scenarioPath = (Resolve-Path -LiteralPath $Scenario).Path
$reportPath = [System.IO.Path]::GetFullPath($Report)
$diagnosticPath = (Resolve-Path -LiteralPath $DiagnosticBinary).Path
if ((Test-Path -LiteralPath $reportPath) -or (Test-Path -LiteralPath ($reportPath + '.policy.json'))) {
    throw 'Choose a new report path to preserve evidence'
}
$run = Start-Process -FilePath $diagnosticPath -ArgumentList @(('"' + $scenarioPath + '"'), ('"' + $reportPath + '"')) -WindowStyle Hidden -PassThru -RedirectStandardOutput ($reportPath + '.stdout.txt') -RedirectStandardError ($reportPath + '.stderr.txt')
$ownedProcesses = [System.Collections.Generic.Dictionary[int,System.Diagnostics.Process]]::new()
$samples = @()
try {
    $processHandle = $run.Handle
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    $delay = 100
    while (-not (Test-Path -LiteralPath ($reportPath + '.jsonl')) -or (Get-Item -LiteralPath ($reportPath + '.jsonl')).Length -eq 0) {
        if ($run.HasExited) { throw 'Diagnostic exited before Ready; inspect stderr/report' }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Timed out waiting for Ready observation' }
        $run.WaitForExit($delay) | Out-Null
        $delay = [Math]::Min($delay * 2, 1000)
    }
    # The scenario emits its first observation only after supervisor.start
    # returns Ready. Transient probe children are gone at this point.
    $worker = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($run.Id)" | Where-Object { $_.Name -match '^vst-host-worker|^worker-x64' })
    if ($worker.Count -ne 1) { throw "Expected one owned x64 worker, found $($worker.Count)" }
    $policy = [AudioWorkerPowerPolicy]::Read($worker[0].ProcessId)
    $samples += @{ workerPid=$worker[0].ProcessId; workerPath=$worker[0].ExecutablePath; controlMask=$policy[0]; stateMask=$policy[1]; atUtc=[DateTime]::UtcNow.ToString('o') }
    $samples | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath ($reportPath + '.policy.json') -Encoding UTF8
    $policyPassed = (($policy[0] -band 5) -eq 5) -and (($policy[1] -band 5) -eq 0)
    $duration = (Get-Content -LiteralPath $scenarioPath -Raw | ConvertFrom-Json).durationUs / 1000000
    $deadline = [DateTime]::UtcNow.AddSeconds($duration + 90)
    do {
        Update-OwnedProcesses -RootProcess $run -OwnedProcesses $ownedProcesses
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Scenario exceeded deadline' }
        $run.WaitForExit(1000) | Out-Null
    } while (-not $run.HasExited)
    $run.WaitForExit()
    if ($run.ExitCode -ne 0) { throw "Scenario failed with exit code $($run.ExitCode)" }
    if (-not $policyPassed) { throw "Worker lost explicit timer policy after Ready: ControlMask=$($policy[0]), StateMask=$($policy[1]); expected 5,0" }
    Write-Output "PASS: worker policy after Ready is 5,0; evidence: $reportPath.policy.json"
} finally {
    try { Update-OwnedProcesses -RootProcess $run -OwnedProcesses $ownedProcesses } catch { Write-Warning $_ }
    if (-not $run.HasExited) { $run.Kill(); $run.WaitForExit() }
    Stop-OwnedProcesses -OwnedProcesses $ownedProcesses
    $run.Dispose()
}
