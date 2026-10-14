# Retain handles so cleanup cannot kill an unrelated process after PID reuse.
function Update-OwnedProcesses {
    param([System.Diagnostics.Process]$RootProcess, [System.Collections.Generic.Dictionary[int,System.Diagnostics.Process]]$OwnedProcesses)
    $snapshot = @(Get-CimInstance Win32_Process)
    $ids = [System.Collections.Generic.HashSet[int]]::new()
    # An exited root's numeric PID may already belong to another process.
    # Continue discovering through retained, live descendants only.
    if (-not $RootProcess.HasExited) { [void]$ids.Add($RootProcess.Id) }
    foreach ($ownedProcess in $OwnedProcesses.Values) {
        if (-not $ownedProcess.HasExited) { [void]$ids.Add($ownedProcess.Id) }
    }
    do {
        $added = $false
        foreach ($item in $snapshot) {
            if ($ids.Contains([int]$item.ParentProcessId) -and $ids.Add([int]$item.ProcessId)) { $added = $true }
        }
    } while ($added)
    foreach ($processId in $ids) {
        if ($OwnedProcesses.ContainsKey($processId)) {
            if (-not $OwnedProcesses[$processId].HasExited) { continue }
            $OwnedProcesses[$processId].Dispose()
            [void]$OwnedProcesses.Remove($processId)
        }
        try {
            $ownedProcess = Get-Process -Id $processId -ErrorAction Stop
            $null = $ownedProcess.Handle
            $OwnedProcesses.Add($processId, $ownedProcess)
        } catch { }
    }
}

function Stop-OwnedProcesses {
    param([System.Collections.Generic.Dictionary[int,System.Diagnostics.Process]]$OwnedProcesses)
    foreach ($ownedProcess in $OwnedProcesses.Values) {
        try {
            if (-not $ownedProcess.HasExited) { $ownedProcess.Kill(); [void]$ownedProcess.WaitForExit(5000) }
        } catch { }
        finally { $ownedProcess.Dispose() }
    }
    $OwnedProcesses.Clear()
}
