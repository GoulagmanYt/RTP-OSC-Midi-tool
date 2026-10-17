[CmdletBinding()]
param(
    [switch]$NoLog,
    [switch]$Clean,
    [switch]$ReinstallDependencies
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'build-support.ps1')
$cargoTargetDirectory = Join-Path $repositoryRoot '.cargo-target'
$artifactDirectory = Join-Path $repositoryRoot 'artifacts'
$buildId = Get-Date -Format 'yyyy-MM-dd_HH-mm-ss'
$persistBuildLog = -not ($NoLog -or $env:OSCMIDI_BUILD_NO_LOG -eq '1')
$logDirectory = Join-Path $artifactDirectory 'logs'
$temporaryLog = if ($persistBuildLog) {
    New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
    Join-Path $logDirectory "build-$buildId-RUNNING.log"
}
else {
    $null
}
$buildLogWriter = if ($temporaryLog) { [System.IO.StreamWriter]::new($temporaryLog, $false, [System.Text.UTF8Encoding]::new($false)) } else { $null }
$totalStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$buildSucceeded = $false
$exitCode = 1

function Remove-AnsiSequences {
    param([AllowNull()][string]$Text)

    if ($null -eq $Text) {
        return ''
    }
    $ansiPattern = "$([char]27)\[[0-?]*[ -/]*[@-~]"
    return [regex]::Replace($Text, $ansiPattern, '')
}

function Write-BuildLog {
    param(
        [Parameter(Mandatory)]
        [string]$Message,
        [ConsoleColor]$Color = [ConsoleColor]::Gray
    )

    $line = "[$(Get-Date -Format 'HH:mm:ss')] $Message"
    Write-Host $line -ForegroundColor $Color
    if ($buildLogWriter) { $buildLogWriter.WriteLine($line) }
}

function Show-BuildStage {
    param(
        [Parameter(Mandatory)]
        [int]$Stage,
        [Parameter(Mandatory)]
        [int]$Percent,
        [Parameter(Mandatory)]
        [string]$Title
    )

    $width = 30
    $filled = [Math]::Min($width, [Math]::Floor($Percent * $width / 100))
    $bar = ('#' * $filled) + ('-' * ($width - $filled))
    Write-Host ''
    $stageColor = if ($Stage -eq 7) { 'Magenta' } else { 'Cyan' }
    Write-Host "[$bar] $Percent%  Stage $Stage/7 - $Title" -ForegroundColor $stageColor
    Write-Progress -Activity 'OSCMidi Windows Build' -Status "Stage $Stage/7 - $Title" -PercentComplete $Percent
    if ($buildLogWriter) { $buildLogWriter.WriteLine("`r`n=== Stage $Stage/7 - $Title ($Percent%) ==="); $buildLogWriter.Flush() }
}

function Invoke-BuildCommand {
    param(
        [Parameter(Mandatory)]
        [string]$Command,
        [string[]]$Arguments = @()
    )

    $display = (@($Command) + $Arguments) -join ' '
    Write-BuildLog "> $display" DarkGray
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $previousErrorActionPreference = $ErrorActionPreference
    $global:LASTEXITCODE = $null
    try {
        # Native tools commonly use stderr for warnings. PowerShell 5 turns
        # those lines into ErrorRecord instances when stderr is merged, so the
        # global Stop policy must not abort before we can inspect LASTEXITCODE.
        $ErrorActionPreference = 'Continue'
        & $Command @Arguments 2>&1 | ForEach-Object {
            $line = $_.ToString()
            Write-Host $line
            if ($buildLogWriter) { $buildLogWriter.WriteLine((Remove-AnsiSequences $line)) }
        }
        $commandExitCode = $global:LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $stopwatch.Stop()
    if ($buildLogWriter) { $buildLogWriter.Flush() }
    if ($commandExitCode -ne 0) {
        throw "Command failed with exit code $commandExitCode after $([Math]::Round($stopwatch.Elapsed.TotalSeconds, 1)) s: $display"
    }
    Write-BuildLog "Command completed in $([Math]::Round($stopwatch.Elapsed.TotalSeconds, 1)) s." DarkGray
}

function Find-Executable {
    param(
        [Parameter(Mandatory)]
        [string]$Name,
        [string[]]$Candidates = @()
    )

    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }
    foreach ($candidate in $Candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return $candidate
        }
    }
    return $null
}

