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
$targetTriple = $hostLine.Substring(6).Trim()
if ($targetTriple -ne 'x86_64-pc-windows-msvc') {
    throw "The isolated VST worker currently requires x86_64-pc-windows-msvc, found $targetTriple."
}

$cargoArgs = @(
    'build',
    '--manifest-path', $manifestPath,
    '--bin', 'vst-host-worker',
    '--features', 'vst-worker-binary'
)
if ($Profile -eq 'release') {
    $cargoArgs += '--release'
}
# Build the sidecar before it exists, so temporarily override the bundle list
# that tauri-build validates for the main application.
$previousTauriConfig = $env:TAURI_CONFIG
try {
    $env:TAURI_CONFIG = '{"bundle":{"externalBin":[]}}'
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Building vst-host-worker failed with exit code $LASTEXITCODE."
    }
}
finally {
    $env:TAURI_CONFIG = $previousTauriConfig
}

$metadata = cargo metadata --manifest-path $manifestPath --format-version 1 --no-deps | ConvertFrom-Json
$source = Join-Path $metadata.target_directory "$Profile\vst-host-worker.exe"
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    throw "The worker build completed but $source was not produced."
}

$binaryDirectory = Join-Path $repositoryRoot 'src-tauri\binaries'
New-Item -ItemType Directory -Force -Path $binaryDirectory | Out-Null
$destination = Join-Path $binaryDirectory "vst-host-worker-$targetTriple.exe"
Copy-Item -LiteralPath $source -Destination $destination -Force
Write-Host "Prepared VST worker sidecar: $destination"
