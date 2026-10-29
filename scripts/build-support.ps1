# Shared by the full build and CI. Compatible with Windows PowerShell 5.1.
function Get-FrontendOutputSnapshot {
    param([string]$Directory)
    $snapshot = @{
        Files = [Collections.Generic.Dictionary[string,object]]::new([StringComparer]::Ordinal)
        Directories = [Collections.Generic.Dictionary[string,datetime]]::new([StringComparer]::Ordinal)
    }
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return $snapshot }
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue((Get-Item -LiteralPath $Directory -Force).FullName)
    while ($pending.Count -gt 0) {
        $folder = Get-Item -LiteralPath $pending.Dequeue() -Force
        # Never change timestamps through filesystem links.
        if ($folder.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $null }
        $snapshot.Directories[$folder.FullName] = $folder.LastWriteTimeUtc
        foreach ($entry in Get-ChildItem -LiteralPath $folder.FullName -Force) {
            if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $null }
            if ($entry.PSIsContainer) { $pending.Enqueue($entry.FullName) }
            else {
                $snapshot.Files[$entry.FullName] = @{
                    Hash = (Get-FileHash -LiteralPath $entry.FullName -Algorithm SHA256).Hash
                    Modified = $entry.LastWriteTimeUtc
                }
            }
        }
    }
    return $snapshot
}

function Restore-FrontendOutputTimestamps {
    param([string]$Directory, $Snapshot)
    if ($null -eq $Snapshot) { return }
    $current = Get-FrontendOutputSnapshot $Directory
    if ($null -eq $current) { return }
    $identical = $Snapshot.Files.Count -eq $current.Files.Count -and $Snapshot.Directories.Count -eq $current.Directories.Count
    foreach ($path in $current.Files.Keys) {
        if (-not $Snapshot.Files.ContainsKey($path) -or $Snapshot.Files[$path].Hash -ne $current.Files[$path].Hash) { $identical = $false }
    }
    foreach ($path in $current.Directories.Keys) {
        if (-not $Snapshot.Directories.ContainsKey($path)) { $identical = $false }
    }
    if ($identical) {
        # Cargo may only track previously embedded files, not the directory.
        # When any name/content changes, retain every rewritten input's new
        # timestamp so additions cannot hide behind unchanged old file hashes.
        foreach ($path in $Snapshot.Files.Keys) { [IO.File]::SetLastWriteTimeUtc($path, $Snapshot.Files[$path].Modified) }
        foreach ($path in $Snapshot.Directories.Keys) { [IO.Directory]::SetLastWriteTimeUtc($path, $Snapshot.Directories[$path]) }
    }
}

function Invoke-CheckedCommand {
    param([Parameter(Mandatory)][string]$Command, [string[]]$Arguments = @())
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Command $($Arguments -join ' ')"
    }
}