function Get-NodeVersion {
    param([Parameter(Mandatory)][string]$NodePath)

    try {
        $rawVersion = (& $NodePath --version 2>$null).Trim().TrimStart('v')
        if ($LASTEXITCODE -ne 0) {
            return $null
        }
        return [Version]$rawVersion
    }
    catch {
        return $null
    }
}

function Test-NodeVersionSupported {
    param([AllowNull()][Version]$Version)

    if (-not $Version) {
        return $false
    }
    return (
        ($Version.Major -eq 20 -and $Version -ge [Version]'20.19.0') -or
        $Version -ge [Version]'22.12.0'
    )
}

function Find-CompatibleNvmNode {
    if (-not $env:NVM_HOME -or -not (Test-Path -LiteralPath $env:NVM_HOME -PathType Container)) {
        return $null
    }

    $candidates = foreach ($directory in Get-ChildItem -LiteralPath $env:NVM_HOME -Directory -ErrorAction SilentlyContinue) {
        $candidateNode = Join-Path $directory.FullName 'node.exe'
        $candidateNpm = Join-Path $directory.FullName 'npm.cmd'
        if (-not (Test-Path -LiteralPath $candidateNode -PathType Leaf) -or
            -not (Test-Path -LiteralPath $candidateNpm -PathType Leaf)) {
            continue
        }
        $candidateVersion = Get-NodeVersion $candidateNode
        if (Test-NodeVersionSupported $candidateVersion) {
            [PSCustomObject]@{
                Directory = $directory.FullName
                Node = $candidateNode
                Npm = $candidateNpm
                Version = $candidateVersion
            }
        }
    }

    return $candidates | Sort-Object Version -Descending | Select-Object -First 1
}

