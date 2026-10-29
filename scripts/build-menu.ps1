# English-only assistant, compatible with Windows PowerShell 5.1 and PowerShell 7.
function Read-BuildChoice {
    param(
        [string]$Prompt,
        [string[]]$Allowed,
        [string]$Default,
        [scriptblock]$ReadInput
    )
    while ($true) {
        $answer = ([string](& $ReadInput $Prompt)).Trim().ToUpperInvariant()
        if ($answer -eq '') { $answer = $Default }
        if ($answer -in $Allowed) { return $answer }
        Write-Host "  Please choose $($Allowed -join ', ')." -ForegroundColor Yellow
    }
}

function Read-BuildSelection {
    param([scriptblock]$ReadInput = { param($Prompt) Read-Host $Prompt })

    while ($true) {
        Write-Host ''
        Write-Host ('  ' + ('=' * 66)) -ForegroundColor DarkCyan
        Write-Host '    OSCMidi  /  WINDOWS BUILD ASSISTANT' -ForegroundColor Cyan
        Write-Host '    Choose once. Build, export, then clean up automatically.' -ForegroundColor DarkGray
        Write-Host ('  ' + ('=' * 66)) -ForegroundColor DarkCyan
        Write-Host ''
        Write-Host '  1 / WHAT WOULD YOU LIKE TO DO?' -ForegroundColor Cyan
        Write-Host '    [1] Dev       Fast debug build, ready to run, no installer'
        Write-Host '    [2] Release   Optimized application, portable ZIP + MSI'
        Write-Host '    [3] Both      Dev + Release, shared dependency/frontend preparation'
        Write-Host '    [4] Clean     Clean the workspace without building'
        Write-Host '    [0] Exit'
        $action = Read-BuildChoice '  Select [1]' @('0', '1', '2', '3', '4', 'Q') '1' $ReadInput
        if ($action -in '0', 'Q') { return @{ Cancelled = $true } }

        $cleanOnly = $action -eq '4'
        $fullClean = $false
        $noLog = $false
        $validationMode = 'None'
        $mode = switch ($action) { '1' { 'Dev' } '2' { 'Release' } '3' { 'Both' } '4' { 'Dev' } }
        if (-not $cleanOnly) {
            Write-Host ''
            Write-Host '  2 / CHECKS AND TESTS' -ForegroundColor Cyan
            Write-Host '    [0] None      Build only; skip optional checks and tests'
            Write-Host '    [1] Quick     ESLint + TypeScript + cargo check, no tests'
            Write-Host '    [2] Full      Tests, Clippy, diagnostics and native audio checks'
            $checkDefault = if ($mode -eq 'Dev') { '1' } else { '2' }
            $checks = Read-BuildChoice "  Select [$checkDefault], or Q to exit" @('0', '1', '2', 'Q') $checkDefault $ReadInput
            if ($checks -eq 'Q') { return @{ Cancelled = $true } }
            $validationMode = switch ($checks) { '0' { 'None' } '1' { 'Quick' } '2' { 'Full' } }
            Write-Host ''
            Write-Host '  3 / SAVE BUILD LOGS TO DISK?' -ForegroundColor Cyan
            Write-Host '    [1] Yes       Live output + a detailed log (keep the latest 10)'
            Write-Host '    [2] No        Live output only, no persistent build log'
            $logging = Read-BuildChoice '  Select [1], or Q to exit' @('1', '2', 'Q') '1' $ReadInput
            if ($logging -eq 'Q') { return @{ Cancelled = $true } }
            $noLog = $logging -eq '2'
        }

        Write-Host ''
        $heading = if ($cleanOnly) { '  2 / CLEANUP LEVEL' } else { '  4 / CLEANUP AFTER A SUCCESSFUL BUILD' }
        Write-Host $heading -ForegroundColor Cyan
        if (-not $cleanOnly) { Write-Host '    [0] None      Keep all caches for the fastest next build' }
        Write-Host '    [1] Light     Remove temporary/incremental caches; keep libraries'
        Write-Host '    [2] Partial   Remove compiled targets; keep frontend dependencies'
        Write-Host '    [3] Full      Also remove node_modules and generated support files'
        if ($cleanOnly) { Write-Host '    [4] Reset     Also delete artifacts, packages and previous logs' -ForegroundColor Yellow }
        Write-Host '    Light / Partial / Full always preserve artifacts and build logs.' -ForegroundColor DarkGray
        $allowed = if ($cleanOnly) { @('1', '2', '3', '4', 'Q') } else { @('0', '1', '2', '3', 'Q') }
        $default = if ($cleanOnly) { '2' } else { '0' }
        $cleanup = Read-BuildChoice "  Select [$default], or Q to exit" $allowed $default $ReadInput
        if ($cleanup -eq 'Q') { return @{ Cancelled = $true } }
        $cleanupMode = switch ($cleanup) { '0' { 'None' } '1' { 'Light' } '2' { 'Partial' } '3' { 'Full' } '4' { 'Full' } }
        $fullClean = $cleanup -eq '4'
        if ($fullClean) { $noLog = $true }

        Write-Host ''
        Write-Host ('  ' + ('-' * 66)) -ForegroundColor DarkCyan
        Write-Host '  READY TO RUN' -ForegroundColor Cyan
        if ($cleanOnly) {
            $operation = if ($fullClean) { 'Workspace reset (includes all exported artifacts)' } else { "$cleanupMode cleanup only" }
            Write-Host "    Operation    $operation"
        } else {
            Write-Host "    Build        $mode"
            Write-Host "    Checks       $validationMode"
            $logLabel = if ($noLog) { 'Console only' } else { 'Console + artifacts\logs' }
            Write-Host "    Logs         $logLabel"
            Write-Host "    Cleanup      $cleanupMode (after all selected builds succeed)"
            if ($mode -in 'Dev', 'Both') { Write-Host '    Dev app      artifacts\dev\OSCMidi.exe' -ForegroundColor Green }
            if ($mode -in 'Release', 'Both') { Write-Host '    Release      artifacts\portable + ZIP + MSI' -ForegroundColor Green }
        }
        Write-Host ('  ' + ('-' * 66)) -ForegroundColor DarkCyan
        if ($fullClean) {
            $review = Read-BuildChoice '  Type RESET to delete all generated outputs, R to change, Q to exit' @('RESET', 'R', 'Q') 'Q' $ReadInput
        } else {
            $review = Read-BuildChoice '  Enter to start, R to change choices, Q to exit' @('START', 'R', 'Q') 'START' $ReadInput
        }
        if ($review -eq 'Q') { return @{ Cancelled = $true } }
        if ($review -eq 'R') { continue }
        return @{
            Cancelled = $false; BuildMode = $mode; ValidationMode = $validationMode; NoLog = $noLog
            CleanupMode = $cleanupMode; CleanOnly = $cleanOnly; FullClean = $fullClean
        }
    }
}
