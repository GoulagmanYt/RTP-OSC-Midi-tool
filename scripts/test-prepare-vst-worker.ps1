$ErrorActionPreference = 'Stop'
$fixture = Join-Path ([IO.Path]::GetTempPath()) "oscmidi-worker-tests-$([guid]::NewGuid().ToString('N'))"
$workspace = Join-Path $fixture 'workspace with spaces'
$caller = Join-Path $fixture 'caller'
$previousLocation = Get-Location
$previousTauriConfig = $env:TAURI_CONFIG
$previousTargetDirectory = $env:CARGO_TARGET_DIR
function Assert-True($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
try {
    foreach ($directory in @('scripts', '.cargo', 'src-tauri/src')) {
        New-Item -ItemType Directory -Force -Path (Join-Path $workspace $directory) | Out-Null
    }
    New-Item -ItemType Directory -Path $caller | Out-Null
    $prepare = Join-Path $workspace 'scripts/prepare-vst-worker.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'prepare-vst-worker.ps1') -Destination $prepare
    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'rust-toolchain.toml') -Destination $workspace
    Set-Content -LiteralPath (Join-Path $workspace '.cargo/config.toml') -Value '[build]', 'target-dir = ".cargo-target"'
    @'
[package]
name = "worker-build-fixture"
version = "0.1.0"
edition = "2021"
[features]
vst-worker-binary = []
latency-diagnostics = []
[[bin]]
name = "vst-host-worker"
path = "src/main.rs"
required-features = ["vst-worker-binary"]
'@ | Set-Content -LiteralPath (Join-Path $workspace 'src-tauri/Cargo.toml')
    Set-Content -LiteralPath (Join-Path $workspace 'src-tauri/src/main.rs') -Value 'fn main() {}'
    # The preparation command uses --locked, so generate the fixture lock first.
    & cargo generate-lockfile --manifest-path (Join-Path $workspace 'src-tauri/Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Could not generate the worker fixture lockfile.' }
    $env:CARGO_TARGET_DIR = $null
    $env:TAURI_CONFIG = '{"bundle":{"externalBin":[]},"identifier":"fixture.original"}'
    Set-Location $caller
    foreach ($profile in @('debug', 'release')) {
        & $prepare -Profile $profile
        Assert-True ((Get-Location).Path -eq $caller) 'Worker preparation must restore the caller directory.'
        Assert-True ($env:TAURI_CONFIG -eq '{"bundle":{"externalBin":[]},"identifier":"fixture.original"}') 'Worker preparation must restore the Tauri override.'
        foreach ($worker in @(
            @{ Suffix = 'x64'; Output = "$profile/vst-host-worker.exe" },
            @{ Suffix = 'x86'; Output = "i686-pc-windows-msvc/$profile/vst-host-worker.exe" }
        )) {
            $source = Join-Path $workspace ('.cargo-target/' + $worker.Output)
            Assert-True (Test-Path -LiteralPath $source) 'Worker preparation outside the repo must use its root Cargo configuration and shared cache.'
            $staged = Join-Path $workspace "src-tauri/binaries/vst-host-worker-$($worker.Suffix)-x86_64-pc-windows-msvc.exe"
            Assert-True ((Get-FileHash -LiteralPath $source).Hash -eq (Get-FileHash -LiteralPath $staged).Hash) 'The staged worker must match the selected profile and architecture.'
        }
    }
    # A real Cargo failure must restore location/config and propagate to callers.
    Set-Content -LiteralPath (Join-Path $workspace 'src-tauri/src/main.rs') -Value 'compile_error!("expected fixture failure"); fn main() {}'
    $failed = $false
    try { & $prepare -Profile debug *> (Join-Path $fixture 'expected-failure.log') } catch { $failed = $true }
    Assert-True $failed 'A worker compilation failure must propagate.'
    Assert-True ((Get-Location).Path -eq $caller) 'A worker compilation failure must restore the caller directory.'
    Assert-True ($env:TAURI_CONFIG -eq '{"bundle":{"externalBin":[]},"identifier":"fixture.original"}') 'A worker compilation failure must restore the Tauri override.'
    Write-Host 'PASS: worker preparation uses root Cargo configuration outside the repo, stages debug/release x64/x86 workers, and restores caller state on success/failure.'
}
finally {
    Set-Location $previousLocation
    $env:TAURI_CONFIG = $previousTauriConfig
    $env:CARGO_TARGET_DIR = $previousTargetDirectory
    $resolved = [IO.Path]::GetFullPath($fixture)
    if (-not $resolved.StartsWith([IO.Path]::GetTempPath(), [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe test cleanup path.' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