function Import-VisualStudioEnvironment {
    $vswhereCandidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'),
        (Join-Path $env:ProgramFiles 'Microsoft Visual Studio\Installer\vswhere.exe')
    )
    $vswhere = $vswhereCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $vswhere) {
        return
    }

    $installationPath = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $installationPath) {
        return
    }
    $developerCommand = Join-Path $installationPath 'Common7\Tools\VsDevCmd.bat'
    if (-not (Test-Path -LiteralPath $developerCommand -PathType Leaf)) {
        return
    }

    & cmd.exe /d /s /c "`"$developerCommand`" -arch=x64 -host_arch=x64 >nul && set" |
        ForEach-Object {
            $separator = $_.IndexOf('=')
            if ($separator -gt 0) {
                [Environment]::SetEnvironmentVariable($_.Substring(0, $separator), $_.Substring($separator + 1), 'Process')
            }
        }
}

function Stop-WorkspaceNodeProcesses {
    # A dev server can outlive its npm/cmd parent. Loaded native modules give
    # us an absolute workspace path even when Node's command line is relative.
    $modulePrefix = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot 'node_modules')).TrimEnd('\') + '\'
    foreach ($nodeProcess in @(Get-Process -Name node -ErrorAction SilentlyContinue)) {
        $lockedModule = $null
        try {
            $lockedModule = $nodeProcess.Modules | Where-Object {
                $_.FileName -and
                $_.FileName.StartsWith($modulePrefix, [System.StringComparison]::OrdinalIgnoreCase) -and
                $_.FileName.EndsWith('.node', [System.StringComparison]::OrdinalIgnoreCase)
            } | Select-Object -First 1
        }
        catch {
            # No evidence of a workspace lock: leave this process alone.
            continue
        }
        if (-not $lockedModule) { continue }

        Write-BuildLog "Stopping workspace Node process $($nodeProcess.Id) holding $($lockedModule.FileName)." Yellow
        Stop-Process -InputObject $nodeProcess -Force -ErrorAction Stop
        if (-not $nodeProcess.WaitForExit(5000)) {
            throw "Workspace Node process $($nodeProcess.Id) did not exit; close its development terminal and retry."
        }
    }
}

function Stop-WorkspaceCargoProcesses {
    $processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -in 'cargo.exe', 'rustc.exe', 'rustup.exe' -and
            $_.CommandLine -like "*$repositoryRoot*"
        })
    if ($processes.Count -gt 0) {
        Stop-Process -Id $processes.ProcessId -Force -ErrorAction SilentlyContinue
        Write-BuildLog "Stopped $($processes.Count) Cargo analysis process(es) still attached to the workspace." Yellow
    }
}

function Stop-WorkspaceAppProcesses {
    $processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -in 'OSCMidi.exe', 'vst-host-worker-x64.exe', 'vst-host-worker-x86.exe' -and
            ($_.ExecutablePath -like "$repositoryRoot*" -or $_.CommandLine -like "*$repositoryRoot*")
        })
    if ($processes.Count -gt 0) {
        Stop-Process -Id $processes.ProcessId -Force -ErrorAction SilentlyContinue
        Write-BuildLog "Stopped $($processes.Count) workspace application process(es)." Yellow
    }
}

function Get-PeSubsystem {
    param([Parameter(Mandatory)][string]$Path)

    $resolvedPath = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    $stream = [System.IO.File]::OpenRead($resolvedPath)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "Not a Windows PE executable: $resolvedPath"
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        if ($peOffset -gt ($stream.Length - 92)) {
            throw "Invalid PE header offset in: $resolvedPath"
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "Invalid PE signature in: $resolvedPath"
        }
        $stream.Position = $peOffset + 4 + 20 + 68
        return $reader.ReadUInt16()
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Assert-WindowsGuiExecutable {
    param([Parameter(Mandatory)][string]$Path)

    $subsystem = Get-PeSubsystem $Path
    if ($subsystem -ne 2) {
        throw "Console executable refused (PE subsystem $subsystem): $Path"
    }
}

function Get-PeMachine {
    param([Parameter(Mandatory)][string]$Path)

    $resolvedPath = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    $stream = [System.IO.File]::OpenRead($resolvedPath)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) { throw "Not a Windows PE executable: $resolvedPath" }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw "Invalid PE signature: $resolvedPath" }
        return $reader.ReadUInt16()
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

function Assert-PeMachine {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][int]$Expected
    )
    $actual = Get-PeMachine $Path
    if ($actual -ne $Expected) {
        throw "Wrong PE architecture for $Path (expected 0x$($Expected.ToString('X4')), got 0x$($actual.ToString('X4')))."
    }
}

function Get-Sha256Hash {
    param([Parameter(Mandatory)][string]$Path)

    $stream = [System.IO.File]::OpenRead((Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $sha256.ComputeHash($stream)
        return ([System.BitConverter]::ToString($bytes)).Replace('-', '')
    }
    finally {
        $sha256.Dispose()
        $stream.Dispose()
    }
}

function Export-BuildArtifacts {
    Stop-WorkspaceAppProcesses
    $version = (Get-Content (Join-Path $repositoryRoot 'package.json') -Raw | ConvertFrom-Json).version
    $application = Get-Item (Join-Path $cargoTargetDirectory 'release\OSCMidi.exe') -ErrorAction Stop
    $installers = @(
        Get-ChildItem (Join-Path $cargoTargetDirectory 'release\bundle\msi') `
            -Filter "OSCMidi_${version}_*.msi" `
            -File `
            -ErrorAction Stop
    )
    if ($installers.Count -ne 1) {
        throw "Expected exactly one MSI installer for v$version; found $($installers.Count)."
    }

    $workerX64 = Get-Item (Join-Path $cargoTargetDirectory 'release\vst-host-worker-x64.exe') -ErrorAction Stop
    $workerX86 = Get-Item (Join-Path $cargoTargetDirectory 'release\vst-host-worker-x86.exe') -ErrorAction Stop

    Assert-WindowsGuiExecutable $application.FullName
    Assert-PeMachine $application.FullName 0x8664
    Assert-WindowsGuiExecutable $workerX64.FullName
    Assert-WindowsGuiExecutable $workerX86.FullName
    Assert-PeMachine $workerX64.FullName 0x8664
    Assert-PeMachine $workerX86.FullName 0x014C
    Write-BuildLog 'Verified GUI subsystem and PE architecture for both VST workers.' Green

    $stagingDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "oscmidi-artifacts-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
    try {
        foreach ($installer in $installers) {
            Copy-Item -LiteralPath $installer.FullName -Destination $stagingDirectory
        }

        $portableDirectory = Join-Path $stagingDirectory 'portable'
        New-Item -ItemType Directory -Path $portableDirectory | Out-Null
        Copy-Item -LiteralPath $application.FullName -Destination (Join-Path $portableDirectory 'OSCMidi.exe')
        Copy-Item -LiteralPath $workerX64.FullName -Destination (Join-Path $portableDirectory 'vst-host-worker-x64.exe')
        Copy-Item -LiteralPath $workerX86.FullName -Destination (Join-Path $portableDirectory 'vst-host-worker-x86.exe')
        Compress-Archive `
            -Path (Join-Path $portableDirectory '*') `
            -DestinationPath (Join-Path $stagingDirectory "OSCMidi_${version}_windows_x64_portable.zip")

        $deliverables = @(Get-ChildItem -LiteralPath $stagingDirectory -File)
        $hashLines = foreach ($deliverable in $deliverables) {
            "$(Get-Sha256Hash $deliverable.FullName)  $($deliverable.Name)"
        }
        [System.IO.File]::WriteAllLines(
            (Join-Path $stagingDirectory 'SHA256SUMS.txt'),
            $hashLines,
            [System.Text.UTF8Encoding]::new($false)
        )

        New-Item -ItemType Directory -Force -Path $artifactDirectory | Out-Null
        Get-ChildItem -LiteralPath $artifactDirectory -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Extension -in '.exe', '.msi', '.zip' -or $_.Name -eq 'SHA256SUMS.txt' } |
            Remove-Item -Force
        Remove-GeneratedDirectory 'artifacts\portable'
        Copy-Item -Path (Join-Path $stagingDirectory '*') -Destination $artifactDirectory -Recurse -Force
    }
    finally {
        if (Test-Path -LiteralPath $stagingDirectory) {
            [System.IO.Directory]::Delete($stagingDirectory, $true)
        }
    }

    Write-BuildLog "Artifacts exported to $artifactDirectory" Green
    Get-ChildItem -LiteralPath $artifactDirectory -File |
        Where-Object { $_.Extension -in '.exe', '.msi', '.zip' } |
        ForEach-Object {
            Write-BuildLog ("  {0,-42} {1,8:N1} MiB" -f $_.Name, ($_.Length / 1MB)) Green
        }
    Get-ChildItem -LiteralPath (Join-Path $artifactDirectory 'portable') -File |
        ForEach-Object {
            Write-BuildLog ("  portable\{0,-33} {1,8:N1} MiB" -f $_.Name, ($_.Length / 1MB)) Green
        }
}

