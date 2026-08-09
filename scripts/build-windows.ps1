[CmdletBinding()]
param(
    [switch]$NoLog
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$cargoTargetDirectory = Join-Path $repositoryRoot '.cargo-target'
$artifactDirectory = Join-Path $repositoryRoot 'artifacts'
$buildId = Get-Date -Format 'yyyy-MM-dd_HH-mm-ss'
$temporaryLog = Join-Path ([System.IO.Path]::GetTempPath()) "oscmidi-build-$buildId-$([guid]::NewGuid().ToString('N')).log"
$totalStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$buildSucceeded = $false
$exitCode = 1
$persistBuildLog = -not ($NoLog -or $env:OSCMIDI_BUILD_NO_LOG -eq '1')

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
    Add-Content -LiteralPath $temporaryLog -Value $line -Encoding UTF8
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
    Add-Content -LiteralPath $temporaryLog -Value "`r`n=== Stage $Stage/7 - $Title ($Percent%) ===" -Encoding UTF8
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
    & $Command @Arguments 2>&1 | ForEach-Object {
        $line = $_.ToString()
        Write-Host $line
        Add-Content -LiteralPath $temporaryLog -Value (Remove-AnsiSequences $line) -Encoding UTF8
    }
    $commandExitCode = $LASTEXITCODE
    $stopwatch.Stop()
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
        ($Version.Major -eq 22 -and $Version -ge [Version]'22.22.2') -or
        ($Version.Major -eq 24 -and $Version -ge [Version]'24.15.0') -or
        $Version.Major -ge 26
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

function Remove-GeneratedDirectory {
    param([Parameter(Mandatory)][string]$RelativePath)

    $candidate = Join-Path $repositoryRoot $RelativePath
    if (-not (Test-Path -LiteralPath $candidate)) {
        return
    }
    $resolved = (Resolve-Path -LiteralPath $candidate).Path.TrimEnd('\')
    $rootPrefix = $repositoryRoot.TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase) -or
        $resolved -eq $repositoryRoot.TrimEnd('\')) {
        throw "Cleanup refused outside the repository: $resolved"
    }

    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Get-ChildItem -LiteralPath $resolved -File -Recurse -Force -ErrorAction SilentlyContinue |
                ForEach-Object { $_.Attributes = [System.IO.FileAttributes]::Normal }
            [System.IO.Directory]::Delete($resolved, $true)
            Write-BuildLog "Cleaned: $RelativePath" DarkGray
            return
        }
        catch {
            if ($attempt -eq 3) {
                throw "Unable to clean ${RelativePath}: $($_.Exception.Message)"
            }
            Start-Sleep -Milliseconds 300
        }
    }
}

