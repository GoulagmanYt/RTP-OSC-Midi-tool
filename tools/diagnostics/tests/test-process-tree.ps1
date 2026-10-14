$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../process-tree.ps1')
$fixtureDirectory = Join-Path ([IO.Path]::GetTempPath()) ('oscmidi-process-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
$childPath = Join-Path $fixtureDirectory 'child.ps1'
$parentPath = Join-Path $fixtureDirectory 'parent.ps1'
$rootPath = Join-Path $fixtureDirectory 'root.ps1'
$powerShellExe = (Get-Command pwsh.exe -ErrorAction Stop).Source
'Start-Sleep -Seconds 60' | Set-Content -LiteralPath $childPath
("Start-Process -FilePath '" + $powerShellExe + "' -ArgumentList '-NoProfile','-File','" + $childPath + "' -WindowStyle Hidden; Start-Sleep -Seconds 60") | Set-Content -LiteralPath $parentPath
("Start-Process -FilePath '" + $powerShellExe + "' -ArgumentList '-NoProfile','-File','" + $parentPath + "' -WindowStyle Hidden; Start-Sleep -Seconds 60") | Set-Content -LiteralPath $rootPath
$owned = [System.Collections.Generic.Dictionary[int,System.Diagnostics.Process]]::new()
$root = Start-Process -FilePath $powerShellExe -ArgumentList '-NoProfile','-File',$rootPath -WindowStyle Hidden -PassThru
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Update-OwnedProcesses -RootProcess $root -OwnedProcesses $owned
        if ($owned.Count -ge 3) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($owned.Count -lt 3) { throw 'Grandchild was not tracked' }
    $processIds = @($owned.Keys)
    Stop-OwnedProcesses -OwnedProcesses $owned
    foreach ($processId in $processIds) {
        if (Get-Process -Id $processId -ErrorAction SilentlyContinue) { throw "Owned process $processId survived cleanup" }
    }
    Write-Output 'PASS: root, child and grandchild are tracked and cleaned up'
    $unrelated = Start-Process -FilePath $powerShellExe -ArgumentList '-NoProfile','-File',$childPath -WindowStyle Hidden -PassThru
    try {
        # Model a reused ancestor PID: the original retained root has exited,
        # while CIM now reports a new process using that parent ID.
        $fakeProcesses = @([pscustomobject]@{ ParentProcessId = $root.Id; ProcessId = $unrelated.Id })
        function Get-CimInstance { $fakeProcesses }
        try { Update-OwnedProcesses -RootProcess $root -OwnedProcesses $owned }
        finally { Remove-Item -LiteralPath Function:\Get-CimInstance }
        if ($owned.Count -ne 0) { throw 'An exited root adopted a process through its obsolete PID' }
        if ($unrelated.HasExited) { throw 'Unrelated process was stopped' }
        Write-Output 'PASS: an exited root cannot adopt processes through a reused PID'
    } finally {
        if (-not $unrelated.HasExited) { $unrelated.Kill(); $unrelated.WaitForExit() }
        $unrelated.Dispose()
    }
} finally {
    Stop-OwnedProcesses -OwnedProcesses $owned
    if (-not $root.HasExited) { $root.Kill(); $root.WaitForExit() }
    $root.Dispose()
    Remove-Item -LiteralPath $rootPath, $parentPath, $childPath
    Remove-Item -LiteralPath $fixtureDirectory
}
