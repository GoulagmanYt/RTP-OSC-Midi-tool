$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$previousLocation = Get-Location
$fixture = Join-Path ([IO.Path]::GetTempPath()) "oscmidi-ci-tests-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $fixture | Out-Null
$environmentNames = @('GITHUB_OUTPUT', 'GITHUB_EVENT_NAME', 'GITHUB_REF_TYPE', 'GITHUB_REF_NAME', 'GITHUB_SHA', 'GITHUB_REPOSITORY', 'REQUEST_PUBLISH', 'CACHE_STAGE', 'CACHE_PLATFORM', 'CONFIG_HASH', 'SOURCE_HASH', 'OSCMIDI_TEST_OUTPUT')
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }

function Assert-True($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Get-YamlScript([string]$Path, [string]$Name) {
    $scriptText = & node -e "const fs=require('fs'),y=require('js-yaml'); const d=y.load(fs.readFileSync(process.argv[1],'utf8')); const s=d.runs?d.runs.steps:d.jobs['build-windows'].steps; console.log(s.find(s=>s.name===process.argv[2]).run);" $Path $Name
    if ($LASTEXITCODE -ne 0) { throw 'Could not read CI script.' }
    return [scriptblock]::Create($scriptText -join "`n")
}
try {
    Set-Location $repositoryRoot
    $policy = Get-YamlScript '.github/workflows/build-windows.yml' 'Validate release publication policy'
    $env:GITHUB_OUTPUT = Join-Path $fixture 'policy.txt'
    $env:GITHUB_SHA = '1111111111111111111111111111111111111111'
    $env:GITHUB_REPOSITORY = 'fixture/repository'
    $env:GITHUB_REF_NAME = 'main'
    $version = (Get-Content package.json -Raw | ConvertFrom-Json).version
    $script:existingTags = @()
    $script:remoteTags = @()
    $script:lookupFailure = $false
    # Only read-only remote boundaries are replaced. The workflow code and
    # application version validation execute unchanged.
    function gh {
        $global:LASTEXITCODE = if ($script:lookupFailure) { 1 } else { 0 }
        $script:existingTags
    }
    function git { $global:LASTEXITCODE = 0; $script:remoteTags }

    $env:GITHUB_EVENT_NAME = 'push'; $env:GITHUB_REF_TYPE = 'branch'; $env:REQUEST_PUBLISH = 'false'
    & $policy
    Assert-True ((Get-Content $env:GITHUB_OUTPUT) -contains 'publish=false') 'A main push must not publish.'
    Clear-Content $env:GITHUB_OUTPUT
    $env:GITHUB_EVENT_NAME = 'workflow_dispatch'; $env:REQUEST_PUBLISH = 'true'
    & $policy
    Assert-True ((Get-Content $env:GITHUB_OUTPUT) -contains 'publish=true') 'An explicit manual request must publish a new version.'
    $script:existingTags = @("v$version")
    $failed = $false
    try { & $policy } catch { $failed = $true }
    Assert-True $failed 'Existing releases must be refused.'
    $script:existingTags = @(); $script:lookupFailure = $true
    $failed = $false
    try { & $policy } catch { $failed = $true }
    Assert-True $failed 'Remote lookup failures must block publication.'
    $script:lookupFailure = $false
    $script:remoteTags = @("2222222222222222222222222222222222222222`trefs/tags/v$version")
    $failed = $false
    try { & $policy } catch { $failed = $true }
    Assert-True $failed 'A tag pointing to another source commit must be refused.'
    $script:remoteTags = @("2222222222222222222222222222222222222222`trefs/tags/v$version", "$env:GITHUB_SHA`trefs/tags/v$version^{}")
    & $policy
    $env:GITHUB_EVENT_NAME = 'push'; $env:GITHUB_REF_TYPE = 'tag'; $env:GITHUB_REF_NAME = "v$version"
    & $policy
    $env:GITHUB_REF_NAME = 'v0.0.0'
    $failed = $false
    try { & $policy } catch { $failed = $true }
    Assert-True $failed 'A version tag must match the package version.'

    $keys = Get-YamlScript '.github/actions/restore-rust-cache/action.yml' 'Compute compiler and source cache keys'
    $env:GITHUB_OUTPUT = Join-Path $fixture 'keys.txt'
    $env:CACHE_PLATFORM = 'Windows-X64'; $env:CACHE_STAGE = 'validation'
    $env:CONFIG_HASH = 'fixture-config'; $env:SOURCE_HASH = 'source-one'
    & $keys
    $first = Get-Content $env:GITHUB_OUTPUT
    Assert-True ($first -contains '.cargo-target/debug') 'Validation must cache the shared diagnostics/application target.'
    Clear-Content $env:GITHUB_OUTPUT
    $env:SOURCE_HASH = 'source-two'
    & $keys
    $second = Get-Content $env:GITHUB_OUTPUT
    Assert-True (($first | Where-Object { $_ -like 'build-key=*' }) -ne ($second | Where-Object { $_ -like 'build-key=*' })) 'Changed sources must produce a new snapshot key.'
    Assert-True (($first | Where-Object { $_ -like 'build-prefix=*' }) -eq ($second | Where-Object { $_ -like 'build-prefix=*' })) 'Changed sources must still restore a compatible previous snapshot.'
    Clear-Content $env:GITHUB_OUTPUT
    $env:CACHE_STAGE = 'release'
    & $keys
    $release = Get-Content $env:GITHUB_OUTPUT
    Assert-True ($release -contains '.cargo-target/release') 'Release must cache the native x64 build.'
    Assert-True ($release -contains '.cargo-target/i686-pc-windows-msvc/release') 'Release must cache the x86 worker.'
    # A real Vite build exercises Node -> PowerShell 5 -> Node argument passing.
    $env:OSCMIDI_TEST_OUTPUT = Join-Path $fixture 'output with spaces'
    & node -e 'const {spawnSync}=require("node:child_process"),p=require("node:path"); const r=spawnSync(process.execPath,["scripts/build-frontend.mjs","--outDir",process.env.OSCMIDI_TEST_OUTPUT+p.sep,"--mode","production"],{stdio:"inherit"}); process.exit(r.status??1);'
    if ($LASTEXITCODE -ne 0) { throw 'Frontend argument forwarding failed.' }
    Assert-True (Test-Path -LiteralPath (Join-Path $env:OSCMIDI_TEST_OUTPUT 'index.html')) 'Frontend output arguments must preserve paths with spaces and trailing separators.'
    Write-Host 'PASS: publication gating, immutable assets/tags, lookup failure, and source-keyed cache behavior.'
}
finally {
    foreach ($name in $environmentNames) { [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process') }
    Set-Location $previousLocation
    $resolved = [IO.Path]::GetFullPath($fixture)
    if (-not $resolved.StartsWith([IO.Path]::GetTempPath(), [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe test cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