function Export-BuildArtifacts {
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

    $workerCandidates = @(
        Get-ChildItem (Join-Path $cargoTargetDirectory 'release') -Filter 'vst-host-worker*.exe' -File -ErrorAction SilentlyContinue
        Get-ChildItem (Join-Path $repositoryRoot 'src-tauri\binaries') -Filter 'vst-host-worker*.exe' -File -ErrorAction SilentlyContinue
    )
    $worker = $workerCandidates | Select-Object -First 1
    if (-not $worker) {
        throw 'The generated VST worker could not be found.'
    }

    $stagingDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "oscmidi-artifacts-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
    try {
        Copy-Item -LiteralPath $application.FullName -Destination (Join-Path $stagingDirectory 'OSCMidi.exe')
        Copy-Item -LiteralPath $worker.FullName -Destination (Join-Path $stagingDirectory 'vst-host-worker.exe')
        foreach ($installer in $installers) {
            Copy-Item -LiteralPath $installer.FullName -Destination $stagingDirectory
        }

        $portableDirectory = Join-Path $stagingDirectory 'portable'
        New-Item -ItemType Directory -Path $portableDirectory | Out-Null
        Copy-Item -LiteralPath $application.FullName -Destination (Join-Path $portableDirectory 'OSCMidi.exe')
        Copy-Item -LiteralPath $worker.FullName -Destination (Join-Path $portableDirectory 'vst-host-worker.exe')
        Compress-Archive `
            -Path (Join-Path $portableDirectory '*') `
            -DestinationPath (Join-Path $stagingDirectory "OSCMidi_${version}_windows_x64_portable.zip")
        [System.IO.Directory]::Delete($portableDirectory, $true)

        $deliverables = @(Get-ChildItem -LiteralPath $stagingDirectory -File)
        $hashLines = foreach ($deliverable in $deliverables) {
            $hash = Get-FileHash -LiteralPath $deliverable.FullName -Algorithm SHA256
            "$($hash.Hash)  $($deliverable.Name)"
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
        Copy-Item -Path (Join-Path $stagingDirectory '*') -Destination $artifactDirectory -Force
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
}

function Save-BuildLog {
    param([Parameter(Mandatory)][string]$Status)

    $logDirectory = Join-Path $artifactDirectory 'logs'
    New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
    $finalLog = Join-Path $logDirectory "build-$buildId-$Status.log"
    Copy-Item -LiteralPath $temporaryLog -Destination $finalLog -Force
    Get-ChildItem -LiteralPath $logDirectory -Filter 'build-*.log' -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -Skip 10 |
        Remove-Item -Force
    Write-Host "Build log: $finalLog" -ForegroundColor DarkGray
}

Set-Location $repositoryRoot
$env:CARGO_TARGET_DIR = $cargoTargetDirectory
$env:CARGO_INCREMENTAL = '0'
[System.IO.File]::WriteAllText($temporaryLog, "OSCMidi build started at $(Get-Date -Format 'u')`r`n")

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
        throw 'Node.js/npm was not found. Install Node 22.22.2+, 24.15+, or 26+.'
    }
    if (-not $nodeSupported) {
        throw "Node.js v$nodeVersion is incompatible with jsdom 30 and no compatible NVM installation was found. Install Node 22.22.2+, 24.15+, or 26+."
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

    Show-BuildStage 2 15 'Installing frontend dependencies'
    Invoke-BuildCommand $npm @('ci', '--no-fund', '--no-audit')

    Show-BuildStage 3 30 'Validating the frontend'
    Invoke-BuildCommand $npm @('run', 'lint')
    Invoke-BuildCommand $npm @('test')

    Show-BuildStage 4 45 'Validating Rust'
    $previousTauriConfig = $env:TAURI_CONFIG
    try {
        $env:TAURI_CONFIG = '{"bundle":{"externalBin":[]}}'
        Invoke-BuildCommand $cargo @('fmt', '--manifest-path', 'src-tauri\Cargo.toml', '--package', 'osc-midi-bridge', '--', '--check')
        Invoke-BuildCommand $cargo @('clippy', '--manifest-path', 'src-tauri\Cargo.toml', '--all-targets', '--all-features', '--', '-D', 'warnings')
        Invoke-BuildCommand $cargo @('test', '--manifest-path', 'src-tauri\Cargo.toml', '--all-targets', '--all-features')
        Invoke-BuildCommand $cargo @('fmt', '--manifest-path', 'tools\diagnostics\Cargo.toml', '--package', 'oscmidi-diagnostics', '--', '--check')
        Invoke-BuildCommand $cargo @('clippy', '--manifest-path', 'tools\diagnostics\Cargo.toml', '--target-dir', 'tools\diagnostics\target', '--all-targets', '--', '-D', 'warnings')
    }
    finally {
        $env:TAURI_CONFIG = $previousTauriConfig
    }

    Show-BuildStage 5 75 'Building the release and MSI installer'
    Invoke-BuildCommand $npm @('run', 'tauri:build', '--', '--bundles', 'msi', '--ci')

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
    Show-BuildStage 7 96 'Automatic cleanup'
    try {
        Stop-WorkspaceCargoProcesses
        @(
            '.cargo-target'
            'src-tauri\target'
            'tools\diagnostics\target'
            'node_modules'
            'vendor\rack\rack-sys\external'
            'src-tauri\binaries'
            'dist'
            'build_logs'
        ) | ForEach-Object { Remove-GeneratedDirectory $_ }
        Write-BuildLog 'Cleanup complete: only artifacts and persistent logs were kept.' Green
    }
    catch {
        Write-BuildLog "CLEANUP WARNING: $($_.Exception.Message)" Yellow
        if ($exitCode -eq 0) { $exitCode = 2 }
    }

    $totalStopwatch.Stop()
    $status = if ($buildSucceeded -and $exitCode -eq 0) { 'OK' } else { 'FAIL' }
    Write-BuildLog "Build $status in $([Math]::Round($totalStopwatch.Elapsed.TotalMinutes, 2)) min." $(if ($status -eq 'OK') { 'Green' } else { 'Red' })
    if ($persistBuildLog) {
        Save-BuildLog $status
    }
    else {
        Write-Host 'Persistent build log disabled for this run.' -ForegroundColor DarkGray
    }
    if (Test-Path -LiteralPath $temporaryLog) {
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
