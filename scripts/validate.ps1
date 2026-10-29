[CmdletBinding()]
param([ValidateSet('Frontend', 'Rust', 'Version', 'All')][string]$Scope = 'All')

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'build-support.ps1')
$previousLocation = Get-Location
$previousTauriConfig = $env:TAURI_CONFIG
$previousTarget = $env:CARGO_TARGET_DIR
try {
    Set-Location $repositoryRoot
    $env:CARGO_TARGET_DIR = Join-Path $repositoryRoot '.cargo-target'
    if ($Scope -in 'Frontend', 'All') {
        Invoke-CheckedCommand npm @('run', 'lint')
        Invoke-CheckedCommand npm @('test')
    }
    if ($Scope -in 'Version', 'Rust', 'All') {
        $package = Get-Content package.json -Raw | ConvertFrom-Json
        # PowerShell 5.1 cannot deserialize the lockfile's empty root-package key.
        $lockVersions = & node -e "const l=require('./package-lock.json'); console.log(JSON.stringify([l.version,l.packages[''].version]));"
        if ($LASTEXITCODE -ne 0) { throw 'Could not read npm lockfile versions.' }
        $lockVersions = $lockVersions | ConvertFrom-Json
        $tauri = Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
        $appMetadata = & cargo metadata --manifest-path src-tauri/Cargo.toml --format-version 1 --no-deps --locked
        if ($LASTEXITCODE -ne 0) { throw 'Could not read application Cargo metadata.' }
        $diagnosticsMetadata = & cargo metadata --manifest-path tools/diagnostics/Cargo.toml --format-version 1 --no-deps --locked
        if ($LASTEXITCODE -ne 0) { throw 'Could not read diagnostics Cargo metadata.' }
        $appVersion = (($appMetadata | ConvertFrom-Json).packages | Where-Object name -eq 'osc-midi-bridge').version
        $diagnosticsVersion = (($diagnosticsMetadata | ConvertFrom-Json).packages | Where-Object name -eq 'oscmidi-diagnostics').version
        $versions = @($package.version, $lockVersions[0], $lockVersions[1], $tauri.version, $appVersion, $diagnosticsVersion)
        if (@($versions | Where-Object { -not $_ }).Count -gt 0 -or @($versions | Sort-Object -Unique).Count -ne 1) {
            throw "Application versions disagree: $($versions -join ', ')"
        }
        if ($env:GITHUB_REF_TYPE -eq 'tag' -and $env:GITHUB_REF_NAME -ne "v$($package.version)") {
            throw "Tag $env:GITHUB_REF_NAME does not match v$($package.version)."
        }
        Write-Host "Validated OSCMidi version $($package.version)."
    }
    if ($Scope -in 'Rust', 'All') {
        $env:TAURI_CONFIG = '{"bundle":{"externalBin":[]}}'
        Invoke-CheckedCommand cargo @('fmt', '--manifest-path', 'src-tauri/Cargo.toml', '--package', 'osc-midi-bridge', '--', '--check')
        Invoke-CheckedCommand cargo @('clippy', '--manifest-path', 'src-tauri/Cargo.toml', '--locked', '--all-targets', '--all-features', '--', '-D', 'warnings')
        Invoke-CheckedCommand cargo @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--locked', '--all-targets', '--all-features')
        Invoke-CheckedCommand cargo @('fmt', '--manifest-path', 'tools/diagnostics/Cargo.toml', '--package', 'oscmidi-diagnostics', '--', '--check')
        Invoke-CheckedCommand cargo @('clippy', '--manifest-path', 'tools/diagnostics/Cargo.toml', '--locked', '--all-targets', '--', '-D', 'warnings')
        Invoke-CheckedCommand cargo @('test', '--manifest-path', 'tools/diagnostics/Cargo.toml', '--locked', '--all-targets')
        Invoke-CheckedCommand cargo @('fmt', '--manifest-path', 'vendor/rtpmidi/Cargo.toml', '--', '--check')
        Invoke-CheckedCommand cargo @('clippy', '--manifest-path', 'vendor/rtpmidi/Cargo.toml', '--locked', '--all-targets', '--', '-D', 'warnings')
        Invoke-CheckedCommand cargo @('test', '--manifest-path', 'vendor/rtpmidi/Cargo.toml', '--locked', '--all-targets')
        Invoke-CheckedCommand cargo @('check', '--manifest-path', 'vendor/rack/Cargo.toml', '--locked', '--lib', '--features', 'vst3')
        Invoke-CheckedCommand cargo @('test', '--manifest-path', 'vendor/rack/Cargo.toml', '--locked', '--lib', '--features', 'vst3', '--no-run')
        Invoke-CheckedCommand cargo @('test', '--manifest-path', 'vendor/rack/Cargo.toml', '--locked', '--lib', '--features', 'vst3', 'midi::tests')
        # These integration tests load installed plug-ins whose DLL lifecycle
        # callbacks share process-global state and cannot run concurrently.
        Invoke-CheckedCommand cargo @('test', '--manifest-path', 'vendor/rack/Cargo.toml', '--locked', '--lib', '--features', 'vst3', 'vst3::scanner::tests', '--', '--test-threads=1')
        Invoke-CheckedCommand cargo @('fmt', '--manifest-path', 'vendor/vst/Cargo.toml', '--', '--check')
        Invoke-CheckedCommand cargo @('check', '--manifest-path', 'vendor/vst/Cargo.toml', '--all-targets', '--features', 'disable_deprecation_warning')
        Invoke-CheckedCommand cargo @('test', '--manifest-path', 'vendor/vst/Cargo.toml', '--all-targets', '--features', 'disable_deprecation_warning')
        Invoke-CheckedCommand cmake @('-S', 'vendor/rack/rack-sys', '-B', '.cargo-target/realtime-tests', '-DBUILD_REALTIME_TESTS=ON')
        Invoke-CheckedCommand cmake @('--build', '.cargo-target/realtime-tests', '--config', 'RelWithDebInfo', '--target', 'rack_realtime_audio_test', '--parallel', '4')
        Invoke-CheckedCommand ctest @('--test-dir', '.cargo-target/realtime-tests', '-C', 'RelWithDebInfo', '--output-on-failure')
        Invoke-CheckedCommand powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/test-build-support.ps1')
        Invoke-CheckedCommand powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/test-build-windows-processes.ps1')
        Invoke-CheckedCommand powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/test-prepare-vst-worker.ps1')
    }
}
finally {
    $env:TAURI_CONFIG = $previousTauriConfig
    $env:CARGO_TARGET_DIR = $previousTarget
    Set-Location $previousLocation
}