function Save-BuildLog {
    param([Parameter(Mandatory)][string]$Status)

    New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
    $finalLog = Join-Path $logDirectory "build-$buildId-$Status.log"
    $sourceLog = (Resolve-Path -LiteralPath $temporaryLog).Path
    $destinationLog = [System.IO.Path]::GetFullPath($finalLog)
    if ($sourceLog.Equals($destinationLog, [System.StringComparison]::OrdinalIgnoreCase)) {
        $finalLog = $sourceLog
    }
    elseif ((Split-Path -Parent $sourceLog).Equals($logDirectory, [System.StringComparison]::OrdinalIgnoreCase)) {
        Move-Item -LiteralPath $sourceLog -Destination $destinationLog -Force
        $finalLog = $destinationLog
    }
    else {
        Copy-Item -LiteralPath $sourceLog -Destination $destinationLog -Force
        $finalLog = $destinationLog
    }
    Get-ChildItem -LiteralPath $logDirectory -Filter 'build-*.log' -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -Skip 10 |
        Remove-Item -Force
    Write-Host "Build log: $finalLog" -ForegroundColor DarkGray
}

Set-Location $repositoryRoot
$env:CARGO_TARGET_DIR = $cargoTargetDirectory
$env:CARGO_INCREMENTAL = '0'
if ($buildLogWriter) { $buildLogWriter.WriteLine("OSCMidi build started at $(Get-Date -Format 'u')") }

