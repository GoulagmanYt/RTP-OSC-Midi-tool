[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$cargoTargetDirectory = Join-Path $repositoryRoot '.cargo-target'
$artifactDirectory = Join-Path $repositoryRoot 'artifacts'
$buildId = Get-Date -Format 'yyyy-MM-dd_HH-mm-ss'
$temporaryLog = Join-Path ([System.IO.Path]::GetTempPath()) "oscmidi-build-$buildId-$([guid]::NewGuid().ToString('N')).log"
$totalStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$buildSucceeded = $false
$exitCode = 1

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
    Write-Host "[$bar] $Percent%  Etape $Stage/7 - $Title" -ForegroundColor Cyan
    Write-Progress -Activity 'Build OSCMidi Windows' -Status "Etape $Stage/7 - $Title" -PercentComplete $Percent
    Add-Content -LiteralPath $temporaryLog -Value "`r`n=== Etape $Stage/7 - $Title ($Percent%) ===" -Encoding UTF8
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
        Add-Content -LiteralPath $temporaryLog -Value $line -Encoding UTF8
    }
    $commandExitCode = $LASTEXITCODE
    $stopwatch.Stop()
    if ($commandExitCode -ne 0) {
        throw "La commande a echoue avec le code $commandExitCode apres $([Math]::Round($stopwatch.Elapsed.TotalSeconds, 1)) s : $display"
    }
    Write-BuildLog "Commande terminee en $([Math]::Round($stopwatch.Elapsed.TotalSeconds, 1)) s." DarkGray
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
        Write-BuildLog "Arret de $($processes.Count) processus Cargo d'analyse encore attaches au workspace." Yellow
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
        throw "Nettoyage refuse hors du repository : $resolved"
    }

    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Get-ChildItem -LiteralPath $resolved -File -Recurse -Force -ErrorAction SilentlyContinue |
                ForEach-Object { $_.Attributes = [System.IO.FileAttributes]::Normal }
            [System.IO.Directory]::Delete($resolved, $true)
            Write-BuildLog "Nettoye : $RelativePath" DarkGray
            return
        }
        catch {
            if ($attempt -eq 3) {
                throw "Impossible de nettoyer $RelativePath : $($_.Exception.Message)"
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
        throw "Un seul installateur MSI v$version etait attendu, $($installers.Count) ont ete trouves."
    }

    $workerCandidates = @(
        Get-ChildItem (Join-Path $cargoTargetDirectory 'release') -Filter 'vst-host-worker*.exe' -File -ErrorAction SilentlyContinue
        Get-ChildItem (Join-Path $repositoryRoot 'src-tauri\binaries') -Filter 'vst-host-worker*.exe' -File -ErrorAction SilentlyContinue
    )
    $worker = $workerCandidates | Select-Object -First 1
    if (-not $worker) {
        throw 'Le worker VST genere est introuvable.'
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

    Write-BuildLog "Artefacts exportes vers $artifactDirectory" Green
    Get-ChildItem -LiteralPath $artifactDirectory -File |
        Where-Object { $_.Extension -in '.exe', '.msi', '.zip' } |
        ForEach-Object {
            Write-BuildLog ("  {0,-42} {1,8:N1} Mio" -f $_.Name, ($_.Length / 1MB)) Green
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
    Write-Host "Journal : $finalLog" -ForegroundColor DarkGray
}

Set-Location $repositoryRoot
$env:CARGO_TARGET_DIR = $cargoTargetDirectory
$env:CARGO_INCREMENTAL = '0'
[System.IO.File]::WriteAllText($temporaryLog, "Build OSCMidi demarre le $(Get-Date -Format 'u')`r`n")

try {
    Show-BuildStage 1 5 'Verification des prerequis'
    Import-VisualStudioEnvironment
    $npm = Find-Executable 'npm.cmd'
    if (-not $npm) { $npm = Find-Executable 'npm' }
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
    if (-not $npm) { throw 'Node.js/npm est introuvable.' }
    if (-not $cargo) { throw 'Rust/Cargo est introuvable.' }
    if (-not $cmake) { throw 'CMake est introuvable.' }
    if (-not $clang -or -not $libclang) { throw 'LLVM clang/libclang est introuvable.' }
    $env:PATH = "$(Split-Path -Parent $cmake);$(Split-Path -Parent $clang);$env:PATH"
    $env:CLANG_PATH = $clang
    $env:LIBCLANG_PATH = Split-Path -Parent $libclang
    Write-BuildLog 'Prerequis Windows, Node, Rust, CMake et LLVM valides.' Green

    Show-BuildStage 2 15 'Installation des dependances frontend'
    Invoke-BuildCommand $npm @('ci', '--no-fund', '--no-audit')

    Show-BuildStage 3 30 'Validation frontend'
    Invoke-BuildCommand $npm @('run', 'lint')
    Invoke-BuildCommand $npm @('test')

    Show-BuildStage 4 45 'Validation Rust'
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

    Show-BuildStage 5 75 'Compilation release et creation du MSI'
    Invoke-BuildCommand $npm @('run', 'tauri:build', '--', '--bundles', 'msi', '--ci')

    Show-BuildStage 6 90 'Verification et export des livrables'
    Export-BuildArtifacts
    $buildSucceeded = $true
    $exitCode = 0
}
catch {
    Write-BuildLog "ECHEC : $($_.Exception.Message)" Red
    $exitCode = 1
}
finally {
    Show-BuildStage 7 96 'Nettoyage automatique'
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
        Write-BuildLog 'Nettoyage termine : seuls les livrables et journaux sont conserves.' Green
    }
    catch {
        Write-BuildLog "AVERTISSEMENT NETTOYAGE : $($_.Exception.Message)" Yellow
        if ($exitCode -eq 0) { $exitCode = 2 }
    }

    $totalStopwatch.Stop()
    $status = if ($buildSucceeded -and $exitCode -eq 0) { 'OK' } else { 'FAIL' }
    Write-BuildLog "Build $status en $([Math]::Round($totalStopwatch.Elapsed.TotalMinutes, 2)) min." $(if ($status -eq 'OK') { 'Green' } else { 'Red' })
    Save-BuildLog $status
    if (Test-Path -LiteralPath $temporaryLog) {
        Remove-Item -LiteralPath $temporaryLog -Force
    }
    Write-Progress -Activity 'Build OSCMidi Windows' -Completed
}

if ($exitCode -eq 0) {
    Write-Host ''
    Write-Host '[##############################] 100%  BUILD TERMINE' -ForegroundColor Green
    Write-Host "Livrables : $artifactDirectory" -ForegroundColor Green
}
else {
    Write-Host ''
    Write-Host "BUILD EN ECHEC (code $exitCode). Consultez le journal dans artifacts\logs." -ForegroundColor Red
}

exit $exitCode
