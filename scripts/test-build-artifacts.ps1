$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'build-support.ps1')
$tokens = $null; $parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'build-windows.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
foreach ($name in @('Get-PeSubsystem', 'Get-PeMachine', 'Assert-WindowsGuiExecutable', 'Assert-PeMachine', 'Export-DevBuildArtifacts')) {
    $function = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
    if (-not $function) { throw "Missing artifact helper: $name" }
    . ([scriptblock]::Create($function.Extent.Text))
}
function Write-BuildLog { param($Message, $Color) }
function Assert-True($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Write-TestPe([string]$Path, [int]$Machine) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    $bytes = [byte[]]::new(256)
    [BitConverter]::GetBytes([uint16]0x5A4D).CopyTo($bytes, 0)
    [BitConverter]::GetBytes([uint32]0x40).CopyTo($bytes, 0x3C)
    [BitConverter]::GetBytes([uint32]0x00004550).CopyTo($bytes, 0x40)
    [BitConverter]::GetBytes([uint16]$Machine).CopyTo($bytes, 0x44)
    [BitConverter]::GetBytes([uint16]2).CopyTo($bytes, 0x9C)
    [IO.File]::WriteAllBytes($Path, $bytes)
}

$repositoryRoot = Join-Path ([IO.Path]::GetTempPath()) "oscmidi-artifact-tests-$([guid]::NewGuid().ToString('N'))"
$cargoTargetDirectory = Join-Path $repositoryRoot '.cargo-target'
$artifactDirectory = Join-Path $repositoryRoot 'artifacts'
try {
    $app = Join-Path $cargoTargetDirectory 'debug/OSCMidi.exe'
    $x64 = Join-Path $repositoryRoot 'src-tauri/binaries/vst-host-worker-x64-x86_64-pc-windows-msvc.exe'
    $x86 = Join-Path $repositoryRoot 'src-tauri/binaries/vst-host-worker-x86-x86_64-pc-windows-msvc.exe'
    Write-TestPe $app 0x8664
    Write-TestPe $x64 0x8664
    Write-TestPe $x86 0x014C
    New-Item -ItemType Directory -Force -Path (Join-Path $artifactDirectory 'portable') | Out-Null
    Set-Content -LiteralPath (Join-Path $artifactDirectory 'portable/release.txt') -Value 'keep release'
    Export-DevBuildArtifacts
    foreach ($name in @('OSCMidi.exe', 'vst-host-worker-x64.exe', 'vst-host-worker-x86.exe')) {
        Assert-True (Test-Path -LiteralPath (Join-Path $artifactDirectory "dev/$name")) "Dev export must include $name."
    }
    Assert-True (Test-Path -LiteralPath (Join-Path $artifactDirectory 'portable/release.txt')) 'Dev export must preserve Release output.'
    $exported = Join-Path $artifactDirectory 'dev/vst-host-worker-x86.exe'
    Assert-True ([Linq.Enumerable]::SequenceEqual([byte[]][IO.File]::ReadAllBytes($exported), [byte[]][IO.File]::ReadAllBytes($x86))) 'Exported x86 worker must come from the staged profile.'
    Write-TestPe $x86 0x8664
    $failed = $false
    try { Export-DevBuildArtifacts } catch { $failed = $true }
    Assert-True $failed 'A worker with the wrong architecture must fail export.'
    Assert-True ((Get-PeMachine $exported) -eq 0x014C) 'A rejected export must preserve the previous runnable Dev output.'
    Remove-Item -LiteralPath $x64
    $failed = $false
    try { Export-DevBuildArtifacts } catch { $failed = $true }
    Assert-True $failed 'An incomplete worker set must fail export.'
    Assert-True (Test-Path -LiteralPath (Join-Path $artifactDirectory 'dev/OSCMidi.exe')) 'Missing workers must not destroy the previous Dev app.'
    Write-Host 'PASS: standalone Dev export, matching worker architectures, Release preservation and failed-export preservation.'
} finally {
    $resolved = [IO.Path]::GetFullPath($repositoryRoot)
    $prefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\oscmidi-artifact-tests-'
    if (-not $resolved.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe artifact fixture cleanup.' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
