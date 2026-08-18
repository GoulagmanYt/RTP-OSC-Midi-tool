param(
    [ValidateSet('debug', 'release')]
    [string]$Profile = 'debug'
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot 'src-tauri\Cargo.toml'

$cmake = Get-Command cmake.exe -ErrorAction SilentlyContinue
if (-not $cmake) {
    $cmakeCandidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\18\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'),
        (Join-Path $env:ProgramFiles 'CMake\bin\cmake.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\CMake\bin\cmake.exe'),
        (Join-Path $env:USERPROFILE '.mozbuild\cmake\bin\cmake.exe')
    )
    $cmakePath = $cmakeCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $cmakePath) {
        throw 'CMake is required to build the VST host worker.'
    }
    $env:PATH = "$(Split-Path -Parent $cmakePath);$env:PATH"
}

if (-not $env:LIBCLANG_PATH) {
    $libclangCandidates = @(
        (Join-Path $env:ProgramFiles 'LLVM\bin\libclang.dll'),
        (Join-Path ${env:ProgramFiles(x86)} 'LLVM\bin\libclang.dll'),
        (Join-Path $env:LOCALAPPDATA 'Programs\LLVM\bin\libclang.dll'),
        (Join-Path $env:USERPROFILE '.mozbuild\clang\bin\libclang.dll')
    )
    $libclangPath = $libclangCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $libclangPath) {
        throw 'libclang.dll is required to generate the ASIO bindings.'
    }
    $env:LIBCLANG_PATH = Split-Path -Parent $libclangPath
}

$hostLine = rustc -vV | Where-Object { $_ -like 'host: *' } | Select-Object -First 1
if (-not $hostLine) {
    throw 'Unable to determine the Rust host target triple.'
}
$hostTriple = $hostLine.Substring(6).Trim()
if ($hostTriple -ne 'x86_64-pc-windows-msvc') {
    throw "The OSCMidi desktop build requires x86_64-pc-windows-msvc, found $hostTriple."
}

$targets = @(
    @{ Triple = 'x86_64-pc-windows-msvc'; Suffix = 'x64' },
    @{ Triple = 'i686-pc-windows-msvc'; Suffix = 'x86' }
)

$installedTargets = @(rustup target list --installed)
foreach ($target in $targets) {
    if ($installedTargets -notcontains $target.Triple) {
        throw "Missing Rust target $($target.Triple). Install it with: rustup target add $($target.Triple)"
    }
}

$metadata = cargo metadata --manifest-path $manifestPath --format-version 1 --no-deps | ConvertFrom-Json
$binaryDirectory = Join-Path $repositoryRoot 'src-tauri\binaries'
New-Item -ItemType Directory -Force -Path $binaryDirectory | Out-Null

function Get-PeMachine {
    param([Parameter(Mandatory)][string]$Path)
    $stream = [System.IO.File]::OpenRead((Resolve-Path -LiteralPath $Path).Path)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) { throw "Not a PE file: $Path" }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw "Invalid PE signature: $Path" }
        return $reader.ReadUInt16()
    }
    finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

$previousTauriConfig = $env:TAURI_CONFIG
try {
    $env:TAURI_CONFIG = '{"bundle":{"externalBin":[]}}'
    foreach ($target in $targets) {
        $cargoArgs = @(
            'build',
            '--manifest-path', $manifestPath,
            '--target', $target.Triple,
            '--bin', 'vst-host-worker',
            '--features', 'vst-worker-binary'
        )
        if ($Profile -eq 'release') {
            $cargoArgs += '--release'
        }
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            throw "Building vst-host-worker for $($target.Triple) failed with exit code $LASTEXITCODE."
        }

        $source = Join-Path $metadata.target_directory "$($target.Triple)\$Profile\vst-host-worker.exe"
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "The worker build completed but $source was not produced."
        }
        # Tauri suffixes every sidecar source with the desktop target triple,
        # including the helper whose PE payload is i686.
        $destination = Join-Path $binaryDirectory "vst-host-worker-$($target.Suffix)-$hostTriple.exe"
        Copy-Item -LiteralPath $source -Destination $destination -Force
        $expectedMachine = if ($target.Suffix -eq 'x64') { 0x8664 } else { 0x014C }
        $actualMachine = Get-PeMachine $destination
        if ($actualMachine -ne $expectedMachine) {
            throw "Wrong PE architecture for $destination (expected 0x$($expectedMachine.ToString('X4')), got 0x$($actualMachine.ToString('X4')))."
        }
        Write-Host "Prepared VST worker $($target.Suffix) sidecar: $destination"
    }
}
finally {
    $env:TAURI_CONFIG = $previousTauriConfig
}