function Get-FrontendDependencyFingerprint {
    param([string]$NodeVersion, [string]$NpmVersion)
    $inputs = @('npm-ci-v1', $NodeVersion, $NpmVersion)
    foreach ($relative in @('package.json', 'package-lock.json', '.npmrc')) {
        $path = Join-Path $repositoryRoot $relative
        $inputs += if (Test-Path -LiteralPath $path -PathType Leaf) {
            (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        } else { '<absent>' }
    }
    return $inputs -join ':'
}

function Test-FrontendDependencies {
    param([string]$Npm, [string]$NodeVersion, [string]$NpmVersion)
    $stampPath = Join-Path $repositoryRoot 'node_modules/.oscmidi-build-dependencies.json'
    try {
        if (-not (Test-Path -LiteralPath $stampPath -PathType Leaf)) { return $false }
        $stamp = Get-Content -LiteralPath $stampPath -Raw | ConvertFrom-Json
        if ($stamp.fingerprint -ne (Get-FrontendDependencyFingerprint $NodeVersion $NpmVersion)) { return $false }
        # Detect missing/invalid top-level packages even if the stamp survived.
        & $Npm ls --depth=0 --include=dev --json *> $null
        return $LASTEXITCODE -eq 0
    }
    catch { return $false }
}

function Ensure-FrontendDependencies {
    param(
        [string]$Npm,
        [string]$NodeVersion,
        [string]$NpmVersion,
        [switch]$Force,
        [scriptblock]$BeforeInstall = {},
        [scriptblock]$CommandRunner = { param($Executable, $CommandArguments) Invoke-CheckedCommand $Executable $CommandArguments }
    )
    if (-not $Force -and (Test-FrontendDependencies $Npm $NodeVersion $NpmVersion)) {
        Write-Host 'Reusing frontend dependencies (lockfiles and Node/npm unchanged).'
        return
    }
    $stampPath = Join-Path $repositoryRoot 'node_modules/.oscmidi-build-dependencies.json'
    & $BeforeInstall
    if (Test-Path -LiteralPath $stampPath) { Remove-Item -LiteralPath $stampPath -Force }
    $installArguments = @('ci', '--include=dev', '--no-fund', '--no-audit', '--prefer-offline')
    & $CommandRunner $Npm $installArguments
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $stampPath) | Out-Null
    @{ fingerprint = Get-FrontendDependencyFingerprint $NodeVersion $NpmVersion } |
        ConvertTo-Json | Set-Content -LiteralPath $stampPath -Encoding UTF8
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

    # Lexical containment does not protect against junctions pointing elsewhere.
    $ancestor = Get-Item -LiteralPath $resolved -Force
    while ($ancestor.FullName.TrimEnd('\') -ne $repositoryRoot.TrimEnd('\')) {
        if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Cleanup refused through a directory link: $($ancestor.FullName)"
        }
        $ancestor = $ancestor.Parent
    }
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($resolved)
    while ($pending.Count -gt 0) {
        foreach ($entry in Get-ChildItem -LiteralPath $pending.Dequeue() -Force) {
            if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Cleanup refused a linked entry: $($entry.FullName)"
            }
            if ($entry.PSIsContainer) { $pending.Enqueue($entry.FullName) }
        }
    }

    $git = Get-Command git.exe -ErrorAction SilentlyContinue
    if (-not $git) {
        $git = Get-Command git -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath (Join-Path $repositoryRoot '.git')) {
        if (-not $git) { throw 'Cleanup requires Git to check for tracked files.' }
        $gitPath = $RelativePath.Replace('\', '/')
        $trackedFiles = @(& $git.Source -C $repositoryRoot ls-files -- $gitPath 2>$null)
        if ($LASTEXITCODE -ne 0) {
            throw "Cleanup could not verify tracked files in ${RelativePath}."
        }
        if ($trackedFiles.Count -gt 0) {
            throw "Cleanup refused to remove ${RelativePath}: it contains Git-tracked files."
        }
    }

    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Get-ChildItem -LiteralPath $resolved -File -Recurse -Force -ErrorAction SilentlyContinue |
                ForEach-Object { $_.Attributes = [System.IO.FileAttributes]::Normal }
            [System.IO.Directory]::Delete($resolved, $true)
            if (Get-Command Write-BuildLog -CommandType Function -ErrorAction SilentlyContinue) {
                Write-BuildLog "Cleaned: $RelativePath" DarkGray
            } else { Write-Host "Cleaned: $RelativePath" }
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

function Clear-BuildCaches {
    param(
        [switch]$IncludeNodeModules,
        [switch]$IncludeOutputs,
        [switch]$PreserveArtifacts,
        [scriptblock]$BeforeClean = {}
    )

    & $BeforeClean

    $targetDirs = @(
        '.cargo-target',
        'src-tauri\target',
        'src-tauri\.cargo-target',
        'tools\diagnostics\target',
        'tools\latency_bench\target',
        'src-tauri\binaries',
        'dist'
    )
    if ($IncludeNodeModules) {
        $targetDirs += 'node_modules'
    }
    if ($IncludeOutputs) {
        $targetDirs += @(
            'reports',
            'build_logs',
            'src-tauri\gen\schemas',
            '__pycache__',
            'scripts\__pycache__',
            'tools\latency_bench\__pycache__',
            'tools\diagnostics\__pycache__',
            'tools\diagnostics\tests\__pycache__',
            'vendor\rack\target',
            'vendor\rtpmidi\target',
            'vendor\vst\target',
            'vendor\midi-types\target'
        )
        if (-not $PreserveArtifacts) { $targetDirs += 'artifacts' }
    }

    foreach ($dir in $targetDirs) {
        Remove-GeneratedDirectory $dir
    }
}

function Invoke-SelectedCleanup {
    param(
        [ValidateSet('None', 'Light', 'Partial', 'Full', 'Reset')][string]$Mode,
        [scriptblock]$BeforeClean = {}
    )
    if ($Mode -eq 'None') { return }
    if ($Mode -eq 'Light') {
        & $BeforeClean
        foreach ($relative in @(
            'dist', 'node_modules\.vite', '.cargo-target\cargo-timings',
            '.cargo-target\debug\incremental', '.cargo-target\release\incremental',
            '.cargo-target\dev-audio\incremental',
            '.cargo-target\i686-pc-windows-msvc\debug\incremental',
            '.cargo-target\i686-pc-windows-msvc\release\incremental'
        )) { Remove-GeneratedDirectory $relative }
        return
    }
    Clear-BuildCaches -IncludeNodeModules:($Mode -in 'Full', 'Reset') `
        -IncludeOutputs:($Mode -in 'Full', 'Reset') -PreserveArtifacts:($Mode -ne 'Reset') -BeforeClean $BeforeClean
}

function Get-WindowsBuildPlan {
    param(
        [ValidateSet('Dev', 'Release', 'Both')][string]$BuildMode,
        [ValidateSet('Auto', 'None', 'Quick', 'Full')][string]$ValidationMode = 'Auto',
        [ValidateSet('None', 'Light', 'Partial', 'Full')][string]$CleanupMode = 'None'
    )
    if ($ValidationMode -eq 'Auto') { $ValidationMode = if ($BuildMode -eq 'Dev') { 'Quick' } else { 'Full' } }
    @(
        @{ Kind = 'Prerequisites'; Title = 'Checking prerequisites' }
        @{ Kind = 'Dependencies'; Title = 'Preparing frontend dependencies' }
        if ($ValidationMode -eq 'Full') {
            @{ Kind = 'ValidateFrontend'; Title = 'Validating the frontend' }
            @{ Kind = 'ValidateRust'; Title = 'Validating Rust, diagnostics and native audio' }
        } elseif ($ValidationMode -eq 'Quick') {
            @{ Kind = 'ValidateQuick'; Title = 'Quick checks: lint, TypeScript and Cargo (no tests)' }
        }
        @{ Kind = 'Frontend'; Title = 'Building shared frontend assets' }
        foreach ($profile in @('debug', 'release')) {
            if (($profile -eq 'debug' -and $BuildMode -eq 'Release') -or ($profile -eq 'release' -and $BuildMode -eq 'Dev')) { continue }
            @{ Kind = 'Workers'; Title = "Building $profile VST workers (x64 + x86)"; Profile = $profile }
            @{ Kind = 'Desktop'; Title = $(if ($profile -eq 'debug') { 'Building the standalone Dev application' } else { 'Building the optimized release and MSI' }); Profile = $profile }
            @{ Kind = 'Export'; Title = "Verifying and exporting $profile artifacts"; Profile = $profile }
        }
        if ($CleanupMode -ne 'None') { @{ Kind = 'Cleanup'; Title = "$CleanupMode post-build cleanup" } }
    )
}

function Get-DesktopBuildArguments {
    param(
        [ValidateSet('debug', 'release')][string]$Profile,
        [Parameter(Mandatory)][string]$ConfigPath,
        [switch]$LatencyDiagnostics
    )
    $arguments = @('build', '--ci', '--config', $ConfigPath)
    if ($Profile -eq 'debug') { $arguments += @('--debug', '--no-bundle') }
    else { $arguments += @('--bundles', 'msi') }
    $arguments += @('--', '--locked', '--timings')
    if ($LatencyDiagnostics) { $arguments += @('--features', 'latency-diagnostics') }
    return $arguments
}

