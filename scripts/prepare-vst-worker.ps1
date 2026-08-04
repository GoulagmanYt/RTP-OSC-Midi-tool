param(
    [ValidateSet('debug', 'release')]
    [string]$Profile = 'debug'
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot 'src-tauri\Cargo.toml'

$hostLine = rustc -vV | Where-Object { $_ -like 'host: *' } | Select-Object -First 1
if (-not $hostLine) {
    throw 'Unable to determine the Rust host target triple.'
}
$targetTriple = $hostLine.Substring(6).Trim()
if ($targetTriple -ne 'x86_64-pc-windows-msvc') {
    throw "The isolated VST worker currently requires x86_64-pc-windows-msvc, found $targetTriple."
}

$cargoArgs = @('build', '--manifest-path', $manifestPath, '--bin', 'vst-host-worker')
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