try {
    Show-BuildStage 1 5 'Checking prerequisites'
    Import-VisualStudioEnvironment
    $npm = Find-Executable 'npm.cmd'
    if (-not $npm) { $npm = Find-Executable 'npm' }
    $node = Find-Executable 'node.exe'
    if (-not $node) { $node = Find-Executable 'node' }
    $cargo = Find-Executable 'cargo.exe'
    if (-not $cargo) { $cargo = Find-Executable 'cargo' }
    $cmake = Find-Executable 'cmake.exe' @(
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\18\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'),
        (Join-Path $env:ProgramFiles 'CMake\bin\cmake.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\CMake\bin\cmake.exe'),
        (Join-Path $env:USERPROFILE '.mozbuild\cmake\bin\cmake.exe')
    )
    $clang = Find-Executable 'clang.exe' @(
        (Join-Path $env:ProgramFiles 'LLVM\bin\clang.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'LLVM\bin\clang.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\LLVM\bin\clang.exe'),
        (Join-Path $env:USERPROFILE '.mozbuild\clang\bin\clang.exe')
    )
    $libclang = @(
        (Join-Path $env:ProgramFiles 'LLVM\bin\libclang.dll'),
        (Join-Path ${env:ProgramFiles(x86)} 'LLVM\bin\libclang.dll'),
        (Join-Path $env:LOCALAPPDATA 'Programs\LLVM\bin\libclang.dll'),
        (Join-Path $env:USERPROFILE '.mozbuild\clang\bin\libclang.dll')
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    $nodeVersion = if ($node) { Get-NodeVersion $node } else { $null }
    $nodeSupported = $npm -and (Test-NodeVersionSupported $nodeVersion)
    $nvmNode = $null
    if (-not $nodeSupported) {
        $nvmNode = Find-CompatibleNvmNode
        if ($nvmNode) {
            $node = $nvmNode.Node
            $npm = $nvmNode.Npm
            $nodeVersion = $nvmNode.Version
            $env:PATH = "$($nvmNode.Directory);$env:PATH"
            $nodeSupported = $true
        }
    }
    if (-not $node -or -not $npm) {
        throw 'Node.js/npm was not found. Install Node 20.19+ or 22.12+.'
    }
    if (-not $nodeSupported) {
        throw "Node.js v$nodeVersion is incompatible with the frontend toolchain and no compatible NVM installation was found. Install Node 20.19+ or 22.12+."
    }
    if (-not $cargo) { throw 'Rust/Cargo was not found.' }
    if (-not $cmake) { throw 'CMake was not found.' }
    if (-not $clang -or -not $libclang) { throw 'LLVM clang/libclang was not found.' }
    $env:PATH = "$(Split-Path -Parent $cmake);$(Split-Path -Parent $clang);$env:PATH"
    $env:CLANG_PATH = $clang
    $env:LIBCLANG_PATH = Split-Path -Parent $libclang
    if ($nvmNode) {
        Write-BuildLog "Automatically selected Node.js v$nodeVersion from NVM_HOME." Yellow
    }
    Write-BuildLog "Windows, Node.js v$nodeVersion, Rust, CMake, and LLVM prerequisites are ready." Green

    if ($Clean) {
        Write-BuildLog 'Explicit clean build requested; removing generated caches before validation.' Yellow
        Stop-WorkspaceNodeProcesses
        Stop-WorkspaceCargoProcesses
        Stop-WorkspaceAppProcesses
        @('.cargo-target', 'src-tauri\target', 'tools\diagnostics\target', 'node_modules', 'src-tauri\binaries', 'dist') |
            ForEach-Object { Remove-GeneratedDirectory $_ }
    }

    Show-BuildStage 2 15 'Preparing frontend dependencies'
    $npmVersion = (& $npm --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Unable to read npm version.' }
    Ensure-FrontendDependencies -Npm $npm -NodeVersion $nodeVersion.ToString() -NpmVersion $npmVersion -Force:$ReinstallDependencies -BeforeInstall { Stop-WorkspaceNodeProcesses } -CommandRunner { param($Executable, $CommandArguments) Invoke-BuildCommand $Executable $CommandArguments }

    $validationShell = (Get-Process -Id $PID).Path
    Show-BuildStage 3 30 'Validating the frontend'
    Invoke-BuildCommand $validationShell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'validate.ps1'), '-Scope', 'Frontend')

    Show-BuildStage 4 45 'Validating Rust, diagnostics and native audio'
    Invoke-BuildCommand $validationShell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'validate.ps1'), '-Scope', 'Rust')

    # Keep compilation outputs but never accept an installer from an earlier run.
    Remove-GeneratedDirectory '.cargo-target\release\bundle'
    Remove-GeneratedDirectory '.cargo-target\release\wix'

    Show-BuildStage 5 75 'Building the release and MSI installer'
    Invoke-BuildCommand $npm @('run', 'tauri:build', '--', '--bundles', 'msi', '--ci', '--', '--locked', '--timings')

    Show-BuildStage 6 90 'Verifying and exporting artifacts'
    Export-BuildArtifacts
    $buildSucceeded = $true
    $exitCode = 0
}
catch {
    Write-BuildLog "FAILED: $($_.Exception.Message)" Red
    $exitCode = 1
}
finally {
    Show-BuildStage 7 96 'Finalizing build logs'
    Write-BuildLog 'Compilation caches and frontend dependencies preserved for the next build.' DarkGray

    $totalStopwatch.Stop()
    $status = if ($buildSucceeded -and $exitCode -eq 0) { 'OK' } else { 'FAIL' }
    Write-BuildLog "Build $status in $([Math]::Round($totalStopwatch.Elapsed.TotalMinutes, 2)) min." $(if ($status -eq 'OK') { 'Green' } else { 'Red' })
    if ($buildLogWriter) { $buildLogWriter.Dispose() }
    if ($persistBuildLog) {
        Save-BuildLog $status
    }
    else {
        Write-Host 'Persistent build log disabled for this run.' -ForegroundColor DarkGray
    }
    if ($temporaryLog -and (Test-Path -LiteralPath $temporaryLog)) {
        Remove-Item -LiteralPath $temporaryLog -Force
    }
    Write-Progress -Activity 'OSCMidi Windows Build' -Completed
}

if ($exitCode -eq 0) {
    Write-Host ''
    Write-Host '[##############################] 100%  BUILD COMPLETE' -ForegroundColor Green
    Write-Host "Artifacts: $artifactDirectory" -ForegroundColor Cyan
}
else {
    Write-Host ''
    if ($persistBuildLog) {
        Write-Host "BUILD FAILED (exit code $exitCode). See the log in artifacts\logs." -ForegroundColor Red
    }
    else {
        Write-Host "BUILD FAILED (exit code $exitCode). Persistent logging was disabled." -ForegroundColor Red
    }
}

exit $exitCode
